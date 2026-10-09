//! Block primitives (motion, looks, sensing, sound, events, control helpers),
//! transcribed behaviour-for-behaviour from scratch-vm's block packages and
//! `RenderedTarget`.

use crate::sched::{HatKind, Wake};
use crate::world::*;
use scrust_value::{is_whitespace, js_number_to_string, js_round, js_to_fixed_string, JsStr, ToNum, ToStr, Value};
use std::cell::Cell;
use std::rc::Rc;

const PI: f64 = std::f64::consts::PI;

#[inline]
fn deg_to_rad(d: f64) -> f64 {
    d * PI / 180.0
}
#[inline]
fn rad_to_deg(r: f64) -> f64 {
    r * 180.0 / PI
}
#[inline]
fn clamp(n: f64, min: f64, max: f64) -> f64 {
    // Math.min(Math.max(n, min), max)
    let m = if n > min || n.is_nan() { n } else { min };
    if m < max || m.is_nan() {
        m
    } else {
        max
    }
}
#[inline]
pub fn wrap_clamp(n: f64, min: f64, max: f64) -> f64 {
    let range = (max - min) + 1.0;
    n - (((n - min) / range).floor() * range)
}

pub struct Glide {
    start_ms: f64,
    duration: f64,
    sx: f64,
    sy: f64,
    ex: f64,
    ey: f64,
}

fn effect_index(name: &str) -> Option<usize> {
    let lower = name.to_lowercase();
    EFFECT_NAMES.iter().position(|e| *e == lower)
}

impl World {
    // ===================================================================== motion

    /// `RenderedTarget.setXY`.
    pub fn set_xy(&mut self, t: TargetRef, x: f64, y: f64) {
        let tg = self.target(t);
        if tg.is_stage || tg.dragging {
            return;
        }
        let (ox, oy) = (tg.x, tg.y);
        let (nx, ny) = match self.with_renderer(|r, w| r.fenced_position(w, t, x, y)) {
            Some(p) => p,
            None => (x, y),
        };
        let tg = self.target_mut(t);
        tg.x = nx;
        tg.y = ny;
        self.visual_change(t);
        self.pen_moved(t, ox, oy);
    }

    pub fn move_steps(&mut self, me: TargetRef, steps: f64) {
        let tg = self.target(me);
        let radians = deg_to_rad(90.0 - tg.direction);
        let dx = steps * libm::cos(radians);
        let dy = steps * libm::sin(radians);
        let (x, y) = (tg.x + dx, tg.y + dy);
        self.set_xy(me, x, y);
    }

    pub fn go_to_xy(&mut self, me: TargetRef, x: f64, y: f64) {
        self.set_xy(me, x, y);
    }

    /// `getTargetXY`: mouse, random position or another sprite.
    pub fn target_xy(&mut self, to: &Value) -> Option<(f64, f64)> {
        if let Value::Str(s) = to {
            if s.eq_str("_mouse_") {
                return Some((self.io.mouse_x, self.io.mouse_y));
            }
            if s.eq_str("_random_") {
                let x = js_round(STAGE_WIDTH * (self.random() - 0.5));
                let y = js_round(STAGE_HEIGHT * (self.random() - 0.5));
                return Some((x, y));
            }
        }
        let t = self.sprite_by_name(&to.to_str())?;
        let tg = self.target(t);
        Some((tg.x, tg.y))
    }

    pub fn go_to(&mut self, me: TargetRef, to: Value) {
        if let Some((x, y)) = self.target_xy(&to) {
            self.set_xy(me, x, y);
        }
    }

    /// `RenderedTarget.setDirection`.
    pub fn set_direction(&mut self, t: TargetRef, direction: f64) {
        if self.target(t).is_stage || !direction.is_finite() {
            return;
        }
        self.target_mut(t).direction = wrap_clamp(direction, -179.0, 180.0);
        self.visual_change(t);
    }

    pub fn turn_right(&mut self, me: TargetRef, degrees: f64) {
        let d = self.target(me).direction + degrees;
        self.set_direction(me, d);
    }

    pub fn turn_left(&mut self, me: TargetRef, degrees: f64) {
        let d = self.target(me).direction - degrees;
        self.set_direction(me, d);
    }

    pub fn point_in_direction(&mut self, me: TargetRef, direction: f64) {
        self.set_direction(me, direction);
    }

    pub fn point_towards(&mut self, me: TargetRef, towards: Value) {
        let (tx, ty);
        match &towards {
            Value::Str(s) if s.eq_str("_mouse_") => {
                tx = self.io.mouse_x;
                ty = self.io.mouse_y;
            }
            Value::Str(s) if s.eq_str("_random_") => {
                let d = js_round(self.random() * 360.0) - 180.0;
                self.set_direction(me, d);
                return;
            }
            other => {
                let Some(t) = self.sprite_by_name(&other.to_str()) else { return };
                let tg = self.target(t);
                tx = tg.x;
                ty = tg.y;
            }
        }
        let tg = self.target(me);
        let dx = tx - tg.x;
        let dy = ty - tg.y;
        let direction = 90.0 - rad_to_deg(libm::atan2(dy, dx));
        self.set_direction(me, direction);
    }

    pub fn change_x(&mut self, me: TargetRef, dx: f64) {
        let tg = self.target(me);
        let (x, y) = (tg.x + dx, tg.y);
        self.set_xy(me, x, y);
    }
    pub fn set_x(&mut self, me: TargetRef, x: f64) {
        let y = self.target(me).y;
        self.set_xy(me, x, y);
    }
    pub fn change_y(&mut self, me: TargetRef, dy: f64) {
        let tg = self.target(me);
        let (x, y) = (tg.x, tg.y + dy);
        self.set_xy(me, x, y);
    }
    pub fn set_y(&mut self, me: TargetRef, y: f64) {
        let x = self.target(me).x;
        self.set_xy(me, x, y);
    }

