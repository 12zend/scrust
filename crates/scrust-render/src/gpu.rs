//! Metal compositor: draws sprites (with their graphic effects), the pen
//! layer and the text overlays on the GPU, and keeps the pen layer as a GPU
//! texture that pen lines and stamps are rendered into.
//!
//! Only presentation moves to the GPU. Collision queries keep sampling the
//! CPU silhouettes (`touching_skin`), as scratch-render does; `touching
//! color` reads the pen layer back when it changed (`pen_pixels`).
//!
//! The shaders mirror the CPU path: the same model transform (`Xf`), the
//! effect formulas of `Effects::point` / `Effects::color`, nearest sampling
//! for distorted or 1:1 drawables and bilinear sampling (transparent outside
//! the texture) otherwise, and premultiplied "over" blending.

use crate::skin::{Raster, Skin};
use crate::{Draw, stage_height, stage_width};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_foundation::NSString;
use objc2_metal::{
    MTLBlendFactor, MTLBlitCommandEncoder, MTLBuffer, MTLClearColor, MTLCommandBuffer, MTLCommandEncoder, MTLCommandQueue, MTLCreateSystemDefaultDevice, MTLDevice,
    MTLLibrary, MTLLoadAction, MTLOrigin, MTLPixelFormat, MTLPrimitiveType, MTLRegion, MTLRenderCommandEncoder, MTLRenderPassDescriptor, MTLRenderPipelineDescriptor,
    MTLRenderPipelineState, MTLResourceOptions, MTLSamplerAddressMode, MTLSamplerDescriptor, MTLSamplerMinMagFilter, MTLSamplerState, MTLSize, MTLStorageMode,
    MTLStoreAction, MTLTexture, MTLTextureDescriptor, MTLTextureUsage, MTLViewport,
};
use objc2_core_foundation::CGSize;
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};
use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::NonNull;

type Tex = Retained<ProtocolObject<dyn MTLTexture>>;
type Buf = Retained<ProtocolObject<dyn MTLBuffer>>;
type Pipeline = Retained<ProtocolObject<dyn MTLRenderPipelineState>>;
type Sampler = Retained<ProtocolObject<dyn MTLSamplerState>>;

const SHADERS: &str = r#"
#include <metal_stdlib>
using namespace metal;

// Per drawable. size: skin size (stage units), distort, recolor;
// fxa: color, fisheye, whirl, pixelate; fxb: mosaic, brightness, ghost.
struct Sprite { float4 size; float4 fxa; float4 fxb; };

struct SpriteOut { float4 pos [[position]]; float2 uv; };

float2 effect_point(float2 uv, constant Sprite& s) {
    float mosaic = s.fxb.x, pixelate = s.fxa.w, whirl = s.fxa.z, fisheye = s.fxa.y;
    if (mosaic != 0.0) {
        float m = clamp(round((abs(mosaic) + 10.0) / 10.0), 1.0, 512.0);
        uv = fract(m * uv);
    }
    if (pixelate != 0.0) {
        float p = abs(pixelate) / 10.0;
        float2 t = s.size.xy / p;
        uv = (floor(uv * t) + 0.5) / t;
    }
    if (whirl != 0.0) {
        float2 o = uv - 0.5;
        float mag = length(o);
        float factor = max(1.0 - mag / 0.5, 0.0);
        float a = (-whirl * M_PI_F / 180.0) * factor * factor;
        float sn = sin(a), cs = cos(a);
        uv = float2(cs * o.x + sn * o.y + 0.5, -sn * o.x + cs * o.y + 0.5);
    }
    if (fisheye != 0.0) {
        float2 v = (uv - 0.5) / 0.5;
        float len = length(v);
        if (len > 0.0) {
            float power = max((fisheye + 100.0) / 100.0, 0.0);
            float r = pow(min(len, 1.0), power) * max(len, 1.0);
            uv = 0.5 + r * (v / len) * 0.5;
        }
    }
    return uv;
}

float3 rgb_to_hsv(float3 c) {
    float mx = max(max(c.r, c.g), c.b), mn = min(min(c.r, c.g), c.b), d = mx - mn;
    float h = 0.0;
    if (d != 0.0) {
        if (mx == c.r) { float q = (c.g - c.b) / d; h = (q - 6.0 * floor(q / 6.0)) / 6.0; }
        else if (mx == c.g) h = ((c.b - c.r) / d + 2.0) / 6.0;
        else h = ((c.r - c.g) / d + 4.0) / 6.0;
    }
    return float3(h, mx == 0.0 ? 0.0 : d / mx, mx);
}

float3 hsv_to_rgb(float3 c) {
    float i = floor(c.x * 6.0), f = c.x * 6.0 - i, v = c.z, s = c.y;
    float p = v * (1.0 - s), q = v * (1.0 - f * s), t = v * (1.0 - (1.0 - f) * s);
    int k = int(i - 6.0 * floor(i / 6.0));
    if (k == 0) return float3(v, t, p);
    if (k == 1) return float3(q, v, p);
    if (k == 2) return float3(p, v, t);
    if (k == 3) return float3(p, q, v);
    if (k == 4) return float3(t, p, v);
    return float3(v, p, q);
}

