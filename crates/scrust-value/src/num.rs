//! ECMAScript number <-> string algorithms and `Math` helpers.
//!
//! `f64::from_str` and `{}` formatting are only used as *subroutines* after the
//! JS grammar has been validated / to obtain shortest round-trip digits.

use crate::jsstr::{FlatRef, JsStr};

/// ECMAScript WhiteSpace + LineTerminator (what `trim()` and `Number()` strip).
#[inline]
pub fn is_js_whitespace(u: u16) -> bool {
    matches!(
        u,
        0x09..=0x0D | 0x20 | 0xA0 | 0x1680 | 0x2000..=0x200A | 0x2028 | 0x2029 | 0x202F | 0x205F | 0x3000 | 0xFEFF
    )
}

/// ECMAScript `StringToNumber` (i.e. `Number(string)`). May return NaN.
#[inline]
pub fn js_string_to_number(s: &JsStr) -> f64 {
    s.cached_number(parse_string_number)
}

fn parse_string_number(s: &JsStr) -> f64 {
    match s.flat() {
        FlatRef::L1(b) => {
            // Fast path: short run of ASCII digits.
            if !b.is_empty() && b.len() <= 15 && b.iter().all(u8::is_ascii_digit) {
                let mut n: u64 = 0;
                for &c in b {
                    n = n * 10 + (c - b'0') as u64;
                }
                return n as f64;
            }
            let mut a = 0;
            let mut e = b.len();
            while a < e && is_js_whitespace(b[a] as u16) {
                a += 1;
            }
            while e > a && is_js_whitespace(b[e - 1] as u16) {
                e -= 1;
            }
            parse_trimmed(&b[a..e])
        }
        FlatRef::U16(u) => {
            let mut a = 0;
            let mut e = u.len();
            while a < e && is_js_whitespace(u[a]) {
                a += 1;
            }
            while e > a && is_js_whitespace(u[e - 1]) {
                e -= 1;
            }
            let t = &u[a..e];
            if t.iter().any(|&c| c >= 0x80) {
                return f64::NAN;
            }
            let v: Vec<u8> = t.iter().map(|&c| c as u8).collect();
            parse_trimmed(&v)
        }
    }
}

/// Parse an already-trimmed byte string per the StringNumericLiteral grammar.
fn parse_trimmed(b: &[u8]) -> f64 {
    if b.is_empty() {
        return 0.0;
    }
    if b.len() > 2 && b[0] == b'0' {
        let radix_bits = match b[1] {
            b'x' | b'X' => 4,
            b'o' | b'O' => 3,
            b'b' | b'B' => 1,
            _ => 0,
        };
        if radix_bits != 0 {
            return parse_pow2_radix(&b[2..], radix_bits);
        }
    }
    let (neg, body) = match b[0] {
        b'+' => (false, &b[1..]),
        b'-' => (true, &b[1..]),
        _ => (false, b),
    };
    if body == b"Infinity" {
        return if neg { f64::NEG_INFINITY } else { f64::INFINITY };
    }
    // StrDecimalLiteral: (digits [. digits?] | . digits) [(e|E) [+-] digits]
    let mut i = 0;
    let n = body.len();
    let int_start = i;
    while i < n && body[i].is_ascii_digit() {
        i += 1;
    }
    let int_digits = i - int_start;
    let mut frac_digits = 0;
    if i < n && body[i] == b'.' {
        i += 1;
        let fs = i;
        while i < n && body[i].is_ascii_digit() {
            i += 1;
        }
        frac_digits = i - fs;
    }
    if int_digits == 0 && frac_digits == 0 {
        return f64::NAN;
    }
    if i < n && (body[i] == b'e' || body[i] == b'E') {
        i += 1;
        if i < n && (body[i] == b'+' || body[i] == b'-') {
            i += 1;
        }
        let es = i;
        while i < n && body[i].is_ascii_digit() {
            i += 1;
        }
        if i == es {
            return f64::NAN;
        }
    }
    if i != n {
        return f64::NAN;
    }
    // Grammar validated: Rust's parser is correctly rounded and accepts this subset.
    // SAFETY-free: bytes are ASCII by construction.
    match core::str::from_utf8(b).ok().and_then(|s| s.parse::<f64>().ok()) {
        Some(v) => v,
        None => f64::NAN,
    }
}