    /// First execution of a glide. `None` means the block already finished.
    pub fn glide_begin(&mut self, me: TargetRef, secs: f64, x: f64, y: f64) -> Option<Glide> {
        let tg = self.target(me);
        let g = Glide { start_ms: self.real_ms(), duration: secs, sx: tg.x, sy: tg.y, ex: x, ey: y };
        if g.duration <= 0.0 {
            self.set_xy(me, x, y);
            return None;
        }
        Some(g)
    }

    /// Re-execution of a glide. Returns true when it has finished.
    pub fn glide_step(&mut self, me: TargetRef, g: &Glide) -> bool {
        let elapsed = self.real_ms() - g.start_ms;
        if elapsed < g.duration * 1000.0 {
            let frac = elapsed / (g.duration * 1000.0);
            let dx = frac * (g.ex - g.sx);
            let dy = frac * (g.ey - g.sy);
            self.set_xy(me, g.sx + dx, g.sy + dy);
            false
        } else {
            self.set_xy(me, g.ex, g.ey);
            true
        }
    }

    pub fn if_on_edge_bounce(&mut self, me: TargetRef) {
        let Some(Some(bounds)) = self.with_renderer(|r, w| r.bounds(w, me)) else { return };
        let dist_left = (STAGE_WIDTH / 2.0 + bounds.left).max(0.0);
        let dist_top = (STAGE_HEIGHT / 2.0 - bounds.top).max(0.0);
        let dist_right = (STAGE_WIDTH / 2.0 - bounds.right).max(0.0);
        let dist_bottom = (STAGE_HEIGHT / 2.0 + bounds.bottom).max(0.0);
        let mut nearest = 0;
        let mut min_dist = f64::INFINITY;
        for (i, d) in [dist_left, dist_top, dist_right, dist_bottom].into_iter().enumerate() {
            if d < min_dist {
                min_dist = d;
                nearest = i;
            }
        }
        if min_dist > 0.0 {
            return;
        }
        let radians = deg_to_rad(90.0 - self.target(me).direction);
        let mut dx = libm::cos(radians);
        let mut dy = -libm::sin(radians);
        match nearest {
            0 => dx = dx.abs().max(0.2),
            1 => dy = dy.abs().max(0.2),
            2 => dx = 0.0 - dx.abs().max(0.2),
            _ => dy = 0.0 - dy.abs().max(0.2),
        }
        let new_direction = rad_to_deg(libm::atan2(dy, dx)) + 90.0;
        self.set_direction(me, new_direction);
        // keepInFence with the default (stage) fence.
        let tg = self.target(me);
        let (x, y) = (tg.x, tg.y);
        if let Some(Some(b)) = self.with_renderer(|r, w| r.bounds(w, me)) {
            let mut dx = 0.0;
            let mut dy = 0.0;
            if b.left < -STAGE_WIDTH / 2.0 {
                dx += -STAGE_WIDTH / 2.0 - b.left;
            }
            if b.right > STAGE_WIDTH / 2.0 {
                dx += STAGE_WIDTH / 2.0 - b.right;
            }
            if b.top > STAGE_HEIGHT / 2.0 {
                dy += STAGE_HEIGHT / 2.0 - b.top;
            }
            if b.bottom < -STAGE_HEIGHT / 2.0 {
                dy += -STAGE_HEIGHT / 2.0 - b.bottom;
            }
            self.set_xy(me, x + dx, y + dy);
        }
    }

    pub fn set_rotation_style(&mut self, me: TargetRef, style: &str) {
        let rs = match style {
            "don't rotate" => RotationStyle::DontRotate,
            "all around" => RotationStyle::AllAround,
            "left-right" => RotationStyle::LeftRight,
            _ => self.target(me).rotation_style,
        };
        self.target_mut(me).rotation_style = rs;
        self.visual_change(me);
    }

    fn limit_precision(c: f64) -> f64 {
        let rounded = js_round(c);
        let delta = c - rounded;
        if delta.abs() < 1e-9 {
            rounded
        } else {
            c
        }
    }

    #[inline]
    pub fn x_position(&mut self, me: TargetRef) -> f64 {
        Self::limit_precision(self.target(me).x)
    }
    #[inline]
    pub fn y_position(&mut self, me: TargetRef) -> f64 {
        Self::limit_precision(self.target(me).y)
    }
    #[inline]
    pub fn direction(&mut self, me: TargetRef) -> f64 {
        self.target(me).direction
    }

    // ====================================================================== looks

    /// `_formatBubbleText`.
    fn format_bubble_text(v: &Value) -> JsStr {
        if let Value::Str(s) = v {
            if s.is_empty() {
                return s.clone();
            }
        }
        let text = match v {
            Value::Num(n) if n.abs() >= 0.01 && (n % 1.0 != 0.0) => JsStr::from(js_to_fixed_string(*n, 2).as_str()),
            other => other.to_str(),
        };
        if text.len() > 330 {
            text.substring(0, 330)
        } else {
            text
        }
    }

    pub(crate) fn update_bubble(&mut self, t: TargetRef, think: bool, text: JsStr) {
        let usage = self.next_bubble_uid();
        self.target_mut(t).bubble = Some(Bubble { think, text, usage });
        // `_renderBubble` only does work (and requests a redraw) with a renderer.
        if self.renderer.is_some() {
            self.request_redraw();
        }
    }

