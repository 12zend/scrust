//! Browser host. The generated program is a `cdylib`; the page (see
//! `web/index.html`) drives it through these exports and provides audio
//! through a few imports. No wasm-bindgen is involved.

use crate::input::key_event;
use scrust_render::SoftRenderer;
use scrust_runtime::{set_world_ptr, Audio, Options, ProgramDef, TargetRef, World};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

extern "C" {
    fn js_sound_play(id: u32, sprite: u32, sound: u32, volume: f64, rate: f64, pan: f64);
    fn js_sound_stop(id: u32);
    fn js_sound_set(id: u32, volume: f64, rate: f64, pan: f64);
}

struct State {
    world: Box<World>,
    rgba: Vec<u8>,
    /// Host time (ms) at which the next tick is due.
    next_tick: f64,
    epoch_offset: f64,
    now: Rc<Cell<f64>>,
    scratch: Vec<u8>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    static SOUNDS: RefCell<Vec<(u32, TargetRef, usize, Rc<Cell<bool>>)>> = const { RefCell::new(Vec::new()) };
    static NEXT_SOUND: Cell<u32> = const { Cell::new(1) };
    /// The program, readable without borrowing `STATE`: the page fetches
    /// sound data from inside `js_sound_play`, i.e. during `scrust_frame`.
    static PROGRAM: Cell<Option<&'static ProgramDef>> = const { Cell::new(None) };
}

struct WebAudio;

impl Audio for WebAudio {
    fn play(&mut self, sprite: usize, sound: usize, target: TargetRef, volume: f64, pitch: f64, pan: f64) -> Option<Rc<Cell<bool>>> {
        // Restart the same sound on the same target.
        SOUNDS.with(|s| {
            s.borrow_mut().retain(|(id, t, snd, done)| {
                if *t == target && *snd == sound {
                    // SAFETY: plain FFI call into the page's glue code.
                    unsafe { js_sound_stop(*id) };
                    done.set(true);
                    false
                } else {
                    true
                }
            })
        });
        let id = NEXT_SOUND.with(|n| {
            let v = n.get();
            n.set(v.wrapping_add(1).max(1));
            v
        });
        let done = Rc::new(Cell::new(false));
        SOUNDS.with(|s| s.borrow_mut().push((id, target, sound, done.clone())));
        // SAFETY: plain FFI call into the page's glue code.
        unsafe { js_sound_play(id, sprite as u32, sound as u32, volume / 100.0, 2f64.powf(pitch / 120.0), pan / 100.0) };
        Some(done)
    }

    fn stop_all(&mut self) {
        SOUNDS.with(|s| {
            for (id, _, _, done) in s.borrow_mut().drain(..) {
                // SAFETY: plain FFI call into the page's glue code.
                unsafe { js_sound_stop(id) };
                done.set(true);
            }
        });
    }

    fn stop_target(&mut self, target: TargetRef) {
        SOUNDS.with(|s| {
            s.borrow_mut().retain(|(id, t, _, done)| {
                if *t == target {
                    // SAFETY: plain FFI call into the page's glue code.
                    unsafe { js_sound_stop(*id) };
                    done.set(true);
                    false
                } else {
                    true
                }
            })
        });
    }

    fn update_target(&mut self, target: TargetRef, volume: f64, pitch: f64, pan: f64) {
        SOUNDS.with(|s| {
            for (id, t, _, _) in s.borrow().iter() {
                if *t == target {
                    // SAFETY: plain FFI call into the page's glue code.
                    unsafe { js_sound_set(*id, volume / 100.0, 2f64.powf(pitch / 120.0), pan / 100.0) };
                }
            }
        });
    }

    fn loudness(&mut self) -> f64 {
        -1.0
    }
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|s| {
        let mut guard = s.borrow_mut();
        let st = guard.as_mut()?;
        set_world_ptr(&mut *st.world);
        Some(f(st))
    })
}

pub fn init(program: ProgramDef, mut options: Options, scale: f64, seed: u32, epoch_ms: f64, tz_offset_min: f64) {
    let program: &'static ProgramDef = Box::leak(Box::new(program));
    PROGRAM.with(|p| p.set(Some(program)));
    options.seed = seed;
    let mut world = World::new(program, options);
    let now = Rc::new(Cell::new(0.0));
    let (n1, n2) = (now.clone(), now.clone());
    world.host_clock = Some(Box::new(move || n1.get()));
    world.host_epoch_ms = Some(Box::new(move || n2.get() + epoch_ms));
    world.tz_offset_min = tz_offset_min;
    world.current_msecs = world.clock_ms();
    world.timer_start = world.current_msecs;
    let renderer = SoftRenderer::new(program, scale.max(0.25));
    let size = renderer.width * renderer.height * 4;
    world.renderer = Some(Box::new(renderer));
    world.audio = Some(Box::new(WebAudio));
    set_world_ptr(&mut *world);
    world.green_flag();
    STATE.with(|s| *s.borrow_mut() = Some(State { world, rgba: vec![255; size], next_tick: 0.0, epoch_offset: epoch_ms, now, scratch: Vec::new() }));
}

