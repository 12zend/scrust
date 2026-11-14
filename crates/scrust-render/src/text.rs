//! Text overlays (say/think bubbles, monitors, the ask prompt), drawn as
//! small SVG documents and rasterised with resvg.

use crate::skin::Raster;
use resvg::tiny_skia;
use resvg::usvg;
use scrust_runtime::MonitorDef;
use scrust_value::{js_number_to_string, js_to_fixed_string, ToStr, Value};

/// Fonts compiled into the binary (all SIL OFL, see `fonts/OFL-*.txt`). They
/// stand in for the families Scratch's costume editor offers.
pub static EMBEDDED_FONTS: &[&[u8]] = &[
    include_bytes!("../fonts/NotoSans.ttf"),
    // scratch-render-fonts' NotoSans-Medium (the "Sans Serif" costume font),
    // renamed so it does not mix with the variable Noto Sans.
    include_bytes!("../fonts/NotoSans-Medium.ttf"),
    include_bytes!("../fonts/SourceSerif4.ttf"),
    include_bytes!("../fonts/Handlee-Regular.ttf"),
    include_bytes!("../fonts/Knewave-Regular.ttf"),
    include_bytes!("../fonts/Griffy-Regular.ttf"),
];

/// Scratch font family -> embedded family. (`Pixel` has no freely
/// redistributable counterpart here and falls back to the sans font.)
const FAMILIES: [(&str, &str); 7] = [
    ("Sans Serif", "Scratch Sans Serif"),
    ("Serif", "Source Serif 4"),
    ("Handwriting", "Handlee"),
    ("Marker", "Knewave"),
    ("Curly", "Griffy"),
    ("Pixel", "Noto Sans"),
    ("Scratch", "Noto Sans"),
];

/// Rewrite Scratch's font family names in an SVG to the embedded fonts.
pub fn map_font_families(svg: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    let Ok(text) = std::str::from_utf8(svg) else { return std::borrow::Cow::Borrowed(svg) };
    if !text.contains("font-family") {
        return std::borrow::Cow::Borrowed(svg);
    }
    let mut out = String::with_capacity(text.len() + 64);
    let mut rest = text;
    while let Some(i) = rest.find("font-family") {
        let (head, tail) = rest.split_at(i + "font-family".len());
        out.push_str(head);
        // The value runs to the closing quote (attribute) or `;` / quote (style).
        let trimmed = tail.trim_start_matches(|c: char| c == '=' || c == ':' || c.is_whitespace());
        let skipped = tail.len() - trimmed.len();
        out.push_str(&tail[..skipped]);
        let (quote, body) = match trimmed.chars().next() {
            Some(q @ ('"' | '\'')) => (Some(q), &trimmed[1..]),
            _ => (None, trimmed),
        };
        let end = body.find(|c: char| Some(c) == quote || c == ';' || (quote.is_none() && c == '"')).unwrap_or(body.len());
        if let Some(q) = quote {
            out.push(q);
        }
        // A comma-separated family list: map each Scratch family in it (the
        // browser falls through unknown families to the inlined Scratch font).
        let inner = if quote == Some('\'') { '"' } else { '\'' };
        let parts: Vec<String> = body[..end]
            .split(',')
            .map(|part| {
                let name = part.trim().trim_start_matches("&quot;").trim_end_matches("&quot;").trim_matches(|c| c == '\'' || c == '"');
                match FAMILIES.iter().find(|(from, _)| name.eq_ignore_ascii_case(from)) {
                    // Quote the name: CSS identifiers cannot contain a bare "4".
                    Some((_, to)) => format!("{inner}{to}{inner}"),
                    None => part.to_string(),
                }
            })
            .collect();
        out.push_str(&parts.join(","));
        rest = &body[end..];
    }
    out.push_str(rest);
    std::borrow::Cow::Owned(out.into_bytes())
}

