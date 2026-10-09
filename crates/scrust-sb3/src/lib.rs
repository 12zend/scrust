//! `.sb3` loader: zip + `project.json` -> a raw, faithful AST.
//!
//! Everything that depends on JavaScript object key order (block order,
//! input evaluation order, variable lookup order) is normalised here to the
//! order scratch-vm would observe: integer-like keys first (ascending), then
//! the remaining keys in document order.

use scrust_value::{JsStr, Value};
use serde_json::Value as J;
use std::collections::HashMap;
use std::io::Read;

#[derive(Debug)]
pub enum Sb3Error {
    Io(std::io::Error),
    Zip(String),
    Json(String),
    Format(String),
}
impl std::fmt::Display for Sb3Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Sb3Error::Io(e) => write!(f, "io error: {e}"),
            Sb3Error::Zip(e) => write!(f, "zip error: {e}"),
            Sb3Error::Json(e) => write!(f, "json error: {e}"),
            Sb3Error::Format(e) => write!(f, "invalid project: {e}"),
        }
    }
}
impl std::error::Error for Sb3Error {}
impl From<std::io::Error> for Sb3Error {
    fn from(e: std::io::Error) -> Self {
        Sb3Error::Io(e)
    }
}

#[derive(Debug, Clone, Default)]
pub struct Project {
    pub targets: Vec<Target>,
    pub monitors: Vec<Monitor>,
    pub extensions: Vec<String>,
    /// Asset file name (md5ext) -> bytes. Empty when loaded from bare JSON.
    pub assets: HashMap<String, Vec<u8>>,
}

#[derive(Debug, Clone, Default)]
pub struct Target {
    pub is_stage: bool,
    pub name: String,
    pub variables: Vec<Variable>,
    pub lists: Vec<List>,
    pub broadcasts: Vec<(String, String)>,
    /// Blocks in scratch-vm `_blocks` iteration order.
    pub blocks: Vec<Block>,
    pub block_index: HashMap<String, usize>,
    pub costumes: Vec<Costume>,
    pub sounds: Vec<Sound>,
    pub current_costume: usize,
    pub layer_order: i64,
    pub volume: f64,
    pub visible: bool,
    pub x: f64,
    pub y: f64,
    pub size: f64,
    pub direction: f64,
    pub draggable: bool,
    pub rotation_style: String,
    pub tempo: f64,
    pub comments: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Variable {
    pub id: String,
    pub name: String,
    pub value: Value,
    pub cloud: bool,
}

#[derive(Debug, Clone)]
pub struct List {
    pub id: String,
    pub name: String,
    pub items: Vec<Value>,
}

#[derive(Debug, Clone, Default)]
pub struct Costume {
    pub name: String,
    pub md5ext: String,
    pub data_format: String,
    pub bitmap_resolution: f64,
    pub rotation_center_x: f64,
    pub rotation_center_y: f64,
}

#[derive(Debug, Clone, Default)]
pub struct Sound {
    pub name: String,
    pub md5ext: String,
    pub data_format: String,
    pub rate: f64,
    pub sample_count: f64,
}

#[derive(Debug, Clone, Default)]
pub struct Monitor {
    pub id: String,
    pub mode: String,
    pub opcode: String,
    pub params: Vec<(String, String)>,
    pub sprite_name: Option<String>,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub visible: bool,
    pub slider_min: f64,
    pub slider_max: f64,
    pub is_discrete: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Block {
    pub id: String,
    pub opcode: String,
    pub next: Option<String>,
    pub parent: Option<String>,
    /// Inputs in JS key order (this is the evaluation order).
    pub inputs: Vec<(String, Input)>,
    pub fields: Vec<(String, Field)>,
    pub shadow: bool,
    pub top_level: bool,
    pub mutation: Option<Mutation>,
}

impl Block {
    pub fn input(&self, name: &str) -> Option<&Input> {
        self.inputs.iter().find(|(n, _)| n == name).map(|(_, i)| i)
    }
    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|(n, _)| n == name).map(|(_, f)| f)
    }
}

#[derive(Debug, Clone)]
pub struct Field {
    pub value: Value,
    pub id: Option<String>,
}

/// What an input slot evaluates (the `block` of scratch-vm's input; an
/// obscured shadow is irrelevant to execution).
#[derive(Debug, Clone)]
pub enum Input {
    /// Reference to another block by id.
    Block(String),
    /// Inline literal (`math_number`, `text`, `colour_picker`, ...).
    Literal(Value),
    /// `[11, name, id]`
    Broadcast { name: String, id: Option<String> },
    /// `[12, name, id]`
    Variable { name: String, id: String },
    /// `[13, name, id]`
    List { name: String, id: String },
    /// `[1, null]` and friends.
    Empty,
}

