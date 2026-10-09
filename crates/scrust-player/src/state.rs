//! JSON snapshot of a world's original targets, for hosts that hand the state
//! a run leaves behind back to an editor (the GUI's embed mode).

use scrust_runtime::{Value, World};

/// `{"targets": [{"name", "stage", "x", "y", "direction", "size", "visible",
/// "costume", "layer", "vars": [[name, value]], "lists": [[name, [values]]]}],
/// "monitors": [{"id", "visible"}]}`. `layer` is the position among the
/// original targets, back to front; non-finite numbers are written as
/// `{"n": "NaN"}` (JSON has no literal for them).
pub fn to_json(w: &World) -> String {
    let mut s = String::from("{\"targets\":[");
    let originals: Vec<_> = w.order.iter().filter(|o| w.target(**o).is_original).collect();
    for (i, r) in w.originals.iter().enumerate() {
        let t = w.target(*r);
        if i > 0 {
            s.push(',');
        }
        let layer = originals.iter().position(|o| *o == r).unwrap_or(0);
        s.push_str("{\"name\":");
        json_str(&mut s, w.program.sprites[t.sprite].name);
        s.push_str(&format!(",\"stage\":{},\"x\":", t.is_stage));
        json_num(&mut s, t.x);
        s.push_str(",\"y\":");
        json_num(&mut s, t.y);
        s.push_str(",\"direction\":");
        json_num(&mut s, t.direction);
        s.push_str(",\"size\":");
        json_num(&mut s, t.size);
        s.push_str(&format!(",\"visible\":{},\"costume\":{},\"layer\":{},\"vars\":[", t.visible, t.costume, layer));
        for (j, (name, v)) in t.locals.vars().iter().enumerate() {
            if j > 0 {
                s.push(',');
            }
            s.push('[');
            json_str(&mut s, name);
            s.push(',');
            json_value(&mut s, v);
            s.push(']');
        }
        s.push_str("],\"lists\":[");
        for (j, (name, items)) in t.locals.lists().iter().enumerate() {
            if j > 0 {
                s.push(',');
            }
            s.push('[');
            json_str(&mut s, name);
            s.push_str(",[");
            for (k, v) in items.iter().enumerate() {
                if k > 0 {
                    s.push(',');
                }
                json_value(&mut s, v);
            }
            s.push_str("]]");
        }
        s.push_str("]}");
    }
    s.push_str("],\"monitors\":[");
    for (i, (m, v)) in w.program.monitors.iter().zip(&w.monitor_visible).enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str("{\"id\":");
        json_str(&mut s, m.id);
        s.push_str(&format!(",\"visible\":{v}}}"));
    }
    s.push_str("]}");
    s
}

fn json_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn json_num(out: &mut String, n: f64) {
    if n.is_finite() {
        // Debug formatting is the shortest round-trip form and valid JSON for finite values.
        out.push_str(&format!("{n:?}"));
    } else {
        out.push_str(&format!("{{\"n\":\"{}\"}}", if n.is_nan() { "NaN" } else if n > 0.0 { "Infinity" } else { "-Infinity" }));
    }
}

fn json_value(out: &mut String, v: &Value) {
    match v {
        Value::Num(n) => json_num(out, *n),
        Value::Str(s) => json_str(out, &s.to_rust_string()),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
    }
}
