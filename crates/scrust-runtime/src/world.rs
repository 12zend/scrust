//! `World`: all mutable game state, plus the static program description
//! emitted by the code generator.

use crate::sched::{BoxFut, HatKind, Sched, Status, Wake};
use scrust_value::{JsStr, Value};
use std::any::Any;
use std::cell::Cell;
use std::rc::Rc;

/// Default stage size; a project's TurboWarp config may change it
/// (`stage_width()` / `stage_height()`).
pub const STAGE_WIDTH: f64 = 480.0;
pub const STAGE_HEIGHT: f64 = 360.0;

static STAGE_SIZE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new((480u64 << 32) | 360);

/// The running program's stage size (TurboWarp's `runtime.stageWidth`).
#[inline]
pub fn stage_width() -> f64 {
    (STAGE_SIZE.load(std::sync::atomic::Ordering::Relaxed) >> 32) as f64
}
#[inline]
pub fn stage_height() -> f64 {
    (STAGE_SIZE.load(std::sync::atomic::Ordering::Relaxed) & 0xFFFF_FFFF) as f64
}
pub fn set_stage_size(width: f64, height: f64) {
    let (w, h) = (width.round().clamp(1.0, 16384.0) as u64, height.round().clamp(1.0, 16384.0) as u64);
    STAGE_SIZE.store((w << 32) | h, std::sync::atomic::Ordering::Relaxed);
}

thread_local! {
    static WORLD: Cell<*mut World> = const { Cell::new(std::ptr::null_mut()) };
}

#[inline]
pub fn world_ptr() -> *mut World {
    WORLD.with(|w| w.get())
}

pub fn set_world_ptr(p: *mut World) {
    WORLD.with(|w| w.set(p));
}

#[cfg(debug_assertions)]
thread_local! {
    static WORLD_BORROWED: Cell<bool> = const { Cell::new(false) };
}

/// Debug-build check used by the scheduler's suspension points: a live
/// `WorldRef` here means generated code kept the world borrowed across `.await`.
#[inline]
pub fn assert_world_released() {
    #[cfg(debug_assertions)]
    WORLD_BORROWED.with(|b| assert!(!b.get(), "scrust: the world was still borrowed at a suspension point (held across .await)"));
}

/// Guard returned by [`world`]: a `&mut World` with RefCell-style checking in
/// debug builds and no cost in release builds (PRODUCT.md 10.6 `WorldCell`).
pub struct WorldRef {
    p: *mut World,
}

impl std::ops::Deref for WorldRef {
    type Target = World;
    #[inline(always)]
    fn deref(&self) -> &World {
        // SAFETY: see `world`.
        unsafe { &*self.p }
    }
}
impl std::ops::DerefMut for WorldRef {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut World {
        // SAFETY: see `world`.
        unsafe { &mut *self.p }
    }
}
#[cfg(debug_assertions)]
impl Drop for WorldRef {
    fn drop(&mut self) {
        WORLD_BORROWED.with(|b| b.set(false));
    }
}