    pub fn say(&mut self, me: TargetRef, message: Value) {
        let text = Self::format_bubble_text(&message);
        self.update_bubble(me, false, text);
    }

    pub fn think(&mut self, me: TargetRef, message: Value) {
        let text = Self::format_bubble_text(&message);
        self.update_bubble(me, true, text);
    }

    /// `say/think for secs`: shows the bubble and returns the promise to wait on.
    pub fn say_for_secs(&mut self, me: TargetRef, think: bool, message: Value, secs: Value) -> Wake {
        let text = Self::format_bubble_text(&message);
        self.update_bubble(me, think, text);
        let usage = self.target(me).bubble.as_ref().map(|b| b.usage).unwrap_or(0);
        // setTimeout(..., 1000 * args.SECS): raw JS multiplication, NaN / negative -> 0.
        let ms = 1000.0 * secs.js_number();
        let ms = if ms.is_nan() || ms < 0.0 { 0.0 } else { ms };
        Wake::Bubble { at: self.clock_ms() + ms, target: me, usage }
    }

    /// `RenderedTarget.setVisible` + `_renderBubble`.
    pub fn show(&mut self, me: TargetRef) {
        if !self.target(me).is_stage {
            self.target_mut(me).visible = true;
            self.visual_change(me);
        }
        self.request_redraw_if_renderer();
    }

    pub fn hide(&mut self, me: TargetRef) {
        if !self.target(me).is_stage {
            self.target_mut(me).visible = false;
        }
        self.request_redraw_if_renderer();
    }

    /// `RenderedTarget.setCostume`.
    pub fn set_costume_index(&mut self, t: TargetRef, index: f64) {
        let mut index = js_round(index);
        if !index.is_finite() {
            index = 0.0;
        }
        let n = self.program.sprites[self.target(t).sprite].costumes.len();
        if n == 0 {
            return;
        }
        self.target_mut(t).costume = wrap_clamp(index, 0.0, n as f64 - 1.0) as usize;
        self.visual_change(t);
    }

    fn costume_index_by_name(&self, t: TargetRef, name: &JsStr) -> Option<usize> {
        self.program.sprites[self.target(t).sprite].costumes.iter().position(|c| name.eq_str(c.name))
    }

    /// `_setCostume` / the costume part of `_setBackdrop`.
    fn set_costume_value(&mut self, t: TargetRef, requested: &Value, backdrop: bool) {
        if let Value::Num(n) = requested {
            self.set_costume_index(t, *n - 1.0);
            return;
        }
        let name = requested.to_str();
        if let Some(i) = self.costume_index_by_name(t, &name) {
            self.set_costume_index(t, i as f64);
            return;
        }
        let cur = self.target(t).costume as f64;
        if let Value::Str(s) = requested {
            let (next, prev) = if backdrop { ("next backdrop", "previous backdrop") } else { ("next costume", "previous costume") };
            if s.eq_str(next) {
                self.set_costume_index(t, cur + 1.0);
                return;
            }
            if s.eq_str(prev) {
                self.set_costume_index(t, cur - 1.0);
                return;
            }
            if backdrop && s.eq_str("random backdrop") {
                let n = self.program.sprites[self.target(t).sprite].costumes.len();
                if n > 1 {
                    // MathUtil.inclusiveRandIntWithout(0, n - 1, current)
                    let possible = (n - 1) as f64;
                    let mut r = (self.random() * possible).floor();
                    if r >= cur {
                        r += 1.0;
                    }
                    self.set_costume_index(t, r);
                }
                return;
            }
        }
        // Numeric strings (but not pure whitespace) and booleans act as indices.
        let n = requested.js_number();
        if !(n.is_nan() || is_whitespace(requested)) {
            self.set_costume_index(t, n - 1.0);
        }
    }

    pub fn switch_costume(&mut self, me: TargetRef, costume: Value) {
        self.set_costume_value(me, &costume, false);
    }

    pub fn next_costume(&mut self, me: TargetRef) {
        let c = self.target(me).costume as f64 + 1.0;
        self.set_costume_index(me, c);
    }

    fn backdrop_hats(&mut self) -> Vec<u64> {
        let stage = self.stage_ref();
        let name = self.program.sprites[0].costumes.get(self.target(stage).costume).map(|c| c.name).unwrap_or("");
        let upper = JsStr::from(name).to_upper().to_rust_string();
        self.start_hats(HatKind::Backdrop, Some(&upper), 0, None)
    }

    /// `_setBackdrop`: returns the threads started by `when backdrop switches to`.
    pub fn switch_backdrop_threads(&mut self, backdrop: Value) -> Vec<u64> {
        let stage = self.stage_ref();
        self.set_costume_value(stage, &backdrop, true);
        self.backdrop_hats()
    }

    pub fn switch_backdrop(&mut self, _me: TargetRef, backdrop: Value) {
        self.switch_backdrop_threads(backdrop);
    }

    pub fn next_backdrop(&mut self, _me: TargetRef) {
        let stage = self.stage_ref();
        let c = self.target(stage).costume as f64 + 1.0;
        self.set_costume_index(stage, c);
        self.backdrop_hats();
    }

    pub fn change_effect(&mut self, me: TargetRef, effect: &str, change: f64) {
        let Some(i) = effect_index(effect) else { return };
        let v = change + self.target(me).effects[i];
        self.store_effect(me, i, v);
    }

