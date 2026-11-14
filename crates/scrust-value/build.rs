//! Compiles the vendored CORE-MATH functions (csrc/core-math, MIT) used for
//! correctly rounded `Math.*` results on native targets; wasm keeps `libm`.
fn main() {
    println!("cargo:rerun-if-changed=csrc/core-math");
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32") {
        return;
    }
    let mut b = cc::Build::new();
    for f in ["exp", "log", "sin", "cos", "tan", "atan", "asin", "acos", "atan2"] {
        b.file(format!("csrc/core-math/{f}/{f}.c"));
    }
    b.opt_level(2).flag_if_supported("-std=gnu11").warnings(false).cargo_warnings(false);
    b.compile("scrust_core_math");
}