pub fn load_fonts(db: &mut usvg::fontdb::Database) {
    for f in EMBEDDED_FONTS {
        db.load_font_data(f.to_vec());
    }
    // System fonts provide glyphs the embedded ones lack (CJK, emoji).
    #[cfg(not(target_arch = "wasm32"))]
    {
        db.load_system_fonts();
        // macOS's LastResort font has a box for every code point; picked as
        // a fallback, it turns whole text runs (e.g. one with an emoji) into
        // boxes. Browsers never draw with it.
        let last: Vec<_> = db.faces().filter(|f| f.families.iter().any(|(n, _)| n.trim_start_matches('.') == "LastResort")).map(|f| f.id).collect();
        for id in last {
            db.remove_face(id);
        }
    }
    db.set_sans_serif_family("Noto Sans");
    db.set_serif_family("Source Serif 4");
}

pub fn configure(opt: &mut usvg::Options) {
    // scratch-svg-renderer's `transformImages` renders embedded images
    // pixelated.
    opt.image_rendering = usvg::ImageRendering::OptimizeSpeed;
    let family = opt.fontdb.faces().next().map(|_| opt.fontdb.family_name(&usvg::fontdb::Family::SansSerif).to_string());
    if let Some(f) = family {
        opt.font_family = f;
    }
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            c if (c as u32) < 0x20 && c != '\n' => o.push(' '),
            c => o.push(c),
        }
    }
    o
}

/// Rough advance width of a character in em.
fn em(c: char) -> f64 {
    match c {
        'i' | 'l' | 'j' | '.' | ',' | ':' | ';' | '!' | '|' | '\'' | 'I' => 0.3,
        ' ' | 'f' | 't' | 'r' | '(' | ')' | '[' | ']' => 0.36,
        'm' | 'w' | 'M' | 'W' | '@' => 0.9,
        c if c.is_ascii_uppercase() => 0.7,
        c if (c as u32) < 0x2E80 => 0.56,
        _ => 1.0,
    }
}

fn width(s: &str, font: f64) -> f64 {
    s.chars().map(em).sum::<f64>() * font
}

/// Greedy word wrap to `max` pixels with a width function.
fn wrap_with(text: &str, max: f64, width_of: &dyn Fn(&str) -> f64) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split(' ') {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if width_of(&candidate) <= max || line.is_empty() {
                line = candidate;
            } else {
                lines.push(std::mem::take(&mut line));
                line = word.to_string();
            }
            // Break words that are longer than a whole line.
            while width_of(&line) > max {
                let mut cut = 0;
                for (i, c) in line.char_indices() {
                    if width_of(&line[..i + c.len_utf8()]) > max {
                        break;
                    }
                    cut = i + c.len_utf8();
                }
                if cut == 0 || cut >= line.len() {
                    break;
                }
                let rest = line.split_off(cut);
                lines.push(std::mem::replace(&mut line, rest));
            }
        }
        lines.push(line);
    }
    lines
}

fn render(svg: &str, opt: &usvg::Options) -> Raster {
    let Ok(tree) = usvg::Tree::from_str(svg, opt) else { return Raster::new(1, 1) };
    let size = tree.size();
    let Some(mut pixmap) = tiny_skia::Pixmap::new((size.width().ceil() as u32).max(1), (size.height().ceil() as u32).max(1)) else {
        return Raster::new(1, 1);
    };
    resvg::render(&tree, tiny_skia::Transform::identity(), &mut pixmap.as_mut());
    Raster::from_pixmap(&pixmap)
}

fn svg_open(w: f64, h: f64, scale: f64) -> String {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="0 0 {} {}" font-family="sans-serif">"#,
        (w * scale).ceil(),
        (h * scale).ceil(),
        w,
        h
    )
}

/// scratch-render's `TextBubbleSkin` style.
pub const BUBBLE_MAX_LINE: f64 = 170.0;
const BUBBLE_MIN_WIDTH: f64 = 50.0;
const BUBBLE_STROKE: f64 = 4.0;
const BUBBLE_PAD: f64 = 10.0;
const BUBBLE_RADIUS: f64 = 16.0;
const BUBBLE_TAIL: f64 = 12.0;
const BUBBLE_FONT: f64 = 14.0;
const BUBBLE_LINE: f64 = 16.0;

/// The bubble font (`14px Helvetica, sans-serif`): (fontdb face, units per em).
fn bubble_face(opt: &usvg::Options) -> Option<usvg::fontdb::ID> {
    use usvg::fontdb::{Family, Query};
    opt.fontdb.query(&Query { families: &[Family::Name("Helvetica"), Family::SansSerif], ..Default::default() })
}