    pub fn set_effect(&mut self, me: TargetRef, effect: &str, value: f64) {
        let Some(i) = effect_index(effect) else { return };
        self.store_effect(me, i, value);
    }

    fn store_effect(&mut self, me: TargetRef, i: usize, value: f64) {
        let v = match EFFECT_NAMES[i] {
            "ghost" => clamp(value, 0.0, 100.0),
            "brightness" => clamp(value, -100.0, 100.0),
            _ => value,
        };
        self.target_mut(me).effects[i] = v;
        self.visual_change(me);
    }

    pub fn clear_effects(&mut self, me: TargetRef) {
        self.target_mut(me).effects = [0.0; 7];
        self.visual_change(me);
    }

    /// `RenderedTarget.setSize` (only has an effect when a renderer is attached).
    pub fn set_size(&mut self, me: TargetRef, size: f64) {
        let tg = self.target(me);
        if tg.is_stage {
            return;
        }
        let (sprite, costume) = (tg.sprite, tg.costume);
        let Some((ow, oh)) = self.with_renderer(|r, _| r.costume_size(sprite, costume)) else { return };
        let min_scale = f64::min(1.0, f64::max(5.0 / ow, 5.0 / oh));
        let max_scale = f64::min((1.5 * STAGE_WIDTH) / ow, (1.5 * STAGE_HEIGHT) / oh);
        self.target_mut(me).size = clamp(size / 100.0, min_scale, max_scale) * 100.0;
        self.visual_change(me);
    }

    pub fn change_size(&mut self, me: TargetRef, change: f64) {
        let s = self.target(me).size + change;
        self.set_size(me, s);
    }

    #[inline]
    pub fn size(&mut self, me: TargetRef) -> f64 {
        js_round(self.target(me).size)
    }

    /// `Runtime.moveExecutable`.
    fn move_executable(&mut self, t: TargetRef, delta: f64) {
        let Some(old) = self.order.iter().position(|x| *x == t) else { return };
        self.order.remove(old);
        let len = self.order.len() as f64;
        let mut new_index = old as f64 + delta;
        if new_index > len {
            new_index = len;
        }
        if new_index <= 0.0 {
            new_index = if !self.order.is_empty() && self.target(self.order[0]).is_stage { 1.0 } else { 0.0 };
        }
        // Array.prototype.splice truncates its start index (NaN -> 0).
        let i = if new_index.is_nan() { 0 } else { new_index.trunc() as usize };
        self.order.insert(i.min(self.order.len()), t);
        self.request_redraw_if_renderer();
    }

    pub fn go_to_front_back(&mut self, me: TargetRef, front_back: &str) {
        if self.target(me).is_stage {
            return;
        }
        let old = self.order.iter().position(|x| *x == me).unwrap_or(0) as f64;
        if front_back == "front" {
            self.move_executable(me, f64::INFINITY - old);
        } else {
            self.move_executable(me, f64::NEG_INFINITY - old);
        }
    }

    pub fn go_layers(&mut self, me: TargetRef, direction: &str, num: f64) {
        if self.target(me).is_stage {
            return;
        }
        if direction == "forward" {
            self.move_executable(me, num);
        } else {
            self.move_executable(me, -num);
        }
    }

    #[inline]
    pub fn costume_number(&mut self, me: TargetRef) -> f64 {
        self.target(me).costume as f64 + 1.0
    }
    pub fn costume_name(&mut self, me: TargetRef) -> JsStr {
        let tg = self.target(me);
        JsStr::from(self.program.sprites[tg.sprite].costumes.get(tg.costume).map(|c| c.name).unwrap_or(""))
    }
    #[inline]
    pub fn backdrop_number(&mut self, _me: TargetRef) -> f64 {
        self.targets[0].costume as f64 + 1.0
    }
    pub fn backdrop_name(&mut self, _me: TargetRef) -> JsStr {
        JsStr::from(self.program.sprites[0].costumes.get(self.targets[0].costume).map(|c| c.name).unwrap_or(""))
    }

    pub fn noop0(&mut self, _me: TargetRef) {}
    pub fn noop1(&mut self, _me: TargetRef, _a: Value) {}

    // ===================================================================== events

    /// `event_broadcast` with a statically resolved message.
    pub fn broadcast(&mut self, msg: usize) -> Vec<u64> {
        self.start_hats(HatKind::Broadcast, None, msg, None)
    }

    /// `lookupBroadcastByInputValue`: resolve a broadcast by name at run time.
    pub fn broadcast_lookup(&self, name: &JsStr) -> Option<usize> {
        if name.is_empty() {
            return None;
        }
        let lower = name.to_lower().to_rust_string();
        self.program.broadcast_lookup.iter().find(|(n, _)| *n == lower).map(|(_, m)| *m)
    }

    pub fn broadcast_dynamic(&mut self, name: JsStr) -> Vec<u64> {
        match self.broadcast_lookup(&name) {
            Some(m) => self.broadcast(m),
            None => Vec::new(),
        }
    }

    // ==================================================================== control

    pub fn clear_counter(&mut self, _me: TargetRef) {
        self.counter = 0.0;
    }
    pub fn incr_counter(&mut self, _me: TargetRef) {
        self.counter += 1.0;
    }
    #[inline]
    pub fn counter(&mut self, _me: TargetRef) -> f64 {
        self.counter
    }

    /// `control_wait`, first execution: duration in ms and the start time.
    pub fn wait_begin(&mut self, secs: f64) -> (f64, f64) {
        let duration = f64::max(0.0, 1000.0 * secs);
        self.request_redraw();
        (self.current_msecs, duration)
    }

