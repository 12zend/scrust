//! CPU compositor for scrust.
//!
//! One sampler is shared by drawing and by collision detection, so
//! `touching ...` blocks see exactly what is drawn (scratch-render likewise
//! answers them on the CPU from its silhouettes).

#[cfg(unix)]
pub mod frames;
#[cfg(target_os = "macos")]
mod gpu;
#[cfg(target_arch = "wasm32")]
#[path = "webgl.rs"]
mod gpu;
pub mod scene;
mod color;
mod skin;
mod text;

/// Placeholder where there is no GPU backend: never constructed.
#[cfg(not(any(target_os = "macos", target_arch = "wasm32")))]
mod gpu {
    use crate::skin::{Raster, Skin};
    use crate::Draw;

    pub(crate) enum Gpu {}

    impl Gpu {
        pub fn new(_width: usize, _height: usize) -> Option<Gpu> {
            None
        }
        pub fn resize(&mut self, _width: usize, _height: usize) -> bool {
            match *self {}
        }
        pub fn begin(&mut self, _background: [u8; 3]) {
            match *self {}
        }
        pub fn sprite(&mut self, _skin: &mut Skin, _d: &Draw, _scale: f64) {
            match *self {}
        }
        pub fn pen_layer(&mut self) {
            match *self {}
        }
        pub fn overlay(&mut self, _slot: u64, _r: &Raster, _x: i64, _y: i64) {
            match *self {}
        }
        pub fn finish(&mut self, _out: &mut [u8]) {
            match *self {}
        }
        pub fn pen_clear(&mut self) {
            match *self {}
        }
        pub fn pen_line(&mut self, _color: [f32; 4], _thickness: f32, _length: f32, _pts: [f32; 4]) {
            match *self {}
        }
        pub fn pen_stamp(&mut self, _skin: &mut Skin, _d: &Draw, _scale: f64, _bounds: (f64, f64, f64, f64)) {
            match *self {}
        }
        pub fn pen_pixels(&mut self) -> &Raster {
            match *self {}
        }
        pub fn preload(&mut self, _skin: &mut Skin) {
            match *self {}
        }
        pub fn present(&mut self) {
            match *self {}
        }
        pub unsafe fn attach_view(&mut self, _ns_view: *mut std::ffi::c_void, _width: usize, _height: usize) -> bool {
            match *self {}
        }
        pub fn set_drawable_size(&mut self, _width: usize, _height: usize) {
            match *self {}
        }
    }
}

use gpu::Gpu;

pub use scene::Scene;

use resvg::tiny_skia;
use resvg::usvg;
use scrust_runtime::{Bounds, PenState, ProgramDef, Renderer, RotationStyle, Target, TargetRef, World, stage_height, stage_width};
use skin::{Raster, Skin};
use std::collections::HashMap;
use std::sync::Arc;

const FENCE_WIDTH: f64 = 15.0;
const PI: f64 = std::f64::consts::PI;

/// Model transform of a drawable: `world = M * local + T` with
/// `local = (0.5 - u, v - 0.5)` for texture coordinates `(u, v)`.
#[derive(Clone, Copy)]
struct Xf {
    m00: f64,
    m01: f64,
    m10: f64,
    m11: f64,
    tx: f64,
    ty: f64,
}

/// scratch-render keeps positions, scales and matrices in `Float32Array`s;
/// rounding through `f32` at the same places reproduces its bounds exactly.
#[inline]
fn f32r(x: f64) -> f64 {
    x as f32 as f64
}

impl Xf {
    /// `Drawable._calculateTransform`.
    fn new(pos: (f64, f64), direction: f64, scale: (f64, f64), size: (f64, f64), center: (f64, f64)) -> Xf {
        let rot = (270.0 - direction) * PI / 180.0;
        let (c, s) = (f32r(scrust_value::jsmath::cos(rot)), f32r(scrust_value::jsmath::sin(rot)));
        let ns = f32r(-s);
        let (scale0, scale1) = (f32r(scale.0), f32r(scale.1));
        let adj0 = f32r((center.0 - size.0 / 2.0) * scale0 / 100.0);
        let adj1 = f32r(((center.1 - size.1 / 2.0) * scale1 / 100.0) * -1.0);
        let s0 = f32r(size.0 * scale0 / 100.0);
        let s1 = f32r(size.1 * scale1 / 100.0);
        let (px, py) = (f32r(pos.0), f32r(pos.1));
        Xf {
            m00: f32r(s0 * c),
            m10: f32r(s0 * s),
            m01: f32r(s1 * ns),
            m11: f32r(s1 * c),
            tx: f32r((c * adj0) + (ns * adj1) + px),
            ty: f32r((s * adj0) + (c * adj1) + py),
        }
    }

    /// Texture coordinates -> world, as `_getTransformedHullPoints` does it
    /// (local point and result stored as float32).
    #[inline]
    fn to_world(&self, u: f64, v: f64) -> (f64, f64) {
        let (lx, ly) = (f32r(0.5 - u), f32r(v - 0.5));
        (f32r(self.m00 * lx + self.m01 * ly + self.tx), f32r(self.m10 * lx + self.m11 * ly + self.ty))
    }

    /// World point -> texture coordinates (`getLocalPosition`, with the
    /// float32 inverse matrix and its error allowance).
    #[inline]
    fn to_uv(&self, x: f64, y: f64) -> (f64, f64) {
        let det = self.m00 * self.m11 - self.m01 * self.m10;
        if det == 0.0 {
            return (-1.0, -1.0);
        }
        let (i00, i01, i10, i11) = (f32r(self.m11 / det), f32r(-self.m01 / det), f32r(-self.m10 / det), f32r(self.m00 / det));
        let itx = f32r(-(self.m11 * self.tx - self.m01 * self.ty) / det);
        let ity = f32r(-(-self.m10 * self.tx + self.m00 * self.ty) / det);
        let mut u = 0.5 - (x * i00 + y * i01 + itx);
        let mut v = (x * i10 + y * i11 + ity) + 0.5;
        const ALLOWANCE: f64 = 1e-6;
        if u.abs() < ALLOWANCE {
            u = 0.0;
        }
        if v.abs() < ALLOWANCE {
            v = 0.0;
        }
        (u, v)
    }

    /// `Rectangle.initFromModelMatrix`.
    fn aabb(&self) -> Bounds {
        let x = (0.5 * self.m00).abs() + (0.5 * self.m01).abs();
        let y = (0.5 * self.m10).abs() + (0.5 * self.m11).abs();
        Bounds { left: -x + self.tx, right: x + self.tx, top: y + self.ty, bottom: -y + self.ty }
    }
}

#[derive(Clone, Copy, Default)]
struct Effects {
    color: f64,
    fisheye: f64,
    whirl: f64,
    pixelate: f64,
    mosaic: f64,
    brightness: f64,
    ghost: f64,
}

impl Effects {
    fn of(t: &Target) -> Effects {
        let e = &t.effects;
        Effects { color: e[0], fisheye: e[1], whirl: e[2], pixelate: e[3], mosaic: e[4], brightness: e[5], ghost: e[6] }
    }
    fn distorts(&self) -> bool {
        self.fisheye != 0.0 || self.whirl != 0.0 || self.pixelate != 0.0 || self.mosaic != 0.0
    }
    fn recolors(&self) -> bool {
        self.color != 0.0 || self.brightness != 0.0 || self.ghost != 0.0
    }

