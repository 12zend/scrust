//! PRODUCT.md §15.3 unit-test table and the §5 corollaries.

use scrust_value::ops::*;
use scrust_value::*;

fn s(x: &str) -> Value {
    Value::str(x)
}
fn n(x: f64) -> Value {
    Value::Num(x)
}
fn add(a: &Value, b: &Value) -> f64 {
    a.to_num() + b.to_num()
}
fn same(a: f64, b: f64) -> bool {
    a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
}
macro_rules! num_eq {
    ($a:expr, $b:expr) => {
        let (a, b): (f64, f64) = ($a, $b);
        assert!(same(a, b), "{} != {}", a, b);
    };
}
fn rng_half() -> impl FnMut() -> f64 {
    || 0.5
}

#[test]
fn value_is_16_bytes() {
    assert_eq!(std::mem::size_of::<Value>(), 16);
    assert_eq!(std::mem::size_of::<JsStr>(), 8);
}

#[test]
fn arithmetic_casts() {
    num_eq!(add(&s(""), &n(1.0)), 1.0);
    num_eq!(s("abc").to_num() * 2.0, 0.0);
    num_eq!(add(&s(" 12 "), &n(0.0)), 12.0);
    num_eq!(add(&s("0x10"), &n(0.0)), 16.0);
    num_eq!(add(&s("-0x10"), &n(0.0)), 0.0);
    num_eq!(add(&s("1_0"), &n(0.0)), 0.0);
    num_eq!(add(&s("Infinity"), &n(0.0)), f64::INFINITY);
    num_eq!(add(&s("infinity"), &n(0.0)), 0.0);
    let nan = 0.0f64 / 0.0;
    assert!(nan.is_nan());
    assert_eq!(n(nan).to_str(), "NaN");
    num_eq!(n(nan).to_num() * 1.0, 0.0);
    num_eq!(1.0f64 / 0.0, f64::INFINITY);
    num_eq!(0.1f64 + 0.2, 0.30000000000000004);
    assert_eq!(n(0.1 + 0.2).to_str(), "0.30000000000000004");
    assert_eq!(n(1e21).to_str().concat(&JsStr::empty()), "1e+21");
    assert_eq!(n(0.0 * -1.0).to_str(), "0");
    num_eq!(1.0 / (0.0f64 * -1.0), f64::NEG_INFINITY);
}

#[test]
fn string_to_number() {
    let c = |x: &str| js_string_to_number(&JsStr::from(x));
    num_eq!(c(""), 0.0);
    num_eq!(c("  "), 0.0);
    num_eq!(c("\t\n"), 0.0);
    num_eq!(c("\u{a0}5\u{feff}"), 5.0);
    num_eq!(c("\u{3000}7\u{2028}"), 7.0);
    num_eq!(c("1."), 1.0);
    num_eq!(c(".5"), 0.5);
    num_eq!(c("1e5"), 1e5);
    num_eq!(c("1E+3"), 1000.0);
    num_eq!(c("+3"), 3.0);
    num_eq!(c("-0"), -0.0);
    num_eq!(c("-0.0"), -0.0);
    num_eq!(c("0X1f"), 31.0);
    num_eq!(c("0b101"), 5.0);
    num_eq!(c("0o17"), 15.0);
    num_eq!(c("-Infinity"), f64::NEG_INFINITY);
    num_eq!(c("+Infinity"), f64::INFINITY);
    num_eq!(c("1e400"), f64::INFINITY);
    num_eq!(c("1e-400"), 0.0);
    num_eq!(c("0x20000000000001"), 9007199254740992.0);
    num_eq!(c("0x20000000000003"), 9007199254740996.0);
    for bad in [".", "1_000", "1,000", "١٢", "e5", "-0x1", "+0x1", "0x", "0b2", "0o8", "infinity", "NaN", "inf", "nan", "1e", "1e+", "--1", "1 2", "+", "-", "+-1", "0x1g", "1.2.3", "Infinity1", "INFINITY", "😀"] {
        assert!(c(bad).is_nan(), "{bad:?} should be NaN");
    }
}