#[derive(Debug, Clone, Default)]
pub struct Mutation {
    pub proccode: String,
    pub argument_ids: Vec<String>,
    pub argument_names: Vec<String>,
    /// JSON-decoded defaults (strings, booleans or numbers).
    pub argument_defaults: Vec<Value>,
    pub has_defaults: bool,
    pub warp: bool,
    pub has_next: Option<bool>,
}

/// Is `k` a canonical array index (`"0"`..`"4294967294"`)?
fn array_index(k: &str) -> Option<u32> {
    if k.is_empty() || k.len() > 10 || (k.len() > 1 && k.starts_with('0')) {
        return None;
    }
    if !k.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: u64 = k.parse().ok()?;
    if n <= 4294967294 {
        Some(n as u32)
    } else {
        None
    }
}

/// Entries of a JSON object in JavaScript property enumeration order.
pub fn js_entries(map: &serde_json::Map<String, J>) -> Vec<(&String, &J)> {
    let mut ints: Vec<(u32, &String, &J)> = Vec::new();
    let mut rest: Vec<(&String, &J)> = Vec::new();
    for (k, v) in map {
        match array_index(k) {
            Some(n) => ints.push((n, k, v)),
            None => rest.push((k, v)),
        }
    }
    ints.sort_by_key(|e| e.0);
    ints.into_iter().map(|(_, k, v)| (k, v)).chain(rest).collect()
}

/// JSON scalar -> Scratch value. Numbers stay numbers, strings stay strings.
pub fn json_to_value(j: &J) -> Value {
    match j {
        J::String(s) => Value::Str(json_str(s)),
        J::Number(n) => Value::Num(n.as_f64().unwrap_or(0.0)),
        J::Bool(b) => Value::Bool(*b),
        // null / objects / arrays never appear as values in valid projects.
        J::Null => Value::Num(0.0),
        other => Value::Str(JsStr::from(other.to_string().as_str())),
    }
}

/// Lone surrogates (`"\ud83d"`) are legal in JS strings and therefore in
/// project.json, but serde_json rejects them. They are smuggled through as
/// supplementary private-use characters and restored here.
const LONE_BASE: u32 = 0xF0000;

fn json_str(s: &str) -> JsStr {
    if !s.chars().any(|c| (LONE_BASE..LONE_BASE + 0x800).contains(&(c as u32))) {
        return JsStr::from(s);
    }
    let mut units: Vec<u16> = Vec::with_capacity(s.len());
    for c in s.chars() {
        let cp = c as u32;
        if (LONE_BASE..LONE_BASE + 0x800).contains(&cp) {
            units.push((0xD800 + (cp - LONE_BASE)) as u16);
        } else {
            let mut buf = [0u16; 2];
            units.extend_from_slice(c.encode_utf16(&mut buf));
        }
    }
    JsStr::from_utf16(&units)
}

/// Rewrite unpaired `\uD800`-`\uDFFF` escapes into escapes of private-use
/// characters (see `json_str`). Returns `None` when nothing had to change.
fn escape_lone_surrogates(text: &str) -> Option<String> {
    let b = text.as_bytes();
    let hex4 = |i: usize| -> Option<u32> {
        if i + 6 <= b.len() && b[i] == b'\\' && b[i + 1] == b'u' {
            std::str::from_utf8(&b[i + 2..i + 6]).ok().and_then(|h| u32::from_str_radix(h, 16).ok())
        } else {
            None
        }
    };
    let mut out: Option<String> = None;
    let mut copied = 0;
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'\\' {
            i += 1;
            continue;
        }
        match hex4(i) {
            Some(u) if (0xD800..0xDC00).contains(&u) => {
                // High surrogate: fine when followed by a low surrogate escape.
                if matches!(hex4(i + 6), Some(l) if (0xDC00..0xE000).contains(&l)) {
                    i += 12;
                    continue;
                }
                let o = out.get_or_insert_with(String::new);
                o.push_str(&text[copied..i]);
                push_pua(o, u);
                i += 6;
                copied = i;
            }
            Some(u) if (0xDC00..0xE000).contains(&u) => {
                let o = out.get_or_insert_with(String::new);
                o.push_str(&text[copied..i]);
                push_pua(o, u);
                i += 6;
                copied = i;
            }
            Some(_) => i += 6,
            // Any other escape: skip the escaped character too (handles `\\\\`).
            None => i += 2,
        }
    }
    out.map(|mut o| {
        o.push_str(&text[copied..]);
        o
    })
}

