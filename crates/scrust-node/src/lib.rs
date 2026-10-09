//! Node-API addon over `scrust_render::Scene`, loaded by the GUI's Electron
//! preload (`gui/electron/preload.js`) and used by `gui/packages/scrust-render`
//! in place of scratch-render's WebGL renderer. The calls are synchronous, as
//! scratch-vm expects (it measures a skin right after creating it).
//!
//! The few Node-API functions needed are declared by hand; they are resolved
//! from the host process when the addon is loaded. Ids are plain numbers, 0
//! meaning "none".

use scrust_render::Scene;
use std::cell::RefCell;
use std::ffi::{c_char, c_void};
use std::ptr;

#[allow(non_camel_case_types)]
mod ffi {
    use std::ffi::{c_char, c_void};

    pub type napi_env = *mut c_void;
    pub type napi_value = *mut c_void;
    pub type napi_callback_info = *mut c_void;
    pub type napi_status = i32;
    pub type napi_callback = unsafe extern "C" fn(napi_env, napi_callback_info) -> napi_value;

    pub const OK: napi_status = 0;

    extern "C" {
        pub fn napi_create_function(env: napi_env, name: *const c_char, len: usize, cb: napi_callback, data: *mut c_void, result: *mut napi_value) -> napi_status;
        pub fn napi_set_named_property(env: napi_env, object: napi_value, name: *const c_char, value: napi_value) -> napi_status;
        pub fn napi_get_cb_info(env: napi_env, info: napi_callback_info, argc: *mut usize, argv: *mut napi_value, this: *mut napi_value, data: *mut *mut c_void) -> napi_status;
        pub fn napi_get_value_double(env: napi_env, value: napi_value, result: *mut f64) -> napi_status;
        pub fn napi_get_value_bool(env: napi_env, value: napi_value, result: *mut bool) -> napi_status;
        pub fn napi_get_value_string_utf8(env: napi_env, value: napi_value, buf: *mut c_char, size: usize, result: *mut usize) -> napi_status;
        pub fn napi_get_typedarray_info(
            env: napi_env,
            value: napi_value,
            kind: *mut i32,
            length: *mut usize,
            data: *mut *mut c_void,
            arraybuffer: *mut napi_value,
            byte_offset: *mut usize,
        ) -> napi_status;
        pub fn napi_create_double(env: napi_env, value: f64, result: *mut napi_value) -> napi_status;
        pub fn napi_create_buffer(env: napi_env, length: usize, data: *mut *mut c_void, result: *mut napi_value) -> napi_status;
        pub fn napi_create_buffer_copy(env: napi_env, length: usize, data: *const c_void, result_data: *mut *mut c_void, result: *mut napi_value) -> napi_status;
        pub fn napi_create_array_with_length(env: napi_env, length: usize, result: *mut napi_value) -> napi_status;
        pub fn napi_set_element(env: napi_env, object: napi_value, index: u32, value: napi_value) -> napi_status;
        pub fn napi_get_null(env: napi_env, result: *mut napi_value) -> napi_status;
        pub fn napi_get_undefined(env: napi_env, result: *mut napi_value) -> napi_status;
    }
}

use ffi::{napi_callback_info, napi_env, napi_value};

thread_local! {
    static SCENE: RefCell<Option<Scene>> = const { RefCell::new(None) };
}

// The shared frame file of the running program (`frame`), by path.
#[cfg(unix)]
thread_local! {
    static FRAMES: RefCell<Option<(String, scrust_render::frames::FrameReader)>> = const { RefCell::new(None) };
}

