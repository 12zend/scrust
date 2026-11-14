//! Scratch lists with a statically chosen element representation.

use scrust_value::{ci_hash_l1, compare, CompareKey, JsStr, ListIndex, ToNum, ToStr, ToValue, Value};
use std::cell::RefCell;
use std::collections::HashMap;


/// Element representations: `f64`, `bool`, `JsStr` or `Value`.
pub trait Elem: Clone + ToNum + ToStr + ToValue + 'static {
    /// Convert a dynamic value known (by type analysis) to fit this representation.
    fn from_value(v: &Value) -> Self;
    /// `Cast.compare(self, other) === 0`
    fn equals_value(&self, other: &Value) -> bool;
    /// Is `String(self)` one code unit long (TurboWarp's `listContents`)?
    fn is_single_char(&self) -> bool;
    /// `Cast.compare(self, key) === 0`
    fn eq_key(&self, key: &CompareKey) -> bool;
    /// A cheap value for slots before the head of a list (see `List::head`).
    fn placeholder() -> Self;
    /// As `F64E` storage (see `crate::f64e_from_value`).
    #[inline]
    fn to_f64e(&self) -> f64 {
        crate::f64e_from_value(&self.to_value())
    }
}

impl Elem for Value {
    #[inline]
    fn to_f64e(&self) -> f64 {
        crate::f64e_from_value(self)
    }
    #[inline]
    fn placeholder() -> Self {
        Value::Num(0.0)
    }
    #[inline]
    fn eq_key(&self, key: &CompareKey) -> bool {
        key.eq_value(self)
    }
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
        match self {
            Value::Str(s) => s.len() == 1,
            Value::Num(n) => n.is_single_char(),
            Value::Bool(_) => false,
        }
    }
}
impl Elem for f64 {
    #[inline]
    fn to_f64e(&self) -> f64 {
        if self.is_nan() {
            f64::NAN
        } else {
            *self
        }
    }
    #[inline]
    fn placeholder() -> Self {
        0.0
    }
    #[inline]
    fn eq_key(&self, key: &CompareKey) -> bool {
        key.eq_num(*self)
    }
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
        // "0".."9" (and -0, which prints as "0").
        *self >= 0.0 && *self <= 9.0 && *self == self.trunc()
    }
}
impl Elem for bool {
    #[inline]
    fn placeholder() -> Self {
        false
    }
    #[inline]
    fn eq_key(&self, key: &CompareKey) -> bool {
        key.eq_bool(*self)
    }
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
    fn to_f64e(&self) -> f64 {
        crate::f64e_from_str(self)
    }
    #[inline]
    fn placeholder() -> Self {
        JsStr::empty()
    }
    #[inline]
    fn eq_key(&self, key: &CompareKey) -> bool {
        key.eq_str(self)
    }
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

/// A list item that is a number or `""` (`Repr::F64E`: `""` is the NaN
/// `F64E_EMPTY`).
#[derive(Clone, Copy, Debug)]
pub struct F64E(pub f64);

impl ToNum for F64E {
    #[inline(always)]
    fn to_num(&self) -> f64 {
        if self.0.is_nan() {
            0.0
        } else {
            self.0
        }
    }
    #[inline(always)]
    fn js_number(&self) -> f64 {
        crate::f64e_num(self.0)
    }
}
impl ToStr for F64E {
    #[inline]
    fn to_str(&self) -> JsStr {
        crate::f64e_str(self.0)
    }
}
impl ToValue for F64E {
    #[inline]
    fn to_value(&self) -> Value {
        crate::f64e_val(self.0)
    }
}
impl Elem for F64E {
    #[inline]
    fn to_f64e(&self) -> f64 {
        self.0
    }
    #[inline]
    fn placeholder() -> Self {
        F64E(0.0)
    }
    #[inline]
    fn eq_key(&self, key: &CompareKey) -> bool {
        match self.to_value() {
            Value::Num(n) => key.eq_num(n),
            v => key.eq_value(&v),
        }
    }
    #[inline]
    fn from_value(v: &Value) -> Self {
        F64E(crate::f64e_from_value(v))
    }
    #[inline]
    fn equals_value(&self, other: &Value) -> bool {
        compare(&self.to_value(), other) == 0.0
    }
    #[inline]
    fn is_single_char(&self) -> bool {
        match self.to_value() {
            Value::Num(n) => n.is_single_char(),
            _ => false,
        }
    }
}

/// A list's initial item as generated code stores it: plain static data, so
/// that large lists cost the compiler nothing beyond the bytes themselves.
#[derive(Clone, Copy)]
pub enum Lit {
    N(f64),
    S(&'static str),
    /// A string with lone surrogates (not representable as `&str`).
    U(&'static [u16]),
    B(bool),
}

impl Lit {
    pub fn to_value(self) -> Value {
        match self {
            Lit::N(n) => Value::Num(n),
            Lit::S(s) => Value::Str(JsStr::from(s)),
            Lit::U(u) => Value::Str(JsStr::from_utf16(u)),
            Lit::B(b) => Value::Bool(b),
        }
    }
}

impl<T: Elem> List<T> {
    /// A list from static initial items, converted as `Elem::from_value` does.
    pub fn from_lits(lits: &[Lit]) -> Self {
        List::new(lits.iter().map(|l| T::from_value(&l.to_value())).collect())
    }
}

impl List<f64> {
    pub fn from_nums(nums: &[f64]) -> Self {
        List::new(nums.to_vec())
    }
}

pub struct List<T: Elem> {
    /// The items are `buf[head..]`. Deleting the first item only advances
    /// `head` (V8 left-trims arrays on `splice(0, 1)` the same way), so a
    /// list used as a queue is not shifted on every removal.
    buf: Vec<T>,
    head: usize,
    /// `buf[head..]` as a raw slice, kept up to date after every change of
    /// `buf` / `head`, so that reading an item costs what it costs in a `Vec`.
    ptr: *mut T,
    len: usize,
    /// Hash index for `item # of` / `contains` with text keys (see `TextIndex`).
    index: RefCell<IndexState>,
}

impl<T: Elem> Clone for List<T> {
    fn clone(&self) -> Self {
        List::new(self.items().to_vec())
    }
}

impl<T: Elem + std::fmt::Debug> std::fmt::Debug for List<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("List").field("items", &self.items()).finish()
    }
}

impl<T: Elem> Default for List<T> {
    fn default() -> Self {
        List::new(Vec::new())
    }
}

/// Lists shorter than this are always scanned.
const INDEX_MIN_LEN: usize = 16;
/// Linear text-key scans after a change before the index is (re)built: a
/// list changed between every two lookups never pays for building it.
const INDEX_AFTER_SCANS: u32 = 3;

enum IndexState {
    None { scans: u32 },
    Built(Box<TextIndex>),
}

/// `compare(item, key) === 0` for a text key (`CompareKey::is_text`) is a
/// case-insensitive comparison of `String(item)` with the key. The index maps
/// the hash of every item's lower-cased string (`ci_hash_l1`) to the first
/// position with that hash; a hit is confirmed with the real comparison, and
/// a hash nobody has means "not in the list". Usable only while every item's
/// string is Latin-1 (`exotic` otherwise: a non-Latin-1 string can still
/// equal a Latin-1 key case-insensitively).
struct TextIndex {
    map: HashMap<u64, u32, std::hash::BuildHasherDefault<IdentityHasher>>,
    exotic: bool,
}

/// The keys are already well-mixed hashes.
#[derive(Default)]
struct IdentityHasher(u64);
impl std::hash::Hasher for IdentityHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 << 8) | b as u64;
        }
    }
    #[inline]
    fn write_u64(&mut self, n: u64) {
        self.0 = n;
    }
}

