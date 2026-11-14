//! Runs a Scratch project from scrust's IR, without generating Rust: the
//! program definition the runtime needs (`ProgramDef`) is built at run time
//! and its scripts are executed by an interpreter (`exec`).
//!
//! This is what the GUI's browser build uses: rustc cannot run in a browser,
//! but this crate (front end, analysis, runtime and renderer) compiles to
//! WebAssembly, so a project goes from .sb3 to running entirely in the page.
//!
//! The interpreter follows the code generator statement for statement in its
//! `--no-unbox` form (every variable a dynamic `Value`): the same runtime
//! calls in the same order, and the same suspension points, decided from the
//! same analysis (`stmt_may_yield`, `yields`, `callee_warp`, `promise_tail`).
//! `scrust trace --interp` compares it with compiled programs.

mod exec;
#[cfg(target_arch = "wasm32")]
mod web;

use scrust_analysis::Analysis;
use scrust_codegen::frontend;
use scrust_ir::{Hat, Program};
use scrust_runtime::{
    CostumeDef, HatKind, List, Locals, MonitorDef, Options, ProgramDef, RotationStyle, ScriptDef, SoundDef, SpriteDef, TargetRef, Value, World,
};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

/// How to build a program.
#[derive(Clone, Debug, Default)]
pub struct Config {
    /// Frames per second; default: the project's TurboWarp setting, else 30.
    pub fps: Option<f64>,
    pub turbo: bool,
    pub warp_timer: bool,
    /// Leave costume and sound data out (as headless compiled programs do).
    pub headless: bool,
}

pub struct Built {
    pub program: ProgramDef,
    pub options: Options,
    /// Blocks scrust cannot run (they do nothing), one line each.
    pub unsupported: Vec<String>,
}

/// The program being interpreted, with the analysis that decides where
/// threads may suspend. Leaked: it lives as long as the `ProgramDef` built
/// from it.
pub(crate) struct Interp {
    pub program: Program,
    pub an: Analysis,
    /// `stmt_may_yield` per (statement address, warp), computed on first use.
    may_yield: RefCell<HashMap<(usize, bool), bool>>,
    /// Does a loop body contain something that sets the skip-loop-yield flag?
    has_promise: RefCell<HashMap<usize, bool>>,
}

thread_local! {
    static INTERP: Cell<Option<&'static Interp>> = const { Cell::new(None) };
}

/// The program the entry points of the current `ProgramDef` belong to.
pub(crate) fn interp() -> &'static Interp {
    INTERP.with(|i| i.get()).expect("scrust-interp: no program loaded")
}

impl Interp {
    pub fn may_yield(&self, s: &scrust_ir::Stmt, warp: bool) -> bool {
        let key = (s as *const _ as usize, warp);
        if let Some(v) = self.may_yield.borrow().get(&key) {
            return *v;
        }
        let v = self.an.stmt_may_yield(&self.program, s, warp);
        self.may_yield.borrow_mut().insert(key, v);
        v
    }

    pub fn has_promise(&self, body: &[scrust_ir::Stmt]) -> bool {
        let key = body.as_ptr() as usize;
        if let Some(v) = self.has_promise.borrow().get(&key) {
            return *v;
        }
        let mut hit = false;
        scrust_ir::walk_stmts(body, &mut |s| match s {
            scrust_ir::Stmt::SayForSecs { .. } | scrust_ir::Stmt::Ask(_) | scrust_ir::Stmt::PlaySoundUntilDone(_) => hit = true,
            scrust_ir::Stmt::Cmd(c, ..) if c.is_promise() => hit = true,
            scrust_ir::Stmt::Call { proc, .. } if self.an.promise_tail.contains(proc) => hit = true,
            _ => {}
        });
        self.has_promise.borrow_mut().insert(key, hit);
        hit
    }
}

fn leak(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

fn leak_bytes(b: &[u8]) -> &'static [u8] {
    Box::leak(b.to_vec().into_boxed_slice())
}

// ------------------------------------------------------------------ storage

/// Names of a target's variables and lists, in declaration order.
pub(crate) struct Names {
    vars: Vec<&'static str>,
    lists: Vec<&'static str>,
}

/// Variable storage of one target: every variable a dynamic `Value`.
#[derive(Clone)]
pub(crate) struct ILocals {
    pub vars: Vec<Value>,
    pub lists: Vec<List<Value>>,
    names: &'static Names,
}

