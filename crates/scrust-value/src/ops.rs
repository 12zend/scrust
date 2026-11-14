//! Operator block semantics (`scratch3_operators.js`) on already-cast inputs.

use crate::jsstr::JsStr;
use crate::num::js_round;
use crate::value::{is_int, ToNum, ToStr, Value};

/// `operator_mod` (floored modulo).
#[inline]
pub fn js_mod(n: f64, modulus: f64) -> f64 {
    // Integer fast path (`fmod` is a slow library call). Exactly equivalent:
    // both operands are integers below 2^53 and the modulus is non-zero.
    let (ni, mi) = (n as i64, modulus as i64);
    if ni as f64 == n && mi as f64 == modulus && mi != 0 && ni.unsigned_abs() < (1 << 53) && mi.unsigned_abs() < (1 << 53) {
        let r = ni % mi;
        if r == 0 {
            // `n % m` keeps the sign of the dividend, including for zero.
            return if n.is_sign_negative() { -0.0 } else { 0.0 };
        }
        return if (r < 0) != (mi < 0) { (r + mi) as f64 } else { r as f64 };
    }
    let mut r = fmod(n, modulus);
    if r / modulus < 0.0 {
        r += modulus;
    }
    r
}

/// JavaScript `%` (C `fmod`), exact. On targets with a fused multiply-add
/// the remainder `n - q * m` of the truncated quotient is computed exactly
/// in one instruction (an `fmod` result is always representable); the
/// quotient is corrected when `n / m` rounded across an integer.
#[inline]
pub fn fmod(n: f64, m: f64) -> f64 {
    #[cfg(any(target_arch = "aarch64", all(target_arch = "x86_64", target_feature = "fma")))]
    {
        if n.is_finite() && m.is_finite() && m != 0.0 {
            let mut q = (n / m).trunc();
            if q.abs() < 4.0e15 {
                let am = m.abs();
                let step = if m > 0.0 { 1.0 } else { -1.0 };
                for _ in 0..3 {
                    let r = (-q).mul_add(m, n);
                    if n >= 0.0 {
                        if r < 0.0 {
                            q -= step;
                            continue;
                        }
                        if r >= am {
                            q += step;
                            continue;
                        }
                    } else {
                        if r > 0.0 {
                            q += step;
                            continue;
                        }
                        if r <= -am {
                            q -= step;
                            continue;
                        }
                    }
                    // A zero remainder has the dividend's sign.
                    return if r == 0.0 { 0.0f64.copysign(n) } else { r };
                }
            }
        }
    }
    n % m
}

#[inline]
pub fn round(n: f64) -> f64 {
    js_round(n)
}

/// `operator_letter_of` with a pre-cast index.
pub fn letter_of(letter: f64, s: &JsStr) -> JsStr {
    let index = letter - 1.0;
    // NaN cannot occur (letter comes from to_num); comparisons mirror the JS.
    if index < 0.0 || index >= s.len() as f64 {
        return JsStr::empty();
    }
    s.char_at(index as usize)
}

/// `operator_contains` (case-insensitive substring).
pub fn contains(a: &JsStr, b: &JsStr) -> bool {
    if b.is_empty() {
        return true;
    }
    a.to_lower().includes(&b.to_lower())
}

#[inline]
pub fn join(a: &JsStr, b: &JsStr) -> JsStr {
    a.concat(b)
}

/// `operator_random`. `from_int` / `to_int` are `Cast.isInt` of the raw inputs.
pub fn random_range(n_from: f64, n_to: f64, both_int: bool, rand: &mut dyn FnMut() -> f64) -> f64 {
    let (low, high) = if n_from <= n_to { (n_from, n_to) } else { (n_to, n_from) };
    if low == high {
        return low;
    }
    if both_int {
        low + (rand() * ((high + 1.0) - low)).floor()
    } else {
        (rand() * (high - low)) + low
    }
}

pub fn random(from: &Value, to: &Value, rand: &mut dyn FnMut() -> f64) -> f64 {
    random_range(from.to_num(), to.to_num(), is_int(from) && is_int(to), rand)
}

