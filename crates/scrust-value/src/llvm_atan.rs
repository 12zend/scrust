//! Port of LLVM libc's `atan` (libc/src/__support/math/atan.h, Apache-2.0
//! WITH LLVM-exception), which V8 uses for `Math.atan`. It is not correctly
//! rounded near halfway cases, so CORE-MATH's `atan` differs from browsers
//! there. Compiled as for a target with FMA (as Chrome on arm64).

#[derive(Clone, Copy)]
struct Dd {
    lo: f64,
    hi: f64,
}

#[inline]
fn b(x: u64) -> f64 {
    f64::from_bits(x)
}

/// `ATAN_I[i]` = atan(i / 64) as (lo, hi).
static ATAN_I: [(u64, u64); 65] = [
    (0x0000000000000000, 0x0000000000000000),
    (0xbc2220c39d4dff50, 0x3f8fff555bbb729b),
    (0xbc35ec431444912c, 0x3f9ffd55bba97625),
    (0xbc086ef8f794f105, 0x3fa7fb818430da2a),
    (0xbc3c934d86d23f1d, 0x3faff55bb72cfdea),
    (0x3c5ac4ce285df847, 0x3fb3f59f0e7c559d),
    (0xbc5cfb654c0c3d98, 0x3fb7ee182602f10f),
    (0x3c5f7b8f29a05987, 0x3fbbe39ebe6f07c3),
    (0xbc4cd37686760c17, 0x3fbfd5ba9aac2f6e),
    (0xbc4b485914dacf8c, 0x3fc1e1fafb043727),
    (0x3c661a3b0ce9281b, 0x3fc3d6eee8c6626c),
    (0xbc5054ab2c010f3d, 0x3fc5c9811e3ec26a),
    (0x3c5347b0b4f881ca, 0x3fc7b97b4bce5b02),
    (0x3c4cf601e7b4348e, 0x3fc9a6a8e96c8626),
    (0x3c217b10d2e0e5ab, 0x3fcb90d7529260a2),
    (0x3c6c648d1534597e, 0x3fcd77d5df205736),
    (0x3c68ab6e3cf7afbd, 0x3fcf5b75f92c80dd),
    (0x3c762e47390cb865, 0x3fd09dc597d86362),
    (0x3c630ca4748b1bf9, 0x3fd18bf5a30bf178),
    (0xbc7077cdd36dfc81, 0x3fd278372057ef46),
    (0xbc6963a544b672d8, 0x3fd362773707ebcc),
    (0xbc75d5e43c55b3ba, 0x3fd44aa436c2af0a),
    (0xbc62566480884082, 0x3fd530ad9951cd4a),
    (0xbc7a725715711f00, 0x3fd614840309cfe2),
    (0xbc7c63aae6f6e918, 0x3fd6f61941e4def1),
    (0x3c769c885c2b249a, 0x3fd7d5604b63b3f7),
    (0x3c7b6d0ba3748fa8, 0x3fd8b24d394a1b25),
    (0x3c79e6c988fd0a77, 0x3fd98cd5454d6b18),
    (0xbc724dec1b50b7ff, 0x3fda64eec3cc23fd),
    (0x3c7ae187b1ca5040, 0x3fdb3a911da65c6c),
    (0xbc7cc1ce70934c34, 0x3fdc0db4c94ec9f0),
    (0xbc7a2cfa4418f1ad, 0x3fdcde53432c1351),
    (0x3c7a2b7f222f65e2, 0x3fddac670561bb4f),
    (0x3c70e53dc1bf3435, 0x3fde77eb7f175a34),
    (0xbc6a3992dc382a23, 0x3fdf40dd0b541418),
    (0xbc8b32c949c9d593, 0x3fe0039c73c1a40c),
    (0xbc7d5b495f6349e6, 0x3fe0657e94db30d0),
    (0x3c5974fa13b5404f, 0x3fe0c6145b5b43da),
    (0xbc52bdaee1c0ee35, 0x3fe1255d9bfbd2a9),
    (0x3c8c621cec00c301, 0x3fe1835a88be7c13),
    (0xbc5928df287a668f, 0x3fe1e00babdefeb4),
    (0x3c6c421c9f38224e, 0x3fe23b71e2cc9e6a),
    (0xbc709e73b0c6c087, 0x3fe2958e59308e31),
    (0x3c8c5d5e9ff0cf8d, 0x3fe2ee628406cbca),
    (0x3c81021137c71102, 0x3fe345f01cce37bb),
    (0xbc82304331d8bf46, 0x3fe39c391cd4171a),
    (0x3c7ecf8b492644f0, 0x3fe3f13fb89e96f4),
    (0xbc7f76d0163f79c8, 0x3fe445065b795b56),
    (0x3c72419a87f2a458, 0x3fe4978fa3269ee1),
    (0x3c84a33dbeb3796c, 0x3fe4e8de5bb6ec04),
    (0xbc81bb74abda520c, 0x3fe538f57b89061f),
    (0xbc75e5c9d8c5a950, 0x3fe587d81f732fbb),
    (0x3c60028e4bc5e7ca, 0x3fe5d58987169b18),
    (0xbc62b785350ee8c1, 0x3fe6220d115d7b8e),
    (0xbc76ea6febe8bbba, 0x3fe66d663923e087),
    (0xbc8a80386188c50e, 0x3fe6b798920b3d99),
    (0xbc78c34d25aadef6, 0x3fe700a7c5784634),
    (0x3c47b2a6165884a1, 0x3fe748978fba8e0f),
    (0x3c8406a089803740, 0x3fe78f6bbd5d315e),
    (0x3c8560821e2f3aa9, 0x3fe7d528289fa093),
    (0xbc7bf76229d3b917, 0x3fe819d0b7158a4d),
    (0x3c66b66e7fc8b8c3, 0x3fe85d69576cc2c5),
    (0xbc855b9a5e177a1b, 0x3fe89ff5ff57f1f8),
    (0xbc7ec182ab042f61, 0x3fe8e17aa99cc05e),
    (0x3c81a62633145c07, 0x3fe921fb54442d18),
];

