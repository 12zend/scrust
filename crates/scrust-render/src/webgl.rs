//! WebGL2 compositor for wasm32: the Metal compositor (`gpu.rs`) for the
//! browser. Sprites (with their graphic effects), the pen layer and the text
//! overlays are drawn on the GPU, and the pen layer lives in a GPU texture
//! that pen lines and stamps are rendered into.
//!
//! The module has no WebGL bindings of its own: drawing is recorded as a
//! stream of `f32`s (`OP_*` below) that the page's glue
//! (`crates/scrust-render/web/gl.js`) replays with WebGL2 when the frame is
//! presented, so a frame costs a handful of calls into JavaScript however
//! many sprites, stamps and pen lines it has. Skin rasters are uploaded once
//! and cached by (skin uid, size), as in `gpu.rs`.
//!
//! Collision queries keep sampling the CPU silhouettes; `touching color`
//! reads the pen layer back when it changed (`pen_pixels`).
//!
//! The geometry is scratch-render's, in its own GL conventions: the stage is
//! drawn into the canvas's drawing buffer (bottom up) and pen lines and
//! stamps into a framebuffer stored top down, so none of the y flips
//! `gpu.rs` needs to emulate ANGLE on Metal apply here. Drawing the stage
//! into the canvas rather than a texture matters: GL implementations orient
//! the two differently, which decides the pixels a quad edge that runs
//! through pixel centres covers. `gl_init` returns 0 when the page has no
//! WebGL2 (or provides no glue), and the renderer then draws on the CPU.

use crate::skin::{Raster, Skin};
use crate::{Draw, stage_height, stage_width};
use std::collections::HashMap;

#[link(wasm_import_module = "scrust_gl")]
extern "C" {
    /// Create the context, a `width` x `height` canvas and the pen layer; 1 when WebGL2 works.
    fn gl_init(width: u32, height: u32) -> u32;
    /// Resize the canvas and the pen layer (which is cleared); 1 on success.
    fn gl_resize(width: u32, height: u32) -> u32;
    /// Create or replace texture `id` with `w` x `h` premultiplied RGBA pixels.
    fn gl_texture(id: u32, w: u32, h: u32, ptr: *const u8);
    fn gl_delete_texture(id: u32);
    /// Replay `len` floats of commands.
    fn gl_run(ptr: *const f32, len: u32);
    /// Read the canvas (0, bottom up) or the pen layer (1, top down) into `ptr`.
    fn gl_read(which: u32, ptr: *mut u8, len: u32);
}

// Commands. A quad is: texture id, nearest (0/1), viewport (x, y, w, h),
// uniforms (size, fxa, fxb: 12 floats), six vertices (clip xy, texcoord: 24).
/// Clear the pen layer.
const OP_PEN_CLEAR: f32 = 1.0;
/// Pen lines: count, then 12 floats per line (`LineInstance`).
const OP_PEN_LINES: f32 = 2.0;
/// Stamp a quad into the pen layer.
const OP_PEN_STAMP: f32 = 3.0;
/// Start the frame: background r, g, b (0..1).
const OP_BEGIN: f32 = 4.0;
/// Draw a quad on the stage.
const OP_SPRITE: f32 = 5.0;
/// Draw texture id over the NDC rectangle (left, y at v = 0, right, y at v = 1).
const OP_RECT: f32 = 6.0;
/// The pen layer over the whole stage.
const OP_PEN_RECT: f32 = 7.0;
/// End of a presented frame (the stage is drawn straight into the canvas).
const OP_PRESENT: f32 = 8.0;

struct Quad {
    tex: u32,
    nearest: bool,
    viewport: [f32; 4],
    u: [f32; 12],
    verts: [[f32; 4]; 6],
}

impl Quad {
    fn encode(&self, out: &mut Vec<f32>) {
        out.push(self.tex as f32);
        out.push(self.nearest as u8 as f32);
        out.extend_from_slice(&self.viewport);
        out.extend_from_slice(&self.u);
        for v in &self.verts {
            out.extend_from_slice(v);
        }
    }
}

