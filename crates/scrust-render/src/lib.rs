//! CPU compositor for scrust.
//!
//! One sampler is shared by drawing and by collision detection, so
//! `touching ...` blocks see exactly what is drawn (scratch-render likewise
//! answers them on the CPU from its silhouettes).

#[cfg(target_os = "macos")]
mod gpu;
pub mod scene;
mod skin;
mod text;

/// Placeholder where there is no GPU backend: never constructed.
#[cfg(not(target_os = "macos"))]
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
        pub fn pen_line(&mut self, _a: (f32, f32), _b: (f32, f32), _width: f32, _rgba: [f32; 4]) {
            match *self {}
        }
        pub fn pen_stamp(&mut self, _skin: &mut Skin, _d: &Draw, _scale: f64) {
            match *self {}
        }
        pub fn pen_pixels(&mut self) -> &Raster {
            match *self {}
        }
    }
}

use gpu::Gpu;

pub use scene::Scene;

use resvg::tiny_skia;
use resvg::usvg;
use scrust_runtime::{Bounds, PenState, ProgramDef, Renderer, RotationStyle, Target, TargetRef, World, STAGE_HEIGHT, STAGE_WIDTH};
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
        let (c, s) = (f32r(libm::cos(rot)), f32r(libm::sin(rot)));
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
}