const PI: f64 = core::f64::consts::PI;

/// TurboWarp's compiled rounding of sin / cos / tan:
/// `Math.round(x * 1e10) / 1e10` (vanilla uses `parseFloat(x.toFixed(10))`).
#[inline]
fn round10(x: f64) -> f64 {
    crate::num::js_round(x * 1e10) / 1e10
}

/// `round10(exact)` from a value `fast` within two ULPs of the correctly
/// rounded `exact()`: the rounding to ten decimals hides the difference
/// unless `fast * 1e10` lies next to a half-integer (or `fast` is zero,
/// whose sign is then unknown), and only then is `exact` computed. The
/// correctly rounded functions are slow exactly where Scratch projects
/// call them most: degree arguments like 90 put `cos` next to a zero.
#[inline]
fn round10_fast(fast: f64, exact: impl FnOnce() -> f64) -> f64 {
    let t = fast * 1e10;
    if fast != 0.0 && t.abs() < 4.0e15 {
        let d = (t - t.floor() - 0.5).abs();
        if d > t.abs() * 2e-15 + 1e-9 {
            return crate::num::js_round(t) / 1e10;
        }
    }
    round10(exact())
}

/// Out of line: the fast path is long, and inlined into every call site it
/// costs more in code size than the call.
pub fn sin_deg(n: f64) -> f64 {
    let r = (PI * n) / 180.0;
    round10_fast(fast_trig::sin(r), || crate::jsmath::sin(r))
}
/// Out of line: the fast path is long, and inlined into every call site it
/// costs more in code size than the call.
pub fn cos_deg(n: f64) -> f64 {
    let r = (PI * n) / 180.0;
    round10_fast(fast_trig::cos(r), || crate::jsmath::cos(r))
}

/// musl's `sin` / `cos` (as in the `libm` crate, within one ULP), inlined,
/// with the argument reduction always taking musl's "medium" path (exact
/// enough for every |x| < 2^20 pi/2, its special cases near small multiples
/// of pi/2 only exist to save a few operations) instead of the out-of-line,
/// branchy `rem_pio2`. Larger arguments go to the `libm` crate.
mod fast_trig {
    const TO_INT: f64 = 1.5 / f64::EPSILON;
    const INV_PIO2: f64 = 6.36619772367581382433e-01;
    const PIO2_1: f64 = 1.57079632673412561417e+00;
    const PIO2_1T: f64 = 6.07710050650619224932e-11;
    const PIO2_2: f64 = 6.07710050630396597660e-11;
    const PIO2_2T: f64 = 2.02226624879595063154e-21;
    const PIO2_3: f64 = 2.02226624871116645580e-21;
    const PIO2_3T: f64 = 8.47842766036889956997e-32;

    #[inline(always)]
    fn medium(x: f64, ix: u32) -> (i32, f64, f64) {
        let f_n = (x * INV_PIO2 + TO_INT) - TO_INT;
        let n = f_n as i32;
        let mut r = x - f_n * PIO2_1;
        let mut w = f_n * PIO2_1T;
        let mut y0 = r - w;
        let ex = (ix >> 20) as i32;
        let ey = (y0.to_bits() >> 52) as i32 & 0x7ff;
        if ex - ey > 16 {
            let t = r;
            w = f_n * PIO2_2;
            r = t - w;
            w = f_n * PIO2_2T - ((t - r) - w);
            y0 = r - w;
            let ey = (y0.to_bits() >> 52) as i32 & 0x7ff;
            if ex - ey > 49 {
                let t = r;
                w = f_n * PIO2_3;
                r = t - w;
                w = f_n * PIO2_3T - ((t - r) - w);
                y0 = r - w;
            }
        }
        let y1 = (r - y0) - w;
        (n, y0, y1)
    }

    const S1: f64 = -1.66666666666666324348e-01;
    const S2: f64 = 8.33333333332248946124e-03;
    const S3: f64 = -1.98412698298579493134e-04;
    const S4: f64 = 2.75573137070700676789e-06;
    const S5: f64 = -2.50507602534068634195e-08;
    const S6: f64 = 1.58969099521155010221e-10;

