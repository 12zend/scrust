//! `jsmath::atan` must match V8's `Math.atan` bit for bit, including inputs
//! where V8 (LLVM libc) is not correctly rounded.
use scrust_value::jsmath::atan;

#[test]
fn matches_v8() {
    for (x, y) in [
        (0x3ff0fffb8e896293u64, 0x3fea1a21c630ad2cu64),
        (0x3ff16965aa089e24, 0x3fea7be7469231ef),
        (0x3ff3997530c2ba87, 0x3fec5b774838fbbe),
        (0x3ff35dd5ba33a496, 0x3fec2b6f9bf15d19),
    ] {
        assert_eq!(atan(f64::from_bits(x)).to_bits(), y, "atan({x:#x})");
        assert_eq!(atan(-f64::from_bits(x)).to_bits(), y | 1 << 63);
    }
    assert_eq!(atan(0.0).to_bits(), 0);
    assert_eq!(atan(-0.0).to_bits(), 1 << 63);
    assert_eq!(atan(1.0), std::f64::consts::FRAC_PI_4);
    assert_eq!(atan(f64::INFINITY), std::f64::consts::FRAC_PI_2);
    assert!(atan(f64::NAN).is_nan());
}

/// Dense samples recorded from the browser (`conformance/out/twref/results/math-atan*.txt`,
/// lines of `<input bits> <Math.atan bits>`), when present.
#[test]
fn matches_recorded_samples() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../conformance/out/twref/results");
    for f in ["math-atan.txt", "math-atan2.txt"] {
        let Ok(s) = std::fs::read_to_string(format!("{dir}/{f}")) else { continue };
        let mut bad = 0;
        for l in s.lines() {
            let mut it = l.split_whitespace();
            let (Some(x), Some(y)) = (it.next(), it.next()) else { continue };
            let (x, y) = (u64::from_str_radix(x, 16).unwrap(), u64::from_str_radix(y, 16).unwrap());
            let got = atan(f64::from_bits(x)).to_bits();
            if got != y {
                bad += 1;
                eprintln!("{f}: atan({x:016x}) = {got:016x}, browser {y:016x}");
            }
        }
        assert_eq!(bad, 0, "{f}");
    }
}

/// `Math.atan2` samples (`math-atan2xy.txt`: `<y> <x> <atan2>` bits), when present.
#[test]
fn atan2_matches_recorded_samples() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../conformance/out/twref/results");
    let Ok(s) = std::fs::read_to_string(format!("{dir}/math-atan2xy.txt")) else { return };
    let mut bad = 0;
    for l in s.lines() {
        let v: Vec<u64> = l.split_whitespace().map(|w| u64::from_str_radix(w, 16).unwrap()).collect();
        let got = scrust_value::jsmath::atan2(f64::from_bits(v[0]), f64::from_bits(v[1])).to_bits();
        if got != v[2] && !(f64::from_bits(got).is_nan() && f64::from_bits(v[2]).is_nan()) {
            bad += 1;
            if bad < 10 {
                eprintln!("atan2({:016x}, {:016x}) = {got:016x}, browser {:016x}", v[0], v[1], v[2]);
            }
        }
    }
    assert_eq!(bad, 0);
}