#[inline]
fn exact_add(a: f64, c: f64) -> Dd {
    let hi = a + c;
    let t = hi - a;
    Dd { hi, lo: c - t }
}

#[inline]
fn exact_mult(a: f64, c: f64) -> Dd {
    let hi = a * c;
    Dd { hi, lo: a.mul_add(c, -hi) }
}

#[inline]
fn div(a: Dd, d: Dd) -> Dd {
    let q = 1.0 / d.hi;
    let hi = a.hi * q;
    let e_hi = d.hi.mul_add(-hi, a.hi);
    let e_lo = d.lo.mul_add(-hi, a.lo);
    Dd { hi, lo: q * (e_hi + e_lo) }
}

#[inline]
fn atan_eval(x: Dd) -> Dd {
    let x_hi_sq = x.hi * x.hi;
    let c0 = x_hi_sq.mul_add(b(0x3fc999999999999a), b(0xbfd5555555555555));
    let c1 = x_hi_sq.mul_add(b(0x3fbc71c71c71c71c), b(0xbfc2492492492492));
    let x_hi_3 = x_hi_sq * x.hi;
    let x_hi_4 = x_hi_sq * x_hi_sq;
    let d0 = x_hi_4.mul_add(c1, c0);
    let d1 = (x_hi_4 - x_hi_sq).mul_add(x.lo, x.lo);
    Dd { hi: x.hi, lo: x_hi_3.mul_add(d0, d1) }
}

pub fn atan(x: f64) -> f64 {
    let pi_over_2 = Dd { lo: b(0x3c91a62633145c07), hi: b(0x3ff921fb54442d18) };
    let mpi_over_2 = Dd { lo: -pi_over_2.lo, hi: -pi_over_2.hi };
    let sign = if x.is_sign_negative() { -1.0 } else { 1.0 };
    let x_abs_bits = x.to_bits() & 0x7FFF_FFFF_FFFF_FFFF;
    let x_exp = (x_abs_bits >> 52) as i32 - 1023;
    let x_d = f64::from_bits(x_abs_bits);
    if x_exp < 0 {
        if x_exp < -26 {
            if x == 0.0 {
                return x;
            }
            return b(0xbc90000000000000).mul_add(x, x);
        }
        let mut k = (64.0 * x_d).round_ties_even();
        let idx = k as usize;
        k *= 1.0 / 64.0;
        let num = Dd { lo: 0.0, hi: x_d - k };
        let den_hi = x_d.mul_add(k, 1.0);
        let prod = exact_mult(x_d, k);
        let den = Dd { hi: den_hi, lo: ((1.0 - den_hi) + prod.hi) + prod.lo };
        let x_r = div(num, den);
        let p = atan_eval(x_r);
        let (ilo, ihi) = (b(ATAN_I[idx].0), b(ATAN_I[idx].1));
        let c0 = exact_add(ihi, p.hi);
        let c1 = c0.lo + (ilo + p.lo);
        return sign * (c0.hi + c1);
    }
    if x_exp >= 53 {
        if x.is_nan() {
            return x;
        }
        return sign.mul_add(pi_over_2.hi, sign * pi_over_2.lo);
    }
    let y = 1.0 / x_d;
    let mut k = (64.0 * y).round_ties_even();
    let idx = k as usize;
    k *= 1.0 / 64.0;
    let den = exact_add(x_d, k);
    let num_hi = (-x_d).mul_add(k, 1.0);
    let prod = exact_mult(x_d, k);
    let num = Dd { hi: num_hi, lo: ((1.0 - num_hi) - prod.hi) - prod.lo };
    let x_r = div(num, den);
    let p = atan_eval(x_r);
    let (ilo, ihi) = (b(ATAN_I[idx].0), b(ATAN_I[idx].1));
    let c0 = exact_add(mpi_over_2.hi, ihi);
    let c1 = exact_add(c0.hi, p.hi);
    let c2 = c1.lo + (c0.lo + p.lo) + (ilo + mpi_over_2.lo);
    -sign * (c1.hi + c2)
}