    /// `stackTimerFinished`.
    #[inline]
    pub fn wait_done(&self, timer: (f64, f64)) -> bool {
        !(self.current_msecs - timer.0 < timer.1)
    }

    /// `control_stop` "other scripts in sprite".
    pub fn stop_other_scripts(&mut self, me: TargetRef) {
        self.stop_for_target(me, true);
    }

    /// Busy-wait guard for warp-mode waits: returns true when the thread
    /// should really yield (scratch-vm's 500 ms warp timer).
    pub fn warp_stuck(&mut self) -> bool {
        self.sched.busy += 1;
        if self.options.deterministic {
            // The virtual clock is frozen within a frame.
            return self.sched.busy % 100_000 == 0;
        }
        if self.sched.busy % 128 != 0 {
            return false;
        }
        self.real_ms() - self.sched.step_start > 500.0
    }

    // ==================================================================== sensing

    #[inline]
    pub fn timer(&mut self, _me: TargetRef) -> f64 {
        (self.current_msecs - self.timer_start) / 1000.0
    }

    /// The timer value seen by `when [timer] > ()` hats.
    #[inline]
    pub fn timer_value(&self) -> f64 {
        (self.current_msecs - self.timer_start) / 1000.0
    }

    pub fn reset_timer(&mut self, _me: TargetRef) {
        self.timer_start = self.current_msecs;
    }

    #[inline]
    pub fn answer(&mut self, _me: TargetRef) -> JsStr {
        self.answer.clone()
    }

    pub fn username(&mut self, _me: TargetRef) -> JsStr {
        self.username.clone()
    }

    #[inline]
    pub fn mouse_x(&mut self, _me: TargetRef) -> f64 {
        self.io.mouse_x
    }
    #[inline]
    pub fn mouse_y(&mut self, _me: TargetRef) -> f64 {
        self.io.mouse_y
    }
    #[inline]
    pub fn mouse_down(&mut self, _me: TargetRef) -> bool {
        self.io.mouse_down
    }

    /// `Keyboard._keyArgToScratchKey`.
    fn key_arg_to_scratch_key(arg: &Value) -> String {
        if let Value::Num(n) = arg {
            if *n >= 48.0 && *n <= 90.0 {
                // String.fromCharCode truncates.
                return char::from_u32(*n as u32).map(|c| c.to_string()).unwrap_or_default();
            }
            match *n {
                x if x == 32.0 => return "space".into(),
                x if x == 37.0 => return "left arrow".into(),
                x if x == 38.0 => return "up arrow".into(),
                x if x == 39.0 => return "right arrow".into(),
                x if x == 40.0 => return "down arrow".into(),
                _ => {}
            }
        }
        let s = arg.to_str();
        for name in ["space", "left arrow", "up arrow", "right arrow", "down arrow", "enter"] {
            if s.eq_str(name) {
                return name.into();
            }
        }
        let first = if s.len() > 1 { s.char_at(0) } else { s };
        if first.eq_str(" ") {
            return "space".into();
        }
        first.to_upper().to_rust_string()
    }

    pub fn key_pressed(&mut self, _me: TargetRef, key: Value) -> bool {
        if let Value::Str(s) = &key {
            if s.eq_str("any") {
                return !self.io.keys.is_empty();
            }
        }
        let k = Self::key_arg_to_scratch_key(&key);
        self.io.keys.iter().any(|x| *x == k)
    }

    pub fn loudness(&mut self, _me: TargetRef) -> f64 {
        match self.audio.as_mut() {
            Some(a) => a.loudness(),
            None => -1.0,
        }
    }

    pub fn loud(&mut self, me: TargetRef) -> bool {
        self.loudness(me) > 10.0
    }

    /// Milliseconds since the Unix epoch, local-time fields derived from it.
    fn local_fields(&self) -> (f64, f64, f64, f64, f64, f64, f64) {
        let ms = self.wall_ms() + self.tz_offset_min * 60000.0;
        let days = (ms / 86400000.0).floor();
        let rem = ms - days * 86400000.0;
        let (h, mi, s) = ((rem / 3600000.0).floor(), ((rem % 3600000.0) / 60000.0).floor(), ((rem % 60000.0) / 1000.0).floor());
        // Civil-from-days (Howard Hinnant's algorithm).
        let z = days as i64 + 719468;
        let era = z.div_euclid(146097);
        let doe = z.rem_euclid(146097);
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = if m <= 2 { y + 1 } else { y };
        let dow = (days as i64 + 4).rem_euclid(7); // 1970-01-01 was a Thursday
        (y as f64, m as f64, d as f64, dow as f64 + 1.0, h, mi, s)
    }

    fn wall_ms(&self) -> f64 {
        if self.options.deterministic {
            return self.clock_ms();
        }
        match &self.host_epoch_ms {
            Some(f) => f(),
            None => self.clock_ms(),
        }
    }

    pub fn current(&mut self, _me: TargetRef, menu: &str) -> f64 {
        let (y, m, d, dow, h, mi, s) = self.local_fields();
        match menu.to_lowercase().as_str() {
            "year" => y,
            "month" => m,
            "date" => d,
            "dayofweek" => dow,
            "hour" => h,
            "minute" => mi,
            "second" => s,
            _ => 0.0,
        }
    }

    pub fn days_since_2000(&mut self, _me: TargetRef) -> f64 {
        // The timezone terms of the original expression cancel out.
        (self.wall_ms() - 946684800000.0) / 86400000.0
    }