fn push_pua(out: &mut String, lone: u32) {
    let cp = LONE_BASE + (lone - 0xD800) - 0x10000;
    out.push_str(&format!("\\u{:04x}\\u{:04x}", 0xD800 + (cp >> 10), 0xDC00 + (cp & 0x3FF)));
}

fn f(j: Option<&J>, default: f64) -> f64 {
    j.and_then(|v| v.as_f64()).unwrap_or(default)
}
fn s(j: Option<&J>) -> String {
    j.and_then(|v| v.as_str()).unwrap_or("").to_string()
}
fn opt_s(j: Option<&J>) -> Option<String> {
    j.and_then(|v| v.as_str()).map(|x| x.to_string())
}

impl Project {
    /// Load an `.sb3` archive (or a bare `project.json`).
    pub fn load(bytes: &[u8]) -> Result<Project, Sb3Error> {
        if bytes.starts_with(b"PK") {
            let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| Sb3Error::Zip(e.to_string()))?;
            let mut json = String::new();
            let mut assets = HashMap::new();
            for i in 0..zip.len() {
                let mut file = zip.by_index(i).map_err(|e| Sb3Error::Zip(e.to_string()))?;
                let name = file.name().rsplit('/').next().unwrap_or("").to_string();
                if name == "project.json" {
                    file.read_to_string(&mut json)?;
                } else if !name.is_empty() {
                    let mut data = Vec::new();
                    file.read_to_end(&mut data)?;
                    assets.insert(name, data);
                }
            }
            if json.is_empty() {
                return Err(Sb3Error::Format("project.json not found in archive".into()));
            }
            let mut p = Project::from_json(&json)?;
            p.assets = assets;
            Ok(p)
        } else {
            let text = std::str::from_utf8(bytes).map_err(|e| Sb3Error::Json(e.to_string()))?;
            Project::from_json(text)
        }
    }

    pub fn from_json(text: &str) -> Result<Project, Sb3Error> {
        let fixed = escape_lone_surrogates(text);
        let text = fixed.as_deref().unwrap_or(text);
        let root: J = serde_json::from_str(text).map_err(|e| Sb3Error::Json(e.to_string()))?;
        let targets_json =
            root.get("targets").and_then(|t| t.as_array()).ok_or_else(|| Sb3Error::Format("missing targets (not an sb3 project?)".into()))?;
        let mut targets: Vec<Target> = targets_json.iter().map(parse_target).collect();
        // Restore layer order (stage is always at the back); stable for ties.
        targets.sort_by_key(|t| (!t.is_stage, t.layer_order));
        if !targets.first().map(|t| t.is_stage).unwrap_or(false) {
            return Err(Sb3Error::Format("project has no stage".into()));
        }
        let monitors = root.get("monitors").and_then(|m| m.as_array()).map(|a| a.iter().map(parse_monitor).collect()).unwrap_or_default();
        let extensions = root
            .get("extensions")
            .and_then(|m| m.as_array())
            .map(|a| a.iter().filter_map(|e| e.as_str().map(String::from)).collect())
            .unwrap_or_default();
        Ok(Project { targets, monitors, extensions, assets: HashMap::new() })
    }
}

fn parse_monitor(j: &J) -> Monitor {
    let params = j
        .get("params")
        .and_then(|p| p.as_object())
        .map(|o| o.iter().map(|(k, v)| (k.clone(), v.as_str().map(String::from).unwrap_or_else(|| v.to_string()))).collect())
        .unwrap_or_default();
    Monitor {
        id: s(j.get("id")),
        mode: s(j.get("mode")),
        opcode: s(j.get("opcode")),
        params,
        sprite_name: opt_s(j.get("spriteName")),
        x: f(j.get("x"), 0.0),
        y: f(j.get("y"), 0.0),
        width: f(j.get("width"), 0.0),
        height: f(j.get("height"), 0.0),
        visible: j.get("visible").and_then(|v| v.as_bool()).unwrap_or(false),
        slider_min: f(j.get("sliderMin"), 0.0),
        slider_max: f(j.get("sliderMax"), 100.0),
        is_discrete: j.get("isDiscrete").and_then(|v| v.as_bool()).unwrap_or(true),
    }
}

