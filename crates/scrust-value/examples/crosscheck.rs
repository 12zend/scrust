//! Differential checker against Node (see conformance/value/gen.js).
//! Reads lines from stdin:
//!   S <utf16 hex>  <f64 bits hex>   Number(string)
//!   N <f64 bits>   <ascii>          String(number)
//!   F <f64 bits>   <f64 bits>       parseFloat(x.toFixed(10))
//!   L <utf16 hex>  <utf16 hex>      toLowerCase
//!   M <op> <f64 bits> <f64 bits>    mathop
//!   I <f64 bits> <0|1>              isInt(number)
//!   R <f64 bits> <f64 bits>         Math.round
use scrust_value::*;
use std::io::{self, BufRead};

fn bits(s: &str) -> f64 {
    f64::from_bits(u64::from_str_radix(s, 16).unwrap())
}
fn u16s(s: &str) -> JsStr {
    if s == "-" {
        return JsStr::empty();
    }
    let v: Vec<u16> = (0..s.len() / 4).map(|i| u16::from_str_radix(&s[i * 4..i * 4 + 4], 16).unwrap()).collect();
    JsStr::from_utf16(&v)
}
fn same(a: f64, b: f64) -> bool {
    a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
}

fn main() {
    let stdin = io::stdin();
    let (mut total, mut bad) = (0u64, 0u64);
    let mut ulp_only = std::collections::BTreeMap::<String, u64>::new();
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        if line.is_empty() {
            continue;
        }
        let mut it = line.split(' ');
        let kind = it.next().unwrap();
        total += 1;
        let ok = match kind {
            "S" => {
                let s = u16s(it.next().unwrap());
                let exp = bits(it.next().unwrap());
                same(js_string_to_number(&s), exp)
            }
            "N" => {
                let x = bits(it.next().unwrap());
                let exp = it.next().unwrap();
                js_num_to_str(x) == exp
            }
            "F" => {
                let x = bits(it.next().unwrap());
                let exp = bits(it.next().unwrap());
                same(js_to_fixed10_parse(x), exp)
            }
            "L" => {
                let s = u16s(it.next().unwrap());
                let exp = u16s(it.next().unwrap());
                s.to_lower() == exp
            }
            "I" => {
                let x = bits(it.next().unwrap());
                let exp = it.next().unwrap() == "1";
                is_int(&Value::Num(x)) == exp
            }
            "R" => {
                let x = bits(it.next().unwrap());
                let exp = bits(it.next().unwrap());
                same(js_round(x), exp)
            }
            "M" => {
                let op = it.next().unwrap().replace('_', " ");
                let x = bits(it.next().unwrap());
                let exp = bits(it.next().unwrap());
                let got = ops::mathop(&JsStr::from(op.as_str()), x);
                if !same(got, exp) {
                    let d = (got.to_bits() as i64).wrapping_sub(exp.to_bits() as i64).unsigned_abs();
                    if d <= 2 && std::env::var("STRICT_ULP").is_err() {
                        *ulp_only.entry(op.clone()).or_default() += 1;
                        true
                    } else {
                        false
                    }
                } else {
                    true
                }
            }
            _ => panic!("bad line {line}"),
        };
        if !ok {
            bad += 1;
            if bad <= 30 {
                eprintln!("MISMATCH: {line}");
            }
        }
    }
    println!("checked {total} cases, {bad} mismatches");
    for (op, c) in ulp_only {
        println!("  libm differences within 2 ULP for {op}: {c}");
    }
    if bad > 0 {
        std::process::exit(1);
    }
}