    #[inline(always)]
    fn k_sin(x: f64, y: f64, iy: bool) -> f64 {
        let z = x * x;
        let w = z * z;
        let r = S2 + z * (S3 + z * S4) + z * w * (S5 + z * S6);
        let v = z * x;
        if !iy {
            x + v * (S1 + z * r)
        } else {
            x - ((z * (0.5 * y - v * r) - y) - v * S1)
        }
    }

    const C1: f64 = 4.16666666666666019037e-02;
    const C2: f64 = -1.38888888888741095749e-03;
    const C3: f64 = 2.48015872894767294178e-05;
    const C4: f64 = -2.75573143513906633035e-07;
    const C5: f64 = 2.08757232129817482790e-09;
    const C6: f64 = -1.13596475577881948265e-11;

    #[inline(always)]
    fn k_cos(x: f64, y: f64) -> f64 {
        let z = x * x;
        let w = z * z;
        let r = z * (C1 + z * (C2 + z * C3)) + w * w * (C4 + z * (C5 + z * C6));
        let hz = 0.5 * z;
        let w = 1.0 - hz;
        w + (((1.0 - w) - hz) + (z * r - x * y))
    }

    #[inline]
    pub fn sin(x: f64) -> f64 {
        let ix = (x.to_bits() >> 32) as u32 & 0x7fffffff;
        if ix <= 0x3fe921fb {
            if ix < 0x3e500000 {
                return x;
            }
            return k_sin(x, 0.0, false);
        }
        if ix >= 0x413921fb {
            return libm::sin(x);
        }
        let (n, y0, y1) = medium(x, ix);
        match n & 3 {
            0 => k_sin(y0, y1, true),
            1 => k_cos(y0, y1),
            2 => -k_sin(y0, y1, true),
            _ => -k_cos(y0, y1),
        }
    }

    #[inline]
    pub fn cos(x: f64) -> f64 {
        let ix = (x.to_bits() >> 32) as u32 & 0x7fffffff;
        if ix <= 0x3fe921fb {
            if ix < 0x3e46a09e {
                return 1.0;
            }
            return k_cos(x, 0.0);
        }
        if ix >= 0x413921fb {
            return libm::cos(x);
        }
        let (n, y0, y1) = medium(x, ix);
        match n & 3 {
            0 => k_cos(y0, y1),
            1 => -k_sin(y0, y1, true),
            2 => -k_cos(y0, y1),
            _ => k_sin(y0, y1, true),
        }
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn matches_libm() {
            let mut s: u64 = 0x9e37_79b9_7f4a_7c15;
            let mut next = || {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                s
            };
            for i in 0..2_000_000u64 {
                let u = (next() >> 11) as f64 / (1u64 << 53) as f64;
                let deg = match i % 4 {
                    0 => (u - 0.5) * 2000.0,
                    1 => ((u - 0.5) * 4000.0).round(),
                    2 => (u - 0.5) * 2.0e7,
                    _ => ((u - 0.5) * 720.0 * 8.0).round() / 8.0,
                };
                let r = (core::f64::consts::PI * deg) / 180.0;
                assert_eq!(super::sin(r).to_bits(), libm::sin(r).to_bits(), "sin({r})");
                assert_eq!(super::cos(r).to_bits(), libm::cos(r).to_bits(), "cos({r})");
            }
        }
    }
}
/// TurboWarp's `tan` (jsexecute.js): the angle reduced by `% 360`, exact
/// infinities at the poles.
/// Out of line: the fast path is long, and inlined into every call site it
/// costs more in code size than the call.
pub fn tan_deg(n: f64) -> f64 {
    let a = fmod(n, 360.0);
    if a == -270.0 || a == 90.0 {
        return f64::INFINITY;
    }
    if a == -90.0 || a == 270.0 {
        return f64::NEG_INFINITY;
    }
    let r = (PI * a) / 180.0;
    round10_fast(libm::tan(r), || crate::jsmath::tan(r))
}
#[inline]
pub fn asin_deg(n: f64) -> f64 {
    (crate::jsmath::asin(n) * 180.0) / PI
}
#[inline]
pub fn acos_deg(n: f64) -> f64 {
    (crate::jsmath::acos(n) * 180.0) / PI
}
#[inline]
pub fn atan_deg(n: f64) -> f64 {
    (crate::jsmath::atan(n) * 180.0) / PI
}
#[inline]
pub fn ln(n: f64) -> f64 {
    crate::jsmath::log(n)
}
/// `Math.log(n) / Math.LN10` — deliberately not `log10`.
#[inline]
pub fn log10(n: f64) -> f64 {
    crate::jsmath::log(n) / core::f64::consts::LN_10
}
#[inline]
pub fn exp(n: f64) -> f64 {
    crate::jsmath::exp(n)
}
/// `Math.pow(10, n)`. Integer exponents whose result (or reciprocal) is
/// exactly representable are computed exactly, which is what every
/// correctly-rounded platform `pow` returns (see docs/decisions/math-libm.md).
#[inline]
pub fn pow10(n: f64) -> f64 {
    const P: [f64; 23] = [
        1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16, 1e17, 1e18, 1e19,
        1e20, 1e21, 1e22,
    ];
    if n == n.trunc() && n.abs() <= 22.0 {
        let i = n.abs() as usize;
        return if n < 0.0 { 1.0 / P[i] } else { P[i] };
    }
    js_pow(10.0, n)
}

