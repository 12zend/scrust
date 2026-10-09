//! `JsStr`: an immutable, reference-counted string with JavaScript semantics.
//!
//! * Indexed by UTF-16 code unit; may contain lone surrogates.
//! * Latin-1 / UTF-16 hybrid storage (like V8 one-byte / two-byte strings).
//! * Concatenation builds a rope which is flattened lazily, once.
//! * `JsStr` is a thin pointer, so `Value` fits in 16 bytes.

use std::cell::{OnceCell, RefCell};
use std::cmp::Ordering;
use std::fmt;
use std::rc::Rc;

/// Concatenations at or below this length are copied eagerly instead of
/// building a rope node.
const ROPE_MIN: usize = 24;

enum Flat {
    L1(Box<[u8]>),
    U16(Box<[u16]>),
}

/// Borrowed view of flattened string contents.
#[derive(Clone, Copy)]
pub enum FlatRef<'a> {
    L1(&'a [u8]),
    U16(&'a [u16]),
}

impl<'a> FlatRef<'a> {
    #[inline]
    pub fn len(&self) -> usize {
        match self {
            FlatRef::L1(s) => s.len(),
            FlatRef::U16(s) => s.len(),
        }
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    #[inline]
    pub fn at(&self, i: usize) -> u16 {
        match self {
            FlatRef::L1(s) => s[i] as u16,
            FlatRef::U16(s) => s[i],
        }
    }
    pub fn units(self) -> impl Iterator<Item = u16> + 'a {
        let (a, b): (&'a [u8], &'a [u16]) = match self {
            FlatRef::L1(s) => (s, &[]),
            FlatRef::U16(s) => (&[], s),
        };
        a.iter().map(|&c| c as u16).chain(b.iter().copied())
    }
}

struct Cons {
    parts: RefCell<Option<(JsStr, JsStr)>>,
    flat: OnceCell<Flat>,
}

enum Repr {
    Flat(Flat),
    Cons(Cons),
}

struct Node {
    len: usize,
    /// True when every code unit is <= 0xFF.
    l1: bool,
    repr: Repr,
}

impl Drop for Node {
    fn drop(&mut self) {
        // Ropes can be very deep (left-leaning chains built by `join` in a loop):
        // tear them down iteratively.
        let Repr::Cons(c) = &mut self.repr else { return };
        let Some((a, b)) = c.parts.get_mut().take() else { return };
        let mut stack = vec![a, b];
        while let Some(s) = stack.pop() {
            if let Ok(mut node) = Rc::try_unwrap(s.0) {
                if let Repr::Cons(c) = &mut node.repr {
                    if let Some((a, b)) = c.parts.get_mut().take() {
                        stack.push(a);
                        stack.push(b);
                    }
                }
            }
        }
    }
}

#[derive(Clone)]
pub struct JsStr(Rc<Node>);

thread_local! {
    static EMPTY: JsStr = JsStr::new_flat(Flat::L1(Box::new([])));
    static SINGLE: Vec<JsStr> = (0..=255u8).map(|c| JsStr::new_flat(Flat::L1(Box::new([c])))).collect();
}

impl JsStr {
    fn new_flat(f: Flat) -> JsStr {
        let (len, l1) = match &f {
            Flat::L1(s) => (s.len(), true),
            Flat::U16(s) => (s.len(), false),
        };
        JsStr(Rc::new(Node { len, l1, repr: Repr::Flat(f) }))
    }

    #[inline]
    pub fn empty() -> JsStr {
        EMPTY.with(|e| e.clone())
    }

    /// One-code-unit string. Latin-1 units share a static table.
    #[inline]
    pub fn from_unit(u: u16) -> JsStr {
        if u < 256 {
            SINGLE.with(|t| t[u as usize].clone())
        } else {
            JsStr::new_flat(Flat::U16(Box::new([u])))
        }
    }

    pub fn from_latin1(b: &[u8]) -> JsStr {
        match b.len() {
            0 => JsStr::empty(),
            1 => JsStr::from_unit(b[0] as u16),
            _ => JsStr::new_flat(Flat::L1(b.into())),
        }
    }

    pub fn from_latin1_vec(b: Vec<u8>) -> JsStr {
        match b.len() {
            0 => JsStr::empty(),
            1 => JsStr::from_unit(b[0] as u16),
            _ => JsStr::new_flat(Flat::L1(b.into_boxed_slice())),
        }
    }

    /// Build from UTF-16 code units (lone surrogates allowed).
    pub fn from_utf16(u: &[u16]) -> JsStr {
        if u.iter().all(|&c| c < 256) {
            let v: Vec<u8> = u.iter().map(|&c| c as u8).collect();
            JsStr::from_latin1_vec(v)
        } else {
            JsStr::new_flat(Flat::U16(u.into()))
        }
    }

    pub fn from_static(s: &'static str) -> JsStr {
        JsStr::from(s)
    }

    /// Length in UTF-16 code units. O(1).
    #[inline]
    pub fn len(&self) -> usize {
        self.0.len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.len == 0
    }

    #[inline]
    pub fn ptr_eq(&self, other: &JsStr) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    /// Flattened contents (flattens a rope once and caches the result).
    #[inline]
    pub fn flat(&self) -> FlatRef<'_> {
        let f = match &self.0.repr {
            Repr::Flat(f) => f,
            Repr::Cons(c) => c.flat.get_or_init(|| {
                let f = self.flatten();
                // Children are no longer needed.
                let parts = c.parts.borrow_mut().take();
                drop_parts(parts);
                f
            }),
        };
        match f {
            Flat::L1(s) => FlatRef::L1(s),
            Flat::U16(s) => FlatRef::U16(s),
        }
    }

    fn flatten(&self) -> Flat {
        // Iterative in-order traversal.
        let mut stack: Vec<JsStr> = vec![self.clone()];
        if self.0.l1 {
            let mut out: Vec<u8> = Vec::with_capacity(self.0.len);
            while let Some(s) = stack.pop() {
                match s.peek() {
                    Peek::Flat(FlatRef::L1(b)) => out.extend_from_slice(b),
                    Peek::Flat(FlatRef::U16(_)) => unreachable!("l1 rope with u16 leaf"),
                    Peek::Parts(a, b) => {
                        stack.push(b);
                        stack.push(a);
                    }
                }
            }
            Flat::L1(out.into_boxed_slice())
        } else {
            let mut out: Vec<u16> = Vec::with_capacity(self.0.len);
            while let Some(s) = stack.pop() {
                match s.peek() {
                    Peek::Flat(FlatRef::L1(b)) => out.extend(b.iter().map(|&c| c as u16)),
                    Peek::Flat(FlatRef::U16(b)) => out.extend_from_slice(b),
                    Peek::Parts(a, b) => {
                        stack.push(b);
                        stack.push(a);
                    }
                }
            }
            Flat::U16(out.into_boxed_slice())
        }
    }

    fn peek(&self) -> Peek<'_> {
        match &self.0.repr {
            Repr::Flat(Flat::L1(s)) => Peek::Flat(FlatRef::L1(s)),
            Repr::Flat(Flat::U16(s)) => Peek::Flat(FlatRef::U16(s)),
            Repr::Cons(c) => match c.flat.get() {
                Some(Flat::L1(s)) => Peek::Flat(FlatRef::L1(s)),
                Some(Flat::U16(s)) => Peek::Flat(FlatRef::U16(s)),
                None => {
                    let p = c.parts.borrow();
                    let (a, b) = p.as_ref().expect("unflattened rope has parts");
                    Peek::Parts(a.clone(), b.clone())
                }
            },
        }
    }

    #[inline]
    pub fn code_unit(&self, i: usize) -> u16 {
        self.flat().at(i)
    }

    /// `charAt(i)` for an in-range index.
    #[inline]
    pub fn char_at(&self, i: usize) -> JsStr {
        JsStr::from_unit(self.flat().at(i))
    }

    /// Concatenate. Long results become rope nodes (O(1)).
    pub fn concat(&self, other: &JsStr) -> JsStr {
        if other.is_empty() {
            return self.clone();
        }
        if self.is_empty() {
            return other.clone();
        }
        let len = self.0.len + other.0.len;
        let l1 = self.0.l1 && other.0.l1;
        if len <= ROPE_MIN {
            return match (self.flat(), other.flat()) {
                (FlatRef::L1(a), FlatRef::L1(b)) => {
                    let mut v = Vec::with_capacity(len);
                    v.extend_from_slice(a);
                    v.extend_from_slice(b);
                    JsStr::new_flat(Flat::L1(v.into_boxed_slice()))
                }
                (a, b) => {
                    let mut v: Vec<u16> = Vec::with_capacity(len);
                    v.extend(a.units());
                    v.extend(b.units());
                    JsStr::new_flat(Flat::U16(v.into_boxed_slice()))
                }
            };
        }
        JsStr(Rc::new(Node {
            len,
            l1,
            repr: Repr::Cons(Cons {
                parts: RefCell::new(Some((self.clone(), other.clone()))),
                flat: OnceCell::new(),
            }),
        }))
    }

    /// UTF-16 code unit lexicographic comparison (JS `<` on strings).
    pub fn cmp_utf16(&self, other: &JsStr) -> Ordering {
        if self.ptr_eq(other) {
            return Ordering::Equal;
        }
        match (self.flat(), other.flat()) {
            (FlatRef::L1(a), FlatRef::L1(b)) => a.cmp(b),
            (FlatRef::U16(a), FlatRef::U16(b)) => a.cmp(b),
            (a, b) => a.units().cmp(b.units()),
        }
    }

    /// Exact (case-sensitive) equality with an ASCII/Unicode `&str`.
    pub fn eq_str(&self, s: &str) -> bool {
        match self.flat() {
            FlatRef::L1(a) if s.is_ascii() => a == s.as_bytes(),
            f => f.units().eq(s.encode_utf16()),
        }
    }

    /// JS `String.prototype.includes`.
    pub fn includes(&self, needle: &JsStr) -> bool {
        self.index_of(needle).is_some()
    }

    pub fn index_of(&self, needle: &JsStr) -> Option<usize> {
        let n = needle.len();
        if n == 0 {
            return Some(0);
        }
        if n > self.len() {
            return None;
        }
        match (self.flat(), needle.flat()) {
            (FlatRef::L1(h), FlatRef::L1(nd)) => h.windows(n).position(|w| w == nd),
            (FlatRef::U16(h), FlatRef::U16(nd)) => h.windows(n).position(|w| w == nd),
            (FlatRef::L1(_), FlatRef::U16(_)) => None, // needle has a unit > 0xFF
            (FlatRef::U16(h), FlatRef::L1(nd)) => {
                h.windows(n).position(|w| w.iter().zip(nd).all(|(&a, &b)| a == b as u16))
            }
        }
    }

    pub fn contains_unit(&self, u: u16) -> bool {
        match self.flat() {
            FlatRef::L1(s) => u < 256 && s.contains(&(u as u8)),
            FlatRef::U16(s) => s.contains(&u),
        }
    }

    /// JS `toLowerCase()`.
    pub fn to_lower(&self) -> JsStr {
        crate::lower::to_lower(self)
    }

    /// JS `toUpperCase()`.
    pub fn to_upper(&self) -> JsStr {
        crate::lower::to_upper(self)
    }

    /// JS `trim()`.
    pub fn trim(&self) -> JsStr {
        let f = self.flat();
        let n = f.len();
        let mut a = 0;
        while a < n && crate::num::is_js_whitespace(f.at(a)) {
            a += 1;
        }
        let mut b = n;
        while b > a && crate::num::is_js_whitespace(f.at(b - 1)) {
            b -= 1;
        }
        self.substring(a, b)
    }

    pub fn substring(&self, a: usize, b: usize) -> JsStr {
        if a == 0 && b == self.len() {
            return self.clone();
        }
        match self.flat() {
            FlatRef::L1(s) => JsStr::from_latin1(&s[a..b]),
            FlatRef::U16(s) => JsStr::from_utf16(&s[a..b]),
        }
    }

    /// True if the string is empty or only JS whitespace.
    pub fn is_whitespace(&self) -> bool {
        self.flat().units().all(crate::num::is_js_whitespace)
    }

    pub fn to_utf16(&self) -> Vec<u16> {
        self.flat().units().collect()
    }

    /// Lossy conversion for display; lone surrogates become U+FFFD.
    pub fn to_rust_string(&self) -> String {
        match self.flat() {
            FlatRef::L1(s) => s.iter().map(|&c| c as char).collect(),
            FlatRef::U16(s) => String::from_utf16_lossy(s),
        }
    }
}

