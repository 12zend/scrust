//! Costume skins: decoded bitmaps and SVG trees rasterised on demand at
//! power-of-two scales (like scratch-render's MIP levels).

use resvg::tiny_skia;
use resvg::usvg;
use std::collections::HashMap;

/// Premultiplied RGBA8 image.
#[derive(Clone)]
pub struct Raster {
    pub w: usize,
    pub h: usize,
    pub data: Vec<u8>,
}

impl Raster {
    pub fn new(w: usize, h: usize) -> Raster {
        Raster { w, h, data: vec![0; w * h * 4] }
    }

    pub fn from_pixmap(p: &tiny_skia::Pixmap) -> Raster {
        Raster { w: p.width() as usize, h: p.height() as usize, data: p.data().to_vec() }
    }

    #[inline]
    pub fn px(&self, x: usize, y: usize) -> [u8; 4] {
        let i = (y * self.w + x) * 4;
        [self.data[i], self.data[i + 1], self.data[i + 2], self.data[i + 3]]
    }

    /// Nearest-neighbour sample at texture coordinates in [0, 1).
    #[inline]
    pub fn nearest(&self, u: f64, v: f64) -> [u8; 4] {
        if self.w == 0 || self.h == 0 {
            return [0; 4];
        }
        let x = ((u * self.w as f64) as usize).min(self.w - 1);
        let y = ((v * self.h as f64) as usize).min(self.h - 1);
        self.px(x, y)
    }

    /// Bilinear sample (premultiplied) at texture coordinates in [0, 1].
    pub fn linear(&self, u: f64, v: f64) -> [u8; 4] {
        if self.w == 0 || self.h == 0 {
            return [0; 4];
        }
        let fx = u * self.w as f64 - 0.5;
        let fy = v * self.h as f64 - 0.5;
        let x0 = fx.floor();
        let y0 = fy.floor();
        let (ax, ay) = (fx - x0, fy - y0);
        let get = |x: f64, y: f64| -> [f64; 4] {
            if x < 0.0 || y < 0.0 || x >= self.w as f64 || y >= self.h as f64 {
                return [0.0; 4];
            }
            let p = self.px(x as usize, y as usize);
            [p[0] as f64, p[1] as f64, p[2] as f64, p[3] as f64]
        };
        let (p00, p10, p01, p11) = (get(x0, y0), get(x0 + 1.0, y0), get(x0, y0 + 1.0), get(x0 + 1.0, y0 + 1.0));
        let mut out = [0u8; 4];
        for i in 0..4 {
            let top = p00[i] * (1.0 - ax) + p10[i] * ax;
            let bot = p01[i] * (1.0 - ax) + p11[i] * ax;
            out[i] = (top * (1.0 - ay) + bot * ay + 0.5) as u8;
        }
        out
    }
}

pub enum SkinKind {
    Svg(Box<usvg::Tree>),
    Bitmap,
}

