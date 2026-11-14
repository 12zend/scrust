//! `Math.*` as the reference browser computes it.
//!
//! Current V8 routes `exp/log/sin/cos/tan/atan/asin/acos/atan2` to LLVM libc,
//! whose results are correctly rounded; on native targets the vendored
//! CORE-MATH functions (csrc/core-math, also correctly rounded) give the same
//! bits. `x ** y` follows `v8::internal::math::pow`, which on macOS calls the
//! platform `pow`. The wasm build keeps the `libm` crate (1 ULP off on a few
//! percent of inputs). See docs/decisions/math-libm.md.

#[cfg(not(target_arch = "wasm32"))]
mod cr {
    extern "C" {
        pub fn cr_exp(x: f64) -> f64;
        pub fn cr_log(x: f64) -> f64;
        pub fn cr_sin(x: f64) -> f64;
        pub fn cr_cos(x: f64) -> f64;
        pub fn cr_tan(x: f64) -> f64;
        pub fn cr_asin(x: f64) -> f64;
        pub fn cr_acos(x: f64) -> f64;
        pub fn cr_atan2(y: f64, x: f64) -> f64;
    }
}

macro_rules! unary {
    ($($name:ident => $cr:ident),*) => {$(
        #[inline]
        pub fn $name(x: f64) -> f64 {
            #[cfg(not(target_arch = "wasm32"))]
            // SAFETY: pure functions of their argument.
            unsafe { cr::$cr(x) }
            #[cfg(target_arch = "wasm32")]
            libm::$name(x)
        }
    )*};
}
unary!(exp => cr_exp, log => cr_log, sin => cr_sin, cos => cr_cos, tan => cr_tan, asin => cr_asin, acos => cr_acos);

/// `Math.atan`: V8's LLVM libc implementation, which is not correctly
/// rounded near halfway cases (see `llvm_atan`).
#[inline]
pub fn atan(x: f64) -> f64 {
    crate::llvm_atan::atan(x)
}

#[inline]
pub fn atan2(y: f64, x: f64) -> f64 {
    #[cfg(not(target_arch = "wasm32"))]
    // SAFETY: pure function of its arguments.
    unsafe {
        cr::cr_atan2(y, x)
    }
    #[cfg(target_arch = "wasm32")]
    libm::atan2(y, x)
}

/// `base ** exponent` (`v8::internal::math::pow`).
pub fn pow(x: f64, y: f64) -> f64 {
    if y.is_nan() {
        return f64::NAN;
    }
    if y.is_infinite() && (x == 1.0 || x == -1.0) {
        return f64::NAN;
    }
    if y == 2.0 {
        return x * x;
    }
    if y == 0.5 {
        return if x.is_infinite() { f64::INFINITY } else { (x + 0.0).sqrt() };
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        // Hidden from the optimiser: LLVM turns `pow(10, y)` into `exp10(y)`,
        // which is a different function from the platform `pow`.
        core::hint::black_box(x).powf(y)
    }
    #[cfg(target_arch = "wasm32")]
    libm::pow(x, y)
}
