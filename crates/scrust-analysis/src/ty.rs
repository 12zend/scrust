//! The type lattice (§8.2): a bitset of value classes.

use scrust_value::{js_number_to_string, js_string_to_number, Value};
use std::ops::{BitAnd, BitOr, BitOrAssign, Not};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Ty(pub u32);

impl Ty {
    pub const NONE: Ty = Ty(0);
    pub const POS_INF: Ty = Ty(1 << 0);
    pub const POS_INT: Ty = Ty(1 << 1);
    pub const POS_FRACT: Ty = Ty(1 << 2);
    pub const ZERO: Ty = Ty(1 << 3);
    pub const NEG_ZERO: Ty = Ty(1 << 4);
    pub const NEG_INT: Ty = Ty(1 << 5);
    pub const NEG_FRACT: Ty = Ty(1 << 6);
    pub const NEG_INF: Ty = Ty(1 << 7);
    pub const NAN: Ty = Ty(1 << 8);
    /// Canonical numeric string without a fraction (`"10"`, `"-3"`, `"0"`).
    pub const STR_CINT: Ty = Ty(1 << 9);
    /// Canonical numeric string with a fraction (`"1.5"`).
    pub const STR_CFRACT: Ty = Ty(1 << 10);
    /// Other strings with a numeric interpretation (`"1.0"`, `" 5"`, `"0x10"`, `"-0"`, `"Infinity"`).
    pub const STR_NUM: Ty = Ty(1 << 11);
    /// Non-numeric strings (including `""` and whitespace).
    pub const STR_NAN: Ty = Ty(1 << 12);
    /// `"true"` / `"false"` in any letter case.
    pub const STR_BOOL: Ty = Ty(1 << 13);
    pub const BOOL_T: Ty = Ty(1 << 14);
    pub const BOOL_F: Ty = Ty(1 << 15);

    pub const INF: Ty = Ty(Self::POS_INF.0 | Self::NEG_INF.0);
    pub const FRACT: Ty = Ty(Self::POS_FRACT.0 | Self::NEG_FRACT.0);
    pub const ANY_ZERO: Ty = Ty(Self::ZERO.0 | Self::NEG_ZERO.0);
    pub const INT: Ty = Ty(Self::POS_INT.0 | Self::NEG_INT.0 | Self::ZERO.0);
    pub const FINITE: Ty = Ty(Self::INT.0 | Self::FRACT.0 | Self::NEG_ZERO.0);
    /// Every number except NaN.
    pub const NUMBER: Ty = Ty(Self::FINITE.0 | Self::INF.0);
    pub const NUMBER_OR_NAN: Ty = Ty(Self::NUMBER.0 | Self::NAN.0);
    pub const STR_CANON: Ty = Ty(Self::STR_CINT.0 | Self::STR_CFRACT.0);
    pub const STRING: Ty = Ty(Self::STR_CANON.0 | Self::STR_NUM.0 | Self::STR_NAN.0 | Self::STR_BOOL.0);
    pub const BOOLEAN: Ty = Ty(Self::BOOL_T.0 | Self::BOOL_F.0);
    pub const ANY: Ty = Ty(Self::NUMBER_OR_NAN.0 | Self::STRING.0 | Self::BOOLEAN.0);

    #[inline]
    pub fn has(self, o: Ty) -> bool {
        self.0 & o.0 != 0
    }
    #[inline]
    pub fn within(self, o: Ty) -> bool {
        self.0 & !o.0 == 0
    }
    #[inline]
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Exact type of a constant.
    pub fn of_value(v: &Value) -> Ty {
        match v {
            Value::Bool(true) => Ty::BOOL_T,
            Value::Bool(false) => Ty::BOOL_F,
            Value::Num(n) => Ty::of_number(*n),
            Value::Str(s) => {
                let n = js_string_to_number(s);
                // Strings `Cast.compare` treats as NaN (TurboWarp's
                // isNotActuallyZero: whitespace with a tab counts as 0).
                if n.is_nan() || (n == 0.0 && s.is_not_actually_zero()) {
                    let lower = s.to_lower();
                    if lower.eq_str("true") || lower.eq_str("false") {
                        Ty::STR_BOOL
                    } else {
                        Ty::STR_NAN
                    }
                } else if is_canonical(&s.to_rust_string(), n) {
                    if n == n.trunc() {
                        Ty::STR_CINT
                    } else {
                        Ty::STR_CFRACT
                    }
                } else {
                    Ty::STR_NUM
                }
            }
        }
    }

