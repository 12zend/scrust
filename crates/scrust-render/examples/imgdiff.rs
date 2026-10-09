//! Screenshot comparison with tolerance.
//!   imgdiff <reference.png> <actual.png> [max % of pixels off by > 32] [max mean abs diff]
//! Exit status 1 when the images differ by more than the tolerance.
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: imgdiff <reference.png> <actual.png> [max_pct_over_32=0.5] [max_mean=1.0]");
        std::process::exit(2);
    }
    let a = image::open(&args[1]).expect("reference image").to_rgb8();
    let b = image::open(&args[2]).expect("actual image").to_rgb8();
    if a.dimensions() != b.dimensions() {
        eprintln!("size mismatch: {:?} vs {:?}", a.dimensions(), b.dimensions());
        std::process::exit(1);
    }
    let max_pct: f64 = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(0.5);
    let max_mean: f64 = args.get(4).and_then(|v| v.parse().ok()).unwrap_or(1.0);
    let (mut sum, mut over32, mut over96, mut max) = (0u64, 0u64, 0u64, 0u8);
    for (p, q) in a.pixels().zip(b.pixels()) {
        let mut d = 0u8;
        for k in 0..3 {
            let x = p.0[k].abs_diff(q.0[k]);
            sum += x as u64;
            d = d.max(x);
        }
        if d > 32 {
            over32 += 1;
        }
        if d > 96 {
            over96 += 1;
        }
        max = max.max(d);
    }
    let n = (a.width() * a.height()) as f64;
    let mean = sum as f64 / (n * 3.0);
    let pct = 100.0 * over32 as f64 / n;
    println!("mean abs diff {:.3}, {:.3}% of pixels off by > 32, {:.3}% by > 96, max {}", mean, pct, 100.0 * over96 as f64 / n, max);
    if pct > max_pct || mean > max_mean {
        println!("FAIL (tolerance: {max_pct}% of pixels, mean {max_mean})");
        std::process::exit(1);
    }
    println!("ok");
}