    /// `EffectTransform.transformPoint` (mosaic, pixelate, whirl, fisheye).
    fn point(&self, mut u: f64, mut v: f64, size: (f64, f64)) -> (f64, f64) {
        if self.mosaic != 0.0 {
            let m = ((self.mosaic.abs() + 10.0) / 10.0).round().clamp(1.0, 512.0);
            u = (m * u).fract();
            v = (m * v).fract();
        }
        if self.pixelate != 0.0 {
            let p = self.pixelate.abs() / 10.0;
            let (tx, ty) = (size.0 / p, size.1 / p);
            u = ((u * tx).floor() + 0.5) / tx;
            v = ((v * ty).floor() + 0.5) / ty;
        }
        if self.whirl != 0.0 {
            let (ox, oy) = (u - 0.5, v - 0.5);
            let mag = (ox * ox + oy * oy).sqrt();
            let factor = (1.0 - mag / 0.5).max(0.0);
            let angle = (-self.whirl * PI / 180.0) * factor * factor;
            let (s, c) = (libm::sin(angle), libm::cos(angle));
            u = c * ox + s * oy + 0.5;
            v = -s * ox + c * oy + 0.5;
        }
        if self.fisheye != 0.0 {
            let (vx, vy) = ((u - 0.5) / 0.5, (v - 0.5) / 0.5);
            let len = (vx * vx + vy * vy).sqrt();
            if len > 0.0 {
                let power = ((self.fisheye + 100.0) / 100.0).max(0.0);
                let r = libm::pow(len.min(1.0), power) * len.max(1.0);
                u = 0.5 + r * (vx / len) * 0.5;
                v = 0.5 + r * (vy / len) * 0.5;
            }
        }
        (u, v)
    }

    /// `EffectTransform.transformColor` on a premultiplied pixel.
    fn color(&self, p: [u8; 4], with_ghost: bool) -> [u8; 4] {
        if p[3] == 0 {
            return p;
        }
        let a = p[3] as f64 / 255.0;
        let (mut r, mut g, mut b) = (p[0] as f64 / 255.0 / a, p[1] as f64 / 255.0 / a, p[2] as f64 / 255.0 / a);
        if self.color != 0.0 {
            let (mut h, mut s, mut v) = rgb_to_hsv(r, g, b);
            const MIN_V: f64 = 0.11 / 2.0;
            const MIN_S: f64 = 0.09;
            if v < MIN_V {
                h = 0.0;
                s = 1.0;
                v = MIN_V;
            } else if s < MIN_S {
                h = 0.0;
                s = MIN_S;
            }
            h = (h + self.color / 200.0).rem_euclid(1.0);
            let c = hsv_to_rgb(h, s, v);
            r = c.0;
            g = c.1;
            b = c.2;
        }
        if self.brightness != 0.0 {
            let d = self.brightness / 100.0;
            r = (r + d).clamp(0.0, 1.0);
            g = (g + d).clamp(0.0, 1.0);
            b = (b + d).clamp(0.0, 1.0);
        }
        let mut alpha = a;
        if with_ghost && self.ghost != 0.0 {
            alpha *= (1.0 - self.ghost / 100.0).clamp(0.0, 1.0);
        }
        [(r * alpha * 255.0 + 0.5) as u8, (g * alpha * 255.0 + 0.5) as u8, (b * alpha * 255.0 + 0.5) as u8, (alpha * 255.0 + 0.5) as u8]
    }
}

/// Store into a `Uint8ClampedArray` element: NaN is 0, clamp to 0..255,
/// round half to even.
#[inline]
fn u8c(x: f64) -> f64 {
    if !(x > 0.0) {
        return 0.0;
    }
    if x >= 255.0 {
        return 255.0;
    }
    let f = x.floor();
    let d = x - f;
    if d > 0.5 || (d == 0.5 && f % 2.0 != 0.0) {
        f + 1.0
    } else {
        f
    }
}

impl Effects {
    /// `EffectTransform.transformColor` as scratch-render runs it on the CPU
    /// for `touching color`: every write goes through a `Uint8ClampedArray`.
    fn color_touch(&self, c: [f64; 4], with_ghost: bool) -> [f64; 4] {
        let mut c = c;
        if c[3] == 0.0 {
            return c;
        }
        let enable_color = self.color != 0.0;
        let enable_brightness = self.brightness != 0.0;
        if enable_color || enable_brightness {
            let alpha = c[3] / 255.0;
            for v in c.iter_mut().take(3) {
                *v = u8c(*v / alpha);
            }
            if enable_color {
                // color-conversions.js rgbToHsv
                let (mut r, mut g, mut b) = (c[0] / 255.0, c[1] / 255.0, c[2] / 255.0);
                let mut k = 0.0;
                if g < b {
                    std::mem::swap(&mut g, &mut b);
                    k = -1.0;
                }
                if r < g {
                    std::mem::swap(&mut r, &mut g);
                    k = -2.0 / 6.0 - k;
                }
                let chroma = r - g.min(b);
                let mut h = (k + (g - b) / (6.0 * chroma + f64::EPSILON)).abs();
                let mut s = chroma / (r + f64::EPSILON);
                let mut v = r;
                const MIN_V: f64 = 0.11 / 2.0;
                const MIN_S: f64 = 0.09;
                if v < MIN_V {
                    h = 0.0;
                    s = 1.0;
                    v = MIN_V;
                } else if s < MIN_S {
                    h = 0.0;
                    s = MIN_S;
                }
                h = (self.color / 200.0) % 1.0 + h + 1.0;
                // hsvToRgb: writes `x * 255 + 0.5`
                let rgb = if s == 0.0 {
                    (v, v, v)
                } else {
                    h %= 1.0;
                    let i = (h * 6.0) as i32;
                    let f = h * 6.0 - i as f64;
                    let p = v * (1.0 - s);
                    let q = v * (1.0 - s * f);
                    let t = v * (1.0 - s * (1.0 - f));
                    match i {
                        0 => (v, t, p),
                        1 => (q, v, p),
                        2 => (p, v, t),
                        3 => (p, q, v),
                        4 => (t, p, v),
                        5 => (v, p, q),
                        _ => (0.0, 0.0, 0.0),
                    }
                };
                c[0] = u8c(rgb.0 * 255.0 + 0.5);
                c[1] = u8c(rgb.1 * 255.0 + 0.5);
                c[2] = u8c(rgb.2 * 255.0 + 0.5);
            }
            if enable_brightness {
                let b = self.brightness.clamp(-100.0, 100.0) / 100.0 * 255.0;
                for v in c.iter_mut().take(3) {
                    *v = u8c(*v + b);
                }
            }
            for v in c.iter_mut().take(3) {
                *v = u8c(*v * alpha);
            }
        }
        if with_ghost && self.ghost != 0.0 {
            let g = 1.0 - self.ghost.clamp(0.0, 100.0) / 100.0;
            for v in c.iter_mut() {
                *v = u8c(*v * g);
            }
        }
        c
    }
}

fn rgb_to_hsv(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        ((g - b) / d).rem_euclid(6.0) / 6.0
    } else if max == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    (h, if max == 0.0 { 0.0 } else { d / max }, max)
}

fn hsv_to_rgb(h: f64, s: f64, v: f64) -> (f64, f64, f64) {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - f * s), v * (1.0 - (1.0 - f) * s));
    match (i as i64).rem_euclid(6) {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    }
}