impl Draw {
    /// Transform and sampling mode for a skin drawn at `pos` with a Scratch
    /// direction and percentage scale (`RenderedTarget` -> `Drawable`).
    fn new(skin: &Skin, pos: (f64, f64), direction: f64, scale: (f64, f64), fx: Effects, sprite: usize, costume: usize) -> Draw {
        let is_svg = matches!(skin.kind, skin::SkinKind::Svg(_));
        let near100 = |v: f64| v.abs() > 99.0 && v.abs() < 101.0;
        let nearest = !is_svg || (!fx.distorts() && direction % 90.0 == 0.0 && near100(scale.0) && near100(scale.1));
        Draw { xf: Xf::new(pos, direction, scale, skin.size, skin.center), fx, sprite, costume, size: skin.size, nearest }
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
fn touching_skin(skin: &Skin, d: &Draw, x: f64, y: f64) -> bool {
    let (mut u, mut v) = d.xf.to_uv(x, y);
    if d.fx.distorts() && (0.0..1.0).contains(&u) && (0.0..1.0).contains(&v) {
        let p = d.fx.point(u, v, d.size);
        u = p.0;
        v = p.1;
    }
    silhouette_touching(&skin.base, u, v, d.nearest)
}

/// Tight bounds (`Drawable.getBounds`): the convex hull scratch-render
/// builds by scanning each row of the skin with the "linear" silhouette
/// test, transformed through pixel centres.
fn tight_bounds(skin: &mut Skin, d: &Draw) -> Bounds {
    let xf = d.xf;
    let (w, h) = d.size;
    let fx = d.fx;
    let distort = fx.distorts();
    if !distort && skin.hull.is_none() {
        skin.hull = Some(hull_rows(&skin.base, (w, h), None));
    }
    let computed;
    let rows: &Vec<(f64, f64, f64)> = if distort {
        computed = hull_rows(&skin.base, (w, h), Some(fx));
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
    let sx = STAGE_WIDTH / 2.0 - FENCE_WIDTH.min(inset);
    if aabb.right + dx < -sx {
        x = (cur.0 - (sx + aabb.right)).ceil();
    } else if aabb.left + dx > sx {
        x = (cur.0 + (sx - aabb.left)).floor();
    }
    let sy = STAGE_HEIGHT / 2.0 - FENCE_WIDTH.min(inset);
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
    let b = d.xf.aabb();
    let x0 = (((b.left + STAGE_WIDTH / 2.0) * s).floor().max(0.0)) as usize;
    let x1 = (((b.right + STAGE_WIDTH / 2.0) * s).ceil().min(target.w as f64)).max(0.0) as usize;
    let y0 = (((STAGE_HEIGHT / 2.0 - b.top) * s).floor().max(0.0)) as usize;
    let y1 = (((STAGE_HEIGHT / 2.0 - b.bottom) * s).ceil().min(target.h as f64)).max(0.0) as usize;
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
    let exact = raster.w as f64 == (b.right - b.left) * s && raster.h as f64 == (b.top - b.bottom) * s;
    for py in y0..y1 {
        let wy = STAGE_HEIGHT / 2.0 - (py as f64 + 0.5) / s;
        for px in x0..x1 {
            let wx = (px as f64 + 0.5) / s - STAGE_WIDTH / 2.0;
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
            let mut c = if exact || distort { raster.nearest(u, v) } else { raster.linear(u, v) };
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
    bubbles: HashMap<u32, (u64, Raster)>,
    monitors: HashMap<usize, (String, Raster)>,
    ask: Option<(String, Raster)>,
    /// Text typed so far into the ask prompt (owned by the player).
    pub ask_input: String,
    /// Metal compositor for drawing and the pen layer; None draws on the CPU.
    gpu: Option<Gpu>,
}

impl SoftRenderer {
    /// A renderer that draws on the GPU when one is available (Metal on
    /// macOS, unless `SCRUST_RENDERER=cpu`), else on the CPU.
    pub fn new(program: &'static ProgramDef, scale: f64) -> SoftRenderer {
        let mut r = Self::new_cpu(program, scale);
        r.gpu = Gpu::new(r.width, r.height);
        r
    }

    /// A renderer that draws everything on the CPU (deterministic output).
    pub fn new_cpu(program: &'static ProgramDef, scale: f64) -> SoftRenderer {
        let width = (STAGE_WIDTH * scale).round() as usize;
        let height = (STAGE_HEIGHT * scale).round() as usize;
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
            monitors: HashMap::new(),
            ask: None,
            ask_input: String::new(),
            gpu: None,
        }
    }

    /// Whether frames are drawn on the GPU.
    pub fn uses_gpu(&self) -> bool {
        self.gpu.is_some()
    }

    /// Premultiplied pen-layer pixel at device coordinates.
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
        let pos = if t.is_stage { (0.0, 0.0) } else { (t.x, t.y) };
        let fx = Effects::of(t);
        let skin = self.skin(t.sprite, costume);
        Some(Draw::new(skin, pos, direction, scale, fx, t.sprite, costume))
    }

    /// Is the drawable opaque at world point `(x, y)`? (`Drawable.isTouching`)
    fn touching(&mut self, d: &Draw, x: f64, y: f64) -> bool {
        touching_skin(self.skin(d.sprite, d.costume), d, x, y)
    }

    /// Colour of the drawable at a world point (`Drawable.sampleColor4b`).
    fn sample(&mut self, d: &Draw, x: f64, y: f64, with_ghost: bool) -> [u8; 4] {
        let (mut u, mut v) = d.xf.to_uv(x, y);
        if u < 0.0 || v < 0.0 || u > 1.0 || v > 1.0 {
            return [0; 4];
        }
        if d.fx.distorts() && u < 1.0 && v < 1.0 {
            let p = d.fx.point(u, v, d.size);
            u = p.0;
            v = p.1;
        }
        // `Silhouette.colorAtNearest`
        let base = &self.skin(d.sprite, d.costume).base;
        let sx = (u * (base.w as f64 - 1.0)).floor();
        let sy = (v * (base.h as f64 - 1.0)).floor();
        if sx < 0.0 || sy < 0.0 || sx >= base.w as f64 || sy >= base.h as f64 {
            return [0; 4];
        }
        let px = base.px(sx as usize, sy as usize);
        if d.fx.recolors() {
            d.fx.color(px, with_ghost)
        } else {
            px
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
    fn scan_bounds(&mut self, d: &Draw) -> Option<(i64, i64, i64, i64)> {
        let b = d.xf.aabb();
        let (xl, xr, yb, yt) = (-STAGE_WIDTH / 2.0, STAGE_WIDTH / 2.0, -STAGE_HEIGHT / 2.0, STAGE_HEIGHT / 2.0);
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

    /// The same frame composited by the GPU.
    fn draw_gpu(&mut self, w: &World, gpu: &mut Gpu) {
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
        let mut canvas = std::mem::replace(&mut self.canvas, Raster::new(0, 0));
        gpu.finish(&mut canvas.data);
        self.present(&canvas);
        self.canvas = canvas;
    }

    fn draw_overlays(&mut self, w: &World, out: &mut Overlays) {
        self.draw_bubbles(w, out);
        self.draw_monitors(w, out);
        self.draw_ask(w, out);
    }

    /// Copy a composed RGBA canvas into `self.frame`.
    fn present(&mut self, canvas: &Raster) {
        for (o, p) in self.frame.iter_mut().zip(canvas.data.chunks_exact(4)) {
            *o = ((p[0] as u32) << 16) | ((p[1] as u32) << 8) | p[2] as u32;
        }
    }

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
            let fresh = !matches!(self.bubbles.get(&t.idx), Some((u, _)) if *u == b.usage);
            if fresh {
                let r = text::bubble(&b.text.to_rust_string(), b.think, s, &self.opt);
                self.bubbles.insert(t.idx, (b.usage, r));
            }
            let Some(d) = self.draw_info(tg) else { continue };
            let bounds = self.tight_bounds(&d);
            let (bw, bh) = {
                let r = &self.bubbles[&t.idx].1;
                (r.w as f64 / s, r.h as f64 / s)
            };
            // Up and to the right of the sprite, kept on stage.
            let mut left = bounds.right - 10.0;
            if left + bw > STAGE_WIDTH / 2.0 {
                left = (bounds.left - bw + 10.0).max(-STAGE_WIDTH / 2.0);
            }
            let top = (bounds.top + bh - 8.0).min(STAGE_HEIGHT / 2.0).max(-STAGE_HEIGHT / 2.0 + bh);
            let x = ((left + STAGE_WIDTH / 2.0) * s).round() as i64;
            let y = ((STAGE_HEIGHT / 2.0 - top) * s).round() as i64;
            let r = &self.bubbles[&t.idx].1;
            out.put(SLOT_BUBBLE | t.idx as u64, r, x, y);
        }
        self.bubbles.retain(|k, _| live.contains(k));
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
        let d = self.draw_info(w.target(t))?;
        Some(self.tight_bounds(&d))
    }

    /// `RenderWebGL.getFencedPositionOfDrawable`.
    fn fenced_position(&mut self, w: &World, t: TargetRef, x: f64, y: f64) -> (f64, f64) {
        let tg = w.target(t);
        let Some(d) = self.draw_info(tg) else { return (x, y) };
        fence(&d, (tg.x, tg.y), x, y)
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
        let Some((l, r, b, top)) = self.scan_bounds(&me) else { return false };
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
        let Some((l, r, b, top)) = self.scan_bounds(&me) else { return false };
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
        let s = self.scale;
        for y in b..=top {
            for x in l..=r {
                let (fx, fy) = (x as f64, y as f64);
                let hit = match mask {
                    Some(m) => {
                        // `maskMatches`: 6 significant bits per channel.
                        let c = self.sample(&me, fx, fy, false);
                        c[3] > 0 && (c[0] & 0xFC) == (m[0] & 0xFC) && (c[1] & 0xFC) == (m[1] & 0xFC) && (c[2] & 0xFC) == (m[2] & 0xFC)
                    }
                    None => self.touching(&me, fx, fy),
                };
                if !hit {
                    continue;
                }
                // `sampleColor3b`: composite the candidates over white.
                let mut acc = [0.0f64; 3];
                let mut blend = 1.0f64;
                let mut layers: Vec<[u8; 4]> = Vec::with_capacity(cands.len() + 2);
                for d in &cands {
                    layers.push(self.sample(d, fx, fy, true));
                }
                if self.pen_used {
                    let px = (((fx + STAGE_WIDTH / 2.0) * s) as i64).clamp(0, self.width as i64 - 1) as u32;
                    let py = (((STAGE_HEIGHT / 2.0 - fy) * s) as i64).clamp(0, self.height as i64 - 1) as u32;
                    if let Some(p) = self.pen_pixel(px, py) {
                        layers.push(p);
                    }
                }
                if let Some(st) = &stage {
                    layers.push(self.sample(st, fx, fy, true));
                }
                for c in layers {
                    if blend <= 0.0 {
                        break;
                    }
                    for i in 0..3 {
                        acc[i] += c[i] as f64 * blend;
                    }
                    blend *= 1.0 - c[3] as f64 / 255.0;
                }
                let got = [
                    (acc[0] + blend * 255.0).round().clamp(0.0, 255.0) as u8,
                    (acc[1] + blend * 255.0).round().clamp(0.0, 255.0) as u8,
                    (acc[2] + blend * 255.0).round().clamp(0.0, 255.0) as u8,
                ];
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
        let map = |x: f64, y: f64| (((x + offset + STAGE_WIDTH / 2.0) as f32) * s, ((STAGE_HEIGHT / 2.0 - (y + offset)) as f32) * s);
        let (ax, ay) = map(x0, y0);
        let (bx, by) = map(x1, y1);
        if let Some(gpu) = self.gpu.as_mut() {
            let c = pen.rgba.map(|v| v.clamp(0.0, 1.0));
            gpu.pen_line((ax, ay), (bx, by), (pen.diameter as f32 * s).max(1.0), c);
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
            gpu.pen_stamp(self.skin(d.sprite, d.costume), &d, s);
            self.gpu = Some(gpu);
            self.pen_used = true;
            return;
        }
        let mut layer = Raster { w: self.width, h: self.height, data: self.pen.data().to_vec() };
        self.draw_into(&d, &mut layer);
        self.pen.data_mut().copy_from_slice(&layer.data);
        self.pen_used = true;
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
