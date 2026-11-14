//! Costume skins: decoded bitmaps and SVG trees rasterised on demand at
//! power-of-two scales (like scratch-render's MIP levels).

use resvg::tiny_skia;
use resvg::usvg;
use std::collections::HashMap;
use std::sync::Arc;

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
    /// The parsed SVG, and its source when it embeds raster images (they are
    /// re-resolved per scale, see `render_svg`).
    Svg(Box<usvg::Tree>, Option<Arc<SvgSource>>),
    Bitmap,
}

/// What `usvg` needs to parse an SVG again: its (font-mapped) text and fonts.
pub struct SvgSource {
    data: Vec<u8>,
    fontdb: Arc<usvg::fontdb::Database>,
}

impl SvgSource {
    fn new(tree: &usvg::Tree, data: Vec<u8>, opt: &usvg::Options) -> Option<Arc<SvgSource>> {
        tree_has_image(tree.root()).then(|| Arc::new(SvgSource { data, fontdb: opt.fontdb.clone() }))
    }
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
    /// Offset at which an SVG is drawn on its canvas (minus the viewBox
    /// origin, scaled to the natural size); zero for bitmaps.
    offset: (f64, f64),
    pub kind: SkinKind,
    /// Base raster: native bitmap, or the SVG at scale 1.
    pub base: Raster,
    /// SVG rasters by MIP level (scale = 2^level).
    mips: HashMap<i32, Raster>,
    /// Cached convex-hull rows (y, first x, last x) in skin units, without effects.
    pub hull: Option<Vec<(f64, f64, f64)>>,
    /// SVG: MIP level of the silhouette (the largest MIP requested so far,
    /// `SVGSkin.createMIP` with `isLargestMIP`); `None` before any.
    sil_level: Option<i32>,
    /// SVG: `_maxTextureScale`.
    max_scale: f64,
    /// SVG: `_largestMIPScale` as a level.
    largest_level: Option<i32>,
}

/// Draw the SVG image at its natural size (`width` x `height`) scaled by
/// `scale`, shifted by `offset` (stage units), onto a canvas of
/// `size * scale` pixels, as `SVGSkin.createMIP` draws the `<img>` onto a
/// canvas sized from the viewBox.
fn render_svg(tree: &usvg::Tree, src: Option<&SvgSource>, size: (f64, f64), offset: (f64, f64), scale: f64) -> Raster {
    let leveled = src.and_then(|src| tree_with_image_levels(tree, src, scale));
    let tree = leveled.as_ref().unwrap_or(tree);
    // Embedded images are drawn pixelated; Skia's nearest sampling picks the
    // texel below an exact texel boundary (an even texel at 2:1), tiny-skia
    // the one above. A 1/512 px shift makes it pick the same texel.
    let bias = if tree_has_image(tree.root()) { 1.0 / 512.0 } else { 0.0 };
    // `canvas.width = width * scale` truncates.
    let w = ((size.0 * scale) as u32).clamp(1, 4096);
    let h = ((size.1 * scale) as u32).clamp(1, 4096);
    let Some(mut pixmap) = tiny_skia::Pixmap::new(w, h) else { return Raster::new(1, 1) };
    let t = tiny_skia::Transform::from_row(scale as f32, 0.0, 0.0, scale as f32, (offset.0 * scale + bias) as f32, (offset.1 * scale + bias) as f32);
    resvg::render(tree, t, &mut pixmap.as_mut());
    Raster::from_pixmap(&pixmap)
}

/// Attributes of the root `<svg>` start tag: (byte range of the attribute
/// text within `text`, (name, value) pairs).
fn root_attrs(text: &str) -> Option<(std::ops::Range<usize>, Vec<(&str, &str)>)> {
    // First `<svg` start tag (skipping the XML declaration, comments, doctype).
    let mut pos = 0;
    let begin = loop {
        let i = text[pos..].find('<')? + pos;
        let rest = &text[i + 1..];
        if rest.starts_with("!--") {
            pos = i + 1 + rest.find("-->")? + 3;
            continue;
        }
        if rest.starts_with('?') || rest.starts_with('!') {
            pos = i + 1;
            continue;
        }
        let name_end = rest.find(|c: char| c.is_whitespace() || c == '>' || c == '/').unwrap_or(rest.len());
        let name = &rest[..name_end];
        if name == "svg" || name.ends_with(":svg") {
            break i + 1 + name_end;
        }
        return None;
    };
    let mut attrs = Vec::new();
    let mut at = begin;
    loop {
        let trimmed = text[at..].trim_start();
        at = text.len() - trimmed.len();
        if trimmed.is_empty() || trimmed.starts_with('>') || trimmed.starts_with('/') {
            return Some((begin..at, attrs));
        }
        let eq = trimmed.find('=')?;
        let name = trimmed[..eq].trim();
        let after = trimmed[eq + 1..].trim_start();
        let quote = after.chars().next()?;
        if quote != '"' && quote != '\'' {
            return None;
        }
        let end = after[1..].find(quote)? + 1;
        attrs.push((name, &after[1..end]));
        at = text.len() - after.len() + end + 1;
    }
}

