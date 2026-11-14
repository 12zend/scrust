//! scrust runtime: scheduler, world state and block primitives used by
//! generated code.

pub mod alloc;
pub mod blocks;
pub mod cli;
pub mod list;
pub mod pen;
pub mod sched;
pub mod trace;
pub mod vaudio;
pub mod world;

pub use list::{Elem, List, Lit};
pub use sched::{die, step, wait_external, yield_loop, yield_now, yield_status, yield_tick, BoxFut, HatKind, Status, Wake};
pub use scrust_value as value;
pub use scrust_value::{JsStr, ListIndex, MathOp, ToBool, ToNum, ToStr, ToValue, Value};
pub use world::*;

/// Everything generated code needs.
pub mod prelude {
    pub use crate::blocks::Glide;
    pub use crate::list::{Elem, List, Lit};
    pub use crate::sched::{die, wait_external, yield_loop, yield_now, yield_status, yield_tick, BoxFut, HatKind, Status, Wake};
    pub use crate::world::*;
    pub use crate::{val_bool, val_num, val_str};
    pub use scrust_value::ops;
    pub use scrust_value::{
        compare, compare_lit, compare_str_ci, compare_str_ci_lit, compare_val_str, letter_eq_ci, js_num_to_str, js_round, num_eq, num_gt, num_lt, num_to_list_index, JsStr, ListIndex, MathOp, ToBool, ToNum, ToStr, ToValue, Value,
    };
}

/// Read a dynamic value that flow analysis proved to be a number.
#[inline(always)]
pub fn val_num(v: &Value) -> f64 {
    match v {
        Value::Num(n) => *n,
        other => other.js_number(),
    }
}

/// Read a dynamic value that flow analysis proved to be a string.
#[inline(always)]
pub fn val_str(v: &Value) -> JsStr {
    match v {
        Value::Str(s) => s.clone(),
        other => other.to_str(),
    }
}

/// Read a dynamic value that flow analysis proved to be a boolean.
#[inline(always)]
pub fn val_bool(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        other => other.to_bool(),
    }
}

/// Load an asset stored next to the executable (`--assets external`).
#[cfg(not(target_arch = "wasm32"))]
pub fn load_asset(name: &str) -> &'static [u8] {
    let mut dirs = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            dirs.push(d.join("assets"));
        }
    }
    dirs.push(std::path::PathBuf::from("assets"));
    for d in dirs {
        if let Ok(data) = std::fs::read(d.join(name)) {
            return Box::leak(data.into_boxed_slice());
        }
    }
    eprintln!("scrust: asset {name} not found");
    &[]
}

#[cfg(target_arch = "wasm32")]
pub fn load_asset(_name: &str) -> &'static [u8] {
    &[]
}

/// A string constant of generated code, created on first use.
#[macro_export]
macro_rules! jstr {
    ($s:expr) => {{
        static S: $crate::StaticStr = $crate::StaticStr::new($s);
        S.get()
    }};
}

/// Storage of a `jstr!` constant.
///
/// `JsStr` is reference counted without atomics, so it is not `Sync`; a
/// thread-local would be the textbook home for it, but on macOS every access
/// to one is a function call. Generated programs evaluate string constants
/// only on the thread that steps the world (scripts never run elsewhere;
/// renderer and audio threads do not touch them), so a static is enough.
pub struct StaticStr {
    text: &'static str,
    cell: std::cell::UnsafeCell<Option<JsStr>>,
}

// SAFETY: see the type's documentation: only the script thread uses it.
unsafe impl Sync for StaticStr {}

impl StaticStr {
    pub const fn new(text: &'static str) -> StaticStr {
        StaticStr { text, cell: std::cell::UnsafeCell::new(None) }
    }

    #[inline]
    pub fn get(&'static self) -> JsStr {
        // SAFETY: single-threaded use (see the type); the reference to the
        // cell's contents ends before it is written.
        unsafe {
            if let Some(s) = &*self.cell.get() {
                return s.clone();
            }
            let s = JsStr::from(self.text);
            *self.cell.get() = Some(s.clone());
            s
        }
    }
}

/// Headless driver: green flag, then run `frames` frames, calling `on_frame`
/// after each one.
pub fn run_headless(w: &mut World, frames: u64, mut on_frame: impl FnMut(&World)) {
    let wp: *mut World = w;
    // SAFETY: `w` is exclusively borrowed for the duration of this call and
    // only accessed through `wp` from here on.
    unsafe {
        set_world_ptr(wp);
        (*wp).green_flag();
        for _ in 0..frames {
            sched::step(wp);
            on_frame(&*wp);
        }
    }
}

/// Headless browser/Node build used for benchmarks (`scrust build --target
/// wasm --headless`): no renderer, no audio, deterministic clock, one pass
/// per frame.
#[cfg(target_arch = "wasm32")]
pub mod webbench {
    use crate::{set_world_ptr, Options, ProgramDef, World};
    use std::cell::Cell;

    thread_local! {
        static WORLD: Cell<*mut World> = const { Cell::new(std::ptr::null_mut()) };
    }

    pub fn init(program: ProgramDef, mut options: Options, seed: u32) {
        let program: &'static ProgramDef = Box::leak(Box::new(program));
        options.deterministic = true;
        options.max_passes = Some(1);
        options.seed = seed;
        let w: &'static mut World = Box::leak(World::new(program, options));
        let wp: *mut World = w;
        WORLD.with(|c| c.set(wp));
        // SAFETY: the world lives for the rest of the program and is only
        // reached through `wp`.
        unsafe {
            set_world_ptr(wp);
            (*wp).green_flag();
        }
    }

    thread_local! {
        static TRACE: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    }

    /// The trace line of the current frame (as `--trace` prints it); returns
    /// its length, the UTF-8 bytes are at `trace_ptr`.
    pub fn trace() -> u32 {
        let wp = WORLD.with(|c| c.get());
        if wp.is_null() {
            return 0;
        }
        // SAFETY: see `init`.
        let line = unsafe { (*wp).trace_line() };
        TRACE.with(|t| {
            *t.borrow_mut() = line;
            t.borrow().len() as u32
        })
    }

    pub fn trace_ptr() -> *const u8 {
        TRACE.with(|t| t.borrow().as_ptr())
    }

    /// One frame; returns the number of live threads.
    pub fn step() -> u32 {
        let wp = WORLD.with(|c| c.get());
        if wp.is_null() {
            return 0;
        }
        // SAFETY: see `init`.
        unsafe {
            crate::sched::step(wp);
            (*wp).live_threads() as u32
        }
    }
}

/// Exports of a headless wasm build (see `webbench`).
#[macro_export]
macro_rules! headless_web_exports {
    ($program:ident, $options:ident) => {
        #[cfg(target_arch = "wasm32")]
        mod __scrust_webbench {
            #[no_mangle]
            pub extern "C" fn scrust_bench_init(seed: u32) {
                $crate::webbench::init(super::$program(), super::$options(), seed);
            }
            #[no_mangle]
            pub extern "C" fn scrust_bench_step() -> u32 {
                $crate::webbench::step()
            }
            #[no_mangle]
            pub extern "C" fn scrust_bench_trace() -> u32 {
                $crate::webbench::trace()
            }
            #[no_mangle]
            pub extern "C" fn scrust_bench_trace_ptr() -> *const u8 {
                $crate::webbench::trace_ptr()
            }
        }
    };
}
