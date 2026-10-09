//! Hosts for generated programs: a native window (winit + softbuffer + rodio)
//! and a browser host driven through plain wasm exports.

#[cfg(not(target_arch = "wasm32"))]
mod audio;
#[cfg(not(target_arch = "wasm32"))]
mod embed;
pub mod input;
pub mod state;
#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
pub mod web;

pub use scrust_render::SoftRenderer;
pub use scrust_runtime::{Options, ProgramDef, World};

#[cfg(not(target_arch = "wasm32"))]
pub fn main(program: ProgramDef, defaults: Options) {
    native::main(program, defaults);
}

#[cfg(target_arch = "wasm32")]
pub fn main(_program: ProgramDef, _defaults: Options) {}

/// Defines the wasm exports of a generated program (browser host).
#[macro_export]
macro_rules! web_exports {
    ($program:ident, $options:ident) => {
        #[cfg(target_arch = "wasm32")]
        mod __scrust_web {
            #[no_mangle]
            pub extern "C" fn scrust_init(scale: f64, seed: u32, epoch_ms: f64, tz_offset_min: f64) {
                $crate::web::init(super::$program(), super::$options(), scale, seed, epoch_ms, tz_offset_min);
            }
        }
    };
}