/// `Math.pow` with the ECMAScript special cases that differ from C `pow`.
pub fn js_pow(base: f64, e: f64) -> f64 {
    crate::jsmath::pow(base, e)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MathOp {
    Abs,
    Floor,
    Ceiling,
    Sqrt,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Ln,
    Log,
    Exp,
    Pow10,
    Unknown,
}

impl MathOp {
    /// Operator names are matched after `toLowerCase()`.
    pub fn from_name(name: &JsStr) -> MathOp {
        match name.to_lower().to_rust_string().as_str() {
            "abs" => MathOp::Abs,
            "floor" => MathOp::Floor,
            "ceiling" => MathOp::Ceiling,
            "sqrt" => MathOp::Sqrt,
            "sin" => MathOp::Sin,
            "cos" => MathOp::Cos,
            "tan" => MathOp::Tan,
            "asin" => MathOp::Asin,
            "acos" => MathOp::Acos,
            "atan" => MathOp::Atan,
            "ln" => MathOp::Ln,
            "log" => MathOp::Log,
            "e ^" => MathOp::Exp,
            "10 ^" => MathOp::Pow10,
            _ => MathOp::Unknown,
        }
    }

    #[inline]
    pub fn apply(self, n: f64) -> f64 {
        match self {
            MathOp::Abs => n.abs(),
            MathOp::Floor => n.floor(),
            MathOp::Ceiling => n.ceil(),
            MathOp::Sqrt => n.sqrt(),
            MathOp::Sin => sin_deg(n),
            MathOp::Cos => cos_deg(n),
            MathOp::Tan => tan_deg(n),
            MathOp::Asin => asin_deg(n),
            MathOp::Acos => acos_deg(n),
            MathOp::Atan => atan_deg(n),
            MathOp::Ln => ln(n),
            MathOp::Log => log10(n),
            MathOp::Exp => exp(n),
            MathOp::Pow10 => pow10(n),
            MathOp::Unknown => 0.0,
        }
    }
}

/// `operator_mathop` with a dynamic operator name.
pub fn mathop(op: &JsStr, n: f64) -> f64 {
    MathOp::from_name(op).apply(n)
}

/// TurboWarp's `listContents`: no separator iff every item's string form
/// has length 1 (vanilla requires items that are strings).
pub fn list_contents<'a>(items: impl Iterator<Item = &'a Value> + Clone) -> JsStr {
    let all_single = items.clone().all(|v| v.to_str().len() == 1);
    let mut units: Vec<u16> = Vec::new();
    for (i, v) in items.enumerate() {
        if i > 0 && !all_single {
            units.push(b' ' as u16);
        }
        units.extend(v.to_str().flat().units());
    }
    JsStr::from_utf16(&units)
}
