//! Colour management of images with an embedded ICC profile, as Chrome
//! applies it when such an image is drawn into a 2D canvas (bitmap costumes,
//! images embedded in SVG costumes).
//!
//! Skia makes an `SkColorSpace` from the profile (`SkColorSpace::Make` /
//! `MakeRGB`): the matrix-and-curves part of the profile, with a transfer
//! function within 0.001 of sRGB, 2.2 or linear snapped to that function and
//! a gamut within 0.01 of sRGB's treated as sRGB. The conversion to the sRGB
//! canvas (`SkColorSpaceXformSteps`) linearises, applies
//! `concat(invert(sRGB.toXYZD50), src.toXYZD50)` and encodes with the sRGB
//! inverse that skcms computes (`skcms_TransferFunction_invert`, which uses
//! its approximate `powf`, so the constants differ from the textbook ones).
//! The GPU evaluates this in float with an accurate `pow`; evaluating it in
//! double here gives the same bytes on all but about 1 in 65,000 colours of a
//! Display P3 test image.

/// skcms parametric transfer function: `c*x + f` below `d`, else `(a*x + b)^g + e`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Tf {
    g: f32,
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: f32,
    f: f32,
}

impl Tf {
    const SRGB: Tf = Tf { g: 2.4, a: (1.0 / 1.055) as f32, b: (0.055 / 1.055) as f32, c: (1.0 / 12.92) as f32, d: 0.04045, e: 0.0, f: 0.0 };
    const GAMMA_2_2: Tf = Tf { g: 2.2, a: 1.0, b: 0.0, c: 0.0, d: 0.0, e: 0.0, f: 0.0 };
    const LINEAR: Tf = Tf { g: 1.0, a: 1.0, b: 0.0, c: 0.0, d: 0.0, e: 0.0, f: 0.0 };
    /// `skcms_TransferFunction_invert(&SkNamedTransferFn::kSRGB)`.
    const SRGB_INV: Tf = Tf { g: f32::from_bits(0x3ed5_5555), a: f32::from_bits(0x3f91_9280), b: 0.0, c: 12.92, d: f32::from_bits(0x3b4d_2e31), e: f32::from_bits(0xbd61_2800), f: 0.0 };

    fn eval(&self, x: f64) -> f64 {
        let (s, x) = (x.signum(), x.abs());
        let v = if x < self.d as f64 { self.c as f64 * x + self.f as f64 } else { (self.a as f64 * x + self.b as f64).powf(self.g as f64) + self.e as f64 };
        s * v
    }

    /// `transfer_fn_almost_equal` for every coefficient.
    fn almost(&self, o: &Tf) -> bool {
        let n = |x: f32, y: f32| (x - y).abs() < 0.001;
        n(self.g, o.g) && n(self.a, o.a) && n(self.b, o.b) && n(self.c, o.c) && n(self.d, o.d) && n(self.e, o.e) && n(self.f, o.f)
    }
}

type M3 = [[f32; 3]; 3];

/// `SkNamedGamut::kSRGB`.
const SRGB_TO_XYZ: M3 = [[0.436_065_67, 0.385_147_1, 0.143_066_41], [0.222_488_4, 0.716_873_17, 0.060_607_91], [0.013_916_016, 0.097_076_416, 0.714_096_07]];

/// `skcms_Matrix3x3_invert` (in double, rounded to float).
fn invert(m: &M3) -> Option<M3> {
    let v = |r: usize, c: usize| m[r][c] as f64;
    let (a00, a01, a02) = (v(0, 0), v(1, 0), v(2, 0));
    let (a10, a11, a12) = (v(0, 1), v(1, 1), v(2, 1));
    let (a20, a21, a22) = (v(0, 2), v(1, 2), v(2, 2));
    let (mut b0, mut b1, mut b2) = (a00 * a11 - a01 * a10, a00 * a12 - a02 * a10, a01 * a12 - a02 * a11);
    let (mut b3, mut b4, mut b5) = (a20, a21, a22);
    let det = b0 * b5 - b1 * b4 + b2 * b3;
    if det == 0.0 {
        return None;
    }
    let inv = 1.0 / det;
    if !(inv as f32).is_finite() {
        return None;
    }
    for b in [&mut b0, &mut b1, &mut b2, &mut b3, &mut b4, &mut b5] {
        *b *= inv;
    }
    let mut d = [[0f32; 3]; 3];
    d[0][0] = (a11 * b5 - a12 * b4) as f32;
    d[1][0] = (a02 * b4 - a01 * b5) as f32;
    d[2][0] = b2 as f32;
    d[0][1] = (a12 * b3 - a10 * b5) as f32;
    d[1][1] = (a00 * b5 - a02 * b3) as f32;
    d[2][1] = (-b1) as f32;
    d[0][2] = (a10 * b4 - a11 * b3) as f32;
    d[1][2] = (a01 * b3 - a00 * b4) as f32;
    d[2][2] = b0 as f32;
    Some(d)
}