impl TextIndex {
    fn build<T: Elem>(items: &[T]) -> TextIndex {
        let mut ix = TextIndex { map: HashMap::with_capacity_and_hasher(items.len(), Default::default()), exotic: false };
        for (i, x) in items.iter().enumerate() {
            ix.note(i, x);
            if ix.exotic {
                ix.map = Default::default();
                break;
            }
        }
        ix
    }

    /// Record item `x` at position `i` (positions are noted in increasing order).
    #[inline]
    fn note<T: Elem>(&mut self, i: usize, x: &T) {
        if self.exotic {
            return;
        }
        match ci_hash_l1(&x.to_str()) {
            Some(h) => {
                self.map.entry(h).or_insert(i as u32);
            }
            None => self.exotic = true,
        }
    }
}

impl<T: Elem> List<T> {
    pub fn new(items: Vec<T>) -> Self {
        let mut l = List { buf: items, head: 0, ptr: std::ptr::null_mut(), len: 0, index: RefCell::new(IndexState::None { scans: 0 }) };
        l.sync();
        l
    }

    /// Refresh `ptr` / `len` after `buf` or `head` changed.
    #[inline]
    fn sync(&mut self) {
        debug_assert!(self.head <= self.buf.len());
        // SAFETY: `head <= buf.len()`.
        self.ptr = unsafe { self.buf.as_mut_ptr().add(self.head) };
        self.len = self.buf.len() - self.head;
    }