/// `frame(path, seen)`: the program's latest frame if newer than frame number `seen`.
#[cfg(unix)]
fn read_frame(a: &Args) -> napi_value {
    let path = a.string(0);
    let seen = a.f64(1);
    let seen = if seen.is_finite() && seen > 0.0 { seen as u64 } else { 0 };
    FRAMES.with(|cell| {
        let mut cell = cell.borrow_mut();
        if cell.as_ref().is_none_or(|(p, _)| *p != path) {
            // The program creates the file when it starts; until then there is nothing to show.
            *cell = scrust_render::frames::FrameReader::open(std::path::Path::new(&path)).map(|r| (path.clone(), r));
        }
        let Some((_, reader)) = cell.as_ref() else { return a.null() };
        let mut buffer = ptr::null_mut();
        let alloc = |len: usize| {
            let mut data = ptr::null_mut();
            // SAFETY: Node allocates a Buffer of `len` bytes and returns its storage.
            let ok = unsafe { ffi::napi_create_buffer(a.env, len, &mut data, &mut buffer) } == ffi::OK;
            (ok && !data.is_null()).then_some(data as *mut u8)
        };
        match reader.read(seen, alloc) {
            Some((n, w, h)) => {
                let items = [a.num(n as f64), a.num(w as f64), a.num(h as f64), buffer];
                a.array(&items)
            }
            None => a.null(),
        }
    })
}

#[cfg(not(unix))]
fn read_frame(a: &Args) -> napi_value {
    a.null()
}

fn with<R>(f: impl FnOnce(&mut Scene) -> R) -> R {
    SCENE.with(|s| f(s.borrow_mut().get_or_insert_with(Scene::new)))
}

/// Arguments of one call.
struct Args {
    env: napi_env,
    argv: [napi_value; 8],
    argc: usize,
}

impl Args {
    unsafe fn new(env: napi_env, info: napi_callback_info) -> Args {
        let mut argv = [ptr::null_mut(); 8];
        let mut argc = argv.len();
        ffi::napi_get_cb_info(env, info, &mut argc, argv.as_mut_ptr(), ptr::null_mut(), ptr::null_mut());
        Args { env, argv, argc: argc.min(8) }
    }

    fn get(&self, i: usize) -> Option<napi_value> {
        (i < self.argc).then(|| self.argv[i])
    }

    fn f64(&self, i: usize) -> f64 {
        let mut out = f64::NAN;
        if let Some(v) = self.get(i) {
            // SAFETY: `v` is a live handle of this call; a non-number leaves NaN.
            unsafe { ffi::napi_get_value_double(self.env, v, &mut out) };
        }
        out
    }

    fn u32(&self, i: usize) -> u32 {
        let v = self.f64(i);
        if v.is_finite() && v > 0.0 {
            v as u32
        } else {
            0
        }
    }

    fn bool(&self, i: usize) -> bool {
        let mut out = false;
        if let Some(v) = self.get(i) {
            // SAFETY: as in `f64`.
            unsafe { ffi::napi_get_value_bool(self.env, v, &mut out) };
        }
        out
    }

    fn string(&self, i: usize) -> String {
        let Some(v) = self.get(i) else { return String::new() };
        let mut len = 0usize;
        // SAFETY: the first call only measures, the second fills a buffer of len + 1 bytes.
        unsafe {
            if ffi::napi_get_value_string_utf8(self.env, v, ptr::null_mut(), 0, &mut len) != ffi::OK {
                return String::new();
            }
            let mut buf = vec![0u8; len + 1];
            ffi::napi_get_value_string_utf8(self.env, v, buf.as_mut_ptr() as *mut c_char, buf.len(), &mut len);
            buf.truncate(len);
            String::from_utf8(buf).unwrap_or_default()
        }
    }

    /// Bytes of a typed array argument (any element type).
    fn bytes(&self, i: usize) -> &[u8] {
        let Some(v) = self.get(i) else { return &[] };
        let (mut kind, mut length, mut data, mut offset) = (0i32, 0usize, ptr::null_mut::<c_void>(), 0usize);
        // SAFETY: on success `data` points at `length` elements owned by the
        // typed array, which outlives this call.
        unsafe {
            if ffi::napi_get_typedarray_info(self.env, v, &mut kind, &mut length, &mut data, ptr::null_mut(), &mut offset) != ffi::OK || data.is_null() {
                return &[];
            }
            let size = match kind {
                0..=2 => 1,
                3 | 4 => 2,
                5..=7 => 4,
                _ => 8,
            };
            std::slice::from_raw_parts(data as *const u8, length * size)
        }
    }