/// Correctly rounded value of a hex / octal / binary digit string (no sign).
fn parse_pow2_radix(digits: &[u8], bits: u32) -> f64 {
    if digits.is_empty() {
        return f64::NAN;
    }
    let mut mant: u64 = 0;
    let mut exp: i32 = 0;
    let mut sticky = false;
    for &c in digits {
        let d = match c {
            b'0'..=b'9' => (c - b'0') as u32,
            b'a'..=b'f' => (c - b'a') as u32 + 10,
            b'A'..=b'F' => (c - b'A') as u32 + 10,
            _ => return f64::NAN,
        };
        if d >= (1 << bits) {
            return f64::NAN;
        }
        for k in (0..bits).rev() {
            let bit = ((d >> k) & 1) as u64;
            if mant < (1u64 << 62) {
                mant = (mant << 1) | bit;
            } else {
                exp += 1;
                sticky |= bit != 0;
            }
        }
    }
    let width = 64 - mant.leading_zeros();
    if width <= 53 {
        return mant as f64; // exp is necessarily 0 here
    }
    let shift = width - 53;
    let rem = mant & ((1u64 << shift) - 1);
    let half = 1u64 << (shift - 1);
    let mut m = mant >> shift;
    if rem > half || (rem == half && (sticky || (m & 1) == 1)) {
        m += 1;
    }
    let e = shift as i32 + exp;
    if e > 1100 {
        return f64::INFINITY;
    }
    (m as f64) * libm::scalbn(1.0, e)
}