    pub fn distance_to(&mut self, me: TargetRef, to: Value) -> f64 {
        if self.target(me).is_stage {
            return 10000.0;
        }
        let (tx, ty) = match &to {
            Value::Str(s) if s.eq_str("_mouse_") => (self.io.mouse_x, self.io.mouse_y),
            other => {
                let Some(t) = self.sprite_by_name(&other.to_str()) else { return 10000.0 };
                (self.target(t).x, self.target(t).y)
            }
        };
        let tg = self.target(me);
        let dx = tg.x - tx;
        let dy = tg.y - ty;
        (dx * dx + dy * dy).sqrt()
    }

    pub fn touching_object(&mut self, me: TargetRef, object: Value) -> bool {
        if self.renderer.is_none() {
            return false;
        }
        match &object {
            Value::Str(s) if s.eq_str("_mouse_") => {
                let (x, y) = (self.io.mouse_x, self.io.mouse_y);
                self.with_renderer(|r, w| r.touching_point(w, me, x, y)).unwrap_or(false)
            }
            Value::Str(s) if s.eq_str("_edge_") => match self.with_renderer(|r, w| r.bounds(w, me)).flatten() {
                Some(b) => {
                    b.left < -STAGE_WIDTH / 2.0 || b.right > STAGE_WIDTH / 2.0 || b.top > STAGE_HEIGHT / 2.0 || b.bottom < -STAGE_HEIGHT / 2.0
                }
                None => false,
            },
            other => {
                let Some(first) = self.sprite_by_name(&other.to_str()) else { return false };
                let sprite = self.target(first).sprite;
                let candidates: Vec<TargetRef> =
                    self.seq.iter().copied().filter(|t| self.target(*t).sprite == sprite && !self.target(*t).dragging).collect();
                self.with_renderer(|r, w| r.touching_targets(w, me, &candidates)).unwrap_or(false)
            }
        }
    }

    pub fn touching_color(&mut self, me: TargetRef, color: Value) -> bool {
        let c = to_rgb(&color);
        self.with_renderer(|r, w| r.touching_color(w, me, c, None)).unwrap_or(false)
    }

    pub fn color_touching_color(&mut self, me: TargetRef, color: Value, color2: Value) -> bool {
        // sensing: colorIsTouchingColor(Cast.toRgbColorList(COLOR2), mask = COLOR)
        let mask = to_rgb(&color);
        let target = to_rgb(&color2);
        self.with_renderer(|r, w| r.touching_color(w, me, target, Some(mask))).unwrap_or(false)
    }

    pub fn set_drag_mode(&mut self, me: TargetRef, mode: &str) {
        if !self.target(me).is_stage {
            self.target_mut(me).draggable = mode == "draggable";
        }
    }

    /// `sensing_of` for built-in attributes and dynamic operands.
    pub fn attr_of(&mut self, _me: TargetRef, property: &str, object: Value) -> Value {
        let target = match &object {
            Value::Str(s) if s.eq_str("_stage_") => Some(self.stage_ref()),
            other => self.sprite_by_name(&other.to_str()),
        };
        let Some(t) = target else { return Value::Num(0.0) };
        let tg = self.target(t);
        let costume_name = || Value::Str(JsStr::from(self.program.sprites[tg.sprite].costumes.get(tg.costume).map(|c| c.name).unwrap_or("")));
        if tg.is_stage {
            match property {
                "background #" | "backdrop #" => return Value::Num(tg.costume as f64 + 1.0),
                "backdrop name" => return costume_name(),
                "volume" => return Value::Num(tg.volume),
                _ => {}
            }
        } else {
            match property {
                "x position" => return Value::Num(tg.x),
                "y position" => return Value::Num(tg.y),
                "direction" => return Value::Num(tg.direction),
                "costume #" => return Value::Num(tg.costume as f64 + 1.0),
                "costume name" => return costume_name(),
                "size" => return Value::Num(tg.size),
                "volume" => return Value::Num(tg.volume),
                _ => {}
            }
        }
        tg.locals.var_by_name(property).unwrap_or(Value::Num(0.0))
    }

    // ------------------------------------------------------------------ ask

    /// `sensing_askandwait`: enqueue the question, return the promise flag.
    pub fn ask(&mut self, me: TargetRef, question: JsStr) -> Wake {
        let done = Rc::new(Cell::new(false));
        let tg = self.target(me);
        let (vis, stage) = (tg.visible, tg.is_stage);
        let asked = !self.questions.is_empty();
        self.questions.push(Question { text: question, target: me, was_visible: vis, was_stage: stage, done: done.clone() });
        if !asked {
            self.ask_next_question();
        }
        Wake::Flag(done)
    }

    fn ask_next_question(&mut self) {
        let Some(q) = self.questions.first() else { return };
        let (text, target, vis, stage) = (q.text.clone(), q.target, q.was_visible, q.was_stage);
        if vis && !stage {
            self.say(target, Value::Str(text));
        }
    }

    /// The question currently shown to the user, if any: (text, shown in a bubble).
    pub fn current_question(&self) -> Option<(JsStr, bool)> {
        self.questions.first().map(|q| (q.text.clone(), q.was_visible && !q.was_stage))
    }

    /// The user answered the current question.
    pub fn submit_answer(&mut self, answer: JsStr) {
        if self.questions.is_empty() {
            return;
        }
        let q = self.questions.remove(0);
        self.answer = answer;
        if q.was_visible && !q.was_stage {
            self.say(q.target, Value::Str(JsStr::empty()));
        }
        q.done.set(true);
        self.ask_next_question();
    }