/// Chrome draws a pixelated image that is scaled down to `s` (device pixels
/// per image pixel) from a smaller copy: level `n`, the smallest power-of-two
/// reduction still at least as large as the drawn size (`2^-n >= s`), of
/// `ceil(size / 2)` per level, made by nearest sampling at pixel centres with
/// ties going to the lower pixel (Skia's nearest rule). Both picks are point
/// samples, so the result is made of the image's own pixels.
fn image_level(s: f64) -> u32 {
    let mut n = 0;
    while n < 16 && 2f64.powi(-(n as i32 + 1)) >= s {
        n += 1;
    }
    n
}

fn downsample_level(img: &image::RgbaImage, n: u32) -> image::RgbaImage {
    let (w0, h0) = (img.width() as u64, img.height() as u64);
    let (mut w, mut h) = (w0, h0);
    for _ in 0..n {
        (w, h) = (w.div_ceil(2).max(1), h.div_ceil(2).max(1));
    }
    // ceil((t + 1/2) * W / w) - 1, exactly.
    let pick = |t: u64, big: u64, small: u64| ((2 * t + 1) * big).div_ceil(2 * small).saturating_sub(1).min(big - 1) as u32;
    image::RgbaImage::from_fn(w as u32, h as u32, |x, y| *img.get_pixel(pick(x as u64, w0, w), pick(y as u64, h0, h)))
}

fn image_data(kind: &usvg::ImageKind) -> Option<&Arc<Vec<u8>>> {
    match kind {
        usvg::ImageKind::JPEG(d) | usvg::ImageKind::PNG(d) | usvg::ImageKind::GIF(d) | usvg::ImageKind::WEBP(d) => Some(d),
        usvg::ImageKind::SVG(_) => None,
    }
}

fn data_key(d: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    d.hash(&mut h);
    h.finish()
}

/// Levels of the embedded raster images drawn at `scale`, by their data.
fn collect_image_levels(g: &usvg::Group, scale: f64, out: &mut HashMap<u64, u32>) {
    for n in g.children() {
        match n {
            usvg::Node::Group(c) => collect_image_levels(c, scale, out),
            usvg::Node::Image(img) => {
                let Some(data) = image_data(img.kind()) else { continue };
                let Ok((pw, ph)) = image::ImageReader::new(std::io::Cursor::new(data.as_slice())).with_guessed_format().map_err(|_| ()).and_then(|r| r.into_dimensions().map_err(|_| ())) else { continue };
                let t = img.abs_transform();
                let (ax, ay) = ((t.sx as f64).hypot(t.ky as f64), (t.kx as f64).hypot(t.sy as f64));
                // preserveAspectRatio "meet": one scale for both axes.
                let fit = (img.size().width() as f64 / pw as f64).min(img.size().height() as f64 / ph as f64);
                let s = scale * ax.min(ay) * fit;
                let level = image_level(s);
                if level > 0 {
                    let e = out.entry(data_key(data)).or_insert(level);
                    *e = (*e).min(level);
                }
            }
            _ => {}
        }
    }
}

/// `tree` parsed again with its downscaled images replaced by Chrome's
/// reduced copies (see `image_level`), when any image needs one at `scale`.
fn tree_with_image_levels(tree: &usvg::Tree, src: &SvgSource, scale: f64) -> Option<usvg::Tree> {
    let mut levels = HashMap::new();
    collect_image_levels(tree.root(), scale, &mut levels);
    if levels.is_empty() {
        return None;
    }
    usvg::Tree::from_data(&src.data, &svg_options(src.fontdb.clone(), levels)).ok()
}