/// `measureText(s).width` for the bubble font.
pub fn bubble_text_width(s: &str, opt: &usvg::Options) -> f64 {
    let Some(id) = bubble_face(opt) else { return width(s, BUBBLE_FONT) };
    opt.fontdb
        .with_face_data(id, |data, index| {
            let face = ttf_parser::Face::parse(data, index).ok()?;
            let upem = face.units_per_em() as f64;
            let mut w = 0.0;
            for c in s.chars() {
                let adv = face.glyph_index(c).and_then(|g| face.glyph_hor_advance(g)).map(|a| a as f64).unwrap_or(upem * em(c));
                w += adv;
            }
            Some(w / upem * BUBBLE_FONT)
        })
        .flatten()
        .unwrap_or_else(|| width(s, BUBBLE_FONT))
}

/// Size of a bubble in stage units (`TextBubbleSkin.size`) and its lines.
pub fn bubble_layout(text: &str, opt: &usvg::Options) -> (Vec<String>, f64, f64) {
    let lines = wrap_with(text, BUBBLE_MAX_LINE, &|l| bubble_text_width(l, opt));
    let longest = lines.iter().map(|l| bubble_text_width(l, opt)).fold(0.0, f64::max);
    let padded_w = longest.max(BUBBLE_MIN_WIDTH) + BUBBLE_PAD * 2.0;
    let padded_h = BUBBLE_LINE * lines.len() as f64 + BUBBLE_PAD * 2.0;
    (lines, padded_w, padded_h)
}