/// Access the world from generated `async` code.
///
/// Invariants upheld by the code generator: the guard is never held across
/// an `.await`, and synchronous code receives `&mut World` as an argument
/// instead of calling this again. Debug builds panic on a violation.
#[inline]
pub fn world() -> WorldRef {
    #[cfg(debug_assertions)]
    WORLD_BORROWED.with(|b| {
        assert!(!b.get(), "scrust: world() called while the world is already borrowed");
        b.set(true);
    });
    // SAFETY (for the derefs): only called from thread futures, which are
    // polled exclusively by `sched::step` after it published the pointer and
    // released its own borrows.
    WorldRef { p: world_ptr() }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TargetRef {
    pub idx: u32,
    pub gen: u32,
}

/// Generated per-sprite variable storage.
pub trait Locals: Any {
    fn clone_box(&self) -> Box<dyn Locals>;
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    /// Scalar variable by name (for `(variable) of (sprite)` with dynamic operands).
    fn var_by_name(&self, name: &str) -> Option<Value>;
    /// All scalar variables as (name, raw value), in declaration order.
    fn vars(&self) -> Vec<(&'static str, Value)>;
    /// All lists as (name, items), in declaration order.
    fn lists(&self) -> Vec<(&'static str, Vec<Value>)>;
    /// Store into a variable by declaration index (monitor sliders).
    fn set_var(&mut self, index: usize, value: Value);
}

/// A cached pointer to a target's variable storage, used by generated code
/// to avoid re-resolving the storage on every variable access.
///
/// Invariants the code generator relies on:
/// * the storage of a target is a separate heap allocation that never moves
///   and outlives every thread running on that target (slots of deleted
///   clones are only recycled once no thread references them);
/// * the runtime never touches variable storage while generated code holds a
///   reference obtained from `r()` / `m()` (those references are temporaries
///   that end before the next runtime call that could reach the storage);
/// * `m()` references are never alive at the same time as another reference
///   to the same storage.
pub struct LocalsPtr<T>(*mut T);
impl<T> Clone for LocalsPtr<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for LocalsPtr<T> {}
impl<T> LocalsPtr<T> {
    /// Shared access.
    #[inline(always)]
    #[allow(clippy::should_implement_trait)]
    pub fn r(&self) -> &T {
        // SAFETY: see the type-level invariants.
        unsafe { &*self.0 }
    }
    /// Exclusive access.
    #[inline(always)]
    #[allow(clippy::mut_from_ref)]
    pub fn m(&self) -> &mut T {
        // SAFETY: see the type-level invariants.
        unsafe { &mut *self.0 }
    }
}

/// Owner of a target's variable storage.
///
/// The storage is held through a raw pointer (not a `Box`) so that every
/// access - by the runtime and through cached `LocalsPtr`s - derives from the
/// same pointer; re-borrowing a `Box` would invalidate the cached pointers
/// under Rust's aliasing rules (checked with Miri).
pub struct LocalsBox {
    p: *mut dyn Locals,
}

impl LocalsBox {
    pub fn new(b: Box<dyn Locals>) -> LocalsBox {
        LocalsBox { p: Box::into_raw(b) }
    }
    /// The storage as a pointer to its concrete type.
    #[inline(always)]
    pub fn raw<T>(&self) -> *mut T {
        self.p as *mut T
    }
}

impl std::ops::Deref for LocalsBox {
    type Target = dyn Locals;
    #[inline]
    fn deref(&self) -> &(dyn Locals + 'static) {
        // SAFETY: `p` comes from `Box::into_raw` and is freed only in `drop`.
        unsafe { &*self.p }
    }
}

impl std::ops::DerefMut for LocalsBox {
    #[inline]
    fn deref_mut(&mut self) -> &mut (dyn Locals + 'static) {
        // SAFETY: as in `deref`.
        unsafe { &mut *self.p }
    }
}

impl Drop for LocalsBox {
    fn drop(&mut self) {
        // SAFETY: `p` comes from `Box::into_raw` and is dropped exactly once.
        unsafe { drop(Box::from_raw(self.p)) };
    }
}

#[derive(Clone, Copy)]
pub struct CostumeDef {
    pub name: &'static str,
    pub md5ext: &'static str,
    pub data_format: &'static str,
    pub data: &'static [u8],
    pub bitmap_resolution: f64,
    pub center_x: f64,
    pub center_y: f64,
}

#[derive(Clone, Copy)]
pub struct SoundDef {
    pub name: &'static str,
    pub md5ext: &'static str,
    pub data_format: &'static str,
    pub data: &'static [u8],
    /// Sample rate and sample count recorded in project.json.
    pub rate: f64,
    pub sample_count: f64,
}

/// Starts a thread of a script.
pub type StartFn = Box<dyn Fn(TargetRef) -> BoxFut + Send + Sync>;
/// Predicate of an edge-activated hat.
pub type PredFn = Box<dyn Fn(&mut World, TargetRef) -> bool + Send + Sync>;
/// Creates a target's variable storage.
pub type NewLocalsFn = Box<dyn Fn() -> Box<dyn Locals> + Send + Sync>;

/// The entry points are boxed closures rather than function pointers so that
/// a program can also be built at run time (`scrust-interp`).
pub struct ScriptDef {
    pub kind: HatKind,
    /// Upper-cased match field (key name, backdrop name).
    pub field: &'static str,
    /// Message index for broadcast hats.
    pub msg: usize,
    pub restart: bool,
    pub start: StartFn,
    /// Predicate of edge-activated hats.
    pub pred: Option<PredFn>,
    /// Undeclared variables (`false`) and lists (`true`) by index that
    /// TurboWarp creates when it compiles this script (its first start).
    pub creates: &'static [(bool, u16)],
}

pub struct SpriteDef {
    pub name: &'static str,
    pub is_stage: bool,
    pub costumes: Vec<CostumeDef>,
    pub sounds: Vec<SoundDef>,
    pub scripts: Vec<ScriptDef>,
    pub new_locals: NewLocalsFn,
    pub x: f64,
    pub y: f64,
    pub size: f64,
    pub direction: f64,
    pub visible: bool,
    pub draggable: bool,
    pub costume: usize,
    pub rotation_style: RotationStyle,
    pub volume: f64,
    /// Variables / lists declared in the project; the rest are created on
    /// demand (see `ScriptDef::creates`).
    pub declared_vars: usize,
    pub declared_lists: usize,
    /// Music extension tempo (stage only).
    pub tempo: f64,
}

#[derive(Clone, Debug)]
pub struct MonitorDef {
    pub id: &'static str,
    pub opcode: &'static str,
    pub mode: &'static str,
    /// Label shown next to the value.
    pub label: &'static str,
    /// Sprite index owning the monitored value (0 = stage / global).
    pub sprite: usize,
    /// Variable / list declaration index for data monitors.
    pub index: usize,
    /// Extra parameter (e.g. NUMBER_NAME, CURRENTMENU).
    pub param: &'static str,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub visible: bool,
    pub slider_min: f64,
    pub slider_max: f64,
    pub is_discrete: bool,
}

pub struct ProgramDef {
    pub sprites: Vec<SpriteDef>,
    /// Upper-cased broadcast names.
    pub messages: Vec<&'static str>,
    /// (lower-cased broadcast variable name, message index) in lookup order.
    pub broadcast_lookup: Vec<(&'static str, usize)>,
    pub monitors: Vec<MonitorDef>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RotationStyle {
    AllAround,
    LeftRight,
    DontRotate,
}

pub const EFFECT_NAMES: [&str; 7] = ["color", "fisheye", "whirl", "pixelate", "mosaic", "brightness", "ghost"];

#[derive(Clone, Debug)]
pub struct Bubble {
    pub think: bool,
    pub text: JsStr,
    pub usage: u64,
}

#[derive(Clone, Debug)]
pub struct PenState {
    pub down: bool,
    pub color: f64,
    pub saturation: f64,
    pub brightness: f64,
    pub transparency: f64,
    pub legacy_shade: f64,
    pub diameter: f64,
    pub rgba: [f32; 4],
}

impl Default for PenState {
    fn default() -> Self {
        PenState {
            down: false,
            color: 66.66,
            saturation: 100.0,
            brightness: 100.0,
            transparency: 0.0,
            legacy_shade: 50.0,
            diameter: 1.0,
            rgba: [0.0, 0.0, 1.0, 1.0],
        }
    }
}

pub struct Target {
    pub gen: u32,
    pub alive: bool,
    pub sprite: usize,
    pub is_original: bool,
    pub is_stage: bool,
    pub locals: LocalsBox,
    pub x: f64,
    pub y: f64,
    pub direction: f64,
    pub size: f64,
    pub visible: bool,
    pub draggable: bool,
    pub dragging: bool,
    pub costume: usize,
    pub rotation_style: RotationStyle,
    pub effects: [f64; 7],
    pub volume: f64,
    pub pitch: f64,
    pub pan: f64,
    /// Last known values of edge-activated hats, by script index.
    pub edge_values: Vec<Option<bool>>,
    pub bubble: Option<Bubble>,
    pub pen: PenState,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Bounds {
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub bottom: f64,
}

/// Rendering backend hooks (the equivalent of scratch-vm's `this.renderer`).
pub trait Renderer {
    fn costume_size(&mut self, sprite: usize, costume: usize) -> (f64, f64);
    fn bounds(&mut self, w: &World, t: TargetRef) -> Option<Bounds>;
    fn fenced_position(&mut self, w: &World, t: TargetRef, x: f64, y: f64) -> (f64, f64);
    fn touching_point(&mut self, w: &World, t: TargetRef, x: f64, y: f64) -> bool;
    fn touching_targets(&mut self, w: &World, t: TargetRef, others: &[TargetRef]) -> bool;
    fn touching_color(&mut self, w: &World, t: TargetRef, color: [u8; 3], mask: Option<[u8; 3]>) -> bool;
    fn pick(&mut self, w: &World, x: f64, y: f64) -> Option<TargetRef>;
    fn pen_clear(&mut self);
    fn pen_line(&mut self, pen: &PenState, x0: f64, y0: f64, x1: f64, y1: f64);
    fn pen_stamp(&mut self, w: &World, t: TargetRef);
    /// End of a frame: scratch-render draws the stage, which creates the MIPs
    /// (and so the silhouettes) visible drawables are drawn with, whether or
    /// not the frame is shown.
    fn frame_drawn(&mut self, _w: &World) {}
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// Audio backend hooks.
pub trait Audio {
    /// Start a sound; the returned flag flips to true when it finishes.
    fn play(&mut self, sprite: usize, sound: usize, target: TargetRef, volume: f64, pitch: f64, pan: f64) -> Option<Rc<Cell<bool>>>;
    fn stop_all(&mut self);
    fn stop_target(&mut self, target: TargetRef);
    fn update_target(&mut self, target: TargetRef, volume: f64, pitch: f64, pan: f64);
    fn loudness(&mut self) -> f64;
    /// Called once per frame with the clock (ms): update the completion flags of finished sounds.
    fn tick(&mut self, _now_ms: f64) {}
}

#[derive(Clone, Debug)]
pub struct Options {
    pub turbo: bool,
    /// Deterministic mode: virtual clock + this many passes per frame at most.
    pub deterministic: bool,
    pub max_passes: Option<u32>,
    pub seed: u32,
    pub fps: f64,
    pub warp_timer: bool,
    /// Scripted answers for `ask and wait` (headless runs).
    pub answers: Vec<String>,
    /// TurboWarp runtime options (project config comment).
    pub fencing: bool,
    pub misc_limits: bool,
    pub max_clones: f64,
    /// High quality pen (no pen size clamp).
    pub hq: bool,
    /// Stage size (TurboWarp config `width` / `height`).
    pub stage_width: f64,
    pub stage_height: f64,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            turbo: false,
            deterministic: false,
            max_passes: None,
            seed: 1,
            fps: 30.0,
            warp_timer: false,
            answers: Vec::new(),
            fencing: true,
            misc_limits: true,
            max_clones: 300.0,
            hq: false,
            stage_width: 480.0,
            stage_height: 360.0,
        }
    }
}

/// mulberry32, mirrored bit-for-bit by the conformance harness.
pub struct Rng(pub u32, pub u64);
impl Rng {
    pub fn next(&mut self) -> f64 {
        self.1 += 1;
        self.0 = self.0.wrapping_add(0x6D2B79F5);
        let a = self.0;
        let mut t = (a ^ (a >> 15)).wrapping_mul(1 | a);
        t = (t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t))) ^ t;
        ((t ^ (t >> 14)) as f64) / 4294967296.0
    }
}

pub struct Question {
    pub text: JsStr,
    pub target: TargetRef,
    pub was_visible: bool,
    pub was_stage: bool,
    pub done: Rc<Cell<bool>>,
}

#[derive(Default)]
pub struct Io {
    /// Scratch key names currently down.
    pub keys: Vec<String>,
    pub mouse_x: f64,
    pub mouse_y: f64,
    pub mouse_down: bool,
    /// Sprite being dragged: (target, grab offset x, grab offset y, moved).
    pub drag: Option<(TargetRef, f64, f64, bool)>,
}

pub struct World {
    pub program: &'static ProgramDef,
    pub options: Options,
    pub targets: Vec<Target>,
    free_slots: Vec<u32>,
    disposed: Vec<u32>,
    /// Execution / layer order: `[0]` is the stage, last is frontmost.
    pub order: Vec<TargetRef>,
    /// `runtime.targets` order (creation order).
    pub seq: Vec<TargetRef>,
    /// Original (non-clone) target of each sprite.
    pub originals: Vec<TargetRef>,
    pub clone_count: usize,
    pub sched: Sched,
    pub frame: u64,
    pub step_time: f64,
    pub current_msecs: f64,
    pub timer_start: f64,
    virtual_ms: f64,
    /// Host clock in ms (monotonic); unused in deterministic mode.
    pub host_clock: Option<Box<dyn Fn() -> f64>>,
    /// Host wall-clock: ms since the Unix epoch.
    pub host_epoch_ms: Option<Box<dyn Fn() -> f64>>,
    /// Minutes to add to UTC to get local time.
    pub tz_offset_min: f64,
    pub rng: Rng,
    pub answer: JsStr,
    pub questions: Vec<Question>,
    pub io: Io,
    pub counter: f64,
    pub renderer: Option<Box<dyn Renderer>>,
    pub audio: Option<Box<dyn Audio>>,
    pub monitor_visible: Vec<bool>,
    /// Per sprite: undeclared variables / lists created so far, in order.
    pub created: Vec<Vec<(bool, u16)>>,
    /// Music extension tempo (beats per minute), from the stage.
    pub tempo: f64,
    pub username: JsStr,
    bubble_uid: u64,
    answers_used: usize,
}

/// Fixed epoch of the deterministic clock: 2020-01-01T00:00:00Z.
pub const DETERMINISTIC_EPOCH_MS: f64 = 1577836800000.0;

impl World {
    pub fn new(program: &'static ProgramDef, options: Options) -> Box<World> {
        set_stage_size(options.stage_width, options.stage_height);
        let step_time = 1000.0 / options.fps;
        let mut w = Box::new(World {
            program,
            rng: Rng(options.seed, 0),
            options,
            targets: Vec::new(),
            free_slots: Vec::new(),
            disposed: Vec::new(),
            order: Vec::new(),
            seq: Vec::new(),
            originals: Vec::new(),
            clone_count: 0,
            sched: Sched::default(),
            frame: 0,
            step_time,
            current_msecs: 0.0,
            timer_start: 0.0,
            virtual_ms: DETERMINISTIC_EPOCH_MS,
            host_clock: None,
            host_epoch_ms: None,
            tz_offset_min: 0.0,
            answer: JsStr::empty(),
            questions: Vec::new(),
            io: Io::default(),
            counter: 0.0,
            renderer: None,
            audio: None,
            monitor_visible: program.monitors.iter().map(|m| m.visible).collect(),
            created: vec![Vec::new(); program.sprites.len()],
            tempo: program.sprites.first().map(|s| s.tempo).unwrap_or(60.0),
            username: JsStr::empty(),
            bubble_uid: 0,
            answers_used: 0,
        });
        for (si, s) in program.sprites.iter().enumerate() {
            let t = Target {
                gen: 0,
                alive: true,
                sprite: si,
                is_original: true,
                is_stage: s.is_stage,
                locals: LocalsBox::new((s.new_locals)()),
                x: s.x,
                y: s.y,
                direction: s.direction,
                size: s.size,
                visible: s.visible,
                draggable: s.draggable,
                dragging: false,
                costume: s.costume,
                rotation_style: s.rotation_style,
                effects: [0.0; 7],
                volume: s.volume,
                pitch: 0.0,
                pan: 0.0,
                edge_values: Vec::new(),
                bubble: None,
                pen: PenState::default(),
            };
            let r = TargetRef { idx: si as u32, gen: 0 };
            w.targets.push(t);
            w.order.push(r);
            w.seq.push(r);
            w.originals.push(r);
        }
        w.current_msecs = w.clock_ms();
        w.timer_start = w.current_msecs;
        set_world_ptr(&mut *w);
        w
    }

    // --- clock -----------------------------------------------------------------

    /// `Date.now()`.
    pub fn clock_ms(&self) -> f64 {
        if self.options.deterministic {
            return self.virtual_ms;
        }
        match &self.host_epoch_ms {
            Some(f) => f(),
            None => self.virtual_ms,
        }
    }

    /// Monotonic milliseconds (work-time budget, warp timer).
    pub fn real_ms(&self) -> f64 {
        if self.options.deterministic {
            return self.virtual_ms;
        }
        match &self.host_clock {
            Some(f) => f(),
            None => self.virtual_ms,
        }
    }

    pub(crate) fn advance_clock(&mut self) {
        if self.options.deterministic || self.host_clock.is_none() {
            self.virtual_ms += self.step_time;
        }
    }

    #[inline]
    pub fn random(&mut self) -> f64 {
        self.rng.next()
    }

    // --- targets ---------------------------------------------------------------

    #[inline]
    pub fn target(&self, t: TargetRef) -> &Target {
        &self.targets[t.idx as usize]
    }

    #[inline]
    pub fn target_mut(&mut self, t: TargetRef) -> &mut Target {
        &mut self.targets[t.idx as usize]
    }

    pub fn target_checked(&self, t: TargetRef) -> Option<&Target> {
        self.targets.get(t.idx as usize).filter(|x| x.gen == t.gen)
    }

    #[inline]
    pub fn stage_ref(&self) -> TargetRef {
        self.originals[0]
    }

    /// Typed access to a target's generated variable storage.
    #[inline]
    pub fn locals<T: 'static>(&mut self, t: TargetRef) -> &mut T {
        let l = &self.targets[t.idx as usize].locals;
        debug_assert!(l.as_any().is::<T>(), "locals type mismatch");
        // SAFETY: the generator only requests the storage type it created for
        // the sprite this target belongs to (checked in debug builds).
        unsafe { &mut *l.raw::<T>() }
    }

    /// Typed access to the stage's (global) variable storage.
    #[inline]
    pub fn globals<T: 'static>(&mut self) -> &mut T {
        let l = &self.targets[0].locals;
        debug_assert!(l.as_any().is::<T>(), "globals type mismatch");
        // SAFETY: as in `locals`.
        unsafe { &mut *l.raw::<T>() }
    }

    /// Cached pointer to a target's variable storage (see `LocalsPtr`).
    #[inline]
    pub fn locals_ptr<T: 'static>(&mut self, t: TargetRef) -> LocalsPtr<T> {
        let l = &self.targets[t.idx as usize].locals;
        debug_assert!(l.as_any().is::<T>(), "locals type mismatch");
        LocalsPtr(l.raw::<T>())
    }

    /// Cached pointer to the stage's variable storage.
    #[inline]
    pub fn globals_ptr<T: 'static>(&mut self) -> LocalsPtr<T> {
        let l = &self.targets[0].locals;
        debug_assert!(l.as_any().is::<T>(), "globals type mismatch");
        LocalsPtr(l.raw::<T>())
    }

    /// Variable storage of the original instance of sprite `sprite`.
    #[inline]
    pub fn locals_of<T: 'static>(&mut self, sprite: usize) -> &mut T {
        let t = self.originals[sprite];
        self.locals::<T>(t)
    }

    /// `runtime.getSpriteTargetByName`.
    pub fn sprite_by_name(&self, name: &JsStr) -> Option<TargetRef> {
        self.seq.iter().copied().find(|t| {
            let tg = self.target(*t);
            !tg.is_stage && name.eq_str(self.program.sprites[tg.sprite].name)
        })
    }

    pub(crate) fn request_redraw(&mut self) {
        self.sched.redraw_requested = true;
    }

    /// Redraw request made by a visible change of a target (renderer only).
    pub(crate) fn visual_change(&mut self, t: TargetRef) {
        if self.renderer.is_some() && self.target(t).visible {
            self.request_redraw();
        }
    }

    /// Call into the renderer with shared access to the world.
    pub(crate) fn with_renderer<R>(&mut self, f: impl FnOnce(&mut dyn Renderer, &World) -> R) -> Option<R> {
        let mut r = self.renderer.take()?;
        let out = f(r.as_mut(), self);
        self.renderer = Some(r);
        Some(out)
    }

    // --- clones ----------------------------------------------------------------

    /// `control_create_clone_of`.
    pub fn create_clone(&mut self, me: TargetRef, option: JsStr) {
        let source = if option.eq_str("_myself_") { Some(me) } else { self.sprite_by_name(&option) };
        let Some(src) = source else { return };
        // `clonesAvailable`: `_cloneCounter < runtimeOptions.maxClones`.
        if !((self.clone_count as f64) < self.options.max_clones) || self.target(src).is_stage {
            return;
        }
        self.clone_count += 1;
        let s = self.target(src);
        let mut t = Target {
            gen: 0,
            alive: true,
            sprite: s.sprite,
            is_original: false,
            is_stage: false,
            locals: LocalsBox::new(s.locals.clone_box()),
            x: s.x,
            y: s.y,
            direction: s.direction,
            size: s.size,
            visible: s.visible,
            draggable: s.draggable,
            dragging: false,
            costume: s.costume,
            rotation_style: s.rotation_style,
            effects: s.effects,
            volume: s.volume,
            pitch: 0.0,
            pan: 0.0,
            edge_values: s.edge_values.clone(),
            bubble: None,
            pen: s.pen.clone(),
        };
        let idx = match self.free_slots.pop() {
            Some(i) => {
                t.gen = self.targets[i as usize].gen.wrapping_add(1);
                self.targets[i as usize] = t;
                i
            }
            None => {
                self.targets.push(t);
                (self.targets.len() - 1) as u32
            }
        };
        let r = TargetRef { idx, gen: self.targets[idx as usize].gen };
        // `targetWasCreated` fires before the clone joins the target lists.
        self.start_hats(HatKind::Clone, None, 0, Some(r));
        self.seq.push(r);
        // addTarget + goBehindOther(source)
        let pos = self.order.iter().position(|x| *x == src).unwrap_or(self.order.len());
        self.order.insert(pos.max(1), r);
        if self.renderer.is_some() && self.target(r).visible {
            self.request_redraw();
        }
    }

    /// `control_delete_this_clone`. Returns true when the calling thread died.
    pub fn delete_this_clone(&mut self, me: TargetRef) -> bool {
        if self.target(me).is_original {
            return false;
        }
        self.dispose_target(me);
        self.stop_for_target(me, false);
        true
    }

    pub(crate) fn dispose_target(&mut self, t: TargetRef) {
        let tg = &mut self.targets[t.idx as usize];
        if !tg.alive || tg.gen != t.gen {
            return;
        }
        tg.alive = false;
        tg.edge_values.clear();
        let was_visible = tg.visible;
        self.clone_count = self.clone_count.saturating_sub(1);
        self.stop_for_target(t, false);
        self.order.retain(|x| *x != t);
        self.seq.retain(|x| *x != t);
        self.clear_target_questions(t);
        if let Some(a) = self.audio.as_mut() {
            a.stop_target(t);
        }
        if self.renderer.is_some() && was_visible {
            self.request_redraw();
        }
        self.disposed.push(t.idx);
    }

    /// Recycle slots of disposed clones once no thread can reference them.
    pub(crate) fn free_disposed(&mut self) {
        if self.disposed.is_empty() {
            return;
        }
        let threads = &self.sched.threads;
        let targets = &self.targets;
        let mut keep = Vec::new();
        for idx in std::mem::take(&mut self.disposed) {
            let gen = targets[idx as usize].gen;
            if threads.iter().any(|th| th.target.idx == idx && th.target.gen == gen) {
                keep.push(idx);
            } else {
                self.free_slots.push(idx);
            }
        }
        self.disposed = keep;
    }

    pub(crate) fn on_stop_all(&mut self) {
        // PROJECT_STOP_ALL listeners: sensing clears questions, sound stops.
        for q in self.questions.drain(..) {
            let _ = q;
        }
        if let Some(a) = self.audio.as_mut() {
            a.stop_all();
        }
        let all: Vec<TargetRef> = self.seq.clone();
        for t in all {
            // looks: PROJECT_STOP_ALL clears every bubble.
            if self.target(t).bubble.is_some() {
                self.target_mut(t).bubble = None;
                self.request_redraw_if_renderer();
            }
            let tg = self.target_mut(t);
            tg.pitch = 0.0;
            tg.pan = 0.0;
        }
    }

    pub(crate) fn request_redraw_if_renderer(&mut self) {
        if self.renderer.is_some() {
            self.request_redraw();
        }
    }

    // --- promises ----------------------------------------------------------------

    pub(crate) fn resolve_promises(&mut self) {
        let now_ms = self.clock_ms();
        if let Some(a) = self.audio.as_mut() {
            a.tick(now_ms);
        }
        self.auto_answer();
        let now = self.clock_ms();
        let mut clear: Vec<(TargetRef, u64)> = Vec::new();
        for th in self.sched.threads.iter_mut() {
            if th.status != Status::PromiseWait {
                continue;
            }
            let ready = match &th.wake {
                Some(Wake::Time(t)) => now >= *t,
                Some(Wake::Flag(f)) => f.get(),
                Some(Wake::Bubble { at, target, usage }) => {
                    if now >= *at {
                        clear.push((*target, *usage));
                        true
                    } else {
                        false
                    }
                }
                None => true,
            };
            if ready {
                th.status = Status::Running;
                th.wake = None;
            }
        }
        for (t, usage) in clear {
            if self.target_checked(t).and_then(|tg| tg.bubble.as_ref()).map(|b| b.usage) == Some(usage) {
                let think = self.target(t).bubble.as_ref().map(|b| b.think).unwrap_or(false);
                self.update_bubble(t, think, JsStr::empty());
            }
        }
    }

    pub(crate) fn next_bubble_uid(&mut self) -> u64 {
        self.bubble_uid += 1;
        self.bubble_uid
    }

    fn auto_answer(&mut self) {
        if self.questions.is_empty() || self.answers_used >= self.options.answers.len() {
            return;
        }
        let a = self.options.answers[self.answers_used].clone();
        self.answers_used += 1;
        self.submit_answer(JsStr::from(a.as_str()));
    }
}