fn encode_png(img: &image::RgbaImage) -> Option<Vec<u8>> {
    let mut png = Vec::new();
    let enc = image::codecs::png::PngEncoder::new_with_quality(&mut png, image::codecs::png::CompressionType::Fast, image::codecs::png::FilterType::NoFilter);
    img.write_with_encoder(enc).ok()?;
    Some(png)
}

/// Parsing options for SVG costumes: embedded images with an ICC profile
/// are converted to sRGB as Chrome draws them (`color::decode_rgba`), and
/// images in `levels` (keyed by their data after that conversion) are
/// replaced by Chrome's reduced copy.
fn svg_options(fontdb: Arc<usvg::fontdb::Database>, levels: HashMap<u64, u32>) -> usvg::Options<'static> {
    let mut opt = usvg::Options { fontdb, ..usvg::Options::default() };
    crate::text::configure(&mut opt);
    let default = usvg::ImageHrefResolver::default_data_resolver();
    opt.image_href_resolver.resolve_data = Box::new(move |mime: &str, data: Arc<Vec<u8>>, o: &usvg::Options| {
        let icc = crate::color::has_icc(&data);
        if !icc && levels.is_empty() {
            return default(mime, data, o);
        }
        let mut data = data;
        let mut decoded = None;
        if icc {
            if let Some(png) = crate::color::decode_rgba(&data).and_then(|img| encode_png(&img).map(|p| (p, img))) {
                data = Arc::new(png.0);
                decoded = Some(png.1);
            }
        }
        if let Some(&n) = levels.get(&data_key(&data)) {
            let img = decoded.take().or_else(|| image::load_from_memory(&data).ok().map(|i| i.to_rgba8()));
            if let Some(png) = img.and_then(|img| encode_png(&downsample_level(&img, n))) {
                return Some(usvg::ImageKind::PNG(Arc::new(png)));
            }
        }
        if icc {
            return Some(usvg::ImageKind::PNG(data));
        }
        default(mime, data, o)
    });
    opt
}

/// Parse an SVG costume (see `svg_options`).
fn parse_svg(data: &[u8], opt: &usvg::Options) -> Option<usvg::Tree> {
    if data.windows(11).any(|w| w == b"data:image/") {
        usvg::Tree::from_data(data, &svg_options(opt.fontdb.clone(), HashMap::new())).ok()
    } else {
        usvg::Tree::from_data(data, opt).ok()
    }
}

fn tree_has_image(g: &usvg::Group) -> bool {
    g.children().iter().any(|n| match n {
        usvg::Node::Image(_) => true,
        usvg::Node::Group(c) => tree_has_image(c),
        _ => false,
    })
}

/// `viewBox` of the root `<svg>` element as the browser parses it
/// (`svgTag.viewBox.baseVal`): four numbers separated by whitespace and/or
/// commas; `None` when missing or malformed.
fn root_view_box(data: &[u8]) -> Option<(f64, f64, f64, f64)> {
    let text = std::str::from_utf8(data).ok()?;
    let (_, attrs) = root_attrs(text)?;
    let value = attrs.iter().find(|(n, _)| *n == "viewBox")?.1;
    let nums: Vec<f64> = value.split(|c: char| c.is_whitespace() || c == ',').filter(|t| !t.is_empty()).map(blink_parse_number).collect::<Option<_>>()?;
    if nums.len() != 4 || nums[2] < 0.0 || nums[3] < 0.0 {
        return None;
    }
    Some((nums[0], nums[1], nums[2], nums[3]))
}

