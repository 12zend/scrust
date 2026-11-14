//! Retained scene of skins and drawables, the object model of scratch-render's
//! `RenderWebGL`. The GUI's editor uses it (through `scrust-render-web`)
//! while no compiled program is running; it shares the transform, sampler and
//! compositor with `SoftRenderer`.

use crate::skin::{Raster, Skin};
use crate::{composite, fence, text, tight_bounds, touching_raster, Draw, Effects, Gpu};
use resvg::usvg;
use scrust_runtime::{Bounds, STAGE_HEIGHT, STAGE_WIDTH};
use std::collections::HashMap;
use std::sync::Arc;

pub struct Drawable {
    pub skin: Option<u32>,
    pub position: (f64, f64),
    pub direction: f64,
    pub scale: (f64, f64),
    pub visible: bool,
    effects: Effects,
}

impl Default for Drawable {
    fn default() -> Self {
        Drawable { skin: None, position: (0.0, 0.0), direction: 90.0, scale: (100.0, 100.0), visible: true, effects: Effects::default() }
    }
}

/// Device-pixel rectangle `(x, y, width, height)` and straight RGBA pixels.
pub type Extracted = ((i64, i64, usize, usize), Vec<u8>);

pub struct Scene {
    opt: usvg::Options<'static>,
    skins: HashMap<u32, Skin>,
    drawables: HashMap<u32, Drawable>,
    next_id: u32,
    /// Device pixels per stage unit.
    pub scale: f64,
    pub width: usize,
    pub height: usize,
    pub background: [u8; 3],
    canvas: Raster,
    /// Straight RGBA of the last `draw`, ready for `ImageData`.
    pub rgba: Vec<u8>,
    /// Metal compositor; None draws on the CPU.
    gpu: Option<Gpu>,
}

impl Default for Scene {
    fn default() -> Self {
        Self::new()
    }
}

impl Scene {
    pub fn new() -> Scene {
        let mut opt = usvg::Options::default();
        text::load_fonts(Arc::make_mut(&mut opt.fontdb));
        text::configure(&mut opt);
        let mut s = Scene {
            opt,
            skins: HashMap::new(),
            drawables: HashMap::new(),
            next_id: 1,
            scale: 1.0,
            width: 0,
            height: 0,
            background: [255, 255, 255],
            canvas: Raster::new(0, 0),
            rgba: Vec::new(),
            gpu: None,
        };
        s.resize(STAGE_WIDTH as usize, STAGE_HEIGHT as usize);
        s.gpu = Gpu::new(s.width, s.height);
        s
    }

    fn id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Set the size of the output in device pixels; the stage keeps its 4:3
    /// coordinate system and is scaled by the width.
    pub fn resize(&mut self, width: usize, height: usize) {
        let width = width.clamp(1, 4096);
        let height = height.clamp(1, 4096);
        if width == self.width && height == self.height {
            return;
        }
        self.width = width;
        self.height = height;
        self.scale = width as f64 / STAGE_WIDTH;
        self.canvas = Raster::new(width, height);
        self.rgba = vec![255; width * height * 4];
        if self.gpu.as_mut().is_some_and(|g| !g.resize(width, height)) {
            self.gpu = None;
        }
    }

    /// Whether `draw` composites on the GPU.
    pub fn uses_gpu(&self) -> bool {
        self.gpu.is_some()
    }

    // ------------------------------------------------------------- skins

    pub fn insert_skin(&mut self, id: Option<u32>, skin: Skin) -> u32 {
        let id = id.unwrap_or_else(|| self.id());
        self.skins.insert(id, skin);
        id
    }

    /// SVG skin; `center` (stage units from the top-left) defaults to the middle.
    pub fn svg_skin(&mut self, id: Option<u32>, svg: &[u8], center: Option<(f64, f64)>) -> u32 {
        let mut skin = Skin::load(svg, "svg", 1.0, (0.0, 0.0), &self.opt);
        skin.center = center.unwrap_or((skin.size.0 / 2.0, skin.size.1 / 2.0));
        self.insert_skin(id, skin)
    }

    pub fn bitmap_skin(&mut self, id: Option<u32>, w: usize, h: usize, rgba: &[u8], resolution: f64, center: Option<(f64, f64)>) -> u32 {
        self.insert_skin(id, Skin::from_rgba(w, h, rgba, resolution, center))
    }

    /// A skin that draws nothing (text bubbles and the pen layer of an editor
    /// that is not running).
    pub fn empty_skin(&mut self) -> u32 {
        self.insert_skin(None, Skin::empty())
    }

    pub fn destroy_skin(&mut self, id: u32) {
        self.skins.remove(&id);
        for d in self.drawables.values_mut() {
            if d.skin == Some(id) {
                d.skin = None;
            }
        }
    }

    pub fn skin(&self, id: u32) -> Option<&Skin> {
        self.skins.get(&id)
    }

    // ------------------------------------------------------------- drawables

    pub fn create_drawable(&mut self) -> u32 {
        let id = self.id();
        self.drawables.insert(id, Drawable::default());
        id
    }

    pub fn destroy_drawable(&mut self, id: u32) {
        self.drawables.remove(&id);
    }

    pub fn drawable_mut(&mut self, id: u32) -> Option<&mut Drawable> {
        self.drawables.get_mut(&id)
    }

    /// Set a graphic effect by scratch-render's name.
    pub fn set_effect(&mut self, id: u32, name: &str, value: f64) {
        let Some(d) = self.drawables.get_mut(&id) else { return };
        let e = &mut d.effects;
        match name {
            "color" => e.color = value,
            "fisheye" => e.fisheye = value,
            "whirl" => e.whirl = value,
            "pixelate" => e.pixelate = value,
            "mosaic" => e.mosaic = value,
            "brightness" => e.brightness = value,
            "ghost" => e.ghost = value,
            _ => {}
        }
    }