    pub(crate) fn clear_target_questions(&mut self, t: TargetRef) {
        if self.questions.is_empty() {
            return;
        }
        let current_was_target = self.questions[0].target == t;
        self.questions.retain(|q| q.target != t);
        if current_was_target {
            self.ask_next_question();
        }
    }

    // ====================================================================== sound

    /// `_getSoundIndex`: name first, then a 1-based (wrapping) index.
    fn sound_index(&self, me: TargetRef, sound: &Value) -> Option<usize> {
        let sounds = &self.program.sprites[self.target(me).sprite].sounds;
        let len = sounds.len();
        if len == 0 {
            return None;
        }
        // `sounds[i].name === soundName` is a strict comparison on the raw value.
        if let Value::Str(name) = sound {
            if let Some(i) = sounds.iter().position(|s| name.eq_str(s.name)) {
                return Some(i);
            }
        }
        // parseInt(soundName, 10)
        let n = js_parse_int(&sound.to_str().to_rust_string())?;
        Some(wrap_clamp(n - 1.0, 0.0, len as f64 - 1.0) as usize)
    }

    fn start_sound(&mut self, me: TargetRef, sound: &Value) -> Option<Rc<Cell<bool>>> {
        let i = self.sound_index(me, sound)?;
        let tg = self.target(me);
        let (sprite, vol, pitch, pan) = (tg.sprite, tg.volume, tg.pitch, tg.pan);
        self.audio.as_mut()?.play(sprite, i, me, vol, pitch, pan)
    }

    pub fn sound_play(&mut self, me: TargetRef, sound: Value) {
        self.start_sound(me, &sound);
    }

    /// `sound_playuntildone`: `None` when there is nothing to wait for.
    pub fn sound_play_until_done(&mut self, me: TargetRef, sound: Value) -> Option<Wake> {
        self.start_sound(me, &sound).map(Wake::Flag)
    }

    pub fn sound_stop_all(&mut self, _me: TargetRef) {
        if let Some(a) = self.audio.as_mut() {
            a.stop_all();
        }
    }

    fn sync_audio(&mut self, me: TargetRef) {
        let tg = self.target(me);
        let (v, p, pan) = (tg.volume, tg.pitch, tg.pan);
        if let Some(a) = self.audio.as_mut() {
            a.update_target(me, v, p, pan);
        }
    }

    pub fn sound_set_effect(&mut self, me: TargetRef, effect: &str, value: f64) {
        self.update_sound_effect(me, effect, value, false);
    }
    pub fn sound_change_effect(&mut self, me: TargetRef, effect: &str, value: f64) {
        self.update_sound_effect(me, effect, value, true);
    }
    fn update_sound_effect(&mut self, me: TargetRef, effect: &str, value: f64, change: bool) {
        let tg = self.target_mut(me);
        match effect.to_lowercase().as_str() {
            "pitch" => {
                let v = if change { tg.pitch + value } else { value };
                tg.pitch = clamp(v, -360.0, 360.0);
            }
            "pan" => {
                let v = if change { tg.pan + value } else { value };
                tg.pan = clamp(v, -100.0, 100.0);
            }
            _ => return,
        }
        self.sync_audio(me);
    }
    pub fn sound_clear_effects(&mut self, me: TargetRef) {
        let tg = self.target_mut(me);
        tg.pitch = 0.0;
        tg.pan = 0.0;
        self.sync_audio(me);
    }
    pub fn sound_set_volume(&mut self, me: TargetRef, volume: f64) {
        self.target_mut(me).volume = clamp(volume, 0.0, 100.0);
        self.sync_audio(me);
    }
    pub fn sound_change_volume(&mut self, me: TargetRef, volume: f64) {
        let v = volume + self.target(me).volume;
        self.sound_set_volume(me, v);
    }
    #[inline]
    pub fn volume(&mut self, me: TargetRef) -> f64 {
        self.target(me).volume
    }

    // ================================================================== data

    /// `Cast.toListIndex` with access to the RNG.
    #[inline]
    pub fn list_index(&mut self, index: &Value, len: usize, accept_all: bool) -> scrust_value::ListIndex {
        if let Value::Num(n) = index {
            return scrust_value::num_to_list_index(if n.is_nan() { 0.0 } else { *n }, len);
        }
        let rng = &mut self.rng;
        scrust_value::to_list_index(index, len, accept_all, &mut || rng.next())
    }

    /// `operator_random` on raw values.
    #[inline]
    pub fn random_values(&mut self, from: &Value, to: &Value) -> f64 {
        let rng = &mut self.rng;
        scrust_value::ops::random(from, to, &mut || rng.next())
    }

    /// `operator_random` on pre-cast numbers.
    #[inline]
    pub fn random_range(&mut self, from: f64, to: f64, both_int: bool) -> f64 {
        let rng = &mut self.rng;
        scrust_value::ops::random_range(from, to, both_int, &mut || rng.next())
    }

    pub fn set_monitor_visible(&mut self, sprite: usize, is_list: bool, index: usize, visible: bool) {
        let opcode = if is_list { "data_listcontents" } else { "data_variable" };
        if let Some(i) = self.program.monitors.iter().position(|m| m.opcode == opcode && m.sprite == sprite && m.index == index) {
            self.monitor_visible[i] = visible;
            self.request_redraw_if_renderer();
        }
    }

    // ================================================================== input

