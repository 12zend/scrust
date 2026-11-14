//! `scrust_render::Scene` as a WebAssembly module, for the browser build of
//! the GUI (`gui/web`). It is the counterpart of the Node-API addon
//! `scrust-node`: `gui/web/scene.js` wraps these exports in the same
//! JavaScript interface, so `gui/packages/scrust-render` runs unchanged.
//!
//! Plain exports, no wasm-bindgen (as for the generated programs, see
//! `docs/decisions/web-host.md`). Arguments that are not numbers (SVG text,
//! RGBA pixels, effect names, id lists) are written by the page into the
//! input buffer returned by `scene_input`; results that are several numbers
//! are read from the eight `f64`s at `scene_out`. Ids are plain numbers, 0
//! meaning "none".

use scrust_render::Scene;
use std::cell::RefCell;

thread_local! {
    static SCENE: RefCell<Option<Scene>> = const { RefCell::new(None) };
    /// Bytes passed in by the page (see `scene_input`).
    static INPUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    /// Numbers passed back to the page (see `scene_out`).
    static OUT: RefCell<[f64; 8]> = const { RefCell::new([0.0; 8]) };
    /// Pixels of the last `scene_extract`.
    static EXTRACTED: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

fn with<R>(f: impl FnOnce(&mut Scene) -> R) -> R {
    SCENE.with(|s| f(s.borrow_mut().get_or_insert_with(Scene::new)))
}

/// The first `len` bytes of the input buffer.
fn input<R>(len: u32, f: impl FnOnce(&[u8]) -> R) -> R {
    INPUT.with(|b| {
        let b = b.borrow();
        f(&b[..(len as usize).min(b.len())])
    })
}

fn input_ids(count: u32) -> Vec<u32> {
    input(count.saturating_mul(4), |b| b.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
}

fn out(values: &[f64]) {
    OUT.with(|o| {
        let mut o = o.borrow_mut();
        for (slot, v) in o.iter_mut().zip(values) {
            *slot = *v;
        }
    });
}

fn opt_id(id: u32) -> Option<u32> {
    (id != 0).then_some(id)
}

fn center(has: u32, x: f64, y: f64) -> Option<(f64, f64)> {
    (has != 0).then_some((x, y))
}

/// Make the input buffer at least `len` bytes long and return it; the page
/// writes an argument there right before the call that reads it.
#[no_mangle]
pub extern "C" fn scene_input(len: u32) -> *mut u8 {
    INPUT.with(|b| {
        let mut b = b.borrow_mut();
        if b.len() < len as usize {
            b.resize(len as usize, 0);
        }
        b.as_mut_ptr()
    })
}

/// Where multi-number results are left (eight `f64`s).
#[no_mangle]
pub extern "C" fn scene_out() -> *const f64 {
    OUT.with(|o| o.as_ptr().cast())
}

#[no_mangle]
pub extern "C" fn scene_resize(width: u32, height: u32) {
    with(|s| s.resize(width as usize, height as usize));
}

#[no_mangle]
pub extern "C" fn scene_background(r: f64, g: f64, b: f64) {
    let c = |v: f64| v.clamp(0.0, 255.0) as u8;
    with(|s| s.background = [c(r), c(g), c(b)]);
}

/// SVG skin from `len` bytes of input; id 0 creates a skin, otherwise it is replaced.
#[no_mangle]
pub extern "C" fn scene_svg_skin(id: u32, len: u32, has_center: u32, cx: f64, cy: f64) -> u32 {
    input(len, |svg| with(|s| s.svg_skin(opt_id(id), svg, center(has_center, cx, cy))))
}

/// Bitmap skin from `width * height * 4` bytes of straight RGBA input.
#[no_mangle]
pub extern "C" fn scene_bitmap_skin(id: u32, width: u32, height: u32, resolution: f64, has_center: u32, cx: f64, cy: f64) -> u32 {
    let len = width.saturating_mul(height).saturating_mul(4);
    input(len, |rgba| with(|s| s.bitmap_skin(opt_id(id), width as usize, height as usize, rgba, resolution, center(has_center, cx, cy))))
}

#[no_mangle]
pub extern "C" fn scene_empty_skin() -> u32 {
    with(|s| s.empty_skin())
}

#[no_mangle]
pub extern "C" fn scene_destroy_skin(id: u32) {
    with(|s| s.destroy_skin(id));
}

/// 1 and `[width, height, centerX, centerY]` in `scene_out`, or 0 for no such skin.
#[no_mangle]
pub extern "C" fn scene_skin_info(id: u32) -> u32 {
    match with(|s| s.skin(id).map(|k| [k.size.0, k.size.1, k.center.0, k.center.1])) {
        Some(v) => {
            out(&v);
            1
        }
        None => 0,
    }
}

#[no_mangle]
pub extern "C" fn scene_create_drawable() -> u32 {
    with(|s| s.create_drawable())
}

#[no_mangle]
pub extern "C" fn scene_destroy_drawable(id: u32) {
    with(|s| s.destroy_drawable(id));
}

#[no_mangle]
pub extern "C" fn scene_drawable_skin(id: u32, skin: u32) {
    with(|s| {
        if let Some(d) = s.drawable_mut(id) {
            d.skin = opt_id(skin);
        }
    });
}

#[no_mangle]
pub extern "C" fn scene_drawable_position(id: u32, x: f64, y: f64) {
    with(|s| {
        if let Some(d) = s.drawable_mut(id) {
            d.position = (x, y);
        }
    });
}

#[no_mangle]
pub extern "C" fn scene_drawable_direction(id: u32, direction: f64) {
    with(|s| {
        if let Some(d) = s.drawable_mut(id) {
            d.direction = direction;
        }
    });
}

#[no_mangle]
pub extern "C" fn scene_drawable_scale(id: u32, x: f64, y: f64) {
    with(|s| {
        if let Some(d) = s.drawable_mut(id) {
            d.scale = (x, y);
        }
    });
}

#[no_mangle]
pub extern "C" fn scene_drawable_visible(id: u32, visible: u32) {
    with(|s| {
        if let Some(d) = s.drawable_mut(id) {
            d.visible = visible != 0;
        }
    });
}

/// Graphic effect; its scratch-render name is `len` bytes of input.
#[no_mangle]
pub extern "C" fn scene_drawable_effect(id: u32, len: u32, value: f64) {
    let name = input(len, |b| String::from_utf8_lossy(b).into_owned());
    with(|s| s.set_effect(id, &name, value));
}

/// 1 and `[left, right, top, bottom]` in `scene_out` (tight or quad bounds), or 0.
#[no_mangle]
pub extern "C" fn scene_bounds(id: u32, tight: u32) -> u32 {
    match with(|s| if tight != 0 { s.bounds(id) } else { s.aabb(id) }) {
        Some(b) => {
            out(&[b.left, b.right, b.top, b.bottom]);
            1
        }
        None => 0,
    }
}

/// `[x, y]` in `scene_out`: the position fenced to the stage.
#[no_mangle]
pub extern "C" fn scene_fenced(id: u32, x: f64, y: f64) {
    let (x, y) = with(|s| s.fenced_position(id, x, y));
    out(&[x, y]);
}

/// Frontmost of `count` candidate ids (u32 LE input, back to front) at a stage point, or 0.
#[no_mangle]
pub extern "C" fn scene_pick(count: u32, x: f64, y: f64) -> u32 {
    let ids = input_ids(count);
    with(|s| s.pick(&ids, x, y)).unwrap_or(0)
}

/// Draw `count` drawables (u32 LE input, back to front); returns the RGBA
/// pixels (`scene_rgba_len` bytes), valid until the next call.
#[no_mangle]
pub extern "C" fn scene_draw(count: u32) -> *const u8 {
    let ids = input_ids(count);
    with(|s| {
        s.draw(&ids);
        s.rgba.as_ptr()
    })
}

#[no_mangle]
pub extern "C" fn scene_rgba_len() -> u32 {
    with(|s| s.rgba.len() as u32)
}

/// One drawable alone: 1 and `[x, y, width, height]` (device pixels) in
/// `scene_out`, its pixels at `scene_extracted` (`width * height * 4`
/// bytes), or 0 for no such drawable.
#[no_mangle]
pub extern "C" fn scene_extract(id: u32) -> u32 {
    match with(|s| s.extract(id)) {
        Some(((x, y, w, h), data)) => {
            out(&[x as f64, y as f64, w as f64, h as f64]);
            EXTRACTED.with(|e| *e.borrow_mut() = data);
            1
        }
        None => 0,
    }
}

#[no_mangle]
pub extern "C" fn scene_extracted() -> *const u8 {
    EXTRACTED.with(|e| e.borrow().as_ptr())
}
