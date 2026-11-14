//! Browser exports. The module is a complete scrust player: the page writes
//! a project into it (`scrust_input`), builds it (`scrust_load`) and then
//! drives it with the same exports as a compiled program (`scrust_init`,
//! `scrust_frame`, input, sound; see `scrust_player::web`), so one
//! WebAssembly module runs any project without compiling it.
//!
//! Each run should use a fresh instance: a loaded program's data lives as
//! long as the instance.

use crate::{build, Config};
use scrust_runtime::{Options, ProgramDef};
use std::cell::RefCell;

thread_local! {
    static INPUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static MESSAGE: RefCell<String> = const { RefCell::new(String::new()) };
    static LOADED: RefCell<Option<(ProgramDef, Options)>> = const { RefCell::new(None) };
}

/// A buffer of `len` bytes for the project (.sb3 or project.json).
#[no_mangle]
pub extern "C" fn scrust_input(len: u32) -> *mut u8 {
    INPUT.with(|b| {
        let mut b = b.borrow_mut();
        b.clear();
        b.resize(len as usize, 0);
        b.as_mut_ptr()
    })
}

/// Build the project in the input buffer. `fps` <= 0 uses the project's
/// setting. Returns 1 on success (the message lists unsupported blocks, one
/// per line, which do nothing) or 0 (the message is the error).
#[no_mangle]
pub extern "C" fn scrust_load(fps: f64, turbo: u32) -> u32 {
    let bytes = INPUT.with(|b| std::mem::take(&mut *b.borrow_mut()));
    let cfg = Config { fps: (fps > 0.0).then_some(fps), turbo: turbo != 0, ..Config::default() };
    let (ok, message) = match build(&bytes, &cfg) {
        Ok(built) => {
            LOADED.with(|l| *l.borrow_mut() = Some((built.program, built.options)));
            (1, built.unsupported.join("\n"))
        }
        Err(e) => (0, e),
    };
    MESSAGE.with(|m| *m.borrow_mut() = message);
    ok
}

#[no_mangle]
pub extern "C" fn scrust_message_ptr() -> *const u8 {
    MESSAGE.with(|m| m.borrow().as_ptr())
}

#[no_mangle]
pub extern "C" fn scrust_message_len() -> u32 {
    MESSAGE.with(|m| m.borrow().len() as u32)
}

/// Start the loaded program (its green flag is clicked), as a compiled
/// program's `scrust_init` does.
#[no_mangle]
pub extern "C" fn scrust_init(scale: f64, seed: u32, epoch_ms: f64, tz_offset_min: f64) {
    if let Some((program, options)) = LOADED.with(|l| l.borrow_mut().take()) {
        scrust_player::web::init(program, options, scale, seed, epoch_ms, tz_offset_min);
    }
}