    fn draw_of(&self, id: u32) -> Option<(Draw, u32)> {
        let d = self.drawables.get(&id)?;
        let skin_id = d.skin?;
        let skin = self.skins.get(&skin_id)?;
        Some((Draw::new(skin, d.position, d.direction, d.scale, d.effects, 0, 0), skin_id))
    }

    /// Tight bounds of a drawable in stage coordinates.
    pub fn bounds(&mut self, id: u32) -> Option<Bounds> {
        let (d, skin) = self.draw_of(id)?;
        Some(tight_bounds(self.skins.get_mut(&skin)?, &d))
    }

    /// Axis-aligned bounds of the drawable's quad.
    pub fn aabb(&self, id: u32) -> Option<Bounds> {
        Some(self.draw_of(id)?.0.xf.aabb())
    }

    pub fn fenced_position(&self, id: u32, x: f64, y: f64) -> (f64, f64) {
        match (self.draw_of(id), self.drawables.get(&id)) {
            (Some((d, _)), Some(dr)) => fence(&d, dr.position, x, y),
            _ => (x, y),
        }
    }

    /// Frontmost drawable of `candidates` (back to front) that is opaque at
    /// the stage point, skipping hidden and fully ghosted ones.
    pub fn pick(&self, candidates: &[u32], x: f64, y: f64) -> Option<u32> {
        candidates.iter().rev().copied().find(|id| {
            let Some(dr) = self.drawables.get(id) else { return false };
            if !dr.visible || dr.effects.ghost >= 100.0 {
                return false;
            }
            match self.draw_of(*id) {
                Some((d, skin)) => touching_raster(self.skins[&skin].sil_raster(), &d, x, y),
                None => false,
            }
        })
    }

    // ------------------------------------------------------------- drawing

    /// Composite `order` (back to front) over the background into `rgba`.
    pub fn draw(&mut self, order: &[u32]) {
        if let Some(mut gpu) = self.gpu.take() {
            gpu.begin(self.background);
            for id in order {
                if !self.drawables.get(id).is_some_and(|d| d.visible) {
                    continue;
                }
                if let Some((d, skin)) = self.draw_of(*id) {
                    if let Some(skin) = self.skins.get_mut(&skin) {
                        gpu.sprite(skin, &d, self.scale);
                    }
                }
            }
            gpu.finish(&mut self.rgba);
            self.gpu = Some(gpu);
            return;
        }
        let mut canvas = std::mem::replace(&mut self.canvas, Raster::new(0, 0));
        for p in canvas.data.chunks_exact_mut(4) {
            p.copy_from_slice(&[self.background[0], self.background[1], self.background[2], 255]);
        }
        self.composite_list(order, &mut canvas);
        for (o, p) in self.rgba.chunks_exact_mut(4).zip(canvas.data.chunks_exact(4)) {
            o.copy_from_slice(&[p[0], p[1], p[2], 255]);
        }
        self.canvas = canvas;
    }

    fn composite_list(&mut self, order: &[u32], canvas: &mut Raster) {
        for id in order {
            if !self.drawables.get(id).is_some_and(|d| d.visible) {
                continue;
            }
            if let Some((d, skin)) = self.draw_of(*id) {
                if let Some(skin) = self.skins.get_mut(&skin) {
                    composite(skin, &d, self.scale, canvas);
                }
            }
        }
    }

    /// One drawable alone on a transparent background, cropped to its quad's
    /// device-pixel bounds: `(x, y, w, h)` relative to the output and the
    /// straight RGBA pixels (`RenderWebGL.extractDrawableScreenSpace`).
    pub fn extract(&mut self, id: u32) -> Option<Extracted> {
        let b = self.aabb(id)?;
        let s = self.scale;
        let x0 = (((b.left + STAGE_WIDTH / 2.0) * s).floor() as i64).clamp(0, self.width as i64);
        let x1 = (((b.right + STAGE_WIDTH / 2.0) * s).ceil() as i64).clamp(0, self.width as i64);
        let y0 = (((STAGE_HEIGHT / 2.0 - b.top) * s).floor() as i64).clamp(0, self.height as i64);
        let y1 = (((STAGE_HEIGHT / 2.0 - b.bottom) * s).ceil() as i64).clamp(0, self.height as i64);
        let (w, h) = ((x1 - x0).max(1) as usize, (y1 - y0).max(1) as usize);
        let mut layer = Raster::new(self.width, self.height);
        let visible = self.drawables.get(&id).map(|d| d.visible);
        if let Some(d) = self.drawables.get_mut(&id) {
            d.visible = true;
        }
        self.composite_list(&[id], &mut layer);
        if let (Some(d), Some(v)) = (self.drawables.get_mut(&id), visible) {
            d.visible = v;
        }
        let mut out = vec![0u8; w * h * 4];
        for row in 0..h {
            let sy = y0 as usize + row;
            if sy >= layer.h {
                break;
            }
            for col in 0..w {
                let sx = x0 as usize + col;
                if sx >= layer.w {
                    break;
                }
                let p = layer.px(sx, sy);
                let o = (row * w + col) * 4;
                // Un-premultiply for ImageData.
                let a = p[3] as u32;
                let unmul = |c: u8| (c as u32 * 255 + a / 2).checked_div(a).map_or(0, |v| v.min(255) as u8);
                out[o..o + 4].copy_from_slice(&[unmul(p[0]), unmul(p[1]), unmul(p[2]), p[3]]);
            }
        }
        Some(((x0, y0, w, h), out))
    }
}