fn parse_target(j: &J) -> Target {
    let mut t = Target {
        is_stage: j.get("isStage").and_then(|v| v.as_bool()).unwrap_or(false),
        name: s(j.get("name")),
        current_costume: f(j.get("currentCostume"), 0.0).max(0.0) as usize,
        layer_order: j.get("layerOrder").and_then(|v| v.as_i64()).unwrap_or(0),
        volume: f(j.get("volume"), 100.0),
        visible: j.get("visible").and_then(|v| v.as_bool()).unwrap_or(true),
        x: f(j.get("x"), 0.0),
        y: f(j.get("y"), 0.0),
        size: f(j.get("size"), 100.0),
        direction: f(j.get("direction"), 90.0),
        draggable: j.get("draggable").and_then(|v| v.as_bool()).unwrap_or(false),
        rotation_style: j.get("rotationStyle").and_then(|v| v.as_str()).unwrap_or("all around").to_string(),
        tempo: f(j.get("tempo"), 60.0),
        ..Default::default()
    };
    if let Some(vars) = j.get("variables").and_then(|v| v.as_object()) {
        for (id, v) in js_entries(vars) {
            let arr = v.as_array();
            let name = arr.and_then(|a| a.first()).and_then(|n| n.as_str()).unwrap_or("").to_string();
            let value = arr.and_then(|a| a.get(1)).map(json_to_value).unwrap_or(Value::Num(0.0));
            let cloud = arr.and_then(|a| a.get(2)).and_then(|c| c.as_bool()).unwrap_or(false);
            t.variables.push(Variable { id: id.clone(), name, value, cloud });
        }
    }
    if let Some(lists) = j.get("lists").and_then(|v| v.as_object()) {
        for (id, v) in js_entries(lists) {
            let arr = v.as_array();
            let name = arr.and_then(|a| a.first()).and_then(|n| n.as_str()).unwrap_or("").to_string();
            let items =
                arr.and_then(|a| a.get(1)).and_then(|i| i.as_array()).map(|i| i.iter().map(json_to_value).collect()).unwrap_or_default();
            t.lists.push(List { id: id.clone(), name, items });
        }
    }
    if let Some(b) = j.get("broadcasts").and_then(|v| v.as_object()) {
        for (id, v) in js_entries(b) {
            t.broadcasts.push((id.clone(), v.as_str().unwrap_or("").to_string()));
        }
    }
    if let Some(blocks) = j.get("blocks").and_then(|v| v.as_object()) {
        for (id, b) in js_entries(blocks) {
            // Top-level primitive arrays (stray variable reporters) never run.
            if let Some(obj) = b.as_object() {
                let blk = parse_block(id, obj);
                t.block_index.insert(id.clone(), t.blocks.len());
                t.blocks.push(blk);
            }
        }
    }
    if let Some(c) = j.get("costumes").and_then(|v| v.as_array()) {
        for c in c {
            let ext = s(c.get("dataFormat"));
            let md5ext = opt_s(c.get("md5ext")).unwrap_or_else(|| format!("{}.{}", s(c.get("assetId")), ext));
            t.costumes.push(Costume {
                name: s(c.get("name")),
                md5ext,
                data_format: ext,
                bitmap_resolution: f(c.get("bitmapResolution"), 1.0),
                rotation_center_x: f(c.get("rotationCenterX"), 0.0),
                rotation_center_y: f(c.get("rotationCenterY"), 0.0),
            });
        }
    }
    if let Some(c) = j.get("sounds").and_then(|v| v.as_array()) {
        for c in c {
            let ext = s(c.get("dataFormat"));
            let md5ext = opt_s(c.get("md5ext")).unwrap_or_else(|| format!("{}.{}", s(c.get("assetId")), ext));
            t.sounds.push(Sound {
                name: s(c.get("name")),
                md5ext,
                data_format: ext,
                rate: f(c.get("rate"), 0.0),
                sample_count: f(c.get("sampleCount"), 0.0),
            });
        }
    }
    if t.current_costume >= t.costumes.len() {
        t.current_costume = 0;
    }
    t
}

fn parse_block(id: &str, obj: &serde_json::Map<String, J>) -> Block {
    let mut b = Block {
        id: id.to_string(),
        opcode: s(obj.get("opcode")),
        next: opt_s(obj.get("next")),
        parent: opt_s(obj.get("parent")),
        shadow: obj.get("shadow").and_then(|v| v.as_bool()).unwrap_or(false),
        top_level: obj.get("topLevel").and_then(|v| v.as_bool()).unwrap_or(false),
        ..Default::default()
    };
    if let Some(inputs) = obj.get("inputs").and_then(|v| v.as_object()) {
        for (name, desc) in js_entries(inputs) {
            b.inputs.push((name.clone(), parse_input(desc)));
        }
    }
    if let Some(fields) = obj.get("fields").and_then(|v| v.as_object()) {
        for (name, desc) in js_entries(fields) {
            let arr = desc.as_array();
            let value = arr.and_then(|a| a.first()).map(json_to_value).unwrap_or(Value::Str(JsStr::empty()));
            let fid = arr.and_then(|a| a.get(1)).and_then(|v| v.as_str()).map(String::from);
            b.fields.push((name.clone(), Field { value, id: fid }));
        }
    }
    if let Some(m) = obj.get("mutation").and_then(|v| v.as_object()) {
        b.mutation = Some(parse_mutation(m));
    }
    b
}

