//! The dynamic `Value` and the Scratch casts (`Cast.*` in scratch-vm).

use crate::jsstr::JsStr;
use crate::num::{js_is_int_number, js_num_to_str, js_string_to_number};
use std::cmp::Ordering;

/// A Scratch value: number, string or boolean. 16 bytes.
#[derive(Clone, Debug)]
pub enum Value {
    Num(f64),
    Str(JsStr),
    Bool(bool),
}

impl Default for Value {
    fn default() -> Self {
        Value::Num(0.0)
    }
}

impl From<f64> for Value {
    #[inline]
    fn from(n: f64) -> Value {
        Value::Num(n)
    }
}
impl From<bool> for Value {
    #[inline]
    fn from(b: bool) -> Value {
        Value::Bool(b)
    }
}
impl From<JsStr> for Value {
    #[inline]
    fn from(s: JsStr) -> Value {
        Value::Str(s)
    }
}
impl From<&str> for Value {
    fn from(s: &str) -> Value {
        Value::Str(JsStr::from(s))
    }
}

impl Value {
    pub fn str(s: &str) -> Value {
        Value::Str(JsStr::from(s))
    }

    /// Strict identity (JS `===` except that NaN is identical to NaN and
    /// +0/-0 are distinguished): used by tests and trace comparison.
    pub fn same(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Num(a), Value::Num(b)) => a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan()),
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            _ => false,
        }
    }

    /// JS strict equality `===` (as used by `Array.prototype.indexOf`).
    pub fn strict_eq(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Num(a), Value::Num(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// Typed cast traits: the code generator picks the impl statically.

/// `Cast.toNumber`: NaN becomes 0.
pub trait ToNum {
    fn to_num(&self) -> f64;
    /// Raw JS `Number(v)`: NaN is preserved.
    fn js_number(&self) -> f64;
}
/// `Cast.toString`.
pub trait ToStr {
    fn to_str(&self) -> JsStr;
}
/// `Cast.toBoolean`.
pub trait ToBool {
    fn to_bool(&self) -> bool;
}
/// Conversion to a dynamic `Value`.
pub trait ToValue {
    fn to_value(&self) -> Value;
}

impl ToNum for f64 {
    #[inline(always)]
    fn to_num(&self) -> f64 {
        if self.is_nan() {
            0.0
        } else {
            *self
        }
    }
    #[inline(always)]
    fn js_number(&self) -> f64 {
        *self
    }
}
impl ToNum for bool {
    #[inline(always)]
    fn to_num(&self) -> f64 {
        *self as u8 as f64
    }
    #[inline(always)]
    fn js_number(&self) -> f64 {
        *self as u8 as f64
    }
}
impl ToNum for JsStr {
    #[inline]
    fn to_num(&self) -> f64 {
        let n = js_string_to_number(self);
        if n.is_nan() {
            0.0
        } else {
            n
        }
    }
    #[inline]
    fn js_number(&self) -> f64 {
        js_string_to_number(self)
    }
}
impl ToNum for Value {
    #[inline]
    fn to_num(&self) -> f64 {
        match self {
            Value::Num(n) => n.to_num(),
            Value::Str(s) => s.to_num(),
            Value::Bool(b) => b.to_num(),
        }
    }
    #[inline]
    fn js_number(&self) -> f64 {
        match self {
            Value::Num(n) => *n,
            Value::Str(s) => js_string_to_number(s),
            Value::Bool(b) => b.to_num(),
        }
    }
}

thread_local! {
    static TRUE_S: JsStr = JsStr::from("true");
    static FALSE_S: JsStr = JsStr::from("false");
}

impl ToStr for f64 {
    #[inline]
    fn to_str(&self) -> JsStr {
        js_num_to_str(*self)
    }
}
impl ToStr for bool {
    #[inline]
    fn to_str(&self) -> JsStr {
        if *self {
            TRUE_S.with(|s| s.clone())
        } else {
            FALSE_S.with(|s| s.clone())
        }
    }
}
impl ToStr for JsStr {
    #[inline(always)]
    fn to_str(&self) -> JsStr {
        self.clone()
    }
}
impl ToStr for Value {
    #[inline]
    fn to_str(&self) -> JsStr {
        match self {
            Value::Num(n) => n.to_str(),
            Value::Str(s) => s.clone(),
            Value::Bool(b) => b.to_str(),
        }
    }
}

impl ToBool for f64 {
    #[inline(always)]
    fn to_bool(&self) -> bool {
        !(*self == 0.0 || self.is_nan())
    }
}
impl ToBool for bool {
    #[inline(always)]
    fn to_bool(&self) -> bool {
        *self
    }
}
impl ToBool for JsStr {
    #[inline]
    fn to_bool(&self) -> bool {
        match self.len() {
            0 => false,
            1 => self.code_unit(0) != b'0' as u16,
            5 => !self.to_lower().eq_str("false"),
            _ => true,
        }
    }
}
impl ToBool for Value {
    #[inline]
    fn to_bool(&self) -> bool {
        match self {
            Value::Num(n) => n.to_bool(),
            Value::Str(s) => s.to_bool(),
            Value::Bool(b) => *b,
        }
    }
}

impl ToValue for f64 {
    #[inline(always)]
    fn to_value(&self) -> Value {
        Value::Num(*self)
    }
}
impl ToValue for bool {
    #[inline(always)]
    fn to_value(&self) -> Value {
        Value::Bool(*self)
    }
}
impl ToValue for JsStr {
    #[inline(always)]
    fn to_value(&self) -> Value {
        Value::Str(self.clone())
    }
}
impl ToValue for Value {
    #[inline(always)]
    fn to_value(&self) -> Value {
        self.clone()
    }
}

#[inline]
pub fn to_number(v: &Value) -> f64 {
    v.to_num()
}
#[inline]
pub fn to_string(v: &Value) -> JsStr {
    v.to_str()
}
#[inline]
pub fn to_boolean(v: &Value) -> bool {
    v.to_bool()
}

/// `Cast.isWhiteSpace` (the value is a string that trims to empty).
#[inline]
pub fn is_whitespace(v: &Value) -> bool {
    match v {
        Value::Str(s) => s.is_whitespace(),
        _ => false,
    }
}

fn ord_to_f64(o: Ordering) -> f64 {
    match o {
        Ordering::Less => -1.0,
        Ordering::Equal => 0.0,
        Ordering::Greater => 1.0,
    }
}

/// `Cast.compare`: negative / zero / positive.
pub fn compare(a: &Value, b: &Value) -> f64 {
    if let (Value::Num(x), Value::Num(y)) = (a, b) {
        if !x.is_nan() && !y.is_nan() {
            return if x == y { 0.0 } else { x - y };
        }
    }
    let mut n1 = a.js_number();
    let mut n2 = b.js_number();
    if n1 == 0.0 && is_whitespace(a) {
        n1 = f64::NAN;
    } else if n2 == 0.0 && is_whitespace(b) {
        n2 = f64::NAN;
    }
    if n1.is_nan() || n2.is_nan() {
        let s1 = a.to_str().to_lower();
        let s2 = b.to_str().to_lower();
        return ord_to_f64(s1.cmp_utf16(&s2));
    }
    if n1 == n2 {
        // Covers the +/-Infinity special case (inf - inf would be NaN).
        return 0.0;
    }
    n1 - n2
}

/// Case-insensitive string comparison branch of `Cast.compare`, for callers
/// that have statically proven the comparison is a string comparison.
pub fn compare_str_ci(a: &JsStr, b: &JsStr) -> f64 {
    ord_to_f64(a.to_lower().cmp_utf16(&b.to_lower()))
}

#[inline]
pub fn equals(a: &Value, b: &Value) -> bool {
    compare(a, b) == 0.0
}
#[inline]
pub fn less_than(a: &Value, b: &Value) -> bool {
    compare(a, b) < 0.0
}
#[inline]
pub fn greater_than(a: &Value, b: &Value) -> bool {
    compare(a, b) > 0.0
}

/// `Cast.isInt`.
pub fn is_int(v: &Value) -> bool {
    match v {
        Value::Num(n) => js_is_int_number(*n),
        Value::Bool(_) => true,
        Value::Str(s) => !s.contains_unit(b'.' as u16),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListIndex {
    /// 1-based index.
    Index(usize),
    All,
    Invalid,
}

/// `Cast.toListIndex`. `rand` is only invoked for `"random"` / `"any"` on a
/// non-empty list.
pub fn to_list_index(v: &Value, len: usize, accept_all: bool, rand: &mut dyn FnMut() -> f64) -> ListIndex {
    if let Value::Str(s) = v {
        if s.eq_str("all") {
            return if accept_all { ListIndex::All } else { ListIndex::Invalid };
        }
        if s.eq_str("last") {
            return if len > 0 { ListIndex::Index(len) } else { ListIndex::Invalid };
        }
        if s.eq_str("random") || s.eq_str("any") {
            return if len > 0 {
                ListIndex::Index(1 + (rand() * len as f64).floor() as usize)
            } else {
                ListIndex::Invalid
            };
        }
    }
    if let Value::Bool(_) = v {
        // typeof index !== 'number' but none of the keywords: falls through.
    }
    num_to_list_index(v.to_num(), len)
}

/// The numeric tail of `Cast.toListIndex`.
#[inline]
pub fn num_to_list_index(n: f64, len: usize) -> ListIndex {
    let idx = n.floor();
    if idx < 1.0 || idx > len as f64 {
        ListIndex::Invalid
    } else {
        ListIndex::Index(idx as usize)
    }
}

/// `Cast.compare(a, b) === 0` for two numbers. NaN stringifies to "NaN",
/// which equals itself and sorts after every other number's string form.
#[inline(always)]
pub fn num_eq(a: f64, b: f64) -> bool {
    a == b || (a.is_nan() && b.is_nan())
}
/// `Cast.compare(a, b) < 0` for two numbers (NaN is the greatest).
#[inline(always)]
pub fn num_lt(a: f64, b: f64) -> bool {
    a < b || (b.is_nan() && !a.is_nan())
}
/// `Cast.compare(a, b) > 0` for two numbers (NaN is the greatest).
#[inline(always)]
pub fn num_gt(a: f64, b: f64) -> bool {
    a > b || (a.is_nan() && !b.is_nan())
}