float4 effect_color(float4 c, constant Sprite& s) {
    float a = c.a;
    float3 rgb = c.rgb / a;
    float color = s.fxa.x, brightness = s.fxb.y, ghost = s.fxb.z;
    if (color != 0.0) {
        float3 hsv = rgb_to_hsv(rgb);
        const float MIN_V = 0.11 / 2.0, MIN_S = 0.09;
        if (hsv.z < MIN_V) hsv = float3(0.0, 1.0, MIN_V);
        else if (hsv.y < MIN_S) hsv = float3(0.0, MIN_S, hsv.z);
        hsv.x = fract(hsv.x + color / 200.0);
        rgb = hsv_to_rgb(hsv);
    }
    if (brightness != 0.0) rgb = clamp(rgb + brightness / 100.0, 0.0, 1.0);
    if (ghost != 0.0) a *= clamp(1.0 - ghost / 100.0, 0.0, 1.0);
    return float4(rgb * a, a);
}

fragment float4 sprite_fs(SpriteOut in [[stage_in]], constant Sprite& s [[buffer(0)]],
                          texture2d<float> tex [[texture(0)]], sampler smp [[sampler(0)]]) {
    float2 uv = in.uv;
    if (s.size.z != 0.0) {
        uv = effect_point(uv, s);
        if (uv.x < 0.0 || uv.y < 0.0 || uv.x >= 1.0 || uv.y >= 1.0) discard_fragment();
    }
    float4 c = tex.sample(smp, uv);
    if (c.a <= 0.0) discard_fragment();
    if (s.size.w != 0.0) c = effect_color(c, s);
    return c;
}

// A drawable as scratch-render draws it: six vertices (two triangles in
// its `_createGeometry` order) whose clip positions were computed on the CPU
// in float32 like its vertex shader; xy = clip position, zw = a_texCoord.
vertex SpriteOut tw_vs(uint vid [[vertex_id]], constant float4* v [[buffer(1)]]) {
    SpriteOut o;
    o.pos = float4(v[vid].xy, 0.0, 1.0);
    o.uv = v[vid].zw;
    return o;
}

// Screen-aligned textured rectangle (pen layer, bubbles, monitors), in NDC.
struct Rect { float4 r; };

vertex SpriteOut rect_vs(uint vid [[vertex_id]], constant Rect& q [[buffer(0)]]) {
    float2 uv = float2(float(vid & 1), float(vid >> 1));
    SpriteOut o;
    o.pos = float4(mix(q.r.x, q.r.z, uv.x), mix(q.r.y, q.r.w, uv.y), 0.0, 1.0);
    o.uv = uv;
    return o;
}

fragment float4 rect_fs(SpriteOut in [[stage_in]], texture2d<float> tex [[texture(0)]], sampler smp [[sampler(0)]]) {
    return tex.sample(smp, in.uv);
}

// Pen line: a round-capped segment in device pixels, anti-aliased by
// distance (p: endpoints; color: straight RGBA; misc: radius, target size).
// scratch-render's pen line (sprite.vert / sprite.frag, DRAW_MODE_line):
// per instance the premultiplied colour, (thickness, length, stage size)
// and the pen points (x0, -y0, dx, -dy) in device pixels from the centre.
struct Line { float4 color; float4 tl; float4 pts; };
struct LineOut {
    float4 pos [[position]];
    float2 tc;
    float4 color;
    float thickness;
    float length;
};
vertex LineOut line_vs(uint vid [[vertex_id]], uint iid [[instance_id]], const device Line* lines [[buffer(0)]]) {
    Line l = lines[iid];
    const float epsilon = 1e-3;
    float2 a_position = float2(float((vid & 1) == 0), float(vid >> 1));
    float2 position = a_position;
    float expandedRadius = (l.tl.x * 0.5) + 1.4142135623730951;
    LineOut o;
    o.tc.x = mix(0.0, l.tl.y + (expandedRadius * 2.0), a_position.x) - expandedRadius;
    o.tc.y = ((a_position.y - 0.5) * expandedRadius) + 0.5;
    position.x *= l.tl.y + (2.0 * expandedRadius);
    position.y *= 2.0 * expandedRadius;
    position -= expandedRadius;
    float2 pointDiff = l.pts.zw;
    pointDiff.x = (abs(pointDiff.x) < epsilon && abs(pointDiff.y) < epsilon) ? epsilon : pointDiff.x;
    float2 normalized = pointDiff / max(l.tl.y, epsilon);
    position = float2x2(float2(normalized.x, normalized.y), float2(-normalized.y, normalized.x)) * position;
    position += l.pts.xy;
    position *= 2.0 / l.tl.zw;
    // The pen texture is top down: GL's y as ANGLE stores it.
    o.pos = float4(position.x, -position.y, 0.0, 1.0);
    o.color = l.color;
    o.thickness = l.tl.x;
    o.length = l.tl.y;
    return o;
}
fragment float4 line_fs(LineOut in [[stage_in]]) {
    float d = ((in.tc.x - clamp(in.tc.x, 0.0, in.length)) * 0.5) + 0.5;
    float line = distance(float2(0.5), float2(d, in.tc.y)) * 2.0;
    line -= ((in.thickness - 1.0) * 0.5);
    return in.color * clamp(1.0 - line, 0.0, 1.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy)]