impl Locals for ILocals {
    fn clone_box(&self) -> Box<dyn Locals> {
        Box::new(self.clone())
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn var_by_name(&self, name: &str) -> Option<Value> {
        // The first variable of that name, as the generated `match` does.
        self.names.vars.iter().position(|n| *n == name).map(|i| self.vars[i].clone())
    }
    fn vars(&self) -> Vec<(&'static str, Value)> {
        self.names.vars.iter().copied().zip(self.vars.iter().cloned()).collect()
    }
    fn lists(&self) -> Vec<(&'static str, Vec<Value>)> {
        self.names.lists.iter().copied().zip(self.lists.iter().map(|l| l.values())).collect()
    }
    fn set_var(&mut self, index: usize, value: Value) {
        if let Some(v) = self.vars.get_mut(index) {
            *v = value;
        }
    }
}

fn new_locals(ti: usize, names: &'static Names) -> Box<dyn Locals> {
    let t = &interp().program.targets[ti];
    Box::new(ILocals {
        vars: t.vars.iter().map(|v| v.init.clone()).collect(),
        lists: t.lists.iter().map(|l| List::new(l.init.clone())).collect(),
        names,
    })
}

// ------------------------------------------------------------------ build

/// Load a project (.sb3 or project.json) and build the program that runs it.
///
/// The program refers to the interpreter state of the calling thread, which
/// it replaces: only one interpreted program runs per thread.
pub fn build(bytes: &[u8], cfg: &Config) -> Result<Built, String> {
    let comp = frontend::compile(bytes, true, cfg.warp_timer)?;
    let unsupported = frontend::unsupported_report(&comp.program);
    let tw = comp.project.tw_config();
    let options = Options {
        fps: cfg.fps.unwrap_or_else(|| frontend::tw_framerate(&comp.project)),
        turbo: cfg.turbo || tw.turbo,
        warp_timer: cfg.warp_timer,
        fencing: tw.fencing,
        misc_limits: tw.misc_limits,
        max_clones: tw.max_clones,
        hq: tw.hq,
        stage_width: tw.width.unwrap_or(480.0),
        stage_height: tw.height.unwrap_or(360.0),
        ..Options::default()
    };
    let project = comp.project;
    let it: &'static Interp = Box::leak(Box::new(Interp {
        program: comp.program,
        an: comp.analysis,
        may_yield: RefCell::new(HashMap::new()),
        has_promise: RefCell::new(HashMap::new()),
    }));
    INTERP.with(|i| i.set(Some(it)));
    let p = &it.program;

    let mut sprites = Vec::with_capacity(p.targets.len());
    for (ti, t) in p.targets.iter().enumerate() {
        let src = &project.targets[ti];
        let data = |md5ext: &str| -> &'static [u8] {
            if cfg.headless {
                &[]
            } else {
                project.assets.get(md5ext).map(|d| leak_bytes(d)).unwrap_or(&[])
            }
        };
        let costumes = src
            .costumes
            .iter()
            .map(|c| CostumeDef {
                name: leak(&c.name),
                md5ext: leak(&c.md5ext),
                data_format: leak(&c.data_format),
                data: data(&c.md5ext),
                bitmap_resolution: c.bitmap_resolution,
                center_x: c.rotation_center_x,
                center_y: c.rotation_center_y,
            })
            .collect();
        let sounds = src
            .sounds
            .iter()
            .map(|c| SoundDef {
                name: leak(&c.name),
                md5ext: leak(&c.md5ext),
                data_format: leak(&c.data_format),
                data: data(&c.md5ext),
                rate: c.rate,
                sample_count: c.sample_count,
            })
            .collect();
        let mut scripts = Vec::with_capacity(t.scripts.len());
        for (si, sc) in t.scripts.iter().enumerate() {
            let (kind, field, msg) = match &sc.hat {
                Hat::FlagClicked => (HatKind::Flag, "", 0),
                Hat::KeyPressed(k) => (HatKind::Key, leak(k), 0),
                Hat::SpriteClicked => (HatKind::SpriteClicked, "", 0),
                Hat::StageClicked => (HatKind::StageClicked, "", 0),
                Hat::BackdropSwitchesTo(b) => (HatKind::Backdrop, leak(b), 0),
                Hat::GreaterThan { .. } => (HatKind::Greater, "", 0),
                Hat::BroadcastReceived(m) => (HatKind::Broadcast, "", *m),
                Hat::StartAsClone => (HatKind::Clone, "", 0),
                Hat::TouchingObject(_) => (HatKind::Touching, "", 0),
            };
            let pred: Option<scrust_runtime::PredFn> = if sc.hat.is_edge_activated() {
                Some(Box::new(move |w: &mut World, me: TargetRef| exec::hat_predicate(w, ti, si, me)))
            } else {
                None
            };
            let creates = scrust_codegen::created_on_compile(&project, p, ti, &sc.body);
            scripts.push(ScriptDef {
                kind,
                field,
                msg,
                restart: sc.hat.restarts_existing(),
                start: Box::new(move |me| exec::start_script(ti, si, me)),
                pred,
                creates: Box::leak(creates.into_boxed_slice()),
            });
        }
        let names: &'static Names = Box::leak(Box::new(Names {
            vars: t.vars.iter().map(|v| leak(&v.name)).collect(),
            lists: t.lists.iter().map(|l| leak(&l.name)).collect(),
        }));
        let rotation_style = match src.rotation_style.as_str() {
            "left-right" => RotationStyle::LeftRight,
            "don't rotate" => RotationStyle::DontRotate,
            _ => RotationStyle::AllAround,
        };
        sprites.push(SpriteDef {
            name: leak(&t.name),
            is_stage: t.is_stage,
            costumes,
            sounds,
            scripts,
            new_locals: Box::new(move || new_locals(ti, names)),
            x: src.x,
            y: src.y,
            size: src.size,
            direction: src.direction,
            visible: src.visible,
            draggable: src.draggable,
            costume: src.current_costume,
            rotation_style,
            volume: src.volume,
            declared_vars: src.variables.len().min(t.vars.len()),
            declared_lists: src.lists.len().min(t.lists.len()),
            tempo: src.tempo,
        });
    }
    let monitors = project
        .monitors
        .iter()
        .filter_map(|m| {
            let info = scrust_codegen::monitor_info(p, m)?;
            Some(MonitorDef {
                id: leak(&m.id),
                opcode: leak(&m.opcode),
                mode: leak(&m.mode),
                label: leak(&info.label),
                sprite: info.sprite,
                index: info.index,
                param: leak(&info.param),
                x: m.x,
                y: m.y,
                width: m.width,
                height: m.height,
                visible: m.visible,
                slider_min: m.slider_min,
                slider_max: m.slider_max,
                is_discrete: m.is_discrete,
            })
        })
        .collect();
    let program = ProgramDef {
        sprites,
        messages: p.messages.iter().map(|m| leak(m)).collect(),
        broadcast_lookup: p.broadcast_lookup.iter().map(|(n, m)| (leak(n), *m)).collect(),
        monitors,
    };
    Ok(Built { program, options, unsupported })
}