    pub fn of_number(n: f64) -> Ty {
        if n.is_nan() {
            Ty::NAN
        } else if n == f64::INFINITY {
            Ty::POS_INF
        } else if n == f64::NEG_INFINITY {
            Ty::NEG_INF
        } else if n == 0.0 {
            if n.is_sign_negative() {
                Ty::NEG_ZERO
            } else {
                Ty::ZERO
            }
        } else if n == n.trunc() {
            if n > 0.0 {
                Ty::POS_INT
            } else {
                Ty::NEG_INT
            }
        } else if n > 0.0 {
            Ty::POS_FRACT
        } else {
            Ty::NEG_FRACT
        }
    }

    /// Type of `Cast.toNumber(x)` for `x: self` (never contains NaN).
    pub fn to_num(self) -> Ty {
        let mut r = Ty(self.0 & Ty::NUMBER.0);
        if self.has(Ty::NAN) || self.has(Ty::STR_NAN) || self.has(Ty::STR_BOOL) || self.has(Ty::BOOL_F) {
            r |= Ty::ZERO;
        }
        if self.has(Ty::BOOL_T) {
            r |= Ty::POS_INT;
        }
        if self.has(Ty::STR_CINT) {
            r |= Ty::INT;
        }
        if self.has(Ty::STR_CFRACT) {
            r |= Ty::FRACT;
        }
        if self.has(Ty::STR_NUM) {
            r |= Ty::NUMBER;
        }
        r
    }

    /// The type a value has once canonical numeric strings are stored as
    /// numbers (`f64` storage).
    pub fn numeric_image(self) -> Ty {
        let mut r = Ty(self.0 & !Ty::STR_CANON.0);
        if self.has(Ty::STR_CINT) {
            r |= Ty::INT;
        }
        if self.has(Ty::STR_CFRACT) {
            r |= Ty::FRACT;
        }
        r
    }
}

/// `String(Number(s)) === s` and `/^-?\d+(\.\d+)?$/`.
pub fn is_canonical(s: &str, n: f64) -> bool {
    if n.is_nan() || n.is_infinite() {
        return false;
    }
    if n == 0.0 && n.is_sign_negative() {
        return false;
    }
    let b = s.as_bytes();
    let body = if b.first() == Some(&b'-') { &b[1..] } else { b };
    if body.is_empty() || !body.iter().all(|c| c.is_ascii_digit() || *c == b'.') {
        return false;
    }
    js_number_to_string(n) == s
}

impl BitOr for Ty {
    type Output = Ty;
    fn bitor(self, o: Ty) -> Ty {
        Ty(self.0 | o.0)
    }
}
impl BitOrAssign for Ty {
    fn bitor_assign(&mut self, o: Ty) {
        self.0 |= o.0;
    }
}
impl BitAnd for Ty {
    type Output = Ty;
    fn bitand(self, o: Ty) -> Ty {
        Ty(self.0 & o.0)
    }
}
impl Not for Ty {
    type Output = Ty;
    fn not(self) -> Ty {
        Ty(!self.0 & Ty::ANY.0)
    }
}

impl std::fmt::Debug for Ty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        const NAMES: [&str; 16] = [
            "+inf", "+int", "+fract", "0", "-0", "-int", "-fract", "-inf", "NaN", "str:int", "str:fract", "str:num", "str:text", "str:bool",
            "true", "false",
        ];
        if *self == Ty::ANY {
            return write!(f, "any");
        }
        let mut parts: Vec<&str> = Vec::new();
        let mut rest = *self;
        for (name, set) in [("number", Ty::NUMBER), ("string", Ty::STRING), ("boolean", Ty::BOOLEAN)] {
            if set.within(rest) {
                parts.push(name);
                rest = Ty(rest.0 & !set.0);
            }
        }
        for (i, n) in NAMES.iter().enumerate() {
            if rest.0 & (1 << i) != 0 {
                parts.push(n);
            }
        }
        if parts.is_empty() {
            write!(f, "never")
        } else {
            write!(f, "{}", parts.join("|"))
        }
    }
}