/// ECMAScript `Number::toString(10)` as a Rust `String`.
pub fn js_number_to_string(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x == 0.0 {
        return "0".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    // Integer fast path.
    if x.abs() < 9007199254740992.0 && x == x.trunc() {
        return (x as i64).to_string();
    }
    // Shortest round-trip digits via `{:e}`: "d.ddddde-7" / "-5e-324".
    let sci = format!("{:e}", x);
    let (mant, exp) = sci.split_once('e').expect("scientific format");
    let exp: i32 = exp.parse().expect("exponent");
    let (neg, mant) = match mant.strip_prefix('-') {
        Some(m) => (true, m),
        None => (false, mant),
    };
    let mut digits = String::with_capacity(mant.len());
    for c in mant.chars() {
        if c != '.' {
            digits.push(c);
        }
    }
    let mut n = exp + 1;
    fix_tie_to_even(x.abs(), &mut digits, &mut n);
    let k = digits.len() as i32;
    let mut out = String::with_capacity(28);
    if neg {
        out.push('-');
    }
    if k <= n && n <= 21 {
        out.push_str(&digits);
        for _ in 0..(n - k) {
            out.push('0');
        }
    } else if 0 < n && n <= 21 {
        out.push_str(&digits[..n as usize]);
        out.push('.');
        out.push_str(&digits[n as usize..]);
    } else if -6 < n && n <= 0 {
        out.push_str("0.");
        for _ in 0..(-n) {
            out.push('0');
        }
        out.push_str(&digits);
    } else {
        let e = n - 1;
        out.push_str(&digits[..1]);
        if k > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push('e');
        out.push(if e < 0 { '-' } else { '+' });
        out.push_str(&e.abs().to_string());
    }
    out
}

/// ECMAScript requires that when two shortest candidates are equally close to
/// `x`, the one with the even last digit is chosen. Shortest-digit generators
/// are free to pick either, so detect exact ties and re-pick.
fn fix_tie_to_even(a: f64, digits: &mut String, n: &mut i32) {
    let k = digits.len() as i32;
    let bits = a.to_bits();
    let raw_exp = ((bits >> 52) & 0x7FF) as i32;
    let frac = bits & ((1u64 << 52) - 1);
    let (m, e) = if raw_exp == 0 { (frac, -1074) } else { (frac | (1u64 << 52), raw_exp - 1075) };
    let e_odd = e + m.trailing_zeros() as i32;
    // The exact decimal expansion of an odd multiple of 2^-j has exactly j
    // fractional digits (the last one is 5). A tie needs exactly one more
    // digit than the shortest form has.
    if e_odd >= 0 || -e_odd != k - *n + 1 {
        return;
    }
    let last = digits.as_bytes()[k as usize - 1];
    if (last - b'0') % 2 == 0 {
        return;
    }
    // Exact value with k+1 significant digits; its first k digits are the lower candidate.
    let exact = format!("{:.*e}", k as usize, a);
    let (mant, exp) = exact.split_once('e').expect("scientific format");
    let exact_n: i32 = exp.parse::<i32>().expect("exponent") + 1;
    let lower: String = mant.chars().filter(|c| *c != '.').take(k as usize).collect();
    let (cand, cand_n) = if lower == *digits && exact_n == *n {
        // Rust picked the lower (odd) one: the even one is lower + 1.
        let mut d: Vec<u8> = lower.into_bytes();
        let mut i = d.len();
        let mut carry = true;
        while carry && i > 0 {
            i -= 1;
            if d[i] == b'9' {
                d[i] = b'0';
            } else {
                d[i] += 1;
                carry = false;
            }
        }
        let mut nn = exact_n;
        if carry {
            d.insert(0, b'1');
            d.pop();
            nn += 1;
        }
        (String::from_utf8(d).expect("ascii"), nn)
    } else {
        (lower, exact_n)
    };
    // Trailing zeros are not part of a shortest representation.
    let trimmed = cand.trim_end_matches('0');
    if trimmed.is_empty() {
        return;
    }
    // Both candidates must round-trip for the tie rule to apply.
    let probe = format!("0.{}e{}", trimmed, cand_n);
    if probe.parse::<f64>().ok() == Some(a) && trimmed.len() <= digits.len() {
        *digits = trimmed.to_string();
        *n = cand_n;
    }
}

/// `String(number)` as a `JsStr`.
pub fn js_num_to_str(x: f64) -> JsStr {
    // Small non-negative integers are extremely common.
    if x >= 0.0 && x < 10.0 && x == x.trunc() {
        return JsStr::from_unit(b'0' as u16 + x as u16);
    }
    // Other integers: digits straight into a stack buffer.
    if x.abs() < 9007199254740992.0 && x == x.trunc() && x != 0.0 {
        let mut buf = [0u8; 20];
        let mut i = buf.len();
        let mut n = x.abs() as u64;
        while n > 0 {
            i -= 1;
            buf[i] = b'0' + (n % 10) as u8;
            n /= 10;
        }
        if x < 0.0 {
            i -= 1;
            buf[i] = b'-';
        }
        return JsStr::from_latin1(&buf[i..]);
    }
    // Like V8's number-string cache: a direct-mapped table of recent results.
    NUM_STR_CACHE.with(|c| {
        let bits = x.to_bits();
        let slot = ((bits ^ (bits >> 29) ^ (bits >> 47)) as usize) & (NUM_STR_CACHE_SIZE - 1);
        let mut c = c.borrow_mut();
        if let Some((b, s)) = &c[slot] {
            if *b == bits {
                return s.clone();
            }
        }
        let s = fmt_number(x);
        c[slot] = Some((bits, s.clone()));
        s
    })
}

const NUM_STR_CACHE_SIZE: usize = 1024;

/// A `fmt::Write` sink on the stack.
struct StackBuf {
    buf: [u8; 40],
    len: usize,
}

impl std::fmt::Write for StackBuf {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        let b = s.as_bytes();
        if self.len + b.len() > self.buf.len() {
            return Err(std::fmt::Error);
        }
        self.buf[self.len..self.len + b.len()].copy_from_slice(b);
        self.len += b.len();
        Ok(())
    }
}