/// `_getConvexHullPointsForDrawable`: per skin row, the first and last
/// column (in skin units) that pass the linear silhouette test.
fn hull_rows(base: &Raster, size: (f64, f64), fx: Option<Effects>) -> Vec<(f64, f64, f64)> {
    let (w, h) = size;
    let mut rows = Vec::new();
    let test = |x: f64, y: f64| -> bool {
        let (mut u, mut v) = (x / w, y / h);
        if let Some(fx) = fx {
            let p = fx.point(u, v, size);
            u = p.0;
            v = p.1;
        }
        silhouette_touching(base, u, v, false)
    };
    let mut y = 0.0;
    while y < h {
        let mut x = 0.0;
        let mut left = None;
        while x < w {
            if test(x, y) {
                left = Some(x);
                break;
            }
            x += 1.0;
        }
        if let Some(l) = left {
            let mut r = l;
            let mut x = w.ceil() - 1.0;
            if x >= w {
                x -= 1.0;
            }
            // scratch-render starts at `width - 1` with the (possibly fractional) width.
            let mut x = if w.fract() == 0.0 { x } else { w - 1.0 };
            while x >= 0.0 {
                if test(x, y) {
                    r = x;
                    break;
                }
                x -= 1.0;
            }
            rows.push((y, l, r));
        }
        y += 1.0;
    }
    rows
}

/// Everything needed to sample one drawable.
struct Draw {
    xf: Xf,
    fx: Effects,
    sprite: usize,
    costume: usize,
    size: (f64, f64),
    /// `Skin.useNearest`: nearest-neighbour silhouette test, else the 2x2 "linear" test.
    nearest: bool,
    /// The drawable's position (rounded unless high quality).
    pos: (f64, f64),
    /// SVG skin that may be sampled nearest at ~100% (no distortion, upright).
    svg_nearest_ok: bool,
    is_svg: bool,
    /// Drawable scale in percent (`drawable.scale`).
    pct: (f64, f64),
}

impl Draw {
    /// Nothing to draw: an empty skin, or a transform that is not finite
    /// (a zero-sized skin at an infinite size gives NaN), which WebGL
    /// rasterises as nothing.
    fn degenerate(&self) -> bool {
        let x = &self.xf;
        self.size.0 <= 0.0 || self.size.1 <= 0.0 || ![x.m00, x.m01, x.m10, x.m11, x.tx, x.ty].iter().all(|v| v.is_finite())
    }

    /// Transform and sampling mode for a skin drawn at `pos` with a Scratch
    /// direction and percentage scale (`RenderedTarget` -> `Drawable`).
    fn new(skin: &Skin, pos: (f64, f64), direction: f64, scale: (f64, f64), fx: Effects, sprite: usize, costume: usize) -> Draw {
        let is_svg = matches!(skin.kind, skin::SkinKind::Svg(..));
        let near100 = |v: f64| v.abs() > 99.0 && v.abs() < 101.0;
        let svg_nearest_ok = !fx.distorts() && direction % 90.0 == 0.0;
        let nearest = !is_svg || (svg_nearest_ok && near100(scale.0) && near100(scale.1));
        Draw { xf: Xf::new(pos, direction, scale, skin.size, skin.center), fx, sprite, costume, size: skin.size, nearest, pos, svg_nearest_ok, is_svg, pct: scale }
    }

    /// `skin.useNearest(drawableScale)` when drawing at `k` device pixels per
    /// stage unit: bitmaps always use NEAREST, SVGs only at about 100%.
    fn draw_nearest(&self, k: f64) -> bool {
        let near100 = |v: f64| v.abs() > 99.0 && v.abs() < 101.0;
        !self.is_svg || (self.svg_nearest_ok && near100(self.pct.0 * k) && near100(self.pct.1 * k))
    }
}

/// `Silhouette.isTouchingNearest` / `isTouchingLinear` at texture
/// coordinates (which may lie slightly outside `[0, 1]`).
fn silhouette_touching(base: &Raster, u: f64, v: f64, nearest: bool) -> bool {
    let x = (u * (base.w as f64 - 1.0)).floor();
    let y = (v * (base.h as f64 - 1.0)).floor();
    let point = |x: f64, y: f64| -> bool {
        if x < 0.0 || y < 0.0 || x >= base.w as f64 || y >= base.h as f64 {
            return false;
        }
        base.px(x as usize, y as usize)[3] > 0
    };
    if nearest {
        point(x, y)
    } else {
        point(x, y) || point(x + 1.0, y) || point(x, y + 1.0) || point(x + 1.0, y + 1.0)
    }
}

/// Is the drawable opaque at world point `(x, y)`? (`Drawable.isTouching`)
fn touching_skin(skin: &mut Skin, d: &Draw, x: f64, y: f64) -> bool {
    touching_raster(skin.silhouette(d.pct), d, x, y)
}

/// `Drawable.isTouching` against a given silhouette raster.
fn touching_raster(sil: &Raster, d: &Draw, x: f64, y: f64) -> bool {
    if d.degenerate() {
        return false;
    }
    let (mut u, mut v) = d.xf.to_uv(x, y);
    if d.fx.distorts() && (0.0..1.0).contains(&u) && (0.0..1.0).contains(&v) {
        let p = d.fx.point(u, v, d.size);
        u = p.0;
        v = p.1;
    }
    silhouette_touching(sil, u, v, d.nearest)
}

/// Tight bounds (`Drawable.getBounds`): the convex hull scratch-render
/// builds by scanning each row of the skin with the "linear" silhouette
/// test, transformed through pixel centres.
fn tight_bounds(skin: &mut Skin, d: &Draw) -> Bounds {
    let xf = d.xf;
    let (w, h) = d.size;
    let fx = d.fx;
    let distort = fx.distorts();
    skin.silhouette(d.pct);
    if !distort && skin.hull.is_none() {
        let rows = hull_rows(skin.sil_raster(), (w, h), None);
        skin.hull = Some(rows);
    }
    let computed;
    let rows: &Vec<(f64, f64, f64)> = if distort {
        computed = hull_rows(skin.sil_raster(), (w, h), Some(fx));
        &computed
    } else {
        skin.hull.as_ref().expect("hull cached")
    };
    let mut b = Bounds { left: f64::INFINITY, right: f64::NEG_INFINITY, top: f64::NEG_INFINITY, bottom: f64::INFINITY };
    let (hx, hy) = (1.0 / w / 2.0, 1.0 / h / 2.0);
    for &(y, x0, x1) in rows {
        for x in [x0, x1] {
            // dst = (0.5 - x/w - halfPixel, y/h - 0.5 + halfPixel)
            let (wx, wy) = xf.to_world(x / w + hx, y / h + hy);
            b.left = b.left.min(wx);
            b.right = b.right.max(wx);
            b.top = b.top.max(wy);
            b.bottom = b.bottom.min(wy);
        }
    }
    if rows.is_empty() {
        xf.aabb()
    } else {
        b
    }
}

/// `RenderWebGL.getFencedPositionOfDrawable`: keep a drawable currently at
/// `cur` within the stage when it moves to `(x, y)`.
fn fence(d: &Draw, cur: (f64, f64), x: f64, y: f64) -> (f64, f64) {
    let (mut x, mut y) = (x, y);
    let dx = x - cur.0;
    let dy = y - cur.1;
    let aabb = d.xf.aabb();
    let inset = ((aabb.right - aabb.left).min(aabb.top - aabb.bottom) / 2.0).floor();
    let sx = stage_width() / 2.0 - FENCE_WIDTH.min(inset);
    if aabb.right + dx < -sx {
        x = (cur.0 - (sx + aabb.right)).ceil();
    } else if aabb.left + dx > sx {
        x = (cur.0 + (sx - aabb.left)).floor();
    }
    let sy = stage_height() / 2.0 - FENCE_WIDTH.min(inset);
    if aabb.top + dy < -sy {
        y = (cur.1 - (sy + aabb.top)).ceil();
    } else if aabb.bottom + dy > sy {
        y = (cur.1 + (sy - aabb.bottom)).floor();
    }
    (x, y)
}

