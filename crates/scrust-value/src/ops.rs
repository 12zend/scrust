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
    let mut r = n % modulus;
    if r / modulus < 0.0 {
        r += modulus;
    }
    r
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
#[inline]
pub fn sin_deg(n: f64) -> f64 {
    round10(crate::jsmath::sin((PI * n) / 180.0))
}
#[inline]
pub fn cos_deg(n: f64) -> f64 {
    round10(crate::jsmath::cos((PI * n) / 180.0))
}
/// TurboWarp's `tan` (jsexecute.js): the angle reduced by `% 360`, exact
/// infinities at the poles.
pub fn tan_deg(n: f64) -> f64 {
    let a = n % 360.0;
    if a == -270.0 || a == 90.0 {
        return f64::INFINITY;
    }
    if a == -90.0 || a == 270.0 {
        return f64::NEG_INFINITY;
    }
    round10(crate::jsmath::tan((PI * a) / 180.0))
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