pub(crate) struct Gpu {
    width: usize,
    height: usize,
    next_tex: u32,
    /// Skin rasters by (skin uid, width, height): texture id and the frame they were last used in.
    skins: HashMap<(u64, usize, usize), (u32, u64)>,
    /// Overlay textures by slot: id, width, height.
    overlays: HashMap<u64, (u32, usize, usize)>,
    frame_no: u64,
    /// Commands of the frame being composed.
    frame: Vec<f32>,
    /// Pen operations not yet run.
    pen: Vec<f32>,
    /// Index of the line count of the last `OP_PEN_LINES` when it is the last pen command.
    lines_at: Option<usize>,
    /// CPU copy of the pen layer (premultiplied), valid until the next pen operation.
    pen_copy: Option<Raster>,
}

/// scratch-render's `penStamp` geometry (see `gpu.rs`): the viewport is the
/// drawable's fast bounds in device pixels, snapped outwards, and the
/// projection is `ortho` over them with top and bottom swapped (the pen
/// framebuffer is top down).
fn tw_stamp_vertices(d: &Draw, scale: f64, bounds: (f64, f64, f64, f64)) -> Option<([[f32; 4]; 6], [f32; 4])> {
    let (l, r, b, t) = bounds;
    let (l, r, b, t) = ((l * scale).floor(), (r * scale).ceil(), (b * scale).floor(), (t * scale).ceil());
    if r - l <= 0.0 || t - b <= 0.0 {
        return None;
    }
    let vp = [(stage_width() / 2.0 * scale + l) as f32, (stage_height() / 2.0 * scale - t) as f32, (r - l) as f32, (t - b) as f32];
    let (ol, or, ob, ot) = (l / scale, r / scale, t / scale, b / scale);
    let p0 = (2.0 / (or - ol)) as f32;
    let p5 = (2.0 / (ot - ob)) as f32;
    let p12 = ((or + ol) / (ol - or)) as f32;
    let p13 = ((ot + ob) / (ob - ot)) as f32;
    Some((tw_clip(&d.xf, p0, p5, p12, p13), vp))
}

/// Clip positions of scratch-render's quad, `P * M * vec4(a_position, 0, 1)`
/// in float32 (see `gpu.rs`, here without the Metal flip).
fn tw_clip(xf: &crate::Xf, p0: f32, p5: f32, p12: f32, p13: f32) -> [[f32; 4]; 6] {
    let (m0, m1, m4, m5, m12, m13) = (xf.m00 as f32, xf.m10 as f32, xf.m01 as f32, xf.m11 as f32, xf.tx as f32, xf.ty as f32);
    let (pm0, pm1, pm4, pm5) = (p0 * m0, p5 * m1, p0 * m4, p5 * m5);
    let (pm12, pm13) = (p0 * m12 + p12, p5 * m13 + p13);
    const QUAD: [[f32; 4]; 6] = [[-0.5, -0.5, 1.0, 0.0], [0.5, -0.5, 0.0, 0.0], [-0.5, 0.5, 1.0, 1.0], [-0.5, 0.5, 1.0, 1.0], [0.5, -0.5, 0.0, 0.0], [0.5, 0.5, 0.0, 1.0]];
    QUAD.map(|[ax, ay, u, v]| [pm0 * ax + pm4 * ay + pm12, pm1 * ax + pm5 * ay + pm13, u, v])
}

fn upload(id: u32, r: &Raster) {
    if r.w == 0 || r.h == 0 || r.data.len() < r.w * r.h * 4 {
        return;
    }
    // SAFETY: plain FFI call; the page copies `w * h * 4` bytes from `ptr` at once.
    unsafe { gl_texture(id, r.w as u32, r.h as u32, r.data.as_ptr()) };
}

fn run(cmds: &[f32]) {
    if !cmds.is_empty() {
        // SAFETY: plain FFI call; the page reads `len` floats from `ptr` during the call.
        unsafe { gl_run(cmds.as_ptr(), cmds.len() as u32) };
    }
}

impl Gpu {
    /// A compositor for a `width` x `height` (device pixels) output, or None
    /// when the page has no WebGL2.
    pub fn new(width: usize, height: usize) -> Option<Gpu> {
        let (w, h) = (width.max(1), height.max(1));
        // SAFETY: plain FFI call into the page's glue code.
        if unsafe { gl_init(w as u32, h as u32) } == 0 {
            return None;
        }
        Some(Gpu {
            width: w,
            height: h,
            next_tex: 1,
            skins: HashMap::new(),
            overlays: HashMap::new(),
            frame_no: 0,
            frame: Vec::new(),
            pen: vec![OP_PEN_CLEAR],
            lines_at: None,
            pen_copy: Some(Raster::new(w, h)),
        })
    }

