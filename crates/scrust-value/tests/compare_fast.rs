//! The specialised comparisons agree with `compare` on a grid of tricky values.
use scrust_value::{compare, compare_lit, is_not_actually_zero, CompareKey, JsStr, ToNum, ToStr, Value};

fn samples() -> Vec<Value> {
    let mut v: Vec<Value> = Vec::new();
    for n in [0.0, -0.0, 1.0, -1.0, 0.5, 1e21, 1e-7, f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 255.0, 12.25, 3.0] {
        v.push(Value::Num(n));
    }
    for s in [
        "", " ", "\t", "\n", "0", "00", "0.0", "-0", "1", "1.0", " 1 ", "+1", "0x10", "1e3", "Infinity", "-Infinity", "infinity", "NaN", "nan", "abc", "ABC", "Abc",
        "true", "TRUE", "false", "a b", "ÀÉ", "àé", "ß", "İ", "Σ", "σ", "12.25", "0.5", ".5", "5.", "1e21", "1e+21", "255", "3", "all", "last",
    ] {
        v.push(Value::Str(JsStr::from(s)));
    }
    v.push(Value::Bool(true));
    v.push(Value::Bool(false));
    v
}

#[test]
fn compare_key_matches_compare() {
    let vs = samples();
    for a in &vs {
        for b in &vs {
            let want = compare(a, b) == 0.0;
            assert_eq!(CompareKey::new(b).eq_value(a), want, "{a:?} vs {b:?}");
        }
    }
}

#[test]
fn compare_lit_matches_compare() {
    let vs = samples();
    for a in &vs {
        for b in &vs {
            let text = b.to_str().to_rust_string();
            if !text.is_ascii() {
                continue;
            }
            let mut n = b.js_number();
            if n == 0.0 && is_not_actually_zero(b) {
                n = f64::NAN;
            }
            let text: &'static str = Box::leak(text.into_boxed_str());
            let want = compare(a, b);
            let got = compare_lit(a, n, text);
            assert_eq!(want.partial_cmp(&0.0), got.partial_cmp(&0.0), "{a:?} vs {b:?}: {want} {got}");
        }
    }
}

/// `Cast.compare` written out plainly (strings formatted and lower-cased).
fn compare_reference(a: &Value, b: &Value) -> f64 {
    if let (Value::Num(x), Value::Num(y)) = (a, b) {
        if !x.is_nan() && !y.is_nan() {
            return if x == y { 0.0 } else { x - y };
        }
    }
    let mut n1 = a.js_number();
    let mut n2 = b.js_number();
    if n1 == 0.0 && is_not_actually_zero(a) {
        n1 = f64::NAN;
    } else if n2 == 0.0 && is_not_actually_zero(b) {
        n2 = f64::NAN;
    }
    if n1.is_nan() || n2.is_nan() {
        return match a.to_str().to_lower().cmp_utf16(&b.to_str().to_lower()) {
            std::cmp::Ordering::Less => -1.0,
            std::cmp::Ordering::Equal => 0.0,
            std::cmp::Ordering::Greater => 1.0,
        };
    }
    if n1 == n2 {
        return 0.0;
    }
    n1 - n2
}

#[test]
fn compare_matches_reference() {
    let mut vs = samples();
    for s in ["-", "-a", "!", " x", "zebra", "Zebra", "nab", "NaN!", "inf", "Infinityx", "m", "o", "ÿ", "\u{212A}", "9z", "0z", "~"] {
        vs.push(Value::Str(JsStr::from(s)));
    }
    for n in [123.456, -0.001, 1e-9, 7.0, f64::NAN, f64::NEG_INFINITY] {
        vs.push(Value::Num(n));
    }
    for a in &vs {
        for b in &vs {
            let (want, got) = (compare_reference(a, b), compare(a, b));
            assert_eq!(want.partial_cmp(&0.0), got.partial_cmp(&0.0), "{a:?} vs {b:?}: {want} {got}");
        }
    }
}

#[test]
fn number_strings_match_reference() {
    let mut seed: u64 = 99;
    for _ in 0..200_000 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let x = match seed % 4 {
            0 => f64::from_bits(seed),
            1 => (seed % 100_000) as f64 / 1000.0,
            2 => ((seed >> 8) % 1_000_000) as f64 * 1e-7,
            _ => (seed as f64) * 1e-3 - 9e15,
        };
        assert_eq!(scrust_value::js_num_to_str(x).to_rust_string(), scrust_value::num::js_number_to_string(x), "{x:e}");
    }
}

#[test]
fn compare_val_str_matches_reference() {
    let vs = samples();
    for a in &vs {
        for b in &vs {
            if let Value::Str(s) = b {
                let want = compare_reference(a, b);
                let got = scrust_value::compare_val_str(a, s);
                assert_eq!(want.partial_cmp(&0.0), got.partial_cmp(&0.0), "{a:?} vs {b:?}: {want} {got}");
            }
        }
    }
}