enum Peek<'a> {
    Flat(FlatRef<'a>),
    Parts(JsStr, JsStr),
}

fn drop_parts(parts: Option<(JsStr, JsStr)>) {
    // Dropping the pair may cascade through `Node::drop`, which is iterative.
    drop(parts);
}

impl From<&str> for JsStr {
    fn from(s: &str) -> JsStr {
        if s.is_ascii() {
            return JsStr::from_latin1(s.as_bytes());
        }
        if s.chars().all(|c| (c as u32) < 256) {
            let v: Vec<u8> = s.chars().map(|c| c as u8).collect();
            return JsStr::from_latin1_vec(v);
        }
        let v: Vec<u16> = s.encode_utf16().collect();
        JsStr::new_flat(Flat::U16(v.into_boxed_slice()))
    }
}

impl From<String> for JsStr {
    fn from(s: String) -> JsStr {
        JsStr::from(s.as_str())
    }
}

impl PartialEq for JsStr {
    fn eq(&self, other: &JsStr) -> bool {
        self.len() == other.len() && self.cmp_utf16(other) == Ordering::Equal
    }
}
impl Eq for JsStr {}

impl PartialEq<str> for JsStr {
    fn eq(&self, other: &str) -> bool {
        self.eq_str(other)
    }
}
impl PartialEq<&str> for JsStr {
    fn eq(&self, other: &&str) -> bool {
        self.eq_str(other)
    }
}

impl std::hash::Hash for JsStr {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        for u in self.flat().units() {
            state.write_u16(u);
        }
    }
}

impl fmt::Display for JsStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_rust_string())
    }
}

impl fmt::Debug for JsStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.to_rust_string())
    }
}

impl Default for JsStr {
    fn default() -> Self {
        JsStr::empty()
    }
}