fn parse_json_array_attr(j: Option<&J>) -> Option<Vec<J>> {
    match j {
        Some(J::String(text)) => serde_json::from_str::<J>(text).ok().and_then(|v| v.as_array().cloned()),
        Some(J::Array(a)) => Some(a.clone()),
        _ => None,
    }
}

fn parse_mutation(m: &serde_json::Map<String, J>) -> Mutation {
    let strs = |j: Option<&J>| -> Vec<String> {
        parse_json_array_attr(j)
            .unwrap_or_default()
            .iter()
            .map(|v| v.as_str().map(String::from).unwrap_or_else(|| v.to_string()))
            .collect()
    };
    let defaults = parse_json_array_attr(m.get("argumentdefaults"));
    let flag = |j: Option<&J>| -> Option<bool> {
        match j {
            Some(J::Bool(b)) => Some(*b),
            Some(J::String(text)) => serde_json::from_str::<J>(text).ok().and_then(|v| v.as_bool()),
            _ => None,
        }
    };
    Mutation {
        proccode: s(m.get("proccode")),
        argument_ids: strs(m.get("argumentids")),
        argument_names: strs(m.get("argumentnames")),
        has_defaults: defaults.is_some(),
        argument_defaults: defaults.unwrap_or_default().iter().map(json_to_value).collect(),
        warp: flag(m.get("warp")).unwrap_or(false),
        has_next: flag(m.get("hasnext")),
    }
}

fn parse_input(desc: &J) -> Input {
    let Some(arr) = desc.as_array() else { return Input::Empty };
    // [shadowKind, block, shadow?]: execution always uses element 1.
    match arr.get(1) {
        Some(J::String(id)) => Input::Block(id.clone()),
        Some(J::Array(prim)) => parse_primitive(prim),
        _ => Input::Empty,
    }
}

fn parse_primitive(prim: &[J]) -> Input {
    let code = prim.first().and_then(|c| c.as_i64()).unwrap_or(0);
    match code {
        4..=10 => Input::Literal(prim.get(1).map(json_to_value).unwrap_or(Value::Str(JsStr::empty()))),
        11 => Input::Broadcast { name: s(prim.get(1)), id: opt_s(prim.get(2)) },
        12 => Input::Variable { name: s(prim.get(1)), id: s(prim.get(2)) },
        13 => Input::List { name: s(prim.get(1)), id: s(prim.get(2)) },
        _ => Input::Empty,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_key_order() {
        let j: J = serde_json::from_str(r#"{"b":1,"10":2,"a":3,"2":4,"01":5,"4294967295":6}"#).unwrap();
        let keys: Vec<&str> = js_entries(j.as_object().unwrap()).iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["2", "10", "b", "a", "01", "4294967295"]);
    }

    #[test]
    fn lone_surrogates_survive() {
        let p = Project::from_json(
            r#"{"targets":[{"isStage":true,"name":"Stage","variables":{"v":["x","a\ud83db\ud83d\ude00c\udc00\\ud83d"]}}]}"#,
        )
        .unwrap();
        let Value::Str(s) = &p.targets[0].variables[0].value else { panic!() };
        let expect: Vec<u16> = vec![0x61, 0xD83D, 0x62, 0xD83D, 0xDE00, 0x63, 0xDC00, 0x5C, 0x75, 0x64, 0x38, 0x33, 0x64];
        assert_eq!(s.to_utf16(), expect);
    }

    #[test]
    fn literals_keep_their_type() {
        let p = Project::from_json(
            r#"{"targets":[{"isStage":true,"name":"Stage","variables":{"v":["x","007"],"w":["y",7]},
            "blocks":{"a":{"opcode":"data_setvariableto","next":null,"parent":null,"topLevel":true,
            "inputs":{"VALUE":[1,[10,"1.0"]]},"fields":{"VARIABLE":["x","v"]}}}}]}"#,
        )
        .unwrap();
        let st = &p.targets[0];
        assert!(matches!(&st.variables[0].value, Value::Str(s) if *s == "007"));
        assert!(matches!(&st.variables[1].value, Value::Num(n) if *n == 7.0));
        assert!(matches!(st.blocks[0].input("VALUE"), Some(Input::Literal(Value::Str(s))) if *s == "1.0"));
    }
}