/// `TextBubbleSkin._renderTextBubble`.
pub fn bubble(text: &str, think: bool, points_left: bool, scale: f64, opt: &usvg::Options) -> Raster {
    let (lines, pw, ph) = bubble_layout(text, opt);
    let (w, h) = (pw + BUBBLE_STROKE, ph + BUBBLE_STROKE + BUBBLE_TAIL);
    let r = BUBBLE_RADIUS;
    let mut s = svg_open(w, h, scale);
    let flip = if points_left { format!(r#" transform="translate({pw},0) scale(-1,1)""#) } else { String::new() };
    // Rounded body (canvas `arcTo`s) and the tail, translated to (pw - r, ph).
    let (tx, ty) = (pw - r, ph);
    let tail = if think {
        format!(
            "A 4 4 0 0 1 {} {} Z M {} {} A 2.25 2.25 0 1 1 {} {} A 2.25 2.25 0 1 1 {} {} M {} {} A 1.5 1.5 0 1 1 {} {} A 1.5 1.5 0 1 1 {} {}",
            tx - 20.0, ty,
            tx - 7.0, ty + 7.25, tx - 11.5, ty + 7.25, tx - 7.0, ty + 7.25,
            tx, ty + 9.5, tx - 3.0, ty + 9.5, tx, ty + 9.5
        )
    } else {
        format!(
            "C {} {} {} {} {} {} A 2 2 0 0 1 {} {} C {} {} {} {} {} {} Z",
            tx, ty + 4.0, tx + 4.0, ty + 8.0, tx + 4.0, ty + 10.0,
            tx + 2.0, ty + 12.0,
            tx - 1.0, ty + 12.0, tx - 11.0, ty + 8.0, tx - 16.0, ty
        )
    };
    let think_start = if think { format!(" L {} {}", tx - 12.0, ty) } else { String::new() };
    let d = format!(
        "M {r} {ph} A {r} {r} 0 0 1 0 {a} L 0 {r} A {r} {r} 0 0 1 {r} 0 L {b} 0 A {r} {r} 0 0 1 {pw} {r} L {pw} {a} A {r} {r} 0 0 1 {b} {ph}{think_start} {tail}",
        a = ph - r,
        b = pw - r
    );
    s.push_str(&format!(
        r##"<g transform="translate({o},{o})"><g{flip}><path d="{d}" fill="#ffffff" stroke="rgba(0,0,0,0.15)" stroke-width="{sw}" paint-order="stroke"/></g>"##,
        o = BUBBLE_STROKE * 0.5,
        sw = BUBBLE_STROKE
    ));
    for (i, l) in lines.iter().enumerate() {
        s.push_str(&format!(
            r##"<text x="{}" y="{}" font-family="Helvetica, sans-serif" font-size="{}" fill="#575e75" xml:space="preserve">{}</text>"##,
            BUBBLE_PAD,
            BUBBLE_PAD + BUBBLE_LINE * i as f64 + 0.9 * BUBBLE_FONT,
            BUBBLE_FONT,
            esc(l)
        ));
    }
    s.push_str("</g>");
    s.push_str("</svg>");
    render(&s, opt)
}

#[derive(Debug, Clone)]
pub enum MonitorValue {
    Scalar(String),
    List(Vec<String>),
}

/// scratch-gui rounds numeric monitor values to six decimals.
pub fn format_number(n: f64) -> String {
    if !n.is_finite() {
        return js_number_to_string(n);
    }
    let rounded: f64 = js_to_fixed_string(n, 6).parse().unwrap_or(n);
    js_number_to_string(rounded)
}

pub fn format_value(v: &Value) -> String {
    match v {
        Value::Num(n) => format_number(*n),
        other => other.to_str().to_rust_string(),
    }
}

pub fn monitor(m: &MonitorDef, value: &MonitorValue, scale: f64, opt: &usvg::Options) -> Raster {
    match value {
        MonitorValue::List(items) => list_monitor(m, items, scale, opt),
        MonitorValue::Scalar(v) => scalar_monitor(m, v, scale, opt),
    }
}

fn scalar_monitor(m: &MonitorDef, value: &str, scale: f64, opt: &usvg::Options) -> Raster {
    let color = if m.opcode.starts_with("data_") { "#ff8c1a" } else if m.opcode.starts_with("sensing_") { "#5cb1d6" } else if m.opcode.starts_with("looks_") { "#9966ff" } else if m.opcode.starts_with("sound_") { "#cf63cf" } else { "#4c97ff" };
    if m.mode == "large" {
        let vw = (width(value, 16.0) + 16.0).max(40.0);
        let mut s = svg_open(vw, 24.0, scale);
        s.push_str(&format!(
            r##"<rect width="{vw}" height="24" rx="4" fill="{color}"/><text x="{}" y="17.5" font-size="16" fill="#ffffff" text-anchor="middle">{}</text></svg>"##,
            vw / 2.0,
            esc(value)
        ));
        return render(&s, opt);
    }
    let label_w = width(m.label, 12.0) * 1.08;
    let value_w = (width(value, 12.0) + 10.0).max(40.0);
    let w = label_w + value_w + 21.0;
    let slider = m.mode == "slider";
    let h = if slider { 38.0 } else { 20.0 };
    let mut s = svg_open(w, h, scale);
    s.push_str(&format!(
        r##"<rect x="0.5" y="0.5" width="{}" height="{}" rx="4" fill="#e6f0ff" stroke="#c4ccd9"/><text x="7" y="14.5" font-size="12" font-weight="bold" fill="#575e75" xml:space="preserve">{}</text><rect x="{}" y="3" width="{}" height="14" rx="4" fill="{}"/><text x="{}" y="14" font-size="12" fill="#ffffff" text-anchor="middle" xml:space="preserve">{}</text>"##,
        w - 1.0,
        h - 1.0,
        esc(m.label),
        label_w + 14.0,
        value_w,
        color,
        label_w + 14.0 + value_w / 2.0,
        esc(value)
    ));
    if slider {
        let n: f64 = value.parse().unwrap_or(0.0);
        let span = (m.slider_max - m.slider_min).abs().max(1e-9);
        let frac = ((n - m.slider_min) / span).clamp(0.0, 1.0);
        s.push_str(&format!(
            r##"<rect x="8" y="26" width="{}" height="5" rx="2.5" fill="#c4ccd9"/><circle cx="{}" cy="28.5" r="6" fill="#4c97ff"/>"##,
            w - 16.0,
            8.0 + frac * (w - 16.0)
        ));
    }
    s.push_str("</svg>");
    render(&s, opt)
}

fn list_monitor(m: &MonitorDef, items: &[String], scale: f64, opt: &usvg::Options) -> Raster {
    let w = if m.width > 0.0 { m.width } else { 100.0 };
    let h = if m.height > 0.0 { m.height } else { 200.0 };
    const ROW: f64 = 24.0;
    let rows = (((h - 48.0) / ROW).floor().max(0.0)) as usize;
    let mut s = svg_open(w, h, scale);
    s.push_str(&format!(
        r##"<rect x="0.5" y="0.5" width="{}" height="{}" rx="4" fill="#e6f0ff" stroke="#c4ccd9"/><rect x="0.5" y="0.5" width="{}" height="24" rx="4" fill="#ffffff" stroke="#c4ccd9"/><text x="{}" y="16.5" font-size="12" font-weight="bold" fill="#575e75" text-anchor="middle" xml:space="preserve">{}</text>"##,
        w - 1.0,
        h - 1.0,
        w - 1.0,
        w / 2.0,
        esc(m.label)
    ));
    if items.is_empty() {
        s.push_str(&format!(r##"<text x="{}" y="44" font-size="12" fill="#575e75" text-anchor="middle">(empty)</text>"##, w / 2.0));
    }
    let index_w = 12.0 + (items.len().max(1) as f64).log10().floor() * 7.0;
    for (i, item) in items.iter().take(rows).enumerate() {
        let y = 26.0 + i as f64 * ROW;
        let mut shown = item.clone();
        while width(&shown, 12.0) > w - index_w - 22.0 && !shown.is_empty() {
            shown.pop();
        }
        s.push_str(&format!(
            r##"<text x="{}" y="{}" font-size="12" fill="#575e75" text-anchor="end">{}</text><rect x="{}" y="{}" width="{}" height="20" rx="3" fill="#fc662c"/><text x="{}" y="{}" font-size="12" fill="#ffffff" xml:space="preserve">{}</text>"##,
            index_w + 3.0,
            y + 15.0,
            i + 1,
            index_w + 7.0,
            y + 1.0,
            (w - index_w - 12.0).max(4.0),
            index_w + 11.0,
            y + 15.0,
            esc(&shown)
        ));
    }
    s.push_str(&format!(
        r##"<rect x="0.5" y="{}" width="{}" height="23.5" fill="#ffffff" stroke="#c4ccd9"/><text x="{}" y="{}" font-size="12" fill="#575e75" text-anchor="middle">length {}</text></svg>"##,
        h - 24.0,
        w - 1.0,
        w / 2.0,
        h - 8.0,
        items.len()
    ));
    render(&s, opt)
}

pub fn ask_box(question: &str, input: &str, scale: f64, opt: &usvg::Options) -> Raster {
    let has_q = !question.is_empty();
    let h = if has_q { 74.0 } else { 52.0 };
    let w = 480.0;
    let mut s = svg_open(w, h, scale);
    s.push_str(&format!(r##"<rect x="6.5" y="4.5" width="467" height="{}" rx="8" fill="#ffffff" stroke="#c4ccd9" stroke-width="2"/>"##, h - 10.0));
    let mut y = 12.0;
    if has_q {
        let mut q = question.to_string();
        while width(&q, 13.0) > 440.0 && !q.is_empty() {
            q.pop();
        }
        s.push_str(&format!(r##"<text x="20" y="26" font-size="13" font-weight="bold" fill="#575e75" xml:space="preserve">{}</text>"##, esc(&q)));
        y = 34.0;
    }
    let mut shown: String = input.to_string();
    while width(&shown, 13.0) > 400.0 && !shown.is_empty() {
        shown.remove(0);
    }
    s.push_str(&format!(
        r##"<rect x="16.5" y="{}" width="447" height="28" rx="14" fill="#ffffff" stroke="#4c97ff" stroke-width="2"/><text x="28" y="{}" font-size="13" fill="#575e75" xml:space="preserve">{}|</text><circle cx="448" cy="{}" r="11" fill="#4c97ff"/><path d="M 443 {c} l 3.5 3.5 l 6 -7" fill="none" stroke="#ffffff" stroke-width="2.2" stroke-linecap="round"/></svg>"##,
        y + 0.5,
        y + 19.0,
        esc(&shown),
        y + 14.5,
        c = y + 15.0
    ));
    render(&s, opt)
}
