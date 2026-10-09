//! Scratch lists with a statically chosen element representation.

use scrust_value::{compare, JsStr, ListIndex, ToNum, ToStr, ToValue, Value};

pub const LIST_ITEM_LIMIT: usize = 200_000;

/// Element representations: `f64`, `bool`, `JsStr` or `Value`.
pub trait Elem: Clone + ToNum + ToStr + ToValue + 'static {
    /// Convert a dynamic value known (by type analysis) to fit this representation.
    fn from_value(v: &Value) -> Self;
    /// `Cast.compare(self, other) === 0`
    fn equals_value(&self, other: &Value) -> bool;
    /// Is this a *string* of length one (for `data_listcontents`)?
    fn is_single_char(&self) -> bool;
}

impl Elem for Value {
    #[inline]
    fn from_value(v: &Value) -> Self {
        v.clone()
    }
    #[inline]
    fn equals_value(&self, other: &Value) -> bool {
        compare(self, other) == 0.0
    }
    #[inline]
    fn is_single_char(&self) -> bool {
        matches!(self, Value::Str(s) if s.len() == 1)
    }
}
impl Elem for f64 {
    #[inline]
    fn from_value(v: &Value) -> Self {
        match v {
            Value::Num(n) => *n,
            // Canonical numeric strings may be stored as numbers (§8.6).
            other => other.js_number(),
        }
    }
    #[inline]
    fn equals_value(&self, other: &Value) -> bool {
        match other {
            Value::Num(o) if !o.is_nan() && !self.is_nan() => self == o,
            _ => compare(&Value::Num(*self), other) == 0.0,
        }
    }
    #[inline]
    fn is_single_char(&self) -> bool {
        false
    }
}
impl Elem for bool {
    #[inline]
    fn from_value(v: &Value) -> Self {
        matches!(v, Value::Bool(true))
    }
    #[inline]
    fn equals_value(&self, other: &Value) -> bool {
        compare(&Value::Bool(*self), other) == 0.0
    }
    #[inline]
    fn is_single_char(&self) -> bool {
        false
    }
}
impl Elem for JsStr {
    #[inline]
    fn from_value(v: &Value) -> Self {
        v.to_str()
    }
    #[inline]
    fn equals_value(&self, other: &Value) -> bool {
        compare(&Value::Str(self.clone()), other) == 0.0
    }
    #[inline]
    fn is_single_char(&self) -> bool {
        self.len() == 1
    }
}

#[derive(Clone, Debug, Default)]
pub struct List<T: Elem> {
    pub items: Vec<T>,
}

impl<T: Elem> List<T> {
    pub fn new(items: Vec<T>) -> Self {
        List { items }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// `data_addtolist`
    #[inline]
    pub fn add(&mut self, v: T) {
        if self.items.len() < LIST_ITEM_LIMIT {
            self.items.push(v);
        }
    }

    /// `data_deleteoflist`
    pub fn delete(&mut self, idx: ListIndex) {
        match idx {
            ListIndex::Invalid => {}
            ListIndex::All => self.items.clear(),
            ListIndex::Index(i) => {
                self.items.remove(i - 1);
            }
        }
    }

    #[inline]
    pub fn delete_all(&mut self) {
        self.items.clear();
    }

    /// `data_insertatlist` (`idx` computed against `len + 1`).
    pub fn insert(&mut self, idx: ListIndex, v: T) {
        let ListIndex::Index(i) = idx else { return };
        if i > LIST_ITEM_LIMIT {
            return;
        }
        self.items.insert(i - 1, v);
        if self.items.len() > LIST_ITEM_LIMIT {
            self.items.pop();
        }
    }

    /// `data_replaceitemoflist`
    #[inline]
    pub fn replace(&mut self, idx: ListIndex, v: T) {
        if let ListIndex::Index(i) = idx {
            self.items[i - 1] = v;
        }
    }

    /// `data_itemoflist`: raw value, `""` when the index is invalid.
    #[inline]
    pub fn item(&self, idx: ListIndex) -> Value {
        match idx {
            ListIndex::Index(i) => self.items[i - 1].to_value(),
            _ => Value::Str(JsStr::empty()),
        }
    }

    /// `Cast.toNumber(item)`
    #[inline]
    pub fn item_num(&self, idx: ListIndex) -> f64 {
        match idx {
            ListIndex::Index(i) => self.items[i - 1].to_num(),
            _ => 0.0,
        }
    }

    /// `Cast.toString(item)`
    #[inline]
    pub fn item_str(&self, idx: ListIndex) -> JsStr {
        match idx {
            ListIndex::Index(i) => self.items[i - 1].to_str(),
            _ => JsStr::empty(),
        }
    }

    /// `data_itemnumoflist`
    pub fn index_of(&self, v: &Value) -> f64 {
        match self.items.iter().position(|x| x.equals_value(v)) {
            Some(i) => (i + 1) as f64,
            None => 0.0,
        }
    }

    /// `data_listcontainsitem` (the identity pre-check is subsumed by compare).
    pub fn contains(&self, v: &Value) -> bool {
        self.items.iter().any(|x| x.equals_value(v))
    }

    /// `data_listcontents`
    pub fn contents(&self) -> JsStr {
        let all_single = self.items.iter().all(|x| x.is_single_char());
        let mut units: Vec<u16> = Vec::new();
        for (i, v) in self.items.iter().enumerate() {
            if i > 0 && !all_single {
                units.push(b' ' as u16);
            }
            units.extend(v.to_str().flat().units());
        }
        JsStr::from_utf16(&units)
    }

    pub fn values(&self) -> Vec<Value> {
        self.items.iter().map(|x| x.to_value()).collect()
    }
}
