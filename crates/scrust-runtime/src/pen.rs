//! Pen extension state machine (`scratch3_pen`). Drawing goes through the
//! renderer; without one only the state changes.

use crate::blocks::{to_rgba, wrap_clamp};
use crate::world::*;
use scrust_value::{JsStr, Value};

fn clamp100(v: f64) -> f64 {
    v.max(0.0).min(100.0)
}

/// `Color.hsvToRgb` (channels floored to integers 0..255).
pub fn hsv_to_rgb(h: f64, s: f64, v: f64) -> [f64; 3] {
    let mut h = h % 360.0;
    if h < 0.0 {
        h += 360.0;
    }
    let s = s.min(1.0).max(0.0);
    let v = v.min(1.0).max(0.0);
    let i = (h / 60.0).floor();
    let f = (h / 60.0) - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - (s * f));
    let t = v * (1.0 - (s * (1.0 - f)));
    let (r, g, b) = match i as i32 {
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        5 => (v, p, q),
        _ => (v, t, p),
    };
    [(r * 255.0).floor(), (g * 255.0).floor(), (b * 255.0).floor()]
}

/// `Color.rgbToHsv`.
pub fn rgb_to_hsv(rgb: [f64; 3]) -> (f64, f64, f64) {
    let r = rgb[0] / 255.0;
    let g = rgb[1] / 255.0;
    let b = rgb[2] / 255.0;
    let x = r.min(g).min(b);
    let v = r.max(g).max(b);
    let mut h = 0.0;
    let mut s = 0.0;
    if x != v {
        let f = if r == x { g - b } else if g == x { b - r } else { r - g };
        let i = if r == x { 3.0 } else if g == x { 5.0 } else { 1.0 };
        h = ((i - (f / (v - x))) * 60.0) % 360.0;
        s = (v - x) / v;
    }
    (h, s, v)
}

fn mix_rgb(a: [f64; 3], b: [f64; 3], f1: f64) -> [f64; 3] {
    if f1 <= 0.0 {
        return a;
    }
    if f1 >= 1.0 {
        return b;
    }
    let f0 = 1.0 - f1;
    [f0 * a[0] + f1 * b[0], f0 * a[1] + f1 * b[1], f0 * a[2] + f1 * b[2]]
}

impl PenState {
    fn update_color(&mut self) {
        let rgb = hsv_to_rgb(self.color * 360.0 / 100.0, self.saturation / 100.0, self.brightness / 100.0);
        self.rgba = [(rgb[0] / 255.0) as f32, (rgb[1] / 255.0) as f32, (rgb[2] / 255.0) as f32, (1.0 - self.transparency / 100.0) as f32];
    }

    fn set_or_change(&mut self, param: &JsStr, value: f64, change: bool) {
        if param.eq_str("color") {
            self.color = wrap_clamp(value + if change { self.color } else { 0.0 }, 0.0, 100.0);
        } else if param.eq_str("saturation") {
            self.saturation = clamp100(value + if change { self.saturation } else { 0.0 });
        } else if param.eq_str("brightness") {
            self.brightness = clamp100(value + if change { self.brightness } else { 0.0 });
        } else if param.eq_str("transparency") {
            self.transparency = clamp100(value + if change { self.transparency } else { 0.0 });
        }
        self.update_color();
    }

    fn legacy_update(&mut self) {
        let mut rgb = hsv_to_rgb(self.color * 360.0 / 100.0, 1.0, 1.0);
        let shade = if self.legacy_shade > 100.0 { 200.0 - self.legacy_shade } else { self.legacy_shade };
        if shade < 50.0 {
            rgb = mix_rgb([0.0, 0.0, 0.0], rgb, (10.0 + shade) / 60.0);
        } else {
            rgb = mix_rgb(rgb, [255.0, 255.0, 255.0], (shade - 50.0) / 60.0);
        }
        let (h, s, v) = rgb_to_hsv(rgb);
        self.color = 100.0 * h / 360.0;
        self.saturation = 100.0 * s;
        self.brightness = 100.0 * v;
        self.update_color();
    }
}

