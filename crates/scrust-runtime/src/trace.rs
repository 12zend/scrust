//! Conformance trace: one JSON object per frame, byte-identical to the
//! output of `conformance/harness/trace.js` for the same project.

use crate::blocks::trace_num;
use crate::world::*;
use scrust_value::{JsStr, Value};

/// JSON string literal with the escaping rules of `JSON.stringify`.
pub fn json_str(s: &JsStr, out: &mut String) {
    out.push('"');
    let units: Vec<u16> = s.to_utf16();
    let mut i = 0;
    while i < units.len() {
        let u = units[i];
        match u {
            0x22 => out.push_str("\\\""),
            0x5C => out.push_str("\\\\"),
            0x08 => out.push_str("\\b"),
            0x0C => out.push_str("\\f"),
            0x0A => out.push_str("\\n"),
            0x0D => out.push_str("\\r"),
            0x09 => out.push_str("\\t"),
            0..=0x1F => out.push_str(&format!("\\u{:04x}", u)),
            0xD800..=0xDBFF => {
                if i + 1 < units.len() && (0xDC00..=0xDFFF).contains(&units[i + 1]) {
                    let c = 0x10000 + (((u as u32) - 0xD800) << 10) + ((units[i + 1] as u32) - 0xDC00);
                    out.push(char::from_u32(c).unwrap_or('\u{fffd}'));
                    i += 1;
                } else {
                    out.push_str(&format!("\\u{:04x}", u));
                }
            }
            0xDC00..=0xDFFF => out.push_str(&format!("\\u{:04x}", u)),
            _ => out.push(char::from_u32(u as u32).unwrap_or('\u{fffd}')),
        }
        i += 1;
    }
    out.push('"');
}

fn json_rust_str(s: &str, out: &mut String) {
    json_str(&JsStr::from(s), out)
}

/// `{"t":"num","v":"1"}` / `{"t":"str","v":"1"}` / `{"t":"bool","v":true}`
pub fn json_value(v: &Value, out: &mut String) {
    match v {
        Value::Num(n) => {
            out.push_str("{\"t\":\"num\",\"v\":\"");
            out.push_str(&trace_num(*n));
            out.push_str("\"}");
        }
        Value::Str(s) => {
            out.push_str("{\"t\":\"str\",\"v\":");
            json_str(s, out);
            out.push('}');
        }
        Value::Bool(b) => {
            out.push_str("{\"t\":\"bool\",\"v\":");
            out.push_str(if *b { "true" } else { "false" });
            out.push('}');
        }
    }
}

impl World {
    /// One trace line describing the state at the end of the current frame.
    pub fn trace_line(&self) -> String {
        let mut o = String::with_capacity(512);
        o.push_str(&format!(
            "{{\"frame\":{},\"threads\":{},\"passes\":{},\"rng\":{},\"targets\":[",
            self.frame,
            self.live_threads(),
            self.sched.last_passes,
            self.rng.1
        ));
        for (i, t) in self.order.iter().enumerate() {
            if i > 0 {
                o.push(',');
            }
            let tg = self.target(*t);
            o.push_str("{\"name\":");
            json_rust_str(self.program.sprites[tg.sprite].name, &mut o);
            o.push_str(&format!(
                ",\"clone\":{},\"x\":\"{}\",\"y\":\"{}\",\"dir\":\"{}\",\"size\":\"{}\",\"costume\":{},\"visible\":{},\"say\":",
                !tg.is_original,
                trace_num(tg.x),
                trace_num(tg.y),
                trace_num(tg.direction),
                trace_num(tg.size),
                tg.costume,
                tg.visible
            ));
            match &tg.bubble {
                Some(b) if !b.text.is_empty() => {
                    o.push_str(if b.think { "{\"type\":\"think\",\"text\":" } else { "{\"type\":\"say\",\"text\":" });
                    json_str(&b.text, &mut o);
                    o.push('}');
                }
                _ => o.push_str("null"),
            }
            // Declared variables, then those created on demand, in creation order.
            let sd = &self.program.sprites[tg.sprite];
            let created = &self.created[tg.sprite];
            let all_vars = tg.locals.vars();
            let vars: Vec<&(&str, Value)> = all_vars
                .iter()
                .take(sd.declared_vars)
                .chain(created.iter().filter(|c| !c.0).filter_map(|c| all_vars.get(c.1 as usize)))
                .collect();
            o.push_str(",\"vars\":[");
            for (j, (name, v)) in vars.iter().map(|p| (&p.0, &p.1)).enumerate() {
                if j > 0 {
                    o.push(',');
                }
                o.push('[');
                json_rust_str(name, &mut o);
                o.push(',');
                json_value(v, &mut o);
                o.push(']');
            }
            let all_lists = tg.locals.lists();
            let lists: Vec<&(&str, Vec<Value>)> = all_lists
                .iter()
                .take(sd.declared_lists)
                .chain(created.iter().filter(|c| c.0).filter_map(|c| all_lists.get(c.1 as usize)))
                .collect();
            o.push_str("],\"lists\":[");
            for (j, (name, items)) in lists.iter().map(|p| (&p.0, &p.1)).enumerate() {
                if j > 0 {
                    o.push(',');
                }
                o.push('[');
                json_rust_str(name, &mut o);
                o.push_str(",[");
                for (k, v) in items.iter().enumerate() {
                    if k > 0 {
                        o.push(',');
                    }
                    json_value(v, &mut o);
                }
                o.push_str("]]");
            }
            o.push_str("]}");
        }
        o.push_str("]}");
        o
    }
}