#[test]
fn number_to_string() {
    let f = js_number_to_string;
    assert_eq!(f(f64::NAN), "NaN");
    assert_eq!(f(-0.0), "0");
    assert_eq!(f(f64::INFINITY), "Infinity");
    assert_eq!(f(f64::NEG_INFINITY), "-Infinity");
    assert_eq!(f(1e21), "1e+21");
    assert_eq!(f(123456789012345680000.0), "123456789012345680000");
    assert_eq!(f(0.000001), "0.000001");
    assert_eq!(f(1e-7), "1e-7");
    assert_eq!(f(1.5e-7), "1.5e-7");
    assert_eq!(f(-1.5), "-1.5");
    assert_eq!(f(5e-324), "5e-324");
    assert_eq!(f(1.7976931348623157e308), "1.7976931348623157e+308");
    assert_eq!(f(9007199254740992.0), "9007199254740992");
    assert_eq!(f(9007199254740994.0), "9007199254740994");
    assert_eq!(f(100.0), "100");
    assert_eq!(f(-123.456), "-123.456");
    assert_eq!(f(1e20), "100000000000000000000");
    assert_eq!(f(0.1), "0.1");
}

#[test]
fn rounding_and_mod() {
    num_eq!(js_round(-2.5), -2.0);
    num_eq!(js_round(2.5), 3.0);
    num_eq!(js_round(-0.4), -0.0);
    num_eq!(js_round(0.49999999999999994), 0.0);
    num_eq!(js_round(-0.5), -0.0);
    num_eq!(js_round(0.5), 1.0);
    num_eq!(js_round(4503599627370497.0), 4503599627370497.0);
    num_eq!(js_mod(-1.0, 3.0), 2.0);
    num_eq!(js_mod(5.0, -3.0), -1.0);
    assert!(js_mod(5.0, 0.0).is_nan());
}

#[test]
fn math_ops() {
    num_eq!(log10(1000.0), 2.9999999999999996);
    num_eq!(sin_deg(180.0), 0.0);
    num_eq!(cos_deg(90.0), 0.0);
    num_eq!(tan_deg(90.0), f64::INFINITY);
    num_eq!(tan_deg(-90.0), f64::NEG_INFINITY);
    num_eq!(tan_deg(270.0), f64::NEG_INFINITY);
    num_eq!(tan_deg(-270.0), f64::INFINITY);
    num_eq!(sin_deg(30.0), 0.5);
    num_eq!(cos_deg(60.0), 0.5);
    num_eq!(sin_deg(-0.000000001), -0.0);
    assert!(MathOp::Sqrt.apply(-1.0).is_nan());
    num_eq!(mathop(&JsStr::from("ABS"), -3.0), 3.0);
    num_eq!(mathop(&JsStr::from("nope"), -3.0), 0.0);
    num_eq!(pow10(2.0), 100.0);
    // toFixed ties pick the larger magnitude: 1/2048 = 0.00048828125 exactly.
    num_eq!(js_to_fixed10_parse(1.0 / 2048.0), 0.0004882813);
    num_eq!(js_to_fixed10_parse(-1.0 / 2048.0), -0.0004882813);
    num_eq!(js_to_fixed10_parse(1e22), 1e22);
    num_eq!(js_to_fixed10_parse(123456789012345.67), 123456789012345.67);
    num_eq!(js_to_fixed10_parse(1.2345678901234567e19), 1.2345678901234567e19);
}

#[test]
fn strings() {
    assert_eq!(JsStr::from("😀").len(), 2);
    assert_eq!(letter_of(1.9, &JsStr::from("abc")), "a");
    assert_eq!(letter_of(0.5, &JsStr::from("abc")), "");
    assert_eq!(letter_of(4.0, &JsStr::from("abc")), "");
    let hi = letter_of(1.0, &JsStr::from("😀"));
    assert_eq!(hi.len(), 1);
    assert_eq!(hi.code_unit(0), 0xD83D);
    assert_eq!(hi.to_rust_string(), "\u{fffd}");
    assert!(contains(&JsStr::from("abc"), &JsStr::from("")));
    assert!(contains(&JsStr::from("Apple"), &JsStr::from("PP")));
    assert!(!contains(&JsStr::from("Apple"), &JsStr::from("x")));
    assert_eq!(JsStr::from("İ").to_lower().len(), 2);
    assert_eq!(JsStr::from("ΑΣ").to_lower(), "ας");
    assert_eq!(JsStr::from("ÀÉ×Þ").to_lower(), "àé×þ");
    assert_eq!(JsStr::from("straße").to_upper(), "STRASSE");
}