/// `js_number_to_string` without heap allocation (finite, non-integer `x`;
/// falls back to it for anything else, including possible digit ties).
fn fmt_number(x: f64) -> JsStr {
    use std::fmt::Write as _;
    if !x.is_finite() || x == x.trunc() {
        return JsStr::from_latin1_vec(js_number_to_string(x).into_bytes());
    }
    let mut sci = StackBuf { buf: [0; 40], len: 0 };
    if write!(sci, "{:e}", x).is_err() {
        return JsStr::from_latin1_vec(js_number_to_string(x).into_bytes());
    }
    let sci = &sci.buf[..sci.len];
    let epos = sci.iter().position(|&c| c == b'e').expect("scientific format");
    let (mant, exp) = (&sci[..epos], &sci[epos + 1..]);
    let (eneg, exp) = match exp.first() {
        Some(b'-') => (true, &exp[1..]),
        _ => (false, exp),
    };
    let mut e: i32 = 0;
    for &c in exp {
        e = e * 10 + (c - b'0') as i32;
    }
    if eneg {
        e = -e;
    }
    let (neg, mant) = match mant.first() {
        Some(b'-') => (true, &mant[1..]),
        _ => (false, mant),
    };
    let mut digits = [0u8; 24];
    let mut k = 0usize;
    for &c in mant {
        if c != b'.' {
            digits[k] = c;
            k += 1;
        }
    }
    let n = e + 1;
    // Possible tie between two shortest candidates (see `fix_tie_to_even`).
    let a = x.abs();
    let bits = a.to_bits();
    let raw_exp = ((bits >> 52) & 0x7FF) as i32;
    let frac = bits & ((1u64 << 52) - 1);
    let (m, be) = if raw_exp == 0 { (frac, -1074) } else { (frac | (1u64 << 52), raw_exp - 1075) };
    let e_odd = be + m.trailing_zeros() as i32;
    if !(e_odd >= 0 || -e_odd != k as i32 - n + 1) {
        return JsStr::from_latin1_vec(js_number_to_string(x).into_bytes());
    }
    let digits = &digits[..k];
    let k = k as i32;
    let mut out = StackBuf { buf: [0; 40], len: 0 };
    let mut push = |b: &[u8]| {
        out.buf[out.len..out.len + b.len()].copy_from_slice(b);
        out.len += b.len();
    };
    if neg {
        push(b"-");
    }
    if k <= n && n <= 21 {
        push(digits);
        for _ in 0..(n - k) {
            push(b"0");
        }
    } else if 0 < n && n <= 21 {
        push(&digits[..n as usize]);
        push(b".");
        push(&digits[n as usize..]);
    } else if -6 < n && n <= 0 {
        push(b"0.");
        for _ in 0..(-n) {
            push(b"0");
        }
        push(digits);
    } else {
        let ee = n - 1;
        push(&digits[..1]);
        if k > 1 {
            push(b".");
            push(&digits[1..]);
        }
        push(b"e");
        push(if ee < 0 { b"-" } else { b"+" });
        let mut eb = [0u8; 4];
        let mut i = eb.len();
        let mut v = ee.unsigned_abs();
        loop {
            i -= 1;
            eb[i] = b'0' + (v % 10) as u8;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        push(&eb[i..]);
    }
    JsStr::from_latin1(&out.buf[..out.len])
}

thread_local! {
    static NUM_STR_CACHE: std::cell::RefCell<Vec<Option<(u64, JsStr)>>> = std::cell::RefCell::new(vec![None; NUM_STR_CACHE_SIZE]);
}

/// `Math.round`: ties go toward +Infinity.
#[inline]
pub fn js_round(x: f64) -> f64 {
    if !x.is_finite() || x == 0.0 {
        return x;
    }
    if x < 0.0 && x >= -0.5 {
        return -0.0;
    }
    let f = x.floor();
    if x - f >= 0.5 {
        f + 1.0
    } else {
        f
    }
}

/// `parseFloat(x.toFixed(10))`.
///
/// `toFixed` picks the integer `n` minimising `|n / 10^10 - x|` on the exact
/// value of `x`, taking the larger `n` on ties (applied to `|x|`).
pub fn js_to_fixed10_parse(x: f64) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    if x.abs() >= 1e21 {
        // toFixed falls back to String(x); parseFloat of that is x itself.
        return x;
    }
    let neg = x < 0.0;
    let a = x.abs();
    if a == 0.0 {
        return 0.0; // (-0).toFixed(10) is "0.0000000000"
    }
    let r = match round_scaled_u128(a) {
        Some(n) if n < (1u128 << 53) => (n as f64) / 1e10,
        _ => to_fixed10_slow(a),
    };
    // (-x).toFixed gives "-" + digits; "-0.0000000000" parses to -0.
    if neg {
        -r
    } else {
        r
    }
}