/// Source of `Skin::uid`.
static NEXT_UID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn next_uid() -> u64 {
    NEXT_UID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

pub struct Skin {
    /// Unique per skin (and per replacement), so GPU textures can be cached by it.
    pub uid: u64,
    /// Size in stage units.
    pub size: (f64, f64),
    /// Rotation centre in stage units from the top-left corner.
    pub center: (f64, f64),
    pub kind: SkinKind,
    /// Base raster: native bitmap, or the SVG at scale 1.
    pub base: Raster,
    /// SVG rasters by MIP level (scale = 2^level).
    mips: HashMap<i32, Raster>,
    /// Cached convex-hull rows (y, first x, last x) in skin units, without effects.
    pub hull: Option<Vec<(f64, f64, f64)>>,
}

fn render_svg(tree: &usvg::Tree, scale: f64) -> Raster {
    let size = tree.size();
    let w = ((size.width() as f64 * scale).ceil() as u32).clamp(1, 4096);
    let h = ((size.height() as f64 * scale).ceil() as u32).clamp(1, 4096);
    let Some(mut pixmap) = tiny_skia::Pixmap::new(w, h) else { return Raster::new(1, 1) };
    let t = tiny_skia::Transform::from_scale(scale as f32, scale as f32);
    resvg::render(tree, t, &mut pixmap.as_mut());
    Raster::from_pixmap(&pixmap)
}

impl Skin {
    pub fn empty() -> Skin {
        Skin { uid: next_uid(), size: (1.0, 1.0), center: (0.0, 0.0), kind: SkinKind::Bitmap, base: Raster::new(1, 1), mips: HashMap::new(), hull: None }
    }

    pub fn load(data: &[u8], format: &str, bitmap_resolution: f64, center: (f64, f64), opt: &usvg::Options) -> Skin {
        if format.eq_ignore_ascii_case("svg") {
            let data = crate::text::map_font_families(data);
            match usvg::Tree::from_data(&data, opt) {
                Ok(tree) => {
                    let size = (tree.size().width() as f64, tree.size().height() as f64);
                    let base = render_svg(&tree, 1.0);
                    Skin { uid: next_uid(), size, center, kind: SkinKind::Svg(Box::new(tree)), base, mips: HashMap::new(), hull: None }
                }
                Err(_) => Skin::empty(),
            }
        } else {
            match image::load_from_memory(data) {
                Ok(img) => {
                    let rgba = img.to_rgba8();
                    let (w, h) = (rgba.width() as usize, rgba.height() as usize);
                    let mut data = rgba.into_raw();
                    for p in data.chunks_exact_mut(4) {
                        let a = p[3] as u32;
                        p[0] = ((p[0] as u32 * a + 127) / 255) as u8;
                        p[1] = ((p[1] as u32 * a + 127) / 255) as u8;
                        p[2] = ((p[2] as u32 * a + 127) / 255) as u8;
                    }
                    let res = if bitmap_resolution > 0.0 { bitmap_resolution } else { 1.0 };
                    let base = Raster { w, h, data };
                    Skin {
                        uid: next_uid(),
                        size: (w as f64 / res, h as f64 / res),
                        center: (center.0 / res, center.1 / res),
                        kind: SkinKind::Bitmap,
                        base,
                        mips: HashMap::new(),
                        hull: None,
                    }
                }
                Err(_) => Skin::empty(),
            }
        }
    }

    /// Bitmap skin from straight-alpha RGBA pixels (a decoded browser image).
    /// `center` is in stage units, i.e. already divided by the resolution.
    pub fn from_rgba(w: usize, h: usize, rgba: &[u8], bitmap_resolution: f64, center: Option<(f64, f64)>) -> Skin {
        if w == 0 || h == 0 || rgba.len() < w * h * 4 {
            return Skin::empty();
        }
        let mut data = rgba[..w * h * 4].to_vec();
        for p in data.chunks_exact_mut(4) {
            let a = p[3] as u32;
            p[0] = ((p[0] as u32 * a + 127) / 255) as u8;
            p[1] = ((p[1] as u32 * a + 127) / 255) as u8;
            p[2] = ((p[2] as u32 * a + 127) / 255) as u8;
        }
        let res = if bitmap_resolution > 0.0 { bitmap_resolution } else { 1.0 };
        let size = (w as f64 / res, h as f64 / res);
        Skin { uid: next_uid(), size, center: center.unwrap_or((size.0 / 2.0, size.1 / 2.0)), kind: SkinKind::Bitmap, base: Raster { w, h, data }, mips: HashMap::new(), hull: None }
    }

    /// Raster suited for drawing at `pixels_per_unit` device pixels per stage unit.
    pub fn raster_for(&mut self, pixels_per_unit: f64) -> &Raster {
        match &self.kind {
            SkinKind::Bitmap => &self.base,
            SkinKind::Svg(tree) => {
                let max_dim = self.size.0.max(self.size.1).max(1.0);
                let mut level = pixels_per_unit.max(1e-3).log2().ceil() as i32;
                // Keep textures within 2048 px, like scratch-render.
                while level > -4 && max_dim * 2f64.powi(level) > 2048.0 {
                    level -= 1;
                }
                let level = level.clamp(-4, 6);
                if level == 0 {
                    return &self.base;
                }
                self.mips.entry(level).or_insert_with(|| render_svg(tree, 2f64.powi(level)))
            }
        }
    }
}