/// `skcms_Matrix3x3_concat` (float).
fn concat(a: &M3, b: &M3) -> M3 {
    let mut m = [[0f32; 3]; 3];
    for (r, row) in m.iter_mut().enumerate() {
        for (c, v) in row.iter_mut().enumerate() {
            *v = a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c];
        }
    }
    m
}

fn be32(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

fn fixed(d: &[u8], at: usize) -> Option<f32> {
    Some(be32(d, at)? as i32 as f32 / 65536.0)
}

/// The matrix/TRC part of an RGB display profile: (toXYZD50, transfer
/// function), when all three curves are the same parametric curve.
fn parse(icc: &[u8]) -> Option<(M3, Tf)> {
    if icc.get(16..20)? != b"RGB " || icc.get(20..24)? != b"XYZ " {
        return None;
    }
    let count = be32(icc, 128)? as usize;
    let tag = |sig: &[u8; 4]| -> Option<&[u8]> {
        (0..count.min(256)).find_map(|i| {
            let at = 132 + 12 * i;
            (icc.get(at..at + 4)? == sig).then(|| {
                let (off, len) = (be32(icc, at + 4)? as usize, be32(icc, at + 8)? as usize);
                icc.get(off..off.checked_add(len)?)
            })?
        })
    };
    let xyz = |sig: &[u8; 4]| -> Option<[f32; 3]> {
        let t = tag(sig)?;
        (t.get(0..4)? == b"XYZ ").then_some(())?;
        Some([fixed(t, 8)?, fixed(t, 12)?, fixed(t, 16)?])
    };
    let (r, g, b) = (xyz(b"rXYZ")?, xyz(b"gXYZ")?, xyz(b"bXYZ")?);
    let m = [[r[0], g[0], b[0]], [r[1], g[1], b[1]], [r[2], g[2], b[2]]];
    let curve = |sig: &[u8; 4]| -> Option<Tf> {
        let t = tag(sig)?;
        let mut tf = Tf { g: 1.0, a: 1.0, b: 0.0, c: 0.0, d: 0.0, e: 0.0, f: 0.0 };
        match t.get(0..4)? {
            b"curv" => match be32(t, 8)? {
                0 => {}
                1 => tf.g = u16::from_be_bytes(t.get(12..14)?.try_into().ok()?) as f32 / 256.0,
                _ => return None,
            },
            b"para" => {
                let kind = u16::from_be_bytes(t.get(8..10)?.try_into().ok()?);
                let p = |i: usize| fixed(t, 12 + 4 * i);
                tf.g = p(0)?;
                match kind {
                    0 => {}
                    1 | 2 => {
                        (tf.a, tf.b) = (p(1)?, p(2)?);
                        if tf.a == 0.0 {
                            return None;
                        }
                        tf.d = -tf.b / tf.a;
                        if kind == 2 {
                            tf.e = p(3)?;
                            tf.f = tf.e;
                        }
                    }
                    3 => (tf.a, tf.b, tf.c, tf.d) = (p(1)?, p(2)?, p(3)?, p(4)?),
                    4 => (tf.a, tf.b, tf.c, tf.d, tf.e, tf.f) = (p(1)?, p(2)?, p(3)?, p(4)?, p(5)?, p(6)?),
                    _ => return None,
                }
            }
            _ => return None,
        }
        Some(tf)
    };
    let (tr, tg, tb) = (curve(b"rTRC")?, curve(b"gTRC")?, curve(b"bTRC")?);
    (tr == tg && tr == tb).then_some((m, tr))
}

/// Convert straight RGBA8 pixels tagged with `icc` to sRGB in place, as
/// Chrome draws them. Returns false (pixels untouched) when the profile is
/// sRGB, unsupported or malformed.
pub fn to_srgb(icc: &[u8], rgba: &mut [u8]) -> bool {
    let Some((to_xyz, tf)) = parse(icc) else { return false };
    // SkColorSpace::MakeRGB snapping.
    let tf = [Tf::SRGB, Tf::GAMMA_2_2, Tf::LINEAR].into_iter().find(|n| tf.almost(n)).unwrap_or(tf);
    let srgb_gamut = to_xyz.iter().flatten().zip(SRGB_TO_XYZ.iter().flatten()).all(|(a, b)| (a - b).abs() < 0.01);
    if tf == Tf::SRGB && srgb_gamut {
        return false;
    }
    let Some(from_xyz) = invert(&SRGB_TO_XYZ) else { return false };
    let m = if srgb_gamut { [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] } else { concat(&from_xyz, &to_xyz) };
    let lut: Vec<f64> = (0..256).map(|i| tf.eval(i as f64 / 255.0)).collect();
    for p in rgba.chunks_exact_mut(4) {
        let l = [lut[p[0] as usize], lut[p[1] as usize], lut[p[2] as usize]];
        for (i, row) in m.iter().enumerate() {
            let v = row[0] as f64 * l[0] + row[1] as f64 * l[1] + row[2] as f64 * l[2];
            let e = Tf::SRGB_INV.eval(v).clamp(0.0, 1.0);
            p[i] = (e * 255.0 + 0.5).floor() as u8;
        }
    }
    true
}

/// Decode an image (PNG, JPEG, GIF, ...) to straight RGBA8 in sRGB, applying
/// its embedded ICC profile like Chrome.
pub fn decode_rgba(data: &[u8]) -> Option<image::RgbaImage> {
    use image::ImageDecoder;
    let reader = image::ImageReader::new(std::io::Cursor::new(data)).with_guessed_format().ok()?;
    let mut decoder = reader.into_decoder().ok()?;
    let icc = decoder.icc_profile().ok().flatten();
    let mut img = image::DynamicImage::from_decoder(decoder).ok()?.to_rgba8();
    if let Some(icc) = icc {
        to_srgb(&icc, &mut img);
    }
    Some(img)
}

/// Whether encoded image data carries an ICC profile (a cheap scan).
pub fn has_icc(data: &[u8]) -> bool {
    if data.starts_with(b"\x89PNG") {
        let mut at = 8;
        while let (Some(len), Some(kind)) = (be32(data, at), data.get(at + 4..at + 8)) {
            match kind {
                b"iCCP" => return true,
                b"IDAT" | b"IEND" => return false,
                _ => at += 12 + len as usize,
            }
        }
        false
    } else {
        data.starts_with(b"\xff\xd8") && data.windows(12).take(1 << 16).any(|w| w == b"ICC_PROFILE\0")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal RGB display profile with the given primaries (rXYZ, gXYZ,
    /// bXYZ) and Display P3's sRGB-like parametric curve.
    fn profile(prims: [[f64; 3]; 3]) -> Vec<u8> {
        let fixed = |v: f64| ((v * 65536.0).round() as i32).to_be_bytes();
        let mut tags: Vec<(&[u8; 4], Vec<u8>)> = Vec::new();
        for (sig, xyz) in [b"rXYZ", b"gXYZ", b"bXYZ"].into_iter().zip(prims) {
            let mut t = b"XYZ \0\0\0\0".to_vec();
            xyz.iter().for_each(|v| t.extend(fixed(*v)));
            tags.push((sig, t));
        }
        let mut para = b"para\0\0\0\0\0\x03\0\0".to_vec();
        for v in [0x0002_6666u32, 0x0000_f2a7, 0x0000_0d59, 0x0000_13d0, 0x0000_0a5b] {
            para.extend(v.to_be_bytes());
        }
        for sig in [b"rTRC", b"gTRC", b"bTRC"] {
            tags.push((sig, para.clone()));
        }
        let mut icc = vec![0u8; 128];
        icc[16..20].copy_from_slice(b"RGB ");
        icc[20..24].copy_from_slice(b"XYZ ");
        icc.extend((tags.len() as u32).to_be_bytes());
        let mut off = 132 + 12 * tags.len();
        let mut body: Vec<u8> = Vec::new();
        for (sig, t) in &tags {
            icc.extend(sig.iter());
            icc.extend((off as u32).to_be_bytes());
            icc.extend((t.len() as u32).to_be_bytes());
            off += t.len();
            body.extend(t);
        }
        icc.extend(body);
        icc
    }

    const DISPLAY_P3: [[f64; 3]; 3] = [[0.5151215, 0.2411957, -0.0010529], [0.291977, 0.6922455, 0.0418854], [0.1571045, 0.0665741, 0.7840729]];

    #[test]
    fn display_p3_matches_chrome() {
        // Colours recorded from Chrome drawing a Display P3 tagged PNG into a canvas.
        let cases = [
            ([55, 0, 129], [61, 0, 135]),
            ([103, 0, 209], [113, 0, 218]),
            ([139, 0, 205], [153, 0, 213]),
            ([200, 150, 22], [209, 147, 0]),
            ([17, 240, 167], [0, 244, 161]),
            ([255, 255, 236], [255, 255, 234]),
            ([128, 128, 0], [128, 128, 0]),
        ];
        let icc = profile(DISPLAY_P3);
        for (src, want) in cases {
            let mut px = [src[0], src[1], src[2], 255];
            assert!(to_srgb(&icc, &mut px));
            assert_eq!(px[..3], want, "{src:?}");
        }
    }

    #[test]
    fn srgb_profile_is_left_alone() {
        let icc = profile([[0.4360657, 0.2224884, 0.0139160], [0.3851471, 0.7168732, 0.0970764], [0.1430664, 0.0606079, 0.7140961]]);
        let mut px = [55, 0, 129, 255];
        assert!(!to_srgb(&icc, &mut px));
        assert_eq!(px, [55, 0, 129, 255]);
    }

    /// The whole grid recorded from Chrome (`conformance/twref`: a Display P3
    /// PNG of 65,536 colours and its canvas pixels), when present.
    #[test]
    fn chrome_grid() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../conformance/out/twref/results");
        let (Ok(png), Ok(raw)) = (std::fs::read(format!("{dir}/p3test.png")), std::fs::read(format!("{dir}/p3-svg.raw"))) else { return };
        let img = decode_rgba(&png).expect("decode");
        let bad = img.as_raw().chunks_exact(4).zip(raw.chunks_exact(4)).filter(|(a, b)| a[..3] != b[..3]).count();
        assert!(bad <= 1, "{bad} of 65536 colours differ from Chrome");
    }
}