impl World {
    /// `_onTargetMoved`: draw a line when the pen is down.
    pub(crate) fn pen_moved(&mut self, t: TargetRef, ox: f64, oy: f64) {
        if !self.target(t).pen.down || self.renderer.is_none() {
            return;
        }
        let tg = self.target(t);
        let (pen, x, y) = (tg.pen.clone(), tg.x, tg.y);
        if let Some(r) = self.renderer.as_mut() {
            r.pen_line(&pen, ox, oy, x, y);
        }
        self.request_redraw();
    }

    pub fn pen_clear(&mut self, _me: TargetRef) {
        if let Some(r) = self.renderer.as_mut() {
            r.pen_clear();
            self.request_redraw();
        }
    }

    pub fn pen_stamp(&mut self, me: TargetRef) {
        if self.with_renderer(|r, w| r.pen_stamp(w, me)).is_some() {
            self.request_redraw();
        }
    }

    pub fn pen_down(&mut self, me: TargetRef) {
        self.target_mut(me).pen.down = true;
        let tg = self.target(me);
        let (pen, x, y) = (tg.pen.clone(), tg.x, tg.y);
        if let Some(r) = self.renderer.as_mut() {
            r.pen_line(&pen, x, y, x, y);
            self.request_redraw();
        }
    }

    pub fn pen_up(&mut self, me: TargetRef) {
        self.target_mut(me).pen.down = false;
    }

    pub fn pen_set_color(&mut self, me: TargetRef, color: Value) {
        let rgba = to_rgba(&color);
        let (h, s, v) = rgb_to_hsv([rgba[0] as f64, rgba[1] as f64, rgba[2] as f64]);
        let pen = &mut self.target_mut(me).pen;
        pen.color = (h / 360.0) * 100.0;
        pen.saturation = s * 100.0;
        pen.brightness = v * 100.0;
        pen.transparency = 100.0 * (1.0 - (rgba[3] as f64 / 255.0));
        pen.legacy_shade = pen.brightness / 2.0;
        pen.update_color();
    }

    pub fn pen_change_param(&mut self, me: TargetRef, param: JsStr, value: f64) {
        self.target_mut(me).pen.set_or_change(&param, value, true);
    }

    pub fn pen_set_param(&mut self, me: TargetRef, param: JsStr, value: f64) {
        self.target_mut(me).pen.set_or_change(&param, value, false);
    }

    pub fn pen_change_size(&mut self, me: TargetRef, size: f64) {
        let pen = &mut self.target_mut(me).pen;
        pen.diameter = (pen.diameter + size).max(1.0).min(1200.0);
    }

    pub fn pen_set_size(&mut self, me: TargetRef, size: f64) {
        self.target_mut(me).pen.diameter = size.max(1.0).min(1200.0);
    }

    pub fn pen_set_hue(&mut self, me: TargetRef, hue: f64) {
        let pen = &mut self.target_mut(me).pen;
        pen.set_or_change(&JsStr::from("color"), hue / 2.0, false);
        pen.set_or_change(&JsStr::from("transparency"), 0.0, false);
        pen.legacy_update();
    }

    pub fn pen_change_hue(&mut self, me: TargetRef, hue: f64) {
        let pen = &mut self.target_mut(me).pen;
        pen.set_or_change(&JsStr::from("color"), hue / 2.0, true);
        pen.legacy_update();
    }

    pub fn pen_set_shade(&mut self, me: TargetRef, shade: f64) {
        let pen = &mut self.target_mut(me).pen;
        let mut s = shade % 200.0;
        if s < 0.0 {
            s += 200.0;
        }
        pen.legacy_shade = s;
        pen.legacy_update();
    }

    pub fn pen_change_shade(&mut self, me: TargetRef, shade: f64) {
        let cur = self.target(me).pen.legacy_shade;
        self.pen_set_shade(me, cur + shade);
    }
}