#[test]
fn rope() {
    let mut s = JsStr::empty();
    let piece = JsStr::from("ab");
    for _ in 0..200_000 {
        s = s.concat(&piece);
    }
    assert_eq!(s.len(), 400_000);
    assert_eq!(s.code_unit(399_999), b'b' as u16);
    let mut t = JsStr::empty();
    for _ in 0..200_000 {
        t = piece.concat(&t);
    }
    assert_eq!(s, t);
    // Deep unflattened rope must drop without overflowing the stack.
    let mut u = JsStr::from("xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx");
    for _ in 0..500_000 {
        u = u.concat(&piece);
    }
    drop(u);
    let mixed = JsStr::from("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").concat(&JsStr::from("あ"));
    assert_eq!(mixed.len(), 31);
    assert_eq!(mixed.code_unit(30), 0x3042);
}

#[test]
fn comparisons() {
    let nan = n(f64::NAN);
    assert!(equals(&nan, &nan));
    assert!(!equals(&s(""), &n(0.0)));
    assert!(!equals(&s(" "), &n(0.0)));
    assert!(!equals(&s("0"), &s("")));
    assert!(equals(&s("1.0"), &n(1.0)));
    assert!(equals(&s("1e3"), &n(1000.0)));
    assert!(equals(&s("0x10"), &n(16.0)));
    assert!(equals(&s("abc"), &s("ABC")));
    assert!(!less_than(&s("10"), &s("9")));
    assert!(less_than(&s("10"), &s("9a")));
    assert!(equals(&Value::Bool(true), &s("TRUE")));
    assert!(equals(&Value::Bool(true), &s("true")));
    assert!(equals(&Value::Bool(true), &n(1.0)));
    assert!(equals(&n(f64::INFINITY), &s("Infinity")));
    assert!(equals(&n(f64::INFINITY), &n(f64::INFINITY)));
    assert!(less_than(&n(f64::NEG_INFINITY), &n(f64::INFINITY)));
    assert!(equals(&n(-0.0), &n(0.0)));
    // else-if: only one side is demoted to NaN.
    assert!(equals(&s(""), &s(" ")) == false);
    assert!(greater_than(&s("b"), &s("A")));
}

#[test]
fn numeric_compare_helpers_agree_with_compare() {
    let xs = [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.0, -0.0, 1.0, -1.0, 2.5, 1e21, 5e-324];
    for &a in &xs {
        for &b in &xs {
            let c = compare(&n(a), &n(b));
            assert_eq!(num_eq(a, b), c == 0.0, "eq {a} {b}");
            assert_eq!(num_lt(a, b), c < 0.0, "lt {a} {b}");
            assert_eq!(num_gt(a, b), c > 0.0, "gt {a} {b}");
        }
    }
}

#[test]
fn mod_fast_path_matches_fmod_definition() {
    let slow = |n: f64, m: f64| {
        let mut r = n % m;
        if r / m < 0.0 {
            r += m;
        }
        r
    };
    let xs = [0.0, -0.0, 1.0, -1.0, 2.0, -2.0, 3.0, -3.0, 7.0, -7.0, 10.0, 1e15, -1e15, 9007199254740991.0, 9007199254740993.0, 0.5, -2.5, f64::INFINITY, f64::NAN];
    for &a in &xs {
        for &b in &xs {
            let (x, y) = (js_mod(a, b), slow(a, b));
            assert!(x.to_bits() == y.to_bits() || (x.is_nan() && y.is_nan()), "mod({a}, {b}) = {x}, expected {y}");
        }
    }
}