fn blend(dst: &mut [u8], src: [u8; 4]) {
    let ia = 255 - src[3] as u32;
    for i in 0..4 {
        dst[i] = (src[i] as u32 + (dst[i] as u32 * ia + 127) / 255).min(255) as u8;
    }
}

/// Composite one drawable into `target` (device pixels).
fn composite(skin: &mut Skin, d: &Draw, s: f64, target: &mut Raster) {
    if d.degenerate() {
        return;
    }
    let b = d.xf.aabb();
    let x0 = (((b.left + stage_width() / 2.0) * s).floor().max(0.0)) as usize;
    let x1 = (((b.right + stage_width() / 2.0) * s).ceil().min(target.w as f64)).max(0.0) as usize;
    let y0 = (((stage_height() / 2.0 - b.top) * s).floor().max(0.0)) as usize;
    let y1 = (((stage_height() / 2.0 - b.bottom) * s).ceil().min(target.h as f64)).max(0.0) as usize;
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    // Device pixels per stage unit of this drawable's texture.
    let unit = (d.xf.m00.hypot(d.xf.m10) / d.size.0.max(1e-9)).max(d.xf.m01.hypot(d.xf.m11) / d.size.1.max(1e-9));
    let fx = d.fx;
    let size = d.size;
    let xf = d.xf;
    let distort = fx.distorts();
    let recolor = fx.recolors();
    let raster = skin.raster_for(unit * s);
    let nearest = d.draw_nearest(s);
    for py in y0..y1 {
        let wy = stage_height() / 2.0 - (py as f64 + 0.5) / s;
        for px in x0..x1 {
            let wx = (px as f64 + 0.5) / s - stage_width() / 2.0;
            let (mut u, mut v) = xf.to_uv(wx, wy);
            if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                continue;
            }
            if distort {
                let p = fx.point(u, v, size);
                u = p.0;
                v = p.1;
                if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                    continue;
                }
            }
            let mut c = if nearest { raster.nearest(u, v) } else { raster.linear(u, v) };
            if c[3] == 0 {
                continue;
            }
            if recolor {
                c = fx.color(c, true);
            }
            let i = (py * target.w + px) * 4;
            blend(&mut target.data[i..i + 4], c);
        }
    }
}

/// Where text overlays go: the CPU canvas or the GPU frame.
enum Overlays<'a> {
    Cpu(&'a mut Raster),
    Gpu(&'a mut Gpu),
}

impl Overlays<'_> {
    /// Draw `r` at device pixel `(x, y)`; `slot` names the overlay (GPU texture reuse).
    fn put(&mut self, slot: u64, r: &Raster, x: i64, y: i64) {
        match self {
            Overlays::Cpu(canvas) => SoftRenderer::blit(canvas, r, x, y),
            Overlays::Gpu(gpu) => gpu.overlay(slot, r, x, y),
        }
    }
}

const SLOT_BUBBLE: u64 = 1 << 32;
const SLOT_MONITOR: u64 = 2 << 32;
const SLOT_ASK: u64 = 3 << 32;

pub struct SoftRenderer {
    program: &'static ProgramDef,
    /// Device pixels per stage unit.
    pub scale: f64,
    pub width: usize,
    pub height: usize,
    skins: Vec<Vec<Option<Skin>>>,
    opt: usvg::Options<'static>,
    pen: tiny_skia::Pixmap,
    pen_used: bool,
    /// Premultiplied RGBA composition buffer.
    canvas: Raster,
    /// `0x00RRGGBB` pixels ready for presentation.
    pub frame: Vec<u32>,
    bubbles: HashMap<u32, (u64, bool, Raster)>,
    /// `bubbleState.onSpriteRight` per target.
    bubble_side: HashMap<u32, bool>,
    monitors: HashMap<usize, (String, Raster)>,
    ask: Option<(String, Raster)>,
    /// Text typed so far into the ask prompt (owned by the player).
    pub ask_input: String,
    /// TurboWarp's high quality render (`hq`): drawables keep fractional
    /// positions; otherwise `Drawable.updatePosition` rounds them.
    pub high_quality: bool,
    /// Draw monitors and the ask prompt into the frame (off for comparisons
    /// with scratch-render's canvas, which has neither).
    pub draw_html_overlays: bool,
    /// Metal compositor for drawing and the pen layer; None draws on the CPU.
    gpu: Option<Gpu>,
}

impl SoftRenderer {
    /// A renderer that draws on the GPU when one is available (Metal on
    /// macOS, unless `SCRUST_RENDERER=cpu`), else on the CPU.
    pub fn new(program: &'static ProgramDef, scale: f64) -> SoftRenderer {
        let mut r = Self::new_cpu(program, scale);
        r.gpu = Gpu::new(r.width, r.height);
        r.preload();
        r
    }