    /// The items, first to last.
    #[inline]
    pub fn items(&self) -> &[T] {
        // SAFETY: `ptr` / `len` describe `buf[head..]` (see `sync`).
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }

    #[inline]
    fn items_mut(&mut self) -> &mut [T] {
        // SAFETY: as in `items`; `&mut self` makes it exclusive.
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
    }

    /// The list changed in a way the index does not follow.
    #[inline]
    fn invalidate(&mut self) {
        let ix = self.index.get_mut();
        match ix {
            IndexState::None { scans: 0 } => {}
            _ => *ix = IndexState::None { scans: 0 },
        }
    }

    /// 0-based position of the first item equal to `key`, through the index
    /// when the key is text; `Err(())` when the caller must scan.
    fn find_indexed(&self, key: &CompareKey) -> Result<Option<usize>, ()> {
        if !key.is_text() || self.len() < INDEX_MIN_LEN {
            return Err(());
        }
        let mut st = self.index.borrow_mut();
        if let IndexState::None { scans } = &mut *st {
            *scans += 1;
            if *scans <= INDEX_AFTER_SCANS {
                return Err(());
            }
            *st = IndexState::Built(Box::new(TextIndex::build(self.items())));
        }
        let IndexState::Built(ix) = &*st else { return Err(()) };
        if ix.exotic {
            return Err(());
        }
        let Some(h) = ci_hash_l1(key.text()) else { return Err(()) };
        match ix.map.get(&h) {
            None => Ok(None),
            Some(&i) if self.items()[i as usize].eq_key(key) => Ok(Some(i as usize)),
            // A different string with the same hash comes first.
            Some(_) => Err(()),
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// `data_addtolist` (TurboWarp has no 200,000 item limit).
    #[inline]
    pub fn add(&mut self, v: T) {
        if let IndexState::Built(ix) = self.index.get_mut() {
            ix.note(self.len, &v);
        }
        self.buf.push(v);
        self.sync();
    }

    /// `data_deleteoflist`
    pub fn delete(&mut self, idx: ListIndex) {
        match idx {
            ListIndex::Invalid => {}
            ListIndex::All => self.delete_all(),
            ListIndex::Index(1) => {
                let h = self.head;
                self.buf[h] = T::placeholder();
                self.head = h + 1;
                if self.head == self.buf.len() {
                    self.buf.clear();
                    self.head = 0;
                } else if self.head >= 32 && self.head * 2 >= self.buf.len() {
                    self.buf.drain(..self.head);
                    self.head = 0;
                }
                self.sync();
                self.invalidate();
            }
            ListIndex::Index(i) => {
                self.buf.remove(self.head + i - 1);
                self.sync();
                self.invalidate();
            }
        }
    }

    #[inline]
    pub fn delete_all(&mut self) {
        self.buf.clear();
        self.head = 0;
        self.sync();
        if let IndexState::Built(ix) = self.index.get_mut() {
            ix.map.clear();
            ix.exotic = false;
        }
    }

    /// `data_insertatlist` (`idx` computed against `len + 1`).
    pub fn insert(&mut self, idx: ListIndex, v: T) {
        let ListIndex::Index(i) = idx else { return };
        if i - 1 == self.len() {
            self.add(v);
            return;
        }
        self.buf.insert(self.head + i - 1, v);
        self.sync();
        self.invalidate();
    }

    /// `data_replaceitemoflist`
    #[inline]
    pub fn replace(&mut self, idx: ListIndex, v: T) {
        if let ListIndex::Index(i) = idx {
            self.items_mut()[i - 1] = v;
            self.invalidate();
        }
    }

    /// `data_itemoflist`: raw value, `""` when the index is invalid.
    #[inline]
    pub fn item(&self, idx: ListIndex) -> Value {
        match idx {
            ListIndex::Index(i) => self.items()[i - 1].to_value(),
            _ => Value::Str(JsStr::empty()),
        }
    }

    /// `Cast.toNumber(item)`
    #[inline]
    pub fn item_num(&self, idx: ListIndex) -> f64 {
        match idx {
            ListIndex::Index(i) => self.items()[i - 1].to_num(),
            _ => 0.0,
        }
    }

    /// The raw item as `F64E` (`""` when the index is invalid); the list
    /// holds only numbers, canonical numeric strings and `""` (by type analysis).
    #[inline]
    pub fn item_f64e(&self, idx: ListIndex) -> f64 {
        match idx {
            ListIndex::Index(i) => self.items()[i - 1].to_f64e(),
            _ => crate::F64E_EMPTY,
        }
    }

    /// `Cast.toString(item)`
    #[inline]
    pub fn item_str(&self, idx: ListIndex) -> JsStr {
        match idx {
            ListIndex::Index(i) => self.items()[i - 1].to_str(),
            _ => JsStr::empty(),
        }
    }

    /// `data_itemnumoflist`
    pub fn index_of(&self, v: &Value) -> f64 {
        let key = CompareKey::new(v);
        let found = match self.find_indexed(&key) {
            Ok(f) => f,
            Err(()) => self.items().iter().position(|x| x.eq_key(&key)),
        };
        match found {
            Some(i) => (i + 1) as f64,
            None => 0.0,
        }
    }

    /// `data_listcontainsitem` (the identity pre-check is subsumed by compare).
    pub fn contains(&self, v: &Value) -> bool {
        let key = CompareKey::new(v);
        match self.find_indexed(&key) {
            Ok(f) => f.is_some(),
            Err(()) => self.items().iter().any(|x| x.eq_key(&key)),
        }
    }

    /// `data_listcontents`
    pub fn contents(&self) -> JsStr {
        let all_single = self.items().iter().all(|x| x.is_single_char());
        let mut units: Vec<u16> = Vec::new();
        for (i, v) in self.items().iter().enumerate() {
            if i > 0 && !all_single {
                units.push(b' ' as u16);
            }
            units.extend(v.to_str().flat().units());
        }
        JsStr::from_utf16(&units)
    }

    pub fn values(&self) -> Vec<Value> {
        self.items().iter().map(|x| x.to_value()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Random edits and lookups against a plain vector scanned with `compare`.
    #[test]
    fn index_and_head_match_a_plain_scan() {
        let words = ["apple", "APPLE", "Banana", "banana ", "1", "1.0", " 1", "NaN", "nan", "Infinity", "infinity", "", "true", "TRUE", "é", "É", "ǅ", "k", "\u{212a}", "0x10", "16", "x"];
        let mut s: u64 = 0x1234_5678_9abc_def1;
        let mut next = move || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        let pick = |r: u64| -> Value {
            match r % 5 {
                0 => Value::Num([1.0, 16.0, f64::NAN, f64::INFINITY, -0.0, 2.5][(r / 5 % 6) as usize]),
                1 => Value::Bool(r / 5 % 2 == 0),
                _ => Value::str(words[(r / 5 % words.len() as u64) as usize]),
            }
        };
        for round in 0..40 {
            let mut list: List<Value> = List::new(Vec::new());
            let mut plain: Vec<Value> = Vec::new();
            let exotic = round % 4 == 0;
            for _ in 0..3000 {
                let r = next();
                let n = plain.len();
                match r % 16 {
                    0..=5 => {
                        let mut v = pick(next());
                        if !exotic {
                            if let Value::Str(t) = &v {
                                if !matches!(t.flat(), scrust_value::FlatRef::L1(_)) {
                                    v = Value::str("plain");
                                }
                            }
                        }
                        list.add(v.clone());
                        plain.push(v);
                    }
                    6 | 7 if n > 0 => {
                        list.delete(ListIndex::Index(1));
                        plain.remove(0);
                    }
                    8 if n > 0 => {
                        let i = (next() % n as u64) as usize;
                        list.delete(ListIndex::Index(i + 1));
                        plain.remove(i);
                    }
                    9 => {
                        let i = (next() % (n as u64 + 1)) as usize;
                        let v = pick(next());
                        list.insert(ListIndex::Index(i + 1), v.clone());
                        plain.insert(i, v);
                    }
                    10 if n > 0 => {
                        let i = (next() % n as u64) as usize;
                        let v = pick(next());
                        list.replace(ListIndex::Index(i + 1), v.clone());
                        plain[i] = v;
                    }
                    11 if next() % 50 == 0 => {
                        list.delete_all();
                        plain.clear();
                    }
                    _ => {
                        let key = pick(next());
                        let want = plain.iter().position(|x| compare(x, &key) == 0.0).map_or(0.0, |i| (i + 1) as f64);
                        assert_eq!(list.index_of(&key), want, "index_of({key:?})");
                        assert_eq!(list.contains(&key), want > 0.0);
                    }
                }
                assert_eq!(list.len(), plain.len());
            }
            assert!(list.items().iter().zip(&plain).all(|(a, b)| a.same(b)));
        }
    }
}