/// Result type of an arithmetic operator on `to_num`-ed operand types.
pub fn arith(op: scrust_ir::Op, l: Ty, r: Ty) -> Ty {
    use scrust_ir::Op;
    let (l, r) = (l.to_num(), r.to_num());
    let l_inf = l.has(Ty::INF);
    let r_inf = r.has(Ty::INF);
    let l_zero = l.has(Ty::ANY_ZERO);
    let r_zero = r.has(Ty::ANY_ZERO);
    let fract = l.has(Ty::FRACT) || r.has(Ty::FRACT);
    // Finite results; overflow to infinity is always possible for non-trivial operands.
    let mut res = Ty::INT | Ty::INF;
    match op {
        Op::Add => {
            if fract {
                res |= Ty::FRACT;
            }
            if l.has(Ty::NEG_ZERO) && r.has(Ty::NEG_ZERO) {
                res |= Ty::NEG_ZERO;
            }
            if (l.has(Ty::POS_INF) && r.has(Ty::NEG_INF)) || (l.has(Ty::NEG_INF) && r.has(Ty::POS_INF)) {
                res |= Ty::NAN;
            }
        }
        Op::Sub => {
            if fract {
                res |= Ty::FRACT;
            }
            if l.has(Ty::NEG_ZERO) && r.has(Ty::ZERO) {
                res |= Ty::NEG_ZERO;
            }
            if (l.has(Ty::POS_INF) && r.has(Ty::POS_INF)) || (l.has(Ty::NEG_INF) && r.has(Ty::NEG_INF)) {
                res |= Ty::NAN;
            }
        }
        Op::Mul => {
            res |= Ty::NEG_ZERO;
            if fract {
                res |= Ty::FRACT;
            }
            if (l_inf && r_zero) || (l_zero && r_inf) {
                res |= Ty::NAN;
            }
        }
        Op::Div => {
            res |= Ty::NEG_ZERO | Ty::FRACT;
            if (l_zero && r_zero) || (l_inf && r_inf) {
                res |= Ty::NAN;
            }
        }
        Op::Mod => {
            res |= Ty::NEG_ZERO;
            if fract {
                res |= Ty::FRACT;
            }
            if r_zero || l_inf {
                res |= Ty::NAN;
            }
        }
        _ => res = Ty::NUMBER_OR_NAN,
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_types() {
        let s = |x: &str| Ty::of_value(&Value::str(x));
        assert_eq!(s("10"), Ty::STR_CINT);
        assert_eq!(s("-3"), Ty::STR_CINT);
        assert_eq!(s("0"), Ty::STR_CINT);
        assert_eq!(s("1.5"), Ty::STR_CFRACT);
        assert_eq!(s("1.0"), Ty::STR_NUM);
        assert_eq!(s("-0"), Ty::STR_NUM);
        assert_eq!(s(" 5"), Ty::STR_NUM);
        assert_eq!(s("1e3"), Ty::STR_NUM);
        assert_eq!(s("0x10"), Ty::STR_NUM);
        assert_eq!(s("Infinity"), Ty::STR_NUM);
        assert_eq!(s("007"), Ty::STR_NUM);
        assert_eq!(s(""), Ty::STR_NAN);
        assert_eq!(s("  "), Ty::STR_NAN);
        assert_eq!(s("abc"), Ty::STR_NAN);
        assert_eq!(s("TRUE"), Ty::STR_BOOL);
        assert_eq!(s("100000000000000000000"), Ty::STR_CINT);
        assert_eq!(s("1000000000000000000000"), Ty::STR_NUM);
        assert_eq!(Ty::of_value(&Value::Num(-0.0)), Ty::NEG_ZERO);
        assert_eq!(Ty::of_value(&Value::Num(10.0)), Ty::POS_INT);
    }
}