    /// Decode every costume up front, in parallel, and upload bitmaps to the
    /// GPU, as Scratch does when it loads a project. Decoding on first use
    /// stalls the frame that first shows a costume, which projects that
    /// cycle through thousands of costumes do on almost every frame.
    fn preload(&mut self) {
        let jobs: Vec<(usize, usize)> = (self.skins.iter().enumerate())
            .flat_map(|(s, cs)| cs.iter().enumerate().filter(|(_, c)| c.is_none()).map(move |(c, _)| (s, c)))
            .collect();
        if jobs.is_empty() {
            return;
        }
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).min(jobs.len());
        let (program, opt) = (self.program, &self.opt);
        let load = |&(s, c): &(usize, usize)| {
            let def = &program.sprites[s].costumes[c];
            ((s, c), Skin::load(def.data, def.data_format, def.bitmap_resolution, (def.center_x, def.center_y), opt))
        };
        let loaded: Vec<((usize, usize), Skin)> = if threads <= 1 {
            jobs.iter().map(load).collect()
        } else {
            let chunk = jobs.len().div_ceil(threads);
            std::thread::scope(|scope| {
                let handles: Vec<_> = jobs.chunks(chunk).map(|part| scope.spawn(move || part.iter().map(load).collect::<Vec<_>>())).collect();
                handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
            })
        };
        for ((s, c), mut skin) in loaded {
            if let (Some(gpu), skin::SkinKind::Bitmap) = (self.gpu.as_mut(), &skin.kind) {
                gpu.preload(&mut skin);
            }
            self.skins[s][c] = Some(skin);
        }
    }

    /// A renderer that draws everything on the CPU (deterministic output).
    pub fn new_cpu(program: &'static ProgramDef, scale: f64) -> SoftRenderer {
        let width = (stage_width() * scale).round() as usize;
        let height = (stage_height() * scale).round() as usize;
        let mut opt = usvg::Options::default();
        {
            let db = Arc::make_mut(&mut opt.fontdb);
            text::load_fonts(db);
        }
        text::configure(&mut opt);
        SoftRenderer {
            program,
            scale,
            width,
            height,
            skins: program.sprites.iter().map(|s| s.costumes.iter().map(|_| None).collect()).collect(),
            opt,
            pen: tiny_skia::Pixmap::new(width as u32, height as u32).expect("pen layer"),
            pen_used: false,
            canvas: Raster::new(width, height),
            frame: vec![0xFFFFFF; width * height],
            bubbles: HashMap::new(),
            bubble_side: HashMap::new(),
            monitors: HashMap::new(),
            ask: None,
            ask_input: String::new(),
            gpu: None,
            high_quality: false,
            draw_html_overlays: true,
        }
    }

    /// Whether frames are drawn on the GPU.
    pub fn uses_gpu(&self) -> bool {
        self.gpu.is_some()
    }

    /// Premultiplied pen-layer pixel at device coordinates.
    /// Pen layer size in device pixels.
    fn pen_size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    fn pen_pixel(&mut self, x: u32, y: u32) -> Option<[u8; 4]> {
        if let Some(gpu) = self.gpu.as_mut() {
            let r = gpu.pen_pixels();
            return ((x as usize) < r.w && (y as usize) < r.h).then(|| r.px(x as usize, y as usize));
        }
        self.pen.pixel(x, y).map(|p| [p.red(), p.green(), p.blue(), p.alpha()])
    }

    fn skin(&mut self, sprite: usize, costume: usize) -> &mut Skin {
        let slot = &mut self.skins[sprite][costume];
        if slot.is_none() {
            let c = &self.program.sprites[sprite].costumes[costume];
            *slot = Some(Skin::load(c.data, c.data_format, c.bitmap_resolution, (c.center_x, c.center_y), &self.opt));
        }
        slot.as_mut().expect("skin loaded")
    }

    /// `RenderedTarget._getRenderedDirectionAndScale` + transform.
    fn draw_info(&mut self, t: &Target) -> Option<Draw> {
        let n = self.program.sprites[t.sprite].costumes.len();
        if n == 0 {
            return None;
        }
        let costume = t.costume.min(n - 1);
        let (direction, scale) = if t.is_stage {
            (90.0, (100.0, 100.0))
        } else {
            match t.rotation_style {
                RotationStyle::AllAround => (t.direction, (t.size, t.size)),
                RotationStyle::DontRotate => (90.0, (t.size, t.size)),
                RotationStyle::LeftRight => (90.0, (if t.direction < 0.0 { -t.size } else { t.size }, t.size)),
            }
        };
        let pos = if t.is_stage {
            (0.0, 0.0)
        } else if self.high_quality {
            (t.x, t.y)
        } else {
            (scrust_value::js_round(t.x), scrust_value::js_round(t.y))
        };
        let fx = Effects::of(t);
        let skin = self.skin(t.sprite, costume);
        Some(Draw::new(skin, pos, direction, scale, fx, t.sprite, costume))
    }

    /// Is the drawable opaque at world point `(x, y)`? (`Drawable.isTouching`)
    fn touching(&mut self, d: &Draw, x: f64, y: f64) -> bool {
        touching_skin(self.skin(d.sprite, d.costume), d, x, y)
    }

    /// Colour of the drawable at a world point (`Drawable.sampleColor4b`).
    /// `Drawable.sampleColor4b`: nearest silhouette texel at the local
    /// position (shape-changing effects are not applied), then
    /// `EffectTransform.transformColor`.
    fn sample(&mut self, d: &Draw, x: f64, y: f64, with_ghost: bool) -> [f64; 4] {
        let (u, v) = d.xf.to_uv(x, y);
        if u < 0.0 || v < 0.0 || u > 1.0 || v > 1.0 {
            return [0.0; 4];
        }
        // `Silhouette.colorAtNearest` (clamped to the edge)
        let base = self.skin(d.sprite, d.costume).silhouette(d.pct);
        if base.w == 0 || base.h == 0 {
            return [0.0; 4];
        }
        let sx = ((u * (base.w as f64 - 1.0)).floor().max(0.0) as usize).min(base.w - 1);
        let sy = ((v * (base.h as f64 - 1.0)).floor().max(0.0) as usize).min(base.h - 1);
        let px = base.px(sx, sy);
        let c = [px[0] as f64, px[1] as f64, px[2] as f64, px[3] as f64];
        if d.fx.recolors() {
            d.fx.color_touch(c, with_ghost)
        } else {
            c
        }
    }

    fn tight_bounds(&mut self, d: &Draw) -> Bounds {
        tight_bounds(self.skin(d.sprite, d.costume), d)
    }

    /// Integer scan region for collision tests: `getFastBounds()` clamped to
    /// the stage and snapped outwards to integers (`_touchingBounds`).
    ///
    /// The quad's AABB is used (scratch-render switches to the tighter hull
    /// bounds once they have been computed for a drawable). Note that the
    /// float noise of `sin(180 deg)` in the transform pushes exact integer
    /// edges just past the integer, so snapping grows the box by one pixel -
    /// that is why sprites one pixel apart still "touch" in Scratch.
    ///
    /// With TurboWarp's fencing off (`offscreenTouching`) the region is not
    /// clamped to the stage.
    fn scan_bounds(&mut self, d: &Draw, offscreen: bool) -> Option<(i64, i64, i64, i64)> {
        let b = d.xf.aabb();
        let (xl, xr, yb, yt) = if offscreen {
            (f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY)
        } else {
            (-stage_width() / 2.0, stage_width() / 2.0, -stage_height() / 2.0, stage_height() / 2.0)
        };
        // Rectangle.clamp
        let left = b.left.max(xl).min(xr);
        let right = b.right.min(xr).max(xl);
        let bottom = b.bottom.max(yb).min(yt);
        let top = b.top.min(yt).max(yb);
        // Rectangle.snapToInt
        let (l, r, bo, t) = (left.floor() as i64, right.ceil() as i64, bottom.floor() as i64, top.ceil() as i64);
        if r - l == 0 || t - bo == 0 {
            None
        } else {
            Some((l, r, bo, t))
        }
    }

    /// Snapped AABB of a candidate (not clamped), as in `_candidatesTouching`.
    fn snapped_aabb(d: &Draw) -> (i64, i64, i64, i64) {
        let b = d.xf.aabb();
        (b.left.floor() as i64, b.right.ceil() as i64, b.bottom.floor() as i64, b.top.ceil() as i64)
    }

    // ------------------------------------------------------------- drawing

    /// Composite one drawable into `target` (device pixels).
    fn draw_into(&mut self, d: &Draw, target: &mut Raster) {
        let s = self.scale;
        composite(self.skin(d.sprite, d.costume), d, s, target);
    }

    fn blit(target: &mut Raster, src: &Raster, x: i64, y: i64) {
        for sy in 0..src.h as i64 {
            let ty = y + sy;
            if ty < 0 || ty >= target.h as i64 {
                continue;
            }
            for sx in 0..src.w as i64 {
                let tx = x + sx;
                if tx < 0 || tx >= target.w as i64 {
                    continue;
                }
                let c = src.px(sx as usize, sy as usize);
                if c[3] == 0 {
                    continue;
                }
                let i = (ty as usize * target.w + tx as usize) * 4;
                blend(&mut target.data[i..i + 4], c);
            }
        }
    }

    /// Render the whole stage into `self.frame`.
    pub fn draw(&mut self, w: &World) {
        if let Some(mut gpu) = self.gpu.take() {
            self.draw_gpu(w, &mut gpu);
            self.gpu = Some(gpu);
            return;
        }
        let mut canvas = std::mem::replace(&mut self.canvas, Raster::new(0, 0));
        canvas.data.fill(255);
        // Stage backdrop.
        if let Some(d) = self.draw_info(w.target(w.order[0])) {
            self.draw_into(&d, &mut canvas);
        }
        if self.pen_used {
            let pen = Raster { w: self.width, h: self.height, data: self.pen.data().to_vec() };
            Self::blit(&mut canvas, &pen, 0, 0);
        }
        for t in w.order.iter().skip(1) {
            let tg = w.target(*t);
            if !tg.visible {
                continue;
            }
            if let Some(d) = self.draw_info(tg) {
                self.draw_into(&d, &mut canvas);
            }
        }
        let mut out = Overlays::Cpu(&mut canvas);
        self.draw_overlays(w, &mut out);
        self.present(&canvas);
        self.canvas = canvas;
    }

    /// Present frames straight from the GPU into the window whose `NSView*`
    /// is `ns_view` (`width` x `height` device pixels), instead of reading
    /// them back for `frame`. False when there is no GPU (draw with `draw`).
    ///
    /// # Safety
    /// `ns_view` must be a live `NSView` and this must run on the main thread.
    pub unsafe fn attach_view(&mut self, ns_view: *mut std::ffi::c_void, width: usize, height: usize) -> bool {
        match self.gpu.as_mut() {
            Some(gpu) => gpu.attach_view(ns_view, width, height),
            None => false,
        }
    }

    /// The attached window changed size (device pixels).
    pub fn resize_view(&mut self, width: usize, height: usize) {
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.set_drawable_size(width, height);
        }
    }

    /// Render the stage and present it in the attached view (`attach_view`).
    pub fn draw_to_view(&mut self, w: &World) {
        if let Some(mut gpu) = self.gpu.take() {
            self.compose_gpu(w, &mut gpu);
            gpu.present();
            self.gpu = Some(gpu);
        }
    }

    /// The last frame drawn by `draw`, as opaque RGBA.
    pub fn rgba(&self) -> &[u8] {
        &self.canvas.data
    }

    /// The same frame composited by the GPU.
    fn draw_gpu(&mut self, w: &World, gpu: &mut Gpu) {
        self.compose_gpu(w, gpu);
        let mut canvas = std::mem::replace(&mut self.canvas, Raster::new(0, 0));
        gpu.finish(&mut canvas.data);
        self.present(&canvas);
        self.canvas = canvas;
    }

    /// Queue the stage, the drawables and the overlays on the GPU.
    fn compose_gpu(&mut self, w: &World, gpu: &mut Gpu) {
        let s = self.scale;
        gpu.begin([255, 255, 255]);
        if let Some(d) = self.draw_info(w.target(w.order[0])) {
            gpu.sprite(self.skin(d.sprite, d.costume), &d, s);
        }
        if self.pen_used {
            gpu.pen_layer();
        }
        for t in w.order.iter().skip(1) {
            let tg = w.target(*t);
            if !tg.visible {
                continue;
            }
            if let Some(d) = self.draw_info(tg) {
                gpu.sprite(self.skin(d.sprite, d.costume), &d, s);
            }
        }
        let mut out = Overlays::Gpu(gpu);
        self.draw_overlays(w, &mut out);
    }

    fn draw_overlays(&mut self, w: &World, out: &mut Overlays) {
        self.draw_bubbles(w, out);
        // Monitors and the ask prompt are HTML over the stage in the GUI.
        if self.draw_html_overlays {
            self.draw_monitors(w, out);
            self.draw_ask(w, out);
        }
    }

    /// Copy a composed RGBA canvas into `self.frame`.
    fn present(&mut self, canvas: &Raster) {
        for (o, p) in self.frame.iter_mut().zip(canvas.data.chunks_exact(4)) {
            *o = ((p[0] as u32) << 16) | ((p[1] as u32) << 8) | p[2] as u32;
        }
    }

    /// Speech bubbles, placed like scratch-vm's `_positionBubble`: to the
    /// right of the top 8 px slice of the sprite, flipped to its left when
    /// it would not fit (the side is remembered), kept on stage; the
    /// bubble drawable's position is rounded like any drawable's.
    fn draw_bubbles(&mut self, w: &World, out: &mut Overlays) {
        let s = self.scale;
        let mut live: Vec<u32> = Vec::new();
        for t in w.order.iter().skip(1) {
            let tg = w.target(*t);
            let Some(b) = &tg.bubble else { continue };
            if b.text.is_empty() || !tg.visible {
                continue;
            }
            live.push(t.idx);
            let text = b.text.to_rust_string();
            let (_, pw, ph) = text::bubble_layout(&text, &self.opt);
            let (bw, bh) = (pw + 4.0, ph + 4.0 + 12.0);
            let tb = match self.draw_info(tg) {
                Some(d) => self.bubble_bounds(&d),
                None => Bounds { left: tg.x, right: tg.x, top: tg.y, bottom: tg.y },
            };
            let (sl, sr, st) = (-stage_width() / 2.0, stage_width() / 2.0, stage_height() / 2.0);
            let right = self.bubble_side.entry(t.idx).or_insert(true);
            if *right && bw + tb.right > sr && tb.left - bw > sl {
                *right = false;
            } else if !*right && tb.left - bw < sl && bw + tb.right < sr {
                *right = true;
            }
            let on_right = *right;
            let x = if on_right { sl.max((sr - bw).min(tb.right)) } else { (sr - bw).min(sl.max(tb.left - bw)) };
            let y = st.min(tb.top + bh);
            let (x, y) = if self.high_quality { (x, y) } else { (scrust_value::js_round(x), scrust_value::js_round(y)) };
            let fresh = !matches!(self.bubbles.get(&t.idx), Some((u, side, _)) if *u == b.usage && *side == on_right);
            if fresh {
                let r = text::bubble(&text, b.think, !on_right, s, &self.opt);
                self.bubbles.insert(t.idx, (b.usage, on_right, r));
            }
            let px = ((x + stage_width() / 2.0) * s).round() as i64;
            let py = ((stage_height() / 2.0 - y) * s).round() as i64;
            let r = &self.bubbles[&t.idx].2;
            out.put(SLOT_BUBBLE | t.idx as u64, r, px, py);
        }
        self.bubbles.retain(|k, _| live.contains(k));
        self.bubble_side.retain(|k, _| w.targets.get(*k as usize).is_some());
    }

    /// `Drawable.getBoundsForBubble`: bounds of the hull points within 8
    /// units of the top.
    fn bubble_bounds(&mut self, d: &Draw) -> Bounds {
        let skin = self.skin(d.sprite, d.costume);
        skin.silhouette(d.pct);
        let rows = hull_rows(skin.sil_raster(), d.size, if d.fx.distorts() { Some(d.fx) } else { None });
        let (w, h) = d.size;
        let (hx, hy) = (1.0 / w / 2.0, 1.0 / h / 2.0);
        let pts: Vec<(f64, f64)> = rows.iter().flat_map(|&(y, x0, x1)| [(x0, y), (x1, y)]).map(|(x, y)| d.xf.to_world(x / w + hx, y / h + hy)).collect();
        if pts.is_empty() {
            return d.xf.aabb();
        }
        let max_y = pts.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
        let mut b = Bounds { left: f64::INFINITY, right: f64::NEG_INFINITY, top: f64::NEG_INFINITY, bottom: f64::INFINITY };
        for &(x, y) in pts.iter().filter(|p| p.1 > max_y - 8.0) {
            b.left = b.left.min(x);
            b.right = b.right.max(x);
            b.top = b.top.max(y);
            b.bottom = b.bottom.min(y);
        }
        b
    }

    fn monitor_value(&self, w: &World, i: usize) -> Option<text::MonitorValue> {
        let m = &self.program.monitors[i];
        let target = w.originals.get(m.sprite).copied()?;
        let tg = w.target(target);
        let num = |n: f64| text::MonitorValue::Scalar(text::format_number(n));
        Some(match m.opcode {
            "data_variable" => {
                let vars = tg.locals.vars();
                text::MonitorValue::Scalar(text::format_value(&vars.get(m.index)?.1))
            }
            "data_listcontents" => {
                let lists = tg.locals.lists();
                text::MonitorValue::List(lists.get(m.index)?.1.iter().map(text::format_value).collect())
            }
            "motion_xposition" => num(tg.x),
            "motion_yposition" => num(tg.y),
            "motion_direction" => num(tg.direction),
            "looks_size" => num(tg.size.round()),
            "sound_volume" => num(tg.volume),
            "looks_costumenumbername" | "looks_backdropnumbername" => {
                let tg = if m.opcode == "looks_backdropnumbername" { w.target(w.order[0]) } else { tg };
                if m.param == "number" {
                    num(tg.costume as f64 + 1.0)
                } else {
                    text::MonitorValue::Scalar(self.program.sprites[tg.sprite].costumes.get(tg.costume).map(|c| c.name).unwrap_or("").to_string())
                }
            }
            "sensing_answer" => text::MonitorValue::Scalar(w.answer.to_rust_string()),
            "sensing_timer" => num((w.current_msecs - w.timer_start) / 1000.0),
            "sensing_username" => text::MonitorValue::Scalar(w.username.to_rust_string()),
            _ => return None,
        })
    }

    fn draw_monitors(&mut self, w: &World, out: &mut Overlays) {
        let s = self.scale;
        for i in 0..self.program.monitors.len() {
            if !w.monitor_visible.get(i).copied().unwrap_or(false) {
                continue;
            }
            let Some(value) = self.monitor_value(w, i) else { continue };
            let m = &self.program.monitors[i];
            let key = format!("{:?}", value);
            let fresh = !matches!(self.monitors.get(&i), Some((k, _)) if *k == key);
            if fresh {
                let r = text::monitor(m, &value, s, &self.opt);
                self.monitors.insert(i, (key, r));
            }
            let r = &self.monitors[&i].1;
            out.put(SLOT_MONITOR | i as u64, r, (m.x * s).round() as i64, (m.y * s).round() as i64);
        }
    }

    fn draw_ask(&mut self, w: &World, out: &mut Overlays) {
        let Some((question, in_bubble)) = w.current_question() else {
            self.ask = None;
            return;
        };
        let q = if in_bubble { String::new() } else { question.to_rust_string() };
        let key = format!("{}\u{0}{}", q, self.ask_input);
        if !matches!(&self.ask, Some((k, _)) if *k == key) {
            self.ask = Some((key, text::ask_box(&q, &self.ask_input, self.scale, &self.opt)));
        }
        if let Some((_, r)) = &self.ask {
            let y = self.height as i64 - r.h as i64;
            out.put(SLOT_ASK, r, 0, y);
        }
    }
}

