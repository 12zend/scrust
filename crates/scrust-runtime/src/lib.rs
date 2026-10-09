//! scrust runtime: scheduler, world state and block primitives used by
//! generated code.

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
        compare, compare_str_ci, js_num_to_str, js_round, num_eq, num_gt, num_lt, num_to_list_index, JsStr, ListIndex, MathOp, ToBool, ToNum, ToStr, ToValue, Value,
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

/// A string constant, interned once per thread.
#[macro_export]
macro_rules! jstr {
    ($s:expr) => {{
        thread_local! { static S: $crate::JsStr = $crate::JsStr::from($s); }
        S.with(|s| s.clone())
    }};
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