    /// Change the output size (the pen layer is cleared).
    pub fn resize(&mut self, width: usize, height: usize) -> bool {
        let (w, h) = (width.max(1), height.max(1));
        if (w, h) == (self.width, self.height) {
            return true;
        }
        // SAFETY: plain FFI call into the page's glue code.
        if unsafe { gl_resize(w as u32, h as u32) } == 0 {
            return false;
        }
        (self.width, self.height) = (w, h);
        self.pen_clear();
        true
    }

    fn new_texture(&mut self) -> u32 {
        let id = self.next_tex;
        self.next_tex += 1;
        id
    }

    fn skin_texture(&mut self, skin: &mut Skin, pixels_per_unit: f64) -> Option<u32> {
        let frame = self.frame_no;
        let uid = skin.uid;
        let raster = skin.raster_for(pixels_per_unit);
        if raster.w == 0 || raster.h == 0 {
            return None;
        }
        let key = (uid, raster.w, raster.h);
        if let Some((id, used)) = self.skins.get_mut(&key) {
            *used = frame;
            return Some(*id);
        }
        let id = self.next_tex;
        self.next_tex += 1;
        upload(id, raster);
        self.skins.insert(key, (id, frame));
        Some(id)
    }

    fn quad(&mut self, skin: &mut Skin, d: &Draw, scale: f64) -> Option<Quad> {
        if d.degenerate() {
            return None;
        }
        let unit = (d.xf.m00.hypot(d.xf.m10) / d.size.0.max(1e-9)).max(d.xf.m01.hypot(d.xf.m11) / d.size.1.max(1e-9));
        let tex = self.skin_texture(skin, unit * scale)?;
        let fx = d.fx;
        let u = [
            d.size.0 as f32,
            d.size.1 as f32,
            fx.distorts() as u8 as f32,
            fx.recolors() as u8 as f32,
            fx.color as f32,
            fx.fisheye as f32,
            fx.whirl as f32,
            fx.pixelate as f32,
            fx.mosaic as f32,
            fx.brightness as f32,
            fx.ghost as f32,
            0.0,
        ];
        let (hw, hh) = (stage_width() / 2.0, stage_height() / 2.0);
        let verts = tw_clip(&d.xf, (2.0 / (2.0 * hw)) as f32, (2.0 / (2.0 * hh)) as f32, 0.0, 0.0);
        Some(Quad { tex, nearest: d.draw_nearest(scale), viewport: [0.0, 0.0, self.width as f32, self.height as f32], u, verts })
    }

    // ------------------------------------------------------------- frame

    /// Upload a skin's texture ahead of its first use.
    pub fn preload(&mut self, skin: &mut Skin) {
        let _ = self.skin_texture(skin, 1.0);
    }

    /// Start a frame over an opaque background colour.
    pub fn begin(&mut self, background: [u8; 3]) {
        self.frame_no += 1;
        self.frame.clear();
        self.frame.extend_from_slice(&[OP_BEGIN, background[0] as f32 / 255.0, background[1] as f32 / 255.0, background[2] as f32 / 255.0]);
    }

    pub fn sprite(&mut self, skin: &mut Skin, d: &Draw, scale: f64) {
        if let Some(q) = self.quad(skin, d, scale) {
            self.frame.push(OP_SPRITE);
            q.encode(&mut self.frame);
        }
    }

    /// Composite the pen layer at this point of the frame.
    pub fn pen_layer(&mut self) {
        self.frame.push(OP_PEN_RECT);
    }

    /// A screen-aligned raster at device pixel `(x, y)` (from the top left);
    /// `slot` identifies the overlay so its texture can be reused.
    pub fn overlay(&mut self, slot: u64, r: &Raster, x: i64, y: i64) {
        if r.w == 0 || r.h == 0 {
            return;
        }
        // `gl_texture` replaces the texture's contents and size.
        let id = match self.overlays.get(&slot) {
            Some(&(id, ..)) => id,
            None => self.new_texture(),
        };
        upload(id, r);
        self.overlays.insert(slot, (id, r.w, r.h));
        let (w, h) = (self.width as f32, self.height as f32);
        let ndc_x = |px: f32| px / w * 2.0 - 1.0;
        // The target is bottom up: device row 0 (the top) is NDC y = 1.
        let ndc_y = |py: f32| 1.0 - py / h * 2.0;
        self.frame.extend_from_slice(&[OP_RECT, id as f32, ndc_x(x as f32), ndc_y(y as f32), ndc_x((x + r.w as i64) as f32), ndc_y((y + r.h as i64) as f32)]);
    }