impl Renderer for SoftRenderer {
    fn costume_size(&mut self, sprite: usize, costume: usize) -> (f64, f64) {
        if costume >= self.program.sprites[sprite].costumes.len() {
            return (1.0, 1.0);
        }
        self.skin(sprite, costume).size
    }

    fn bounds(&mut self, w: &World, t: TargetRef) -> Option<Bounds> {
        let tg = w.target(t);
        let d = self.draw_info(tg)?;
        // `_getConvexHullPointsForDrawable` returns no points for a hidden
        // drawable, and `getFastBounds` then falls back to the quad's AABB.
        if !tg.visible {
            return Some(d.xf.aabb());
        }
        Some(self.tight_bounds(&d))
    }

    /// `RenderWebGL.getFencedPositionOfDrawable`.
    fn fenced_position(&mut self, w: &World, t: TargetRef, x: f64, y: f64) -> (f64, f64) {
        let tg = w.target(t);
        let Some(d) = self.draw_info(tg) else { return (x, y) };
        // `getFencedPositionOfDrawable` measures from the drawable's (rounded) position.
        fence(&d, d.pos, x, y)
    }

    fn touching_point(&mut self, w: &World, t: TargetRef, x: f64, y: f64) -> bool {
        match self.draw_info(w.target(t)) {
            Some(d) => self.touching(&d, x, y),
            None => false,
        }
    }