struct SpriteUniforms {
    size: [f32; 4],
    fxa: [f32; 4],
    fxb: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct LineInstance {
    color: [f32; 4],
    tl: [f32; 4],
    pts: [f32; 4],
}

#[derive(Clone)]
struct Quad {
    tex: Tex,
    u: SpriteUniforms,
    nearest: bool,
    /// scratch-render's six vertices (clip xy, texcoord) and the viewport.
    verts: [[f32; 4]; 6],
    viewport: MTLViewport,
}

enum Item {
    Sprite(Quad),
    /// Texture and NDC rectangle (left, top, right, bottom).
    Rect(Tex, [f32; 4]),
}

enum PenOp {
    Clear,
    Lines(Vec<LineInstance>),
    Stamp(Quad),
}

pub(crate) struct Gpu {
    device: Retained<ProtocolObject<dyn MTLDevice>>,
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    sprite: Pipeline,
    rect: Pipeline,
    line: Pipeline,
    linear: Sampler,
    nearest: Sampler,
    width: usize,
    height: usize,
    target: Tex,
    pen: Tex,
    readback: Buf,
    /// Skin rasters by (skin uid, width, height) with the frame they were last used in.
    skins: HashMap<(u64, usize, usize), (Tex, u64)>,
    /// Overlay textures by slot (reused while the size stays the same).
    overlays: HashMap<u64, Tex>,
    frame_no: u64,
    background: [u8; 3],
    items: Vec<Item>,
    pen_ops: Vec<PenOp>,
    /// CPU copy of the pen layer (premultiplied), valid until the next pen operation.
    pen_copy: Option<Raster>,
    /// Storage for sampled textures: shared on unified memory (no second copy).
    storage: MTLStorageMode,
    /// Layer of the window frames are presented to (`attach_view`).
    layer: Option<Retained<CAMetalLayer>>,
    /// Pipeline drawing the composed frame into the layer's BGRA drawable.
    blit: Option<Pipeline>,
    /// The last presented frame, still running on the GPU; waited for before
    /// the CPU touches resources it may read (overlay uploads, readbacks).
    in_flight: Option<Retained<ProtocolObject<dyn MTLCommandBuffer>>>,
    /// Instance buffers used by `in_flight`.
    in_flight_buffers: Vec<Buf>,
}

fn pipeline(device: &ProtocolObject<dyn MTLDevice>, lib: &ProtocolObject<dyn MTLLibrary>, vs: &str, fs: &str) -> Option<Pipeline> {
    pipeline_with(device, lib, vs, fs, MTLPixelFormat::RGBA8Unorm, true)
}

fn pipeline_with(device: &ProtocolObject<dyn MTLDevice>, lib: &ProtocolObject<dyn MTLLibrary>, vs: &str, fs: &str, format: MTLPixelFormat, blend: bool) -> Option<Pipeline> {
    let desc = MTLRenderPipelineDescriptor::new();
    let vs = lib.newFunctionWithName(&NSString::from_str(vs))?;
    let fs = lib.newFunctionWithName(&NSString::from_str(fs))?;
    desc.setVertexFunction(Some(&vs));
    desc.setFragmentFunction(Some(&fs));
    // SAFETY: index 0 always exists.
    let att = unsafe { desc.colorAttachments().objectAtIndexedSubscript(0) };
    att.setPixelFormat(format);
    att.setBlendingEnabled(blend);
    att.setSourceRGBBlendFactor(MTLBlendFactor::One);
    att.setSourceAlphaBlendFactor(MTLBlendFactor::One);
    att.setDestinationRGBBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
    att.setDestinationAlphaBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
    device.newRenderPipelineStateWithDescriptor_error(&desc).ok()
}

/// scratch-render's `penStamp` geometry: the viewport is the drawable's fast
/// bounds `(left, right, bottom, top)` in stage units, scaled to device pixels
/// and snapped outwards, and the projection is `ortho` over those bounds
/// (top and bottom swapped: the pen framebuffer is stored top down). The
/// vertex shader's `u_projectionMatrix * u_modelMatrix * vec4(a_position, 0, 1)`
/// is evaluated here in float32, so pixel centres that fall exactly on
/// texel or quad edges pick the same texel and coverage as in the browser.
fn tw_stamp_vertices(d: &Draw, scale: f64, bounds: (f64, f64, f64, f64)) -> Option<([[f32; 4]; 6], MTLViewport)> {
    let (l, r, b, t) = bounds;
    let (l, r, b, t) = ((l * scale).floor(), (r * scale).ceil(), (b * scale).floor(), (t * scale).ceil());
    if r - l <= 0.0 || t - b <= 0.0 {
        return None;
    }
    let vp = MTLViewport {
        originX: stage_width() / 2.0 * scale + l,
        originY: stage_height() / 2.0 * scale - t,
        width: r - l,
        height: t - b,
        znear: 0.0,
        zfar: 1.0,
    };
    // twgl.m4.ortho(left, right, bottom = top, top = bottom)
    let (ol, or, ob, ot) = (l / scale, r / scale, t / scale, b / scale);
    let p0 = (2.0 / (or - ol)) as f32;
    let p5 = (2.0 / (ot - ob)) as f32;
    let p12 = ((or + ol) / (ol - or)) as f32;
    let p13 = ((ot + ob) / (ob - ot)) as f32;
    Some((tw_clip(&d.xf, p0, p5, p12, p13, true), vp))
}

/// Clip positions of scratch-render's quad: `P * M * vec4(a_position, 0, 1)`
/// in float32, with P an orthographic projection (p0, p5, p12, p13) and M the
/// drawable's float32 model matrix. `flip` negates y: ANGLE stores a GL
/// framebuffer's row 0 as the Metal texture's row 0, so Metal's y-up clip
/// space is the GL one upside down.
fn tw_clip(xf: &crate::Xf, p0: f32, p5: f32, p12: f32, p13: f32, flip: bool) -> [[f32; 4]; 6] {
    let (m0, m1, m4, m5, m12, m13) = (xf.m00 as f32, xf.m10 as f32, xf.m01 as f32, xf.m11 as f32, xf.tx as f32, xf.ty as f32);
    // P * M (column-major, as GLSL multiplies mat4s); the zero terms are exact.
    let (pm0, pm1, pm4, pm5) = (p0 * m0, p5 * m1, p0 * m4, p5 * m5);
    let (pm12, pm13) = (p0 * m12 + p12, p5 * m13 + p13);
    // a_position, a_texCoord (RenderWebGL._createGeometry)
    const QUAD: [[f32; 4]; 6] = [[-0.5, -0.5, 1.0, 0.0], [0.5, -0.5, 0.0, 0.0], [-0.5, 0.5, 1.0, 1.0], [-0.5, 0.5, 1.0, 1.0], [0.5, -0.5, 0.0, 0.0], [0.5, 0.5, 0.0, 1.0]];
    let sign = if flip { -1.0 } else { 1.0 };
    QUAD.map(|[ax, ay, u, v]| [pm0 * ax + pm4 * ay + pm12, sign * (pm1 * ax + pm5 * ay + pm13), u, v])
}

fn sampler(device: &ProtocolObject<dyn MTLDevice>, filter: MTLSamplerMinMagFilter) -> Option<Sampler> {
    let desc = MTLSamplerDescriptor::new();
    desc.setMinFilter(filter);
    desc.setMagFilter(filter);
    // scratch-render creates skin textures with CLAMP_TO_EDGE: a pixel centre
    // exactly on the quad's edge may sample just outside [0, 1].
    desc.setSAddressMode(MTLSamplerAddressMode::ClampToEdge);
    desc.setTAddressMode(MTLSamplerAddressMode::ClampToEdge);
    device.newSamplerStateWithDescriptor(&desc)
}

fn texture(device: &ProtocolObject<dyn MTLDevice>, w: usize, h: usize, target: bool) -> Option<Tex> {
    texture_in(device, w, h, target, MTLStorageMode::Managed)
}

fn texture_in(device: &ProtocolObject<dyn MTLDevice>, w: usize, h: usize, target: bool, storage: MTLStorageMode) -> Option<Tex> {
    // SAFETY: plain descriptor construction.
    let desc = unsafe { MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(MTLPixelFormat::RGBA8Unorm, w.max(1), h.max(1), false) };
    if target {
        desc.setUsage(MTLTextureUsage::ShaderRead | MTLTextureUsage::RenderTarget);
        desc.setStorageMode(MTLStorageMode::Private);
    } else {
        desc.setUsage(MTLTextureUsage::ShaderRead);
        desc.setStorageMode(storage);
    }
    device.newTextureWithDescriptor(&desc)
}

fn upload(tex: &ProtocolObject<dyn MTLTexture>, r: &Raster) {
    if r.w == 0 || r.h == 0 || r.data.len() < r.w * r.h * 4 {
        return;
    }
    let region = MTLRegion { origin: MTLOrigin { x: 0, y: 0, z: 0 }, size: MTLSize { width: r.w, height: r.h, depth: 1 } };
    // SAFETY: the texture is `r.w` x `r.h` RGBA8 and `r.data` holds that many pixels.
    unsafe { tex.replaceRegion_mipmapLevel_withBytes_bytesPerRow(region, 0, NonNull::new_unchecked(r.data.as_ptr() as *mut c_void), r.w * 4) };
}

fn pass(texture: &ProtocolObject<dyn MTLTexture>, clear: Option<[f64; 4]>) -> Retained<MTLRenderPassDescriptor> {
    let desc = MTLRenderPassDescriptor::renderPassDescriptor();
    // SAFETY: index 0 always exists.
    let att = unsafe { desc.colorAttachments().objectAtIndexedSubscript(0) };
    att.setTexture(Some(texture));
    match clear {
        Some(c) => {
            att.setLoadAction(MTLLoadAction::Clear);
            att.setClearColor(MTLClearColor { red: c[0], green: c[1], blue: c[2], alpha: c[3] });
        }
        None => att.setLoadAction(MTLLoadAction::Load),
    }
    att.setStoreAction(MTLStoreAction::Store);
    desc
}

impl Gpu {
    /// A compositor for a `width` x `height` (device pixels) output, or None
    /// when there is no Metal device or `SCRUST_RENDERER=cpu` is set.
    pub fn new(width: usize, height: usize) -> Option<Gpu> {
        if std::env::var("SCRUST_RENDERER").is_ok_and(|v| v.eq_ignore_ascii_case("cpu")) {
            return None;
        }
        let device = MTLCreateSystemDefaultDevice()?;
        let queue = device.newCommandQueue()?;
        let lib = match device.newLibraryWithSource_options_error(&NSString::from_str(SHADERS), None) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("scrust: Metal shaders failed to compile, drawing on the CPU: {e}");
                return None;
            }
        };
        let (w, h) = (width.max(1), height.max(1));
        let mut gpu = Gpu {
            sprite: pipeline(&device, &lib, "tw_vs", "sprite_fs")?,
            rect: pipeline(&device, &lib, "rect_vs", "rect_fs")?,
            line: pipeline(&device, &lib, "line_vs", "line_fs")?,
            linear: sampler(&device, MTLSamplerMinMagFilter::Linear)?,
            nearest: sampler(&device, MTLSamplerMinMagFilter::Nearest)?,
            target: texture(&device, w, h, true)?,
            pen: texture(&device, w, h, true)?,
            readback: device.newBufferWithLength_options(w * h * 4, MTLResourceOptions::StorageModeShared)?,
            width: w,
            height: h,
            skins: HashMap::new(),
            overlays: HashMap::new(),
            frame_no: 0,
            background: [255, 255, 255],
            items: Vec::new(),
            pen_ops: vec![PenOp::Clear],
            pen_copy: None,
            storage: if device.hasUnifiedMemory() { MTLStorageMode::Shared } else { MTLStorageMode::Managed },
            layer: None,
            blit: pipeline_with(&device, &lib, "rect_vs", "rect_fs", MTLPixelFormat::BGRA8Unorm, false),
            in_flight: None,
            in_flight_buffers: Vec::new(),
            device,
            queue,
        };
        gpu.pen_copy = Some(Raster::new(w, h));
        Some(gpu)
    }

    /// Change the output size (the pen layer is cleared).
    pub fn resize(&mut self, width: usize, height: usize) -> bool {
        let (w, h) = (width.max(1), height.max(1));
        if (w, h) == (self.width, self.height) {
            return true;
        }
        self.settle();
        let (Some(target), Some(pen), Some(readback)) = (
            texture(&self.device, w, h, true),
            texture(&self.device, w, h, true),
            self.device.newBufferWithLength_options(w * h * 4, MTLResourceOptions::StorageModeShared),
        ) else {
            return false;
        };
        (self.target, self.pen, self.readback, self.width, self.height) = (target, pen, readback, w, h);
        self.pen_ops = vec![PenOp::Clear];
        self.pen_copy = Some(Raster::new(w, h));
        true
    }

    fn skin_texture(&mut self, skin: &mut Skin, pixels_per_unit: f64) -> Option<(Tex, usize, usize)> {
        let frame = self.frame_no;
        let uid = skin.uid;
        let raster = skin.raster_for(pixels_per_unit);
        let key = (uid, raster.w, raster.h);
        if let Some((tex, used)) = self.skins.get_mut(&key) {
            *used = frame;
            return Some((tex.clone(), raster.w, raster.h));
        }
        let tex = texture_in(&self.device, raster.w, raster.h, false, self.storage)?;
        upload(&tex, raster);
        self.skins.insert(key, (tex.clone(), frame));
        Some((tex, raster.w, raster.h))
    }

    /// Uniforms and sampling for drawing `d` at `scale` device pixels per stage unit.
    fn quad(&mut self, skin: &mut Skin, d: &Draw, scale: f64) -> Option<Quad> {
        if d.degenerate() {
            return None;
        }
        // Same raster choice as `composite`: the MIP level for the drawable's
        // on-screen density, sampled as `Skin.useNearest` decides.
        let unit = (d.xf.m00.hypot(d.xf.m10) / d.size.0.max(1e-9)).max(d.xf.m01.hypot(d.xf.m11) / d.size.1.max(1e-9));
        let (tex, tw, th) = self.skin_texture(skin, unit * scale)?;
        let fx = d.fx;
        let u = SpriteUniforms {
            size: [d.size.0 as f32, d.size.1 as f32, fx.distorts() as u8 as f32, fx.recolors() as u8 as f32],
            fxa: [fx.color as f32, fx.fisheye as f32, fx.whirl as f32, fx.pixelate as f32],
            fxb: [fx.mosaic as f32, fx.brightness as f32, fx.ghost as f32, 0.0],
        };
        let _ = (tw, th);
        // Drawn to the stage: `ortho(-w/2, w/2, -h/2, h/2)` over the whole target.
        let (hw, hh) = (stage_width() / 2.0, stage_height() / 2.0);
        // The stage target is stored bottom up like WebGL's drawing buffer (see `finish`).
        let verts = tw_clip(&d.xf, (2.0 / (2.0 * hw)) as f32, (2.0 / (2.0 * hh)) as f32, 0.0, 0.0, true);
        Some(Quad { tex, u, nearest: d.draw_nearest(scale), verts, viewport: self.full_viewport() })
    }

    // ------------------------------------------------------------- frame

    /// Upload a skin's texture ahead of its first use.
    pub fn preload(&mut self, skin: &mut Skin) {
        let _ = self.skin_texture(skin, 1.0);
    }

    /// Wait for the previously presented frame (see `in_flight`).
    fn settle(&mut self) {
        if let Some(cmd) = self.in_flight.take() {
            cmd.waitUntilCompleted();
        }
        self.in_flight_buffers.clear();
    }

    /// Start a frame over an opaque background colour.
    pub fn begin(&mut self, background: [u8; 3]) {
        // Overlay textures of the last frame are rewritten below.
        self.settle();
        self.frame_no += 1;
        self.background = background;
        self.items.clear();
    }

    pub fn sprite(&mut self, skin: &mut Skin, d: &Draw, scale: f64) {
        if let Some(q) = self.quad(skin, d, scale) {
            self.items.push(Item::Sprite(q));
        }
    }

    /// Composite the pen layer at this point of the frame.
    pub fn pen_layer(&mut self) {
        self.items.push(Item::Rect(self.pen.clone(), [-1.0, -1.0, 1.0, 1.0]));
    }

    /// A screen-aligned raster at device pixel `(x, y)`; `slot` identifies
    /// the overlay so its texture can be reused.
    pub fn overlay(&mut self, slot: u64, r: &Raster, x: i64, y: i64) {
        if r.w == 0 || r.h == 0 {
            return;
        }
        let reuse = self.overlays.get(&slot).filter(|t| t.width() == r.w && t.height() == r.h).cloned();
        let Some(tex) = reuse.or_else(|| texture(&self.device, r.w, r.h, false)) else { return };
        upload(&tex, r);
        self.overlays.insert(slot, tex.clone());
        let (w, h) = (self.width as f32, self.height as f32);
        let ndc_x = |px: f32| px / w * 2.0 - 1.0;
        // The target is bottom up: device row 0 is NDC y = -1.
        let ndc_y = |py: f32| py / h * 2.0 - 1.0;
        let rect = [ndc_x(x as f32), ndc_y(y as f32), ndc_x((x + r.w as i64) as f32), ndc_y((y + r.h as i64) as f32)];
        self.items.push(Item::Rect(tex, rect));
    }

    /// Render the frame and read it back as opaque RGBA into `out`
    /// (`width * height * 4` bytes).
    pub fn finish(&mut self, out: &mut [u8]) {
        self.settle();
        let Some(cmd) = self.queue.commandBuffer() else { return };
        let _keep = self.encode_frame(&cmd);
        self.read_into(&cmd, &self.target, out);
        // The target holds the stage bottom up (WebGL's drawing buffer as
        // ANGLE stores it, which decides edge rules and texel choice).
        let row = self.width * 4;
        let n = (out.len() / row).min(self.height);
        for y in 0..n / 2 {
            let (top, bottom) = out.split_at_mut((n - 1 - y) * row);
            top[y * row..(y + 1) * row].swap_with_slice(&mut bottom[..row]);
        }
    }

    /// Present the frame in the attached layer (`attach_view`) without
    /// reading it back. The GPU finishes it while the CPU runs the next step.
    pub fn present(&mut self) {
        let (Some(layer), Some(blit)) = (self.layer.clone(), self.blit.clone()) else { return };
        self.settle();
        let Some(cmd) = self.queue.commandBuffer() else { return };
        self.in_flight_buffers = self.encode_frame(&cmd);
        if let Some(drawable) = layer.nextDrawable() {
            let desc = pass(&drawable.texture(), Some([1.0; 4]));
            if let Some(enc) = cmd.renderCommandEncoderWithDescriptor(&desc) {
                enc.setRenderPipelineState(&blit);
                // The target is bottom up.
                let rect = [-1.0f32, -1.0, 1.0, 1.0];
                // SAFETY: `rect` is the shader's `Rect` (one float4).
                unsafe {
                    enc.setVertexBytes_length_atIndex(NonNull::from(&rect).cast::<c_void>(), 16, 0);
                    enc.setFragmentTexture_atIndex(Some(&self.target), 0);
                    enc.setFragmentSamplerState_atIndex(Some(&self.linear), 0);
                    enc.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::TriangleStrip, 0, 4);
                }
                enc.endEncoding();
            }
            cmd.presentDrawable(ProtocolObject::from_ref(&*drawable));
        }
        cmd.commit();
        self.in_flight = Some(cmd);
    }

    /// Present frames in a CAMetalLayer installed as the backing layer of
    /// `ns_view` (an `NSView*`), with a drawable of `width` x `height` pixels.
    ///
    /// # Safety
    /// `ns_view` must be a live `NSView` and this must run on the main thread.
    pub unsafe fn attach_view(&mut self, ns_view: *mut c_void, width: usize, height: usize) -> bool {
        let Some(view) = (ns_view as *mut objc2::runtime::AnyObject).as_ref() else { return false };
        if self.blit.is_none() {
            return false;
        }
        let layer = CAMetalLayer::new();
        layer.setDevice(Some(&self.device));
        layer.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
        layer.setFramebufferOnly(true);
        layer.setDrawableSize(CGSize { width: width as f64, height: height as f64 });
        let _: () = objc2::msg_send![view, setWantsLayer: true];
        let _: () = objc2::msg_send![view, setLayer: &*layer];
        let scale: f64 = match objc2::msg_send![view, window] {
            Some(window) => {
                let window: &objc2::runtime::AnyObject = window;
                objc2::msg_send![window, backingScaleFactor]
            }
            None => 1.0,
        };
        layer.setContentsScale(scale);
        self.layer = Some(layer);
        true
    }

    /// Change the size of the attached layer's drawables (window resize).
    pub fn set_drawable_size(&mut self, width: usize, height: usize) {
        if let Some(layer) = &self.layer {
            layer.setDrawableSize(CGSize { width: width.max(1) as f64, height: height.max(1) as f64 });
        }
    }

    /// Pen operations and the frame's items into `cmd`, rendering into
    /// `target`; returns the buffers that must live until `cmd` has run.
    fn encode_frame(&mut self, cmd: &ProtocolObject<dyn MTLCommandBuffer>) -> Vec<Buf> {
        let mut keep: Vec<Buf> = Vec::new();
        self.encode_pen(cmd, &mut keep);
        let bg = self.background;
        let desc = pass(&self.target, Some([bg[0] as f64 / 255.0, bg[1] as f64 / 255.0, bg[2] as f64 / 255.0, 1.0]));
        if let Some(enc) = cmd.renderCommandEncoderWithDescriptor(&desc) {
            for item in &self.items {
                match item {
                    Item::Sprite(q) => self.draw_quad(&enc, q),
                    Item::Rect(tex, rect) => self.draw_rect(&enc, tex, rect),
                }
            }
            enc.endEncoding();
        }
        self.items.clear();
        // Forget skin textures that have not been drawn for a while.
        let frame = self.frame_no;
        self.skins.retain(|_, (_, used)| frame - *used < 300);
        keep
    }

    fn draw_quad(&self, enc: &ProtocolObject<dyn MTLRenderCommandEncoder>, q: &Quad) {
        enc.setRenderPipelineState(&self.sprite);
        let u = NonNull::from(&q.u).cast::<c_void>();
        let len = std::mem::size_of::<SpriteUniforms>();
        // SAFETY: `u` points at `len` bytes of uniforms laid out as the shader's `Sprite`;
        // the vertices are six float4s.
        unsafe {
            enc.setVertexBytes_length_atIndex(u, len, 0);
            enc.setFragmentBytes_length_atIndex(u, len, 0);
            enc.setFragmentTexture_atIndex(Some(&q.tex), 0);
            enc.setFragmentSamplerState_atIndex(Some(if q.nearest { &self.nearest } else { &self.linear }), 0);
            enc.setVertexBytes_length_atIndex(NonNull::from(&q.verts).cast::<c_void>(), std::mem::size_of_val(&q.verts), 1);
            enc.setViewport(q.viewport);
            enc.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::Triangle, 0, 6);
            enc.setViewport(self.full_viewport());
        }
    }

    fn full_viewport(&self) -> MTLViewport {
        MTLViewport { originX: 0.0, originY: 0.0, width: self.width as f64, height: self.height as f64, znear: 0.0, zfar: 1.0 }
    }

    fn draw_rect(&self, enc: &ProtocolObject<dyn MTLRenderCommandEncoder>, tex: &Tex, rect: &[f32; 4]) {
        enc.setRenderPipelineState(&self.rect);
        // SAFETY: `rect` is the shader's `Rect` (one float4).
        unsafe {
            enc.setVertexBytes_length_atIndex(NonNull::from(rect).cast::<c_void>(), 16, 0);
            enc.setFragmentTexture_atIndex(Some(tex), 0);
            enc.setFragmentSamplerState_atIndex(Some(&self.nearest), 0);
            enc.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::TriangleStrip, 0, 4);
        }
    }

    /// Copy `tex` into `out` through the readback buffer (waits for the GPU).
    fn read_into(&self, cmd: &ProtocolObject<dyn MTLCommandBuffer>, tex: &Tex, out: &mut [u8]) {
        let (w, h) = (self.width, self.height);
        if let Some(blit) = cmd.blitCommandEncoder() {
            // SAFETY: the buffer holds w * h * 4 bytes, the size of the copied texture.
            unsafe {
                blit.copyFromTexture_sourceSlice_sourceLevel_sourceOrigin_sourceSize_toBuffer_destinationOffset_destinationBytesPerRow_destinationBytesPerImage(
                    tex,
                    0,
                    0,
                    MTLOrigin { x: 0, y: 0, z: 0 },
                    MTLSize { width: w, height: h, depth: 1 },
                    &self.readback,
                    0,
                    w * 4,
                    w * h * 4,
                );
            }
            blit.endEncoding();
        }
        cmd.commit();
        cmd.waitUntilCompleted();
        let n = (w * h * 4).min(out.len());
        // SAFETY: the shared buffer is w * h * 4 bytes and the GPU is done with it.
        let src = unsafe { std::slice::from_raw_parts(self.readback.contents().as_ptr() as *const u8, n) };
        out[..n].copy_from_slice(src);
    }

    // ------------------------------------------------------------- pen

    pub fn pen_clear(&mut self) {
        self.pen_ops.clear();
        self.pen_ops.push(PenOp::Clear);
        self.pen_copy = Some(Raster::new(self.width, self.height));
    }

    /// A line with `PenSkin._drawLineOnBuffer`'s attributes: premultiplied
    /// colour, thickness and length, and the pen points `(x0, -y0, dx, -dy)`
    /// in device pixels from the stage centre.
    pub fn pen_line(&mut self, color: [f32; 4], thickness: f32, length: f32, pts: [f32; 4]) {
        let line = LineInstance { color, tl: [thickness, length, self.width as f32, self.height as f32], pts };
        match self.pen_ops.last_mut() {
            Some(PenOp::Lines(lines)) => lines.push(line),
            _ => self.pen_ops.push(PenOp::Lines(vec![line])),
        }
        self.pen_copy = None;
    }

    pub fn pen_stamp(&mut self, skin: &mut Skin, d: &Draw, scale: f64, bounds: (f64, f64, f64, f64)) {
        if let Some(mut q) = self.quad(skin, d, scale) {
            let Some((verts, viewport)) = tw_stamp_vertices(d, scale, bounds) else { return };
            (q.verts, q.viewport) = (verts, viewport);
            self.pen_ops.push(PenOp::Stamp(q));
            self.pen_copy = None;
        }
    }

    /// Encode pending pen operations into `cmd`; instance buffers go to `keep`
    /// (they must live until the command buffer has run).
    ///
    /// All of them go into one render pass: a pass loads and stores the whole
    /// pen texture, so a pass per stamp costs a full-screen copy per stamp
    /// (projects that stamp thousands of times per frame were GPU-bound on
    /// that alone). A clear can only be the first operation (`pen_clear`
    /// drops everything before it) and becomes the pass's load action.
    fn encode_pen(&mut self, cmd: &ProtocolObject<dyn MTLCommandBuffer>, keep: &mut Vec<Buf>) {
        let ops = std::mem::take(&mut self.pen_ops);
        if ops.is_empty() {
            return;
        }
        let clear = matches!(ops.first(), Some(PenOp::Clear));
        let desc = pass(&self.pen, clear.then_some([0.0; 4]));
        let Some(enc) = cmd.renderCommandEncoderWithDescriptor(&desc) else { return };
        for op in &ops {
            match op {
                PenOp::Clear => {}
                PenOp::Lines(lines) => {
                    let bytes = std::mem::size_of_val(lines.as_slice());
                    // SAFETY: `lines` is `bytes` long; Metal copies it into the new buffer.
                    let buf = unsafe {
                        self.device.newBufferWithBytes_length_options(NonNull::new_unchecked(lines.as_ptr() as *mut c_void), bytes, MTLResourceOptions::StorageModeShared)
                    };
                    if let Some(buf) = buf {
                        enc.setRenderPipelineState(&self.line);
                        // SAFETY: the buffer holds `lines.len()` `Line` structs.
                        unsafe {
                            enc.setVertexBuffer_offset_atIndex(Some(&buf), 0, 0);
                            enc.drawPrimitives_vertexStart_vertexCount_instanceCount(MTLPrimitiveType::TriangleStrip, 0, 4, lines.len());
                        }
                        keep.push(buf);
                    }
                }
                PenOp::Stamp(q) => self.draw_quad(&enc, q),
            }
        }
        enc.endEncoding();
    }

    /// The pen layer as premultiplied RGBA (for `touching color`); reads it
    /// back from the GPU when it changed.
    pub fn pen_pixels(&mut self) -> &Raster {
        if self.pen_copy.is_none() {
            let mut r = Raster::new(self.width, self.height);
            if let Some(cmd) = self.queue.commandBuffer() {
                let mut keep = Vec::new();
                self.encode_pen(&cmd, &mut keep);
                let pen = self.pen.clone();
                self.read_into(&cmd, &pen, &mut r.data);
            }
            self.pen_copy = Some(r);
        }
        self.pen_copy.as_ref().expect("pen copy")
    }
}