#[test]
fn booleans() {
    assert!(!(!s("false").to_bool()) == false);
    assert!(!s("false").to_bool());
    assert!(!s("FALSE").to_bool());
    assert!(!s("False").to_bool());
    assert!(s("0.0").to_bool());
    assert!(s(" 0").to_bool());
    assert!(s("00").to_bool());
    assert!(s("no").to_bool());
    assert!(!s("0").to_bool());
    assert!(!s("").to_bool());
    assert!(!n(-0.0).to_bool());
    assert!(!n(f64::NAN).to_bool());
    assert!(n(0.1).to_bool());
}

#[test]
fn is_int_cases() {
    assert!(is_int(&n(f64::NAN)));
    // TurboWarp: `val === Math.floor(val)` (vanilla's parseInt rejected these two).
    assert!(is_int(&n(1e21)));
    assert!(!is_int(&n(1.5)));
    assert!(is_int(&n(-0.0)));
    assert!(is_int(&n(1e20)));
    assert!(is_int(&n(f64::INFINITY)));
    assert!(is_int(&Value::Bool(true)));
    assert!(is_int(&s("abc")));
    assert!(is_int(&s("1e5")));
    assert!(!is_int(&s("1.0")));
}

#[test]
fn list_index() {
    let mut r = rng_half();
    let li = |v: &Value, len: usize, all: bool, r: &mut dyn FnMut() -> f64| to_list_index(v, len, all, r);
    assert_eq!(li(&n(1.9), 2, false, &mut r), ListIndex::Index(1));
    assert_eq!(li(&s("2"), 2, false, &mut r), ListIndex::Index(2));
    assert_eq!(li(&s("last"), 0, false, &mut r), ListIndex::Invalid);
    assert_eq!(li(&s("last"), 3, false, &mut r), ListIndex::Index(3));
    assert_eq!(li(&s("LAST"), 3, false, &mut r), ListIndex::Invalid);
    assert_eq!(li(&s("all"), 3, true, &mut r), ListIndex::All);
    assert_eq!(li(&s("all"), 3, false, &mut r), ListIndex::Invalid);
    assert_eq!(li(&s("any"), 4, false, &mut r), ListIndex::Index(3));
    assert_eq!(li(&n(f64::INFINITY), 3, false, &mut r), ListIndex::Invalid);
    assert_eq!(li(&n(0.0), 3, false, &mut r), ListIndex::Invalid);
    assert_eq!(li(&n(4.0), 3, false, &mut r), ListIndex::Invalid);
}

#[test]
fn compare_tab_is_zero() {
    // TurboWarp's isNotActuallyZero: whitespace containing a tab counts as 0.
    assert_eq!(compare(&s("\t"), &n(0.0)), 0.0);
    assert!(compare(&s(" "), &n(0.0)) != 0.0);
    assert!(compare(&s(""), &n(0.0)) != 0.0);
}

#[test]
fn listcontents() {
    let a = [s("a"), s("b")];
    assert_eq!(list_contents(a.iter()), "ab");
    // TurboWarp: the string form decides, so a one-digit number counts.
    let b = [s("a"), n(1.0)];
    assert_eq!(list_contents(b.iter()), "a1");
    let b2 = [s("a"), n(10.0)];
    assert_eq!(list_contents(b2.iter()), "a 10");
    let c = [s("ab"), s("c")];
    assert_eq!(list_contents(c.iter()), "ab c");
    let d = [s("1"), s("2")];
    assert_eq!(list_contents(d.iter()), "12");
    let e = [n(1.0), n(2.0)];
    assert_eq!(list_contents(e.iter()), "12");
    let empty: [Value; 0] = [];
    assert_eq!(list_contents(empty.iter()), "");
}

#[test]
fn random() {
    let mut r = rng_half();
    num_eq!(ops::random(&n(1.0), &n(1.0), &mut r), 1.0);
    let v = ops::random(&s("1.0"), &n(2.0), &mut r);
    num_eq!(v, 1.5);
    let v = ops::random(&n(1.0), &n(10.0), &mut r);
    num_eq!(v, 6.0);
    let v = ops::random(&n(10.0), &s("1"), &mut r);
    num_eq!(v, 6.0);
}