/// Blink's `GenericParseNumber` (svg_parser_utilities.cc): the number is
/// built digit by digit in float32, so e.g. "0.13503" becomes
/// 0.13502998650074005 rather than the nearest float. scratch-render reads
/// the viewBox through `SVGRect`, i.e. these values.
fn blink_parse_number(t: &str) -> Option<f64> {
    let f = |v: f32| v;
    let b = t.as_bytes();
    let mut i = 0;
    let mut sign = 1.0f32;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        if b[i] == b'-' {
            sign = -1.0;
        }
        i += 1;
    }
    let start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i == start && !(i < b.len() && b[i] == b'.') {
        return None;
    }
    let mut integer = 0.0f32;
    let mut multiplier = 1.0f32;
    for &c in b[start..i].iter().rev() {
        integer = f(integer + f(multiplier * (c - b'0') as f32));
        multiplier = f(multiplier * 10.0);
    }
    let mut decimal = 0.0f32;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        if i >= b.len() || !b[i].is_ascii_digit() {
            return None;
        }
        let mut frac = 1.0f32;
        while i < b.len() && b[i].is_ascii_digit() {
            frac = f(frac * 0.1f32);
            decimal = f(decimal + f((b[i] - b'0') as f32 * frac));
            i += 1;
        }
    }
    let mut exponent = 0.0f32;
    let mut exponent_sign = 1.0f32;
    if i + 1 < b.len() && (b[i] == b'e' || b[i] == b'E') && b[i + 1] != b'x' && b[i + 1] != b'm' {
        i += 1;
        if b[i] == b'+' {
            i += 1;
        } else if b[i] == b'-' {
            exponent_sign = -1.0;
            i += 1;
        }
        if i >= b.len() || !b[i].is_ascii_digit() {
            return None;
        }
        while i < b.len() && b[i].is_ascii_digit() {
            exponent = f(f(exponent * 10.0) + (b[i] - b'0') as f32);
            i += 1;
        }
    }
    if i != b.len() {
        return None;
    }
    let mut number = f(f(integer + decimal) * sign);
    if exponent != 0.0 {
        number = f(number * 10f64.powi((exponent_sign * exponent) as i32) as f32);
    }
    Some(number as f64)
}

/// The SVG with the root's `width` / `height` replaced by a large canvas, so
/// that user units are pixels and a zero size does not stop parsing: the
/// document `getBBox` sees in `transformMeasurements`.
fn without_root_size(data: &[u8]) -> Option<Vec<u8>> {
    let text = std::str::from_utf8(data).ok()?;
    let (range, attrs) = root_attrs(text)?;
    let mut out = String::with_capacity(text.len() + 40);
    out.push_str(&text[..range.start]);
    out.push_str(" width=\"100000\" height=\"100000\"");
    for (n, v) in attrs {
        if n != "width" && n != "height" {
            out.push_str(&format!(" {n}=\"{v}\""));
        }
    }
    out.push_str(&text[range.end..]);
    Some(out.into_bytes())
}

/// `getBBox()` of the document: the union of the shapes' geometry (no
/// stroke, clipping ignored) in user space. usvg's own `abs_bounding_box`
/// applies an image's transform twice, so images are measured here.
fn content_bbox(g: &usvg::Group) -> Option<tiny_skia::Rect> {
    let mut acc: Option<tiny_skia::Rect> = None;
    let mut add = |r: Option<tiny_skia::Rect>| {
        if let Some(r) = r {
            acc = Some(match acc {
                None => r,
                Some(a) => tiny_skia::Rect::from_ltrb(a.left().min(r.left()), a.top().min(r.top()), a.right().max(r.right()), a.bottom().max(r.bottom())).unwrap_or(a),
            });
        }
    };
    for n in g.children() {
        match n {
            usvg::Node::Group(c) => add(content_bbox(c)),
            usvg::Node::Path(p) => add(Some(p.abs_bounding_box())),
            usvg::Node::Image(i) => add(i.bounding_box().transform(i.abs_transform())),
            usvg::Node::Text(t) => add(Some(t.abs_bounding_box())),
        }
    }
    acc
}

/// `SVGSkin._maxTextureScale`: the largest power of two from 2 up whose MIP
/// stays within the renderer's 2048 px texture limit, else 1.
fn max_texture_scale(size: (f64, f64)) -> f64 {
    let max_dim = size.0.max(size.1).ceil();
    let mut scale = 1.0;
    let mut test = 2.0;
    while max_dim * test <= 2048.0 {
        scale = test;
        test *= 2.0;
    }
    scale
}

/// scratch-svg-renderer's `findLargestStrokeWidth`, on the raw attributes:
/// 1 for any element with a `stroke` attribute (even "none"), and the value
/// of any `stroke-width` attribute (`Number(...) || 0`).
fn largest_stroke_width(data: &[u8]) -> f64 {
    let Ok(text) = std::str::from_utf8(data) else { return 0.0 };
    let Ok(doc) = usvg::roxmltree::Document::parse(text) else { return 0.0 };
    let mut w: f64 = 0.0;
    for n in doc.descendants().filter(|n| n.is_element()) {
        if n.attribute("stroke").is_some_and(|v| !v.is_empty()) {
            w = w.max(1.0);
        }
        if let Some(v) = n.attribute("stroke-width").filter(|v| !v.is_empty()) {
            let x = scrust_value::js_string_to_number(&scrust_value::JsStr::from(v));
            w = w.max(if x.is_nan() { 0.0 } else { x });
        }
    }
    w
}