    /// `RenderWebGL.isTouchingDrawables`.
    fn touching_targets(&mut self, w: &World, t: TargetRef, others: &[TargetRef]) -> bool {
        if !w.target(t).visible {
            return false;
        }
        let Some(me) = self.draw_info(w.target(t)) else { return false };
        self.skin(me.sprite, me.costume).note_drawn((100.0, 100.0), 1.0);
        let Some((l, r, b, top)) = self.scan_bounds(&me, !w.options.fencing) else { return false };
        let mut cands: Vec<(Draw, (i64, i64, i64, i64))> = Vec::new();
        for o in others {
            if *o == t || !w.target(*o).visible {
                continue;
            }
            let Some(d) = self.draw_info(w.target(*o)) else { continue };
            let ob = Self::snapped_aabb(&d);
            // Rectangle.intersects (inclusive), then Rectangle.intersect.
            if l <= ob.1 && ob.0 <= r && top >= ob.2 && ob.3 >= b {
                cands.push((d, (l.max(ob.0), r.min(ob.1), b.max(ob.2), top.min(ob.3))));
            }
        }
        if cands.is_empty() {
            return false;
        }
        // `_candidatesBounds`: union of the intersections.
        let ul = cands.iter().map(|c| c.1 .0).min().unwrap_or(l);
        let ur = cands.iter().map(|c| c.1 .1).max().unwrap_or(r);
        let ub = cands.iter().map(|c| c.1 .2).min().unwrap_or(b);
        let ut = cands.iter().map(|c| c.1 .3).max().unwrap_or(top);
        for x in ul..=ur {
            for y in ub..=ut {
                let (fx, fy) = (x as f64, y as f64);
                if self.touching(&me, fx, fy) {
                    for (d, _) in &cands {
                        if self.touching(d, fx, fy) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// `RenderWebGL.isTouchingColor` (CPU path).
    fn touching_color(&mut self, w: &World, t: TargetRef, color: [u8; 3], mask: Option<[u8; 3]>) -> bool {
        let Some(me) = self.draw_info(w.target(t)) else { return false };
        // `_touchingBounds` asks for the 100% texture first.
        self.skin(me.sprite, me.costume).note_drawn((100.0, 100.0), 1.0);
        let Some((l, r, b, top)) = self.scan_bounds(&me, !w.options.fencing) else { return false };
        // Everything else that is visible, front to back, then the pen layer and the stage.
        let mut cands: Vec<Draw> = Vec::new();
        for o in w.order.iter().skip(1).rev() {
            if *o == t || !w.target(*o).visible {
                continue;
            }
            if let Some(d) = self.draw_info(w.target(*o)) {
                cands.push(d);
            }
        }
        let stage = self.draw_info(w.target(w.order[0]));
        let matches = |a: [u8; 3], b: [u8; 3]| (a[0] & 0xF8) == (b[0] & 0xF8) && (a[1] & 0xF8) == (b[1] & 0xF8) && (a[2] & 0xF0) == (b[2] & 0xF0);
        for y in b..=top {
            for x in l..=r {
                let (fx, fy) = (x as f64, y as f64);
                let hit = match mask {
                    Some(m) => {
                        // `maskMatches`: 6 significant bits per channel.
                        let c = self.sample(&me, fx, fy, false);
                        let c = [c[0] as u8, c[1] as u8, c[2] as u8, c[3] as u8];
                        c[3] > 0 && (c[0] & 0xFC) == (m[0] & 0xFC) && (c[1] & 0xFC) == (m[1] & 0xFC) && (c[2] & 0xFC) == (m[2] & 0xFC)
                    }
                    None => self.touching(&me, fx, fy),
                };
                if !hit {
                    continue;
                }
                // `RenderWebGL.sampleColor4b`: blend front to back into a
                // `Uint8ClampedArray` (rounded at every step), white behind.
                let mut layers: Vec<[f64; 4]> = Vec::with_capacity(cands.len() + 2);
                for d in &cands {
                    layers.push(self.sample(d, fx, fy, true));
                }
                if self.pen_used {
                    // The pen drawable covers the stage; its silhouette is
                    // sampled at floor(u * (width - 1)) like any skin.
                    let (pw, ph) = self.pen_size();
                    let u = (fx + stage_width() / 2.0) / stage_width();
                    let v = (stage_height() / 2.0 - fy) / stage_height();
                    if (0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v) && pw > 0 && ph > 0 {
                        let px = ((u * (pw as f64 - 1.0)).floor() as u32).min(pw as u32 - 1);
                        let py = ((v * (ph as f64 - 1.0)).floor() as u32).min(ph as u32 - 1);
                        if let Some(p) = self.pen_pixel(px, py) {
                            layers.push([p[0] as f64, p[1] as f64, p[2] as f64, p[3] as f64]);
                        }
                    }
                }
                if let Some(st) = &stage {
                    layers.push(self.sample(st, fx, fy, true));
                }
                let mut dst = [0.0f64; 3];
                let mut blend = 1.0f64;
                for c in layers {
                    if blend == 0.0 {
                        break;
                    }
                    for i in 0..3 {
                        dst[i] = u8c(dst[i] + c[i] * blend);
                    }
                    blend *= 1.0 - c[3] / 255.0;
                }
                let got = [u8c(dst[0] + 255.0 * blend) as u8, u8c(dst[1] + 255.0 * blend) as u8, u8c(dst[2] + 255.0 * blend) as u8];
                if matches(got, color) {
                    return true;
                }
            }
        }
        false
    }

    /// Frontmost visible sprite under a point (`RenderWebGL.pick`).
    fn pick(&mut self, w: &World, x: f64, y: f64) -> Option<TargetRef> {
        for t in w.order.iter().skip(1).rev() {
            let tg = w.target(*t);
            if !tg.visible {
                continue;
            }
            // Fully ghosted sprites cannot be clicked.
            if tg.effects[6] >= 100.0 {
                continue;
            }
            if let Some(d) = self.draw_info(tg) {
                if self.touching(&d, x, y) {
                    return Some(*t);
                }
            }
        }
        None
    }

    fn pen_clear(&mut self) {
        match self.gpu.as_mut() {
            Some(gpu) => gpu.pen_clear(),
            None => self.pen.fill(tiny_skia::Color::TRANSPARENT),
        }
        self.pen_used = false;
    }

    /// `PenSkin.drawLine`.
    fn pen_line(&mut self, pen: &PenState, x0: f64, y0: f64, x1: f64, y1: f64) {
        let s = self.scale as f32;
        let offset = if pen.diameter == 1.0 || pen.diameter == 3.0 { 0.5 } else { 0.0 };
        let map = |x: f64, y: f64| (((x + offset + stage_width() / 2.0) as f32) * s, ((stage_height() / 2.0 - (y + offset)) as f32) * s);
        let (ax, ay) = map(x0, y0);
        let (bx, by) = map(x1, y1);
        if let Some(gpu) = self.gpu.as_mut() {
            // `PenSkin.drawLine` / `_drawLineOnBuffer`, in float64 until the
            // attribute buffer (Float32Array).
            let q = self.scale;
            let (x0, y0, x1, y1) = ((x0 + offset) * q, (y0 + offset) * q, (x1 + offset) * q, (y1 + offset) * q);
            let (dx, dy) = (x1 - x0, y1 - y0);
            let length = (dx * dx + dy * dy).sqrt();
            // color4f: rgb / 255 (pen.rgba holds them as f32) and 1 - transparency / 100.
            let a = 1.0 - pen.transparency / 100.0;
            let ch = |v: f32| (v as f64 * 255.0).round() / 255.0;
            let color = [(ch(pen.rgba[0]) * a) as f32, (ch(pen.rgba[1]) * a) as f32, (ch(pen.rgba[2]) * a) as f32, a as f32];
            gpu.pen_line(color, (pen.diameter * q) as f32, length as f32, [x0 as f32, -y0 as f32, dx as f32, -dy as f32]);
            self.pen_used = true;
            return;
        }
        let mut paint = tiny_skia::Paint::default();
        let c = pen.rgba;
        paint.set_color(tiny_skia::Color::from_rgba(c[0].clamp(0.0, 1.0), c[1].clamp(0.0, 1.0), c[2].clamp(0.0, 1.0), c[3].clamp(0.0, 1.0)).unwrap_or(tiny_skia::Color::BLACK));
        paint.anti_alias = true;
        let width = (pen.diameter as f32 * s).max(1.0);
        if (ax - bx).abs() < 1e-4 && (ay - by).abs() < 1e-4 {
            if let Some(path) = tiny_skia::PathBuilder::from_circle(ax, ay, width / 2.0) {
                self.pen.fill_path(&path, &paint, tiny_skia::FillRule::Winding, tiny_skia::Transform::identity(), None);
            }
        } else {
            let mut pb = tiny_skia::PathBuilder::new();
            pb.move_to(ax, ay);
            pb.line_to(bx, by);
            if let Some(path) = pb.finish() {
                let stroke = tiny_skia::Stroke { width, line_cap: tiny_skia::LineCap::Round, ..Default::default() };
                self.pen.stroke_path(&path, &paint, &stroke, tiny_skia::Transform::identity(), None);
            }
        }
        self.pen_used = true;
    }

    fn pen_stamp(&mut self, w: &World, t: TargetRef) {
        let Some(d) = self.draw_info(w.target(t)) else { return };
        if let Some(mut gpu) = self.gpu.take() {
            let s = self.scale;
            // `getFastBounds()`, clamped to the stage unless offscreen touching is on.
            let b = d.xf.aabb();
            let mut bounds = (b.left, b.right, b.bottom, b.top);
            if w.options.fencing {
                let (hw, hh) = (stage_width() / 2.0, stage_height() / 2.0);
                bounds = (bounds.0.max(-hw).min(hw), bounds.1.min(hw).max(-hw), bounds.2.max(-hh).min(hh), bounds.3.min(hh).max(-hh));
            }
            gpu.pen_stamp(self.skin(d.sprite, d.costume), &d, s, bounds);
            self.gpu = Some(gpu);
            self.pen_used = true;
            return;
        }
        let mut layer = Raster { w: self.width, h: self.height, data: self.pen.data().to_vec() };
        self.draw_into(&d, &mut layer);
        self.pen.data_mut().copy_from_slice(&layer.data);
        self.pen_used = true;
    }

    fn frame_drawn(&mut self, w: &World) {
        let s = self.scale;
        for (i, t) in w.order.iter().enumerate() {
            let tg = w.target(*t);
            if i > 0 && !tg.visible {
                continue;
            }
            let n = self.program.sprites[tg.sprite].costumes.len();
            if n == 0 {
                continue;
            }
            let pct = if tg.is_stage { (100.0, 100.0) } else { (tg.size, tg.size) };
            self.skin(tg.sprite, tg.costume.min(n - 1)).note_drawn(pct, s);
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