    fn ids(&self, i: usize) -> Vec<u32> {
        self.bytes(i).chunks_exact(4).map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]])).collect()
    }

    fn center(&self, has: usize) -> Option<(f64, f64)> {
        self.bool(has).then(|| (self.f64(has + 1), self.f64(has + 2)))
    }

    // ------------------------------------------------------------- results

    fn num(&self, v: f64) -> napi_value {
        let mut out = ptr::null_mut();
        // SAFETY: plain value creation in this call's scope.
        unsafe { ffi::napi_create_double(self.env, v, &mut out) };
        out
    }

    fn undefined(&self) -> napi_value {
        let mut out = ptr::null_mut();
        // SAFETY: as in `num`.
        unsafe { ffi::napi_get_undefined(self.env, &mut out) };
        out
    }

    fn null(&self) -> napi_value {
        let mut out = ptr::null_mut();
        // SAFETY: as in `num`.
        unsafe { ffi::napi_get_null(self.env, &mut out) };
        out
    }

    fn array(&self, items: &[napi_value]) -> napi_value {
        let mut out = ptr::null_mut();
        // SAFETY: as in `num`; the items are handles of this call.
        unsafe {
            ffi::napi_create_array_with_length(self.env, items.len(), &mut out);
            for (i, v) in items.iter().enumerate() {
                ffi::napi_set_element(self.env, out, i as u32, *v);
            }
        }
        out
    }

    fn nums(&self, values: &[f64]) -> napi_value {
        let items: Vec<napi_value> = values.iter().map(|v| self.num(*v)).collect();
        self.array(&items)
    }

    fn buffer(&self, data: &[u8]) -> napi_value {
        let mut out = ptr::null_mut();
        let mut copy = ptr::null_mut();
        // SAFETY: Node copies `data.len()` bytes into a new Buffer.
        unsafe { ffi::napi_create_buffer_copy(self.env, data.len(), data.as_ptr() as *const c_void, &mut copy, &mut out) };
        out
    }
}

macro_rules! export {
    ($($name:ident => |$a:ident| $body:expr;)*) => {
        $(
            #[allow(non_snake_case)]
            unsafe extern "C" fn $name(env: napi_env, info: napi_callback_info) -> napi_value {
                let $a = Args::new(env, info);
                $body
            }
        )*
        const EXPORTS: &[(&str, ffi::napi_callback)] = &[$((stringify!($name), $name)),*];
    };
}