/// round-half-up of `a * 10^10` using exact integer arithmetic, when it fits.
fn round_scaled_u128(a: f64) -> Option<u128> {
    let bits = a.to_bits();
    let raw_exp = ((bits >> 52) & 0x7FF) as i32;
    let frac = bits & ((1u64 << 52) - 1);
    let (m, e) = if raw_exp == 0 { (frac, -1074) } else { (frac | (1u64 << 52), raw_exp - 1075) };
    let big = (m as u128) * 10_000_000_000u128; // < 2^87
    if e >= 0 {
        if e > 39 {
            return None;
        }
        Some(big << e)
    } else {
        let s = (-e) as u32;
        if s >= 128 {
            return Some(0);
        }
        let q = big >> s;
        let rem = big & ((1u128 << s) - 1);
        let half = 1u128 << (s - 1);
        Some(if rem >= half { q + 1 } else { q })
    }
}

/// Slow path via the exact decimal expansion (Rust prints exact digits for a
/// requested precision; 1100 fractional digits always suffice for an f64).
fn to_fixed10_slow(a: f64) -> f64 {
    let exact = format!("{:.1100}", a);
    let (int, frac) = exact.split_once('.').expect("fixed format");
    let mut digits: Vec<u8> = int.bytes().chain(frac.bytes().take(10)).map(|c| c - b'0').collect();
    let rest = &frac.as_bytes()[10..];
    if rest[0] >= b'5' {
        // Round up (ties included: exact tie means rest == "5000...").
        let mut i = digits.len();
        loop {
            if i == 0 {
                digits.insert(0, 1);
                break;
            }
            i -= 1;
            if digits[i] == 9 {
                digits[i] = 0;
            } else {
                digits[i] += 1;
                break;
            }
        }
    }
    let n = digits.len();
    let mut s = String::with_capacity(n + 1);
    for (i, d) in digits.iter().enumerate() {
        if i == n - 10 {
            s.push('.');
        }
        s.push((b'0' + d) as char);
    }
    s.parse::<f64>().unwrap_or(f64::NAN)
}

/// `Cast.isInt` for a number, TurboWarp's `val === Math.floor(val)` (NaN
/// counts as an integer; vanilla scratch-vm compared with `parseInt`, which
/// rejected `Infinity` and numbers from 1e21 up).
pub fn js_is_int_number(x: f64) -> bool {
    x.is_nan() || x == x.floor()
}

/// `Number.prototype.toFixed(digits)` for `0 <= digits <= 20`.
pub fn js_to_fixed_string(x: f64, digits: usize) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.abs() >= 1e21 {
        return js_number_to_string(x);
    }
    let neg = x < 0.0;
    let a = x.abs();
    // Exact expansion, then round half up at `digits` fractional digits.
    let exact = format!("{:.1100}", a);
    let (int, frac) = exact.split_once('.').expect("fixed format");
    let mut d: Vec<u8> = int.bytes().chain(frac.bytes().take(digits)).map(|c| c - b'0').collect();
    if frac.as_bytes()[digits] >= b'5' {
        let mut i = d.len();
        loop {
            if i == 0 {
                d.insert(0, 1);
                break;
            }
            i -= 1;
            if d[i] == 9 {
                d[i] = 0;
            } else {
                d[i] += 1;
                break;
            }
        }
    }
    let n = d.len();
    let mut s = String::with_capacity(n + 2);
    if neg {
        s.push('-');
    }
    for (i, c) in d.iter().enumerate() {
        if i == n - digits && digits > 0 {
            s.push('.');
        }
        s.push((b'0' + c) as char);
    }
    s
}