    /// Keyboard event from the host. `key` is a DOM `KeyboardEvent.key` string.
    pub fn post_key(&mut self, key: &str, down: bool) {
        let scratch_key = match key {
            " " => "space".to_string(),
            "ArrowLeft" | "Left" => "left arrow".to_string(),
            "ArrowUp" | "Up" => "up arrow".to_string(),
            "ArrowRight" | "Right" => "right arrow".to_string(),
            "ArrowDown" | "Down" => "down arrow".to_string(),
            "Enter" => "enter".to_string(),
            k if k.encode_utf16().count() > 1 => return,
            k => JsStr::from(k).to_upper().to_rust_string(),
        };
        if scratch_key.is_empty() {
            return;
        }
        let index = self.io.keys.iter().position(|k| *k == scratch_key);
        if down {
            let upper = JsStr::from(scratch_key.as_str()).to_upper().to_rust_string();
            self.start_hats(HatKind::Key, Some(&upper), 0, None);
            self.start_hats(HatKind::Key, Some("ANY"), 0, None);
            if index.is_none() {
                self.io.keys.push(scratch_key);
            }
        } else if let Some(i) = index {
            self.io.keys.remove(i);
        }
    }

    /// Mouse move in Scratch coordinates.
    pub fn post_mouse_move(&mut self, x: f64, y: f64) {
        self.io.mouse_x = js_round(clamp(x, -240.0, 240.0));
        self.io.mouse_y = js_round(clamp(y, -180.0, 180.0));
        // Dragging a draggable sprite (what scratch-gui does through
        // `startDrag` / `postSpriteInfo({force: true})`).
        if let Some((t, dx, dy, _)) = self.io.drag {
            if self.target_checked(t).map(|tg| tg.alive).unwrap_or(false) {
                self.io.drag = Some((t, dx, dy, true));
                self.target_mut(t).dragging = false;
                self.set_xy(t, x - dx, y - dy);
                self.target_mut(t).dragging = true;
            } else {
                self.io.drag = None;
            }
        }
    }

    /// Mouse button change at Scratch coordinates `(x, y)`.
    pub fn post_mouse_button(&mut self, x: f64, y: f64, down: bool) {
        let prev = self.io.mouse_down;
        self.io.mouse_down = down;
        if prev == down {
            return;
        }
        if !(x > -240.0 && x < 240.0 && y > -180.0 && y < 180.0) {
            return;
        }
        // Finish a drag: a sprite that was actually moved does not get a click.
        if !down {
            if let Some((t, _, _, moved)) = self.io.drag.take() {
                if self.target_checked(t).is_some() {
                    self.target_mut(t).dragging = false;
                }
                if moved {
                    return;
                }
            }
        }
        let target = self.with_renderer(|r, w| r.pick(w, x, y)).flatten().unwrap_or(self.stage_ref());
        let draggable = self.target(target).draggable;
        if draggable && down {
            let tg = self.target(target);
            self.io.drag = Some((target, x - tg.x, y - tg.y, false));
            self.target_mut(target).dragging = true;
            self.go_to_front_back(target, "front");
        }
        if (draggable && prev && !down) || (!draggable && !prev && down) {
            self.start_hats(HatKind::SpriteClicked, None, 0, Some(target));
            self.start_hats(HatKind::StageClicked, None, 0, Some(target));
        }
    }
}

/// JS `parseInt(s, 10)`; `None` for NaN.
pub fn js_parse_int(s: &str) -> Option<f64> {
    let t = s.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    let (neg, t) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let digits: &str = &t[..t.bytes().take_while(|b| b.is_ascii_digit()).count()];
    if digits.is_empty() {
        return None;
    }
    let v: f64 = digits.parse().ok()?;
    Some(if neg { -v } else { v })
}

/// `Cast.toRgbColorList`.
pub fn to_rgb(v: &Value) -> [u8; 3] {
    let c = to_rgba(v);
    [c[0], c[1], c[2]]
}

/// `Cast.toRgbColorObject` (with alpha).
pub fn to_rgba(v: &Value) -> [u8; 4] {
    if let Value::Str(s) = v {
        if s.len() > 0 && s.code_unit(0) == b'#' as u16 {
            return hex_to_rgb(&s.to_rust_string()).unwrap_or([0, 0, 0, 255]);
        }
    }
    // Color.decimalToRgb
    let n = v.to_num();
    let d = js_to_int32(n);
    let a = ((d >> 24) & 0xFF) as u8;
    let r = ((d >> 16) & 0xFF) as u8;
    let g = ((d >> 8) & 0xFF) as u8;
    let b = (d & 0xFF) as u8;
    [r, g, b, if a > 0 { a } else { 255 }]
}

/// ECMAScript ToInt32.
pub fn js_to_int32(n: f64) -> i32 {
    if !n.is_finite() {
        return 0;
    }
    let m = n.trunc() % 4294967296.0;
    let m = if m < 0.0 { m + 4294967296.0 } else { m };
    (m as u64 as u32) as i32
}

/// `Color.hexToRgb`: `#rgb` or `#rrggbb`.
fn hex_to_rgb(hex: &str) -> Option<[u8; 4]> {
    let h = hex.strip_prefix('#')?;
    let full: String = if h.len() == 3 { h.chars().flat_map(|c| [c, c]).collect() } else { h.to_string() };
    if full.len() != 6 || !full.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let p = |i: usize| u8::from_str_radix(&full[i..i + 2], 16).ok();
    Some([p(0)?, p(2)?, p(4)?, 255])
}

/// Trace helper: a number in the conformance trace encoding.
pub fn trace_num(n: f64) -> String {
    if n == 0.0 && n.is_sign_negative() {
        "-0".into()
    } else {
        js_number_to_string(n)
    }
}
