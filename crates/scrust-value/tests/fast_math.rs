//! The fast paths of the degree trigonometry and of `%` give exactly the
//! results of the reference formulas.
use scrust_value::{jsmath, num::js_round, ops};
use std::f64::consts::PI;

fn round10(x: f64) -> f64 {
    js_round(x * 1e10) / 1e10
}

fn inputs() -> Vec<f64> {
    let mut v = Vec::new();
    for i in -2000..=2000 {
        v.push(i as f64);
        v.push(i as f64 * 0.5);
        v.push(i as f64 * 0.1);
        v.push(i as f64 * 7.5);
    }
    let mut seed: u64 = 7;
    for _ in 0..300_000 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let x = match seed % 5 {
            0 => (seed >> 11) as f64 / (1u64 << 53) as f64 * 720.0 - 360.0,
            1 => ((seed >> 20) % 100_000) as f64 / 100.0 - 500.0,
            2 => ((seed >> 12) as f64 / (1u64 << 52) as f64 - 0.5) * 1e7,
            3 => f64::from_bits(seed) ,
            _ => (((seed >> 30) % 4000) as f64 - 2000.0) * 90.0 + ((seed >> 8) % 3) as f64 * 1e-12,
        };
        if x.is_finite() {
            v.push(x);
        }
    }
    v
}

#[test]
fn degree_trig_matches_correctly_rounded() {
    for n in inputs() {
        let r = (PI * n) / 180.0;
        let want = round10(jsmath::sin(r));
        assert_eq!(ops::sin_deg(n).to_bits(), want.to_bits(), "sin {n:e}");
        let want = round10(jsmath::cos(r));
        assert_eq!(ops::cos_deg(n).to_bits(), want.to_bits(), "cos {n:e}");
        let a = n % 360.0;
        if a.abs() != 90.0 && a.abs() != 270.0 {
            let want = round10(jsmath::tan((PI * a) / 180.0));
            let got = ops::tan_deg(n);
            assert!(got.to_bits() == want.to_bits() || (got.is_nan() && want.is_nan()), "tan {n:e}: {got} {want}");
        }
    }
}

#[test]
fn fmod_matches_operator() {
    let xs = inputs();
    let ms = [1.0, -1.0, 0.5, 3.0, 360.0, -7.25, 0.1, 1e-5, 2.0f64.powi(-30), 1e10, 1e300, f64::MIN_POSITIVE, 5e-324, f64::INFINITY, 0.0, -0.0, f64::NAN];
    let mut seed: u64 = 3;
    for &n in xs.iter().chain([0.0, -0.0, f64::INFINITY, f64::NAN, 1e308, -1e308].iter()) {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let rnd = (((seed >> 11) as f64 / (1u64 << 53) as f64) - 0.5) * 100.0;
        for m in ms.iter().copied().chain([rnd, n * 0.37, 1.0 / 3.0]) {
            let (want, got) = (n % m, ops::fmod(n, m));
            assert!(want.to_bits() == got.to_bits() || (want.is_nan() && got.is_nan()), "{n:e} % {m:e}: {got:e} {want:e}");
        }
    }
}