export! {
    resize => |a| { with(|s| s.resize(a.u32(0) as usize, a.u32(1) as usize)); a.undefined() };
    // usesGpu() -> whether the scene composites with Metal
    usesGpu => |a| a.num(with(|s| s.uses_gpu()) as u8 as f64);
    background => |a| {
        let c = |i| a.f64(i).clamp(0.0, 255.0) as u8;
        with(|s| s.background = [c(0), c(1), c(2)]);
        a.undefined()
    };
    // svgSkin(id, svg, hasCenter, cx, cy) -> id; id 0 creates a skin, otherwise it is replaced.
    svgSkin => |a| {
        let svg = a.string(1);
        let id = with(|s| s.svg_skin((a.u32(0) != 0).then(|| a.u32(0)), svg.as_bytes(), a.center(2)));
        a.num(id as f64)
    };
    // bitmapSkin(id, width, height, rgba, resolution, hasCenter, cx, cy) -> id
    bitmapSkin => |a| {
        let id = with(|s| s.bitmap_skin((a.u32(0) != 0).then(|| a.u32(0)), a.u32(1) as usize, a.u32(2) as usize, a.bytes(3), a.f64(4), a.center(5)));
        a.num(id as f64)
    };
    emptySkin => |a| a.num(with(|s| s.empty_skin()) as f64);
    destroySkin => |a| { with(|s| s.destroy_skin(a.u32(0))); a.undefined() };
    // skinInfo(id) -> [width, height, centerX, centerY] | null
    skinInfo => |a| match with(|s| s.skin(a.u32(0)).map(|k| [k.size.0, k.size.1, k.center.0, k.center.1])) {
        Some(v) => a.nums(&v),
        None => a.null(),
    };
    createDrawable => |a| a.num(with(|s| s.create_drawable()) as f64);
    destroyDrawable => |a| { with(|s| s.destroy_drawable(a.u32(0))); a.undefined() };
    drawableSkin => |a| {
        with(|s| if let Some(d) = s.drawable_mut(a.u32(0)) { d.skin = (a.u32(1) != 0).then(|| a.u32(1)) });
        a.undefined()
    };
    drawablePosition => |a| { with(|s| if let Some(d) = s.drawable_mut(a.u32(0)) { d.position = (a.f64(1), a.f64(2)) }); a.undefined() };
    drawableDirection => |a| { with(|s| if let Some(d) = s.drawable_mut(a.u32(0)) { d.direction = a.f64(1) }); a.undefined() };
    drawableScale => |a| { with(|s| if let Some(d) = s.drawable_mut(a.u32(0)) { d.scale = (a.f64(1), a.f64(2)) }); a.undefined() };
    drawableVisible => |a| { with(|s| if let Some(d) = s.drawable_mut(a.u32(0)) { d.visible = a.bool(1) }); a.undefined() };
    drawableEffect => |a| { let name = a.string(1); with(|s| s.set_effect(a.u32(0), &name, a.f64(2))); a.undefined() };
    // bounds(id, tight) -> [left, right, top, bottom] | null
    bounds => |a| {
        let b = with(|s| if a.bool(1) { s.bounds(a.u32(0)) } else { s.aabb(a.u32(0)) });
        match b {
            Some(b) => a.nums(&[b.left, b.right, b.top, b.bottom]),
            None => a.null(),
        }
    };
    // fenced(id, x, y) -> [x, y]
    fenced => |a| { let (x, y) = with(|s| s.fenced_position(a.u32(0), a.f64(1), a.f64(2))); a.nums(&[x, y]) };
    // pick(Uint32Array candidates back to front, x, y) -> id | 0
    pick => |a| a.num(with(|s| s.pick(&a.ids(0), a.f64(1), a.f64(2))).unwrap_or(0) as f64);
    // draw(Uint32Array order back to front) -> Buffer of width * height RGBA
    draw => |a| {
        let ids = a.ids(0);
        with(|s| {
            s.draw(&ids);
            a.buffer(&s.rgba)
        })
    };
    // frame(path, seen) -> [frame number, width, height, Buffer of RGBA] | null:
    // the running program's latest frame from its shared frame file (see
    // scrust_render::frames), when it is newer than frame number `seen`.
    frame => |a| read_frame(&a);
    // extract(id) -> [x, y, width, height, Buffer] (device pixels) | null
    extract => |a| match with(|s| s.extract(a.u32(0))) {
        Some(((x, y, w, h), data)) => {
            let items = [a.num(x as f64), a.num(y as f64), a.num(w as f64), a.num(h as f64), a.buffer(&data)];
            a.array(&items)
        }
        None => a.null(),
    };
}

/// Entry point called by Node-API when the addon is loaded.
///
/// # Safety
/// Called by the host with a valid environment and exports object.
#[no_mangle]
pub unsafe extern "C" fn napi_register_module_v1(env: napi_env, exports: napi_value) -> napi_value {
    for (name, cb) in EXPORTS {
        let cname = std::ffi::CString::new(*name).expect("export name");
        let mut f = ptr::null_mut();
        if ffi::napi_create_function(env, cname.as_ptr(), name.len(), *cb, ptr::null_mut(), &mut f) == ffi::OK {
            ffi::napi_set_named_property(env, exports, cname.as_ptr(), f);
        }
    }
    exports
}