/// Advance to host time `now_ms` (stepping as many ticks as are due, at most
/// a few) and redraw. Returns 1 when a new frame was drawn.
#[no_mangle]
pub extern "C" fn scrust_frame(now_ms: f64) -> u32 {
    with(|st| {
        st.now.set(now_ms);
        let _ = st.epoch_offset;
        let step = st.world.step_time;
        let mut stepped = 0;
        while now_ms >= st.next_tick && stepped < 3 {
            let wp: *mut World = &mut *st.world;
            // SAFETY: the world is only accessed through `wp` during the step.
            unsafe { scrust_runtime::step(wp) };
            st.next_tick += step;
            stepped += 1;
        }
        if st.next_tick < now_ms {
            st.next_tick = now_ms + step;
        }
        if stepped == 0 {
            return 0;
        }
        let Some(mut r) = st.world.renderer.take() else { return 0 };
        if let Some(sr) = r.as_any_mut().downcast_mut::<SoftRenderer>() {
            sr.draw(&st.world);
            for (o, p) in st.rgba.chunks_exact_mut(4).zip(sr.frame.iter()) {
                o[0] = (p >> 16) as u8;
                o[1] = (p >> 8) as u8;
                o[2] = *p as u8;
                o[3] = 255;
            }
        }
        st.world.renderer = Some(r);
        1
    })
    .unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn scrust_frame_ptr() -> *const u8 {
    with(|st| st.rgba.as_ptr()).unwrap_or(std::ptr::null())
}

fn renderer_dim(f: impl Fn(&SoftRenderer) -> usize) -> u32 {
    with(|st| st.world.renderer.as_mut().and_then(|r| r.as_any_mut().downcast_mut::<SoftRenderer>()).map(|r| f(r) as u32).unwrap_or(0)).unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn scrust_width() -> u32 {
    renderer_dim(|r| r.width)
}

#[no_mangle]
pub extern "C" fn scrust_height() -> u32 {
    renderer_dim(|r| r.height)
}

/// Scratch buffer for passing strings from the page.
#[no_mangle]
pub extern "C" fn scrust_buffer(len: u32) -> *mut u8 {
    with(|st| {
        st.scratch.resize(len as usize, 0);
        st.scratch.as_mut_ptr()
    })
    .unwrap_or(std::ptr::null_mut())
}

/// Key event; the key name (`KeyboardEvent.key`, UTF-8) is in the scratch buffer.
#[no_mangle]
pub extern "C" fn scrust_key(len: u32, down: u32) {
    with(|st| {
        let key = String::from_utf8_lossy(&st.scratch[..(len as usize).min(st.scratch.len())]).to_string();
        let text = if key.chars().count() == 1 { Some(key.clone()) } else { None };
        key_event(&mut st.world, &key, text.as_deref(), down != 0);
    });
}

#[no_mangle]
pub extern "C" fn scrust_mouse_move(x: f64, y: f64) {
    with(|st| st.world.post_mouse_move(x, y));
}

#[no_mangle]
pub extern "C" fn scrust_mouse_button(x: f64, y: f64, down: u32) {
    with(|st| st.world.post_mouse_button(x, y, down != 0));
}

#[no_mangle]
pub extern "C" fn scrust_green_flag() {
    with(|st| st.world.green_flag());
}

#[no_mangle]
pub extern "C" fn scrust_stop() {
    with(|st| st.world.stop_all());
}

/// Encoded bytes of a sound asset (decoded by the page with Web Audio).
#[no_mangle]
pub extern "C" fn scrust_sound_ptr(sprite: u32, sound: u32) -> *const u8 {
    sound_data(sprite, sound).map(|d| d.as_ptr()).unwrap_or(std::ptr::null())
}

#[no_mangle]
pub extern "C" fn scrust_sound_len(sprite: u32, sound: u32) -> u32 {
    sound_data(sprite, sound).map(|d| d.len() as u32).unwrap_or(0)
}

fn sound_data(sprite: u32, sound: u32) -> Option<&'static [u8]> {
    let program = PROGRAM.with(|p| p.get())?;
    Some(program.sprites.get(sprite as usize)?.sounds.get(sound as usize)?.data)
}

/// The page reports that a sound finished playing.
#[no_mangle]
pub extern "C" fn scrust_sound_done(id: u32) {
    SOUNDS.with(|s| {
        s.borrow_mut().retain(|(i, _, _, done)| {
            if *i == id {
                done.set(true);
                false
            } else {
                true
            }
        })
    });
}