    /// Run the pending pen operations and the frame, then forget skin
    /// textures that have not been drawn for a while.
    fn flush(&mut self, present: bool) {
        self.flush_pen();
        if present {
            self.frame.push(OP_PRESENT);
        }
        run(&self.frame);
        self.frame.clear();
        let frame = self.frame_no;
        self.skins.retain(|_, (id, used)| {
            let keep = frame - *used < 300;
            if !keep {
                // SAFETY: plain FFI call into the page's glue code.
                unsafe { gl_delete_texture(*id) };
            }
            keep
        });
    }

    fn flush_pen(&mut self) {
        run(&self.pen);
        self.pen.clear();
        self.lines_at = None;
    }

    /// Render the frame and read it back as opaque RGBA into `out`
    /// (`width * height * 4` bytes).
    pub fn finish(&mut self, out: &mut [u8]) {
        self.flush(false);
        let n = (self.width * self.height * 4).min(out.len());
        // SAFETY: plain FFI call; the page writes at most `n` bytes at `out`.
        unsafe { gl_read(0, out.as_mut_ptr(), n as u32) };
        // WebGL reads the bottom-up target bottom row first.
        let row = self.width * 4;
        let rows = (n / row).min(self.height);
        for y in 0..rows / 2 {
            let (top, bottom) = out.split_at_mut((rows - 1 - y) * row);
            top[y * row..(y + 1) * row].swap_with_slice(&mut bottom[..row]);
        }
    }

    /// Render the frame into the page's canvas.
    pub fn present(&mut self) {
        self.flush(true);
    }

    /// Frames are always presented in the page's own canvas.
    ///
    /// # Safety
    /// Nothing is dereferenced.
    pub unsafe fn attach_view(&mut self, _ns_view: *mut std::ffi::c_void, _width: usize, _height: usize) -> bool {
        false
    }

    pub fn set_drawable_size(&mut self, _width: usize, _height: usize) {}

    // ------------------------------------------------------------- pen

    pub fn pen_clear(&mut self) {
        self.pen.clear();
        self.pen.push(OP_PEN_CLEAR);
        self.lines_at = None;
        self.pen_copy = Some(Raster::new(self.width, self.height));
    }

    /// A line with `PenSkin._drawLineOnBuffer`'s attributes (see `gpu.rs`).
    pub fn pen_line(&mut self, color: [f32; 4], thickness: f32, length: f32, pts: [f32; 4]) {
        let at = match self.lines_at {
            Some(at) => at,
            None => {
                self.pen.extend_from_slice(&[OP_PEN_LINES, 0.0]);
                let at = self.pen.len() - 1;
                self.lines_at = Some(at);
                at
            }
        };
        self.pen[at] += 1.0;
        self.pen.extend_from_slice(&color);
        self.pen.extend_from_slice(&[thickness, length, self.width as f32, self.height as f32]);
        self.pen.extend_from_slice(&pts);
        // Counts stay exact in f32 below 2^24 lines per batch.
        if self.pen[at] >= 16_000_000.0 {
            self.lines_at = None;
        }
        self.pen_copy = None;
    }

    pub fn pen_stamp(&mut self, skin: &mut Skin, d: &Draw, scale: f64, bounds: (f64, f64, f64, f64)) {
        if let Some(mut q) = self.quad(skin, d, scale) {
            let Some((verts, viewport)) = tw_stamp_vertices(d, scale, bounds) else { return };
            (q.verts, q.viewport) = (verts, viewport);
            self.pen.push(OP_PEN_STAMP);
            q.encode(&mut self.pen);
            self.lines_at = None;
            self.pen_copy = None;
        }
    }

    /// The pen layer as premultiplied RGBA (for `touching color`); reads it
    /// back from the GPU when it changed.
    pub fn pen_pixels(&mut self) -> &Raster {
        if self.pen_copy.is_none() {
            self.flush_pen();
            let mut r = Raster::new(self.width, self.height);
            // SAFETY: plain FFI call; the page writes at most `len` bytes at `ptr`.
            unsafe { gl_read(1, r.data.as_mut_ptr(), r.data.len() as u32) };
            self.pen_copy = Some(r);
        }
        self.pen_copy.as_ref().expect("pen copy")
    }
}
