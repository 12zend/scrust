//! JS `toLowerCase()` / `toUpperCase()` (Unicode default case mapping,
//! including SpecialCasing and the final-sigma rule). Lone surrogates pass through.

use crate::jsstr::{FlatRef, JsStr};

#[inline]
fn l1_lower(c: u8) -> u8 {
    match c {
        b'A'..=b'Z' => c + 32,
        0xC0..=0xDE if c != 0xD7 => c + 32,
        _ => c,
    }
}

pub fn to_lower(s: &JsStr) -> JsStr {
    match s.flat() {
        FlatRef::L1(b) => {
            if b.iter().all(|&c| l1_lower(c) == c) {
                return s.clone();
            }
            JsStr::from_latin1_vec(b.iter().map(|&c| l1_lower(c)).collect())
        }
        FlatRef::U16(u) => map_utf16(u, |seg| seg.to_lowercase()),
    }
}

pub fn to_upper(s: &JsStr) -> JsStr {
    match s.flat() {
        FlatRef::L1(b) => {
            if b.is_ascii() {
                if !b.iter().any(|c| c.is_ascii_lowercase()) {
                    return s.clone();
                }
                return JsStr::from_latin1_vec(b.to_ascii_uppercase());
            }
            // ß -> SS, ÿ -> Ÿ, µ -> Μ leave Latin-1: go through the general path.
            let st: String = b.iter().map(|&c| c as char).collect();
            JsStr::from(st.to_uppercase())
        }
        FlatRef::U16(u) => map_utf16(u, |seg| seg.to_uppercase()),
    }
}

/// Apply a `str` case mapping to maximal well-formed segments, leaving lone
/// surrogates untouched. A lone surrogate is neither cased nor case-ignorable,
/// so splitting there does not change the final-sigma context.
fn map_utf16(u: &[u16], f: impl Fn(&str) -> String) -> JsStr {
    let mut out: Vec<u16> = Vec::with_capacity(u.len());
    let mut seg = String::new();
    for r in char::decode_utf16(u.iter().copied()) {
        match r {
            Ok(c) => seg.push(c),
            Err(e) => {
                if !seg.is_empty() {
                    out.extend(f(&seg).encode_utf16());
                    seg.clear();
                }
                out.push(e.unpaired_surrogate());
            }
        }
    }
    if !seg.is_empty() {
        out.extend(f(&seg).encode_utf16());
    }
    JsStr::from_utf16(&out)
}