impl Skin {
    pub fn empty() -> Skin {
        Skin { uid: next_uid(), size: (1.0, 1.0), center: (0.0, 0.0), offset: (0.0, 0.0), kind: SkinKind::Bitmap, base: Raster::new(1, 1), mips: HashMap::new(), hull: None, sil_level: None, max_scale: 1.0, largest_level: None }
    }

    pub fn load(data: &[u8], format: &str, bitmap_resolution: f64, center: (f64, f64), opt: &usvg::Options) -> Skin {
        if format.eq_ignore_ascii_case("svg") {
            let data = crate::text::map_font_families(data);
            let view_box = root_view_box(&data);
            // `SVGSkin.setSVG`: the skin's size is the viewBox's; a zero-sized
            // one becomes an empty image (rotation centre 0, 0) while keeping
            // that size, so `set size` is unclamped and the drawable's matrix
            // (size x scale) can be NaN.
            if let Some((_, _, 0.0, _) | (_, _, _, 0.0)) = view_box {
                let (_, _, w, h) = view_box.expect("matched");
                return Skin { uid: next_uid(), size: (w, h), center: (0.0, 0.0), offset: (0.0, 0.0), kind: SkinKind::Bitmap, base: Raster::new(1, 1), mips: HashMap::new(), hull: None, sil_level: None, max_scale: 1.0, largest_level: None };
            }
            let zero = |w: f64, h: f64| Skin { uid: next_uid(), size: (w, h), center: (0.0, 0.0), offset: (0.0, 0.0), kind: SkinKind::Bitmap, base: Raster::new(1, 1), mips: HashMap::new(), hull: None, sil_level: None, max_scale: 1.0, largest_level: None };
            if view_box.is_none() {
                // No viewBox: scratch-svg-renderer's `transformMeasurements`
                // crops to the content's bounding box (`getBBox`), enlarged by
                // half the widest stroke unless it is empty in a direction.
                let Some(tree) = without_root_size(&data).and_then(|d| parse_svg(&d, opt)) else { return zero(0.0, 0.0) };
                let Some(b) = content_bbox(tree.root()) else { return zero(0.0, 0.0) };
                let (bw, bh) = (b.width() as f64, b.height() as f64);
                if !(bw > 0.0 && bh > 0.0) {
                    return zero(bw.max(0.0), bh.max(0.0));
                }
                let half = largest_stroke_width(&data) / 2.0;
                // Written back as the viewBox and read as float32 (`SVGRect`).
                let f = |v: f64| v as f32 as f64;
                let size = (f(bw + half * 2.0), f(bh + half * 2.0));
                let origin = (f(b.x() as f64 - half), f(b.y() as f64 - half));
                let offset = (-origin.0, -origin.1);
                let center = (center.0 - origin.0, center.1 - origin.1);
                let Some(cropped) = without_root_size(&data) else { return zero(0.0, 0.0) };
                let src = SvgSource::new(&tree, cropped, opt);
                let base = render_svg(&tree, src.as_deref(), size, offset, 1.0);
                return Skin { uid: next_uid(), size, center, offset, kind: SkinKind::Svg(Box::new(tree), src), base, mips: HashMap::new(), hull: None, sil_level: None, max_scale: max_texture_scale(size), largest_level: None };
            }
            match parse_svg(&data, opt) {
                Some(tree) => {
                    let (x, y, w, h) = view_box.expect("checked");
                    // The image is drawn at its natural size at the canvas
                    // origin; "compensate for viewbox offset" (scratch-render#90).
                    let (size, offset) = ((w, h), (0.0, 0.0));
                    let center = (center.0 - x, center.1 - y);
                    let src = SvgSource::new(&tree, data.to_vec(), opt);
                    let base = render_svg(&tree, src.as_deref(), size, offset, 1.0);
                    Skin { uid: next_uid(), size, center, offset, kind: SkinKind::Svg(Box::new(tree), src), base, mips: HashMap::new(), hull: None, sil_level: None, max_scale: max_texture_scale(size), largest_level: None }
                }
                None => Skin::empty(),
            }
        } else {
            match crate::color::decode_rgba(data) {
                Some(rgba) => {
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
                        offset: (0.0, 0.0),
                        kind: SkinKind::Bitmap,
                        base,
                        mips: HashMap::new(),
                        hull: None,
                        sil_level: None,
                        max_scale: 1.0,
                        largest_level: None,
                    }
                }
                None => Skin::empty(),
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
        Skin { uid: next_uid(), size, center: center.unwrap_or((size.0 / 2.0, size.1 / 2.0)), offset: (0.0, 0.0), kind: SkinKind::Bitmap, base: Raster { w, h, data }, mips: HashMap::new(), hull: None, sil_level: None, max_scale: 1.0, largest_level: None }
    }

    /// `SVGSkin.getTexture`: the MIP level for `pixels_per_unit`
    /// (`ceil(log2(min(scale, _maxTextureScale)))`, at least 2^-8).
    fn mip_level(&self, pixels_per_unit: f64) -> i32 {
        let requested = pixels_per_unit.min(self.max_scale);
        let l = requested.log2().ceil();
        if l.is_finite() {
            (l as i32).max(-8)
        } else {
            -8
        }
    }

    fn mip(&mut self, level: i32) -> &Raster {
        let SkinKind::Svg(tree, src) = &self.kind else { return &self.base };
        if level == 0 {
            return &self.base;
        }
        let (size, offset) = (self.size, self.offset);
        self.mips.entry(level).or_insert_with(|| render_svg(tree, src.as_deref(), size, offset, 2f64.powi(level)))
    }

    /// Creating a MIP larger than any before replaces the silhouette
    /// (`isLargestMIP`), unless its canvas is empty: `canvas.width` truncates
    /// `size * scale`, and `Silhouette.update` ignores a 0-sized image.
    fn note_level(&mut self, level: i32) {
        if self.largest_level.is_some_and(|l| level <= l) {
            return;
        }
        self.largest_level = Some(level);
        let k = 2f64.powi(level);
        if (self.size.0 * k) as u32 >= 1 && (self.size.1 * k) as u32 >= 1 {
            self.sil_level = Some(level);
            self.hull = None;
        }
    }

    /// The MIP level a drawable at `scale_pct` (percent, ×`device` pixels per
    /// unit) is drawn with: noted as if `getTexture` had run.
    pub fn note_drawn(&mut self, scale_pct: (f64, f64), device: f64) {
        if matches!(self.kind, SkinKind::Svg(..)) {
            let k = scale_pct.0.abs().max(scale_pct.1.abs()) / 100.0 * device;
            let level = self.mip_level(k);
            self.note_level(level);
        }
    }

    /// Raster suited for drawing at `pixels_per_unit` device pixels per stage unit.
    pub fn raster_for(&mut self, pixels_per_unit: f64) -> &Raster {
        if matches!(self.kind, SkinKind::Bitmap) {
            return &self.base;
        }
        let level = self.mip_level(pixels_per_unit);
        self.note_level(level);
        self.mip(level)
    }

    /// `updateSilhouette(drawable.scale)` before a CPU query, then the
    /// silhouette raster (empty until a non-empty MIP has been made).
    pub fn silhouette(&mut self, scale_pct: (f64, f64)) -> &Raster {
        if matches!(self.kind, SkinKind::Bitmap) {
            return &self.base;
        }
        let k = scale_pct.0.abs().max(scale_pct.1.abs()) / 100.0;
        let level = self.mip_level(k);
        self.note_level(level);
        if let Some(l) = self.sil_level {
            self.mip(l);
        }
        self.sil_raster()
    }

    /// The current silhouette raster (after `silhouette`).
    pub fn sil_raster(&self) -> &Raster {
        static EMPTY: std::sync::OnceLock<Raster> = std::sync::OnceLock::new();
        match (&self.kind, self.sil_level) {
            (SkinKind::Bitmap, _) => &self.base,
            (_, None) => EMPTY.get_or_init(|| Raster::new(1, 1)),
            (_, Some(0)) => &self.base,
            (_, Some(l)) => self.mips.get(&l).unwrap_or(&self.base),
        }
    }
}
