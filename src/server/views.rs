// What a plugin's view may hold: its shapes (the protocol's viewNode: each element's type and fields), how much of it
// (nodes, depth, bytes), that every action it names is one the plugin offered, its Rasters' and Images' bytes, and its
// text made safe to draw.
use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{json, Map, Value};

use crate::core::text::{clean_text, width};
use crate::protocol::conn::{fail, RpcResult};
use crate::protocol::schema::invalid;

pub struct ViewLimit {
    pub views: usize,
    pub session_views: usize,
    pub nodes: usize,
    pub depth: usize,
    pub bytes: usize,
    pub label: usize,
    pub block: usize,
    pub image: usize,
}

pub const VIEW_LIMIT: ViewLimit = ViewLimit { views: 4, session_views: 8, nodes: 5000, depth: 40, bytes: 2 * 1024 * 1024, label: 200, block: 512 * 1024, image: 4 * 1024 * 1024 };
pub const BLITS: (f64, f64) = (120.0, 60.0); // Raster repaints per run (burst, a second): animation, apart from ui.* updates

static OSC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)?").unwrap());
static CSI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]").unwrap());
static CONTROL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\x00-\x08\x0b-\x1f\x7f-\x9f]|\p{Cf}").unwrap());

// Multi-line text (Markdown, code, a diff, a Text, what a field holds): no escape sequences or control characters, but
// newlines and tabs kept, and at most `chars` of it.
pub fn clean_block(text: &str, chars: usize) -> String {
    let head: String = text.chars().take(chars * 2).collect();
    let s = OSC.replace_all(&head, "");
    let s = CSI.replace_all(&s, "");
    CONTROL.replace_all(&s, "").chars().take(chars).collect()
}

fn unb64(s: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(s).ok()
}

// A Raster's cells: exactly columns × rows [codePoint, fg, bg] triplets, each code point a printable width-1 character.
pub fn check_cells(cells: &str, columns: u64, rows: u64) -> RpcResult<()> {
    let bytes = unb64(cells).unwrap_or_default();
    let want = (columns * rows * 12) as usize;
    if bytes.len() != want {
        return Err(fail("invalid_params", format!("a {columns}×{rows} raster has {want} bytes of cells, not {}", bytes.len())));
    }
    for (i, word) in bytes.chunks_exact(12).enumerate() {
        let cp = u32::from_le_bytes([word[0], word[1], word[2], word[3]]);
        let ok = cp >= 0x20 && !(0x7f..=0x9f).contains(&cp) && !(0xd800..=0xdfff).contains(&cp) && cp <= 0xffff && char::from_u32(cp).is_some_and(|c| width(c.encode_utf8(&mut [0; 4])) == 1);
        if !ok {
            return Err(fail("invalid_params", format!("raster cell {i}: U+{cp:X} isn't a printable one-cell character")));
        }
    }
    Ok(())
}

// ---------- shapes ----------

// A size in cells, or a share of the parent ("50%").
pub fn check_size(path: &str, v: Option<&Value>) -> RpcResult<()> {
    match v {
        None => Ok(()),
        Some(Value::Number(n)) if n.as_f64().is_some_and(|f| f.fract() == 0.0 && (0.0..=1000.0).contains(&f)) => Ok(()),
        Some(Value::String(s)) if s.strip_suffix('%').and_then(|n| n.parse::<f64>().ok()).is_some_and(|n| (0.0..=100.0).contains(&n)) => Ok(()),
        Some(_) => Err(invalid(path, "Invalid input")),
    }
}

const LAYOUT: &[&str] = &["key", "width", "height", "minWidth", "maxWidth", "minHeight", "maxHeight", "grow", "shrink"];
const LOOK: &[&str] = &["tone", "bold", "italic", "underline", "dim", "strike"];
const TONES: &[&str] = &["fg", "dim", "accent", "warn", "working", "blocked", "done", "idle"];

// each element type's own fields
fn fields(kind: &str) -> Option<Vec<&'static str>> {
    let own: &[&str] = match kind {
        "box" => &["direction", "gap", "padding", "paddingX", "paddingY", "align", "justify", "wrap", "border", "title", "tone", "bg", "children"],
        "scroll" => &["sticky", "children"],
        "text" => &["tone", "bold", "italic", "underline", "dim", "strike", "wrap", "children"],
        "markdown" => &["content"],
        "code" => &["content", "filetype", "lineNumbers"],
        "diff" => &["action", "params", "diff", "view", "filetype", "lineNumbers", "cursor", "marks", "change"],
        "table" => &["rows", "header", "border"],
        "bigtext" => &["text", "font", "tone"],
        "progress" => &["value", "tone"],
        "sparkline" => &["values", "tone", "min", "max"],
        "chart" => &["series", "min", "max"],
        "gauge" => &["value", "tone", "label"],
        "heatmap" => &["values", "tone", "min", "max"],
        "raster" => &["key", "columns", "rows", "cells"],
        "image" => &["png", "alt", "fit"],
        "spinner" => &["tone", "label"],
        "button" => &["action", "params", "label", "tone"],
        "input" => &["action", "params", "placeholder", "value", "maxLength"],
        "textarea" => &["action", "params", "placeholder", "value"],
        "select" => &["action", "params", "options", "selected", "change"],
        "tabs" => &["action", "params", "options", "selected"],
        _ => return None,
    };
    Some(LAYOUT.iter().chain(own).copied().chain(std::iter::once("type")).collect())
}

struct Shape<'a> {
    m: &'a Map<String, Value>,
    path: String,
}

impl Shape<'_> {
    fn at(&self, k: &str) -> String {
        format!("{}.{k}", self.path)
    }
    fn get(&self, k: &str) -> Option<&Value> {
        self.m.get(k)
    }
    fn string(&self, k: &str, required: bool, max: Option<usize>) -> RpcResult<()> {
        match self.get(k) {
            None if !required => Ok(()),
            Some(Value::String(s)) if max.is_none_or(|m| s.chars().count() <= m) && (!required || k != "key" || !s.is_empty()) => Ok(()),
            Some(Value::String(_)) => Err(invalid(&self.at(k), "Too big")),
            other => Err(invalid(&self.at(k), format!("Invalid input: expected string, received {}", kind(other)))),
        }
    }
    fn number(&self, k: &str, required: bool, lo: f64, hi: f64, int: bool) -> RpcResult<()> {
        match self.get(k) {
            None if !required => Ok(()),
            Some(Value::Number(n)) => {
                let f = n.as_f64().unwrap_or(f64::NAN);
                if (int && f.fract() != 0.0) || !(lo..=hi).contains(&f) {
                    return Err(invalid(&self.at(k), "Invalid input"));
                }
                Ok(())
            }
            other => Err(invalid(&self.at(k), format!("Invalid input: expected number, received {}", kind(other)))),
        }
    }
    fn boolean(&self, k: &str) -> RpcResult<()> {
        match self.get(k) {
            None | Some(Value::Bool(_)) => Ok(()),
            other => Err(invalid(&self.at(k), format!("Invalid input: expected boolean, received {}", kind(other)))),
        }
    }
    fn one_of(&self, k: &str, options: &[&str]) -> RpcResult<()> {
        match self.get(k) {
            None => Ok(()),
            Some(Value::String(s)) if options.contains(&s.as_str()) => Ok(()),
            _ => Err(invalid(&self.at(k), format!("Invalid option: expected one of {}", options.iter().map(|o| format!("\"{o}\"")).collect::<Vec<_>>().join("|")))),
        }
    }
    fn numbers(&self, k: &str, v: Option<&Value>, max: usize) -> RpcResult<()> {
        match v {
            Some(Value::Array(a)) if a.len() <= max && a.iter().all(Value::is_number) => Ok(()),
            _ => Err(invalid(&self.at(k), "Invalid input")),
        }
    }
    fn options(&self, max: usize) -> RpcResult<()> {
        let Some(Value::Array(a)) = self.get("options") else { return Err(invalid(&self.at("options"), "Invalid input: expected array")) };
        if a.len() > max {
            return Err(invalid(&self.at("options"), format!("Too big: expected array to have <={max} items")));
        }
        for (i, o) in a.iter().enumerate() {
            let Value::Object(m) = o else { return Err(invalid(&format!("{}.options.{i}", self.path), "Invalid input: expected object")) };
            let s = Shape { m, path: format!("{}.options.{i}", self.path) };
            if let Some(k) = m.keys().find(|k| !["name", "description", "value"].contains(&k.as_str())) {
                return Err(invalid(&s.path, format!("Unrecognized key: \"{k}\"")));
            }
            s.string("name", true, None)?;
            s.string("description", false, None)?;
            s.string("value", false, None)?;
        }
        Ok(())
    }
}

fn kind(v: Option<&Value>) -> &'static str {
    match v {
        None => "undefined",
        Some(Value::Null) => "null",
        Some(Value::Bool(_)) => "boolean",
        Some(Value::Number(_)) => "number",
        Some(Value::String(_)) => "string",
        Some(Value::Array(_)) => "array",
        Some(Value::Object(_)) => "object",
    }
}

fn check_inline(v: &Value, path: &str) -> RpcResult<()> {
    match v {
        Value::String(_) => Ok(()),
        Value::Object(m) => match m.get("type").and_then(Value::as_str) {
            Some("icon") => {
                let s = Shape { m, path: path.into() };
                s.string("agent", true, Some(40))
            }
            Some("span") => {
                let s = Shape { m, path: path.into() };
                s.one_of("tone", TONES)?;
                for b in &LOOK[1..] {
                    s.boolean(b)?;
                }
                match m.get("children") {
                    None => Ok(()),
                    Some(Value::Array(c)) => c.iter().enumerate().try_for_each(|(i, x)| check_inline(x, &format!("{path}.children.{i}"))),
                    Some(_) => Err(invalid(&format!("{path}.children"), "Invalid input: expected array")),
                }
            }
            _ => Err(invalid(path, "Invalid input")),
        },
        _ => Err(invalid(path, "Invalid input")),
    }
}

// The element tree's shapes, as the protocol's viewNode says them.
pub fn check_shape(v: &Value, path: &str) -> RpcResult<()> {
    let Value::Object(m) = v else { return Err(invalid(path, format!("Invalid input: expected object, received {}", kind(Some(v))))) };
    let t = m.get("type").and_then(Value::as_str).unwrap_or("");
    let Some(allowed) = fields(t) else { return Err(invalid(&format!("{path}.type"), "Invalid input")) };
    if let Some(k) = m.keys().find(|k| !allowed.contains(&k.as_str())) {
        return Err(invalid(path, format!("Unrecognized key: \"{k}\"")));
    }
    let s = Shape { m, path: path.to_string() };
    s.string("key", false, Some(80))?;
    check_size(&s.at("width"), m.get("width"))?;
    check_size(&s.at("height"), m.get("height"))?;
    for k in ["minWidth", "maxWidth", "minHeight", "maxHeight"] {
        s.number(k, false, 0.0, 1000.0, true)?;
    }
    s.number("grow", false, 0.0, 100.0, false)?;
    s.number("shrink", false, 0.0, 100.0, false)?;
    if allowed.contains(&"action") {
        s.string("action", false, None)?;
        if m.get("params").is_some_and(|p| !p.is_object()) {
            return Err(invalid(&s.at("params"), "Invalid input: expected record"));
        }
    }
    if allowed.contains(&"tone") {
        s.one_of("tone", TONES)?;
    }
    let children = |s: &Shape| -> RpcResult<()> {
        match s.get("children") {
            None => Ok(()),
            Some(Value::Array(c)) => c.iter().enumerate().try_for_each(|(i, x)| check_shape(x, &format!("{}.children.{i}", s.path))),
            Some(other) => Err(invalid(&s.at("children"), format!("Invalid input: expected array, received {}", kind(Some(other))))),
        }
    };
    match t {
        "box" => {
            s.one_of("direction", &["row", "column"])?;
            for k in ["gap", "padding", "paddingX", "paddingY"] {
                s.number(k, false, 0.0, 1000.0, true)?;
            }
            s.one_of("align", &["start", "center", "end", "stretch"])?;
            s.one_of("justify", &["start", "center", "end", "between", "around", "evenly"])?;
            s.boolean("wrap")?;
            if !matches!(m.get("border"), None | Some(Value::Bool(_))) {
                s.one_of("border", &["single", "double", "rounded", "heavy"])?;
            }
            s.string("title", false, None)?;
            s.one_of("bg", TONES)?;
            children(&s)
        }
        "scroll" => {
            s.one_of("sticky", &["top", "bottom"])?;
            children(&s)
        }
        "text" => {
            for b in &LOOK[1..] {
                s.boolean(b)?;
            }
            s.one_of("wrap", &["word", "char", "none"])?;
            match m.get("children") {
                None => Ok(()),
                Some(Value::Array(c)) => c.iter().enumerate().try_for_each(|(i, x)| check_inline(x, &format!("{path}.children.{i}"))),
                Some(_) => Err(invalid(&s.at("children"), "Invalid input: expected array")),
            }
        }
        "markdown" => s.string("content", true, None),
        "code" => {
            s.string("content", true, None)?;
            s.string("filetype", false, Some(40))?;
            s.boolean("lineNumbers")
        }
        "diff" => {
            s.string("diff", true, None)?;
            s.one_of("view", &["unified", "split"])?;
            s.string("filetype", false, Some(40))?;
            s.boolean("lineNumbers")?;
            s.boolean("cursor")?;
            s.string("change", false, None)?;
            match m.get("marks") {
                None => Ok(()),
                Some(Value::Array(a)) if a.len() <= 10_000 && a.iter().all(|x| x.as_u64().is_some()) => Ok(()),
                _ => Err(invalid(&s.at("marks"), "Invalid input")),
            }
        }
        "table" => {
            let Some(Value::Array(rows)) = m.get("rows") else { return Err(invalid(&s.at("rows"), "Invalid input: expected array")) };
            if rows.len() > 500 {
                return Err(invalid(&s.at("rows"), "Too big: expected array to have <=500 items"));
            }
            for (i, r) in rows.iter().enumerate() {
                let Value::Array(cells) = r else { return Err(invalid(&format!("{path}.rows.{i}"), "Invalid input: expected array")) };
                for (j, c) in cells.iter().enumerate() {
                    match c {
                        Value::String(_) => {}
                        Value::Array(parts) => parts.iter().enumerate().try_for_each(|(k, x)| check_inline(x, &format!("{path}.rows.{i}.{j}.{k}")))?,
                        _ => return Err(invalid(&format!("{path}.rows.{i}.{j}"), "Invalid input")),
                    }
                }
            }
            s.boolean("header")?;
            s.boolean("border")
        }
        "bigtext" => {
            s.string("text", true, None)?;
            s.one_of("font", &["tiny", "block", "shade", "slick", "huge", "grid", "pallet"])
        }
        "progress" | "gauge" => {
            s.number("value", true, f64::MIN, f64::MAX, false)?;
            s.string("label", false, None)
        }
        "sparkline" => {
            s.numbers("values", m.get("values"), 4096)?;
            s.number("min", false, f64::MIN, f64::MAX, false)?;
            s.number("max", false, f64::MIN, f64::MAX, false)
        }
        "chart" => {
            let Some(Value::Array(series)) = m.get("series") else { return Err(invalid(&s.at("series"), "Invalid input: expected array")) };
            if series.len() > 8 {
                return Err(invalid(&s.at("series"), "Too big: expected array to have <=8 items"));
            }
            for (i, x) in series.iter().enumerate() {
                let Value::Object(sm) = x else { return Err(invalid(&format!("{path}.series.{i}"), "Invalid input")) };
                let ss = Shape { m: sm, path: format!("{path}.series.{i}") };
                ss.numbers("values", sm.get("values"), 4096)?;
                ss.one_of("tone", TONES)?;
            }
            s.number("min", false, f64::MIN, f64::MAX, false)?;
            s.number("max", false, f64::MIN, f64::MAX, false)
        }
        "heatmap" => {
            let Some(Value::Array(rows)) = m.get("values") else { return Err(invalid(&s.at("values"), "Invalid input: expected array")) };
            if rows.len() > 256 {
                return Err(invalid(&s.at("values"), "Too big: expected array to have <=256 items"));
            }
            for r in rows {
                s.numbers("values", Some(r), 4096)?;
            }
            Ok(())
        }
        "raster" => {
            s.string("key", true, Some(80))?;
            s.number("columns", true, 1.0, 512.0, true)?;
            s.number("rows", true, 1.0, 256.0, true)?;
            s.string("cells", true, None)
        }
        "image" => {
            s.string("png", true, None)?;
            s.string("alt", false, None)?;
            s.one_of("fit", &["fit", "cover", "fill"])
        }
        "spinner" => s.string("label", false, None),
        "button" => s.string("label", true, None),
        "input" | "textarea" => {
            s.string("placeholder", false, None)?;
            s.string("value", false, None)?;
            s.number("maxLength", false, 1.0, 100_000.0, true)
        }
        "select" => {
            s.options(1000)?;
            s.number("selected", false, 0.0, f64::MAX, true)?;
            s.string("change", false, None)
        }
        "tabs" => {
            s.options(50)?;
            s.number("selected", false, 0.0, f64::MAX, true)
        }
        _ => Ok(()),
    }
}

// Keys a view binds while it has focus: at most 40, each a key and the action it runs.
pub fn check_keys(keys: &Value) -> RpcResult<()> {
    let Value::Array(a) = keys else { return Err(invalid("keys", "Invalid input: expected array")) };
    if a.len() > 40 {
        return Err(invalid("keys", "Too big: expected array to have <=40 items"));
    }
    for (i, k) in a.iter().enumerate() {
        let Value::Object(m) = k else { return Err(invalid(&format!("keys.{i}"), "Invalid input: expected object")) };
        let s = Shape { m, path: format!("keys.{i}") };
        if let Some(x) = m.keys().find(|x| !["key", "action", "params", "description"].contains(&x.as_str())) {
            return Err(invalid(&s.path, format!("Unrecognized key: \"{x}\"")));
        }
        s.string("key", true, Some(20))?;
        s.string("action", true, None)?;
        s.string("description", false, None)?;
    }
    Ok(())
}

const PNG: [u8; 8] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];

// A view as the plugin sent it, checked and cleaned: what's drawn, its keys, and its Rasters' sizes by key (for blits).
pub fn check_view(root: &Value, keys: &Value, close: Option<&str>, offered: &[String]) -> RpcResult<(Value, Value, HashMap<String, (u64, u64)>)> {
    let size = root.to_string().len();
    if size > VIEW_LIMIT.bytes {
        return Err(fail("invalid_params", format!("a view is at most {} MB of JSON; this one is {:.1} MB", VIEW_LIMIT.bytes / 1024 / 1024, size as f64 / 1024.0 / 1024.0)));
    }
    let need = |action: Option<&str>| -> RpcResult<()> {
        match action {
            Some(a) if !offered.iter().any(|o| o == a) => Err(fail("no_such_action", format!("the view names action {a}, which wasn't offered in hello (offered: {})", if offered.is_empty() { "none".to_string() } else { offered.join(", ") }))),
            _ => Ok(()),
        }
    };
    for k in keys.as_array().into_iter().flatten() {
        need(k["action"].as_str())?;
    }
    need(close)?;
    let mut rasters = HashMap::new();
    let mut nodes = 0usize;
    let label = |v: &Value| v.as_str().map(|s| json!(clean_text(s, VIEW_LIMIT.label)));
    fn inline(x: &Value, nodes: &mut usize) -> Value {
        *nodes += 1;
        match x {
            Value::String(s) => json!(clean_block(s, VIEW_LIMIT.block)),
            Value::Object(m) if m.get("type").and_then(Value::as_str) == Some("icon") => json!({ "type": "icon", "agent": clean_text(m["agent"].as_str().unwrap_or(""), 40) }),
            Value::Object(m) => {
                let mut o = m.clone();
                if let Some(Value::Array(c)) = m.get("children") {
                    o.insert("children".into(), json!(c.iter().map(|x| inline(x, nodes)).collect::<Vec<_>>()));
                }
                Value::Object(o)
            }
            other => other.clone(),
        }
    }
    fn walk(n: &Value, depth: usize, nodes: &mut usize, rasters: &mut HashMap<String, (u64, u64)>, need: &dyn Fn(Option<&str>) -> RpcResult<()>, label: &dyn Fn(&Value) -> Option<Value>) -> RpcResult<Value> {
        *nodes += 1;
        if *nodes > VIEW_LIMIT.nodes {
            return Err(fail("invalid_params", format!("a view has at most {} elements", VIEW_LIMIT.nodes)));
        }
        if depth > VIEW_LIMIT.depth {
            return Err(fail("invalid_params", format!("a view nests at most {} deep", VIEW_LIMIT.depth)));
        }
        let mut o = n.as_object().cloned().unwrap_or_default();
        let set = |o: &mut Map<String, Value>, k: &str, v: Option<Value>| {
            match v {
                Some(v) => o.insert(k.into(), v),
                None => o.remove(k),
            };
        };
        let kids = |o: &mut Map<String, Value>, nodes: &mut usize, rasters: &mut HashMap<String, (u64, u64)>| -> RpcResult<()> {
            if let Some(Value::Array(c)) = o.get("children").cloned() {
                let walked = c.iter().map(|x| walk(x, depth + 1, nodes, rasters, need, label)).collect::<RpcResult<Vec<_>>>()?;
                o.insert("children".into(), json!(walked));
            }
            Ok(())
        };
        match n["type"].as_str().unwrap_or("") {
            "box" => {
                let t = o.get("title").and_then(label);
                set(&mut o, "title", t);
                kids(&mut o, nodes, rasters)?;
            }
            "scroll" => kids(&mut o, nodes, rasters)?,
            "text" => {
                if let Some(Value::Array(c)) = o.get("children").cloned() {
                    o.insert("children".into(), json!(c.iter().map(|x| inline(x, nodes)).collect::<Vec<_>>()));
                }
            }
            "markdown" | "code" => {
                let c = clean_block(o["content"].as_str().unwrap_or(""), VIEW_LIMIT.block);
                o.insert("content".into(), json!(c));
            }
            "diff" => {
                need(o.get("action").and_then(Value::as_str))?;
                need(o.get("change").and_then(Value::as_str))?;
                let d = clean_block(o["diff"].as_str().unwrap_or(""), VIEW_LIMIT.block);
                o.insert("diff".into(), json!(d));
            }
            "table" => {
                let rows: Vec<Value> = o["rows"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|r| json!(r.as_array().into_iter().flatten().map(|c| match c {
                        Value::String(s) => json!(clean_text(s, VIEW_LIMIT.label)),
                        Value::Array(parts) => json!(parts.iter().map(|x| inline(x, nodes)).collect::<Vec<_>>()),
                        other => other.clone(),
                    }).collect::<Vec<_>>()))
                    .collect();
                o.insert("rows".into(), json!(rows));
            }
            "bigtext" => {
                let t = clean_text(o["text"].as_str().unwrap_or(""), 40);
                o.insert("text".into(), json!(t));
            }
            "gauge" | "spinner" => {
                let l = o.get("label").and_then(label);
                set(&mut o, "label", l);
            }
            "raster" => {
                let key = o["key"].as_str().unwrap_or("").to_string();
                if rasters.contains_key(&key) {
                    return Err(fail("invalid_params", format!("two rasters in one view have the key {key}")));
                }
                let (cols, rows) = (o["columns"].as_u64().unwrap_or(0), o["rows"].as_u64().unwrap_or(0));
                check_cells(o["cells"].as_str().unwrap_or(""), cols, rows)?;
                rasters.insert(key, (cols, rows));
            }
            "image" => {
                let png = o["png"].as_str().unwrap_or("");
                if png.len() > VIEW_LIMIT.image {
                    return Err(fail("invalid_params", format!("an image is at most {} MB of base64", VIEW_LIMIT.image / 1024 / 1024)));
                }
                let head = unb64(&png.chars().take(12).collect::<String>()).unwrap_or_default();
                if head.len() < 8 || head[..8] != PNG {
                    return Err(fail("invalid_params", "an image's png isn't base64 of a PNG"));
                }
                let a = o.get("alt").and_then(label);
                set(&mut o, "alt", a);
            }
            "button" => {
                need(o.get("action").and_then(Value::as_str))?;
                let l = clean_text(o["label"].as_str().unwrap_or(""), VIEW_LIMIT.label);
                o.insert("label".into(), json!(l));
            }
            "input" | "textarea" => {
                need(o.get("action").and_then(Value::as_str))?;
                let p = o.get("placeholder").and_then(label);
                set(&mut o, "placeholder", p);
                if let Some(v) = o.get("value").and_then(Value::as_str) {
                    let v = clean_block(v, 100_000);
                    o.insert("value".into(), json!(v));
                }
            }
            "select" | "tabs" => {
                need(o.get("action").and_then(Value::as_str))?;
                need(o.get("change").and_then(Value::as_str))?;
                let options: Vec<Value> = o["options"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|x| {
                        let mut opt = x.as_object().cloned().unwrap_or_default();
                        opt.insert("name".into(), json!(clean_text(x["name"].as_str().unwrap_or(""), VIEW_LIMIT.label)));
                        if let Some(d) = x.get("description").and_then(label) {
                            opt.insert("description".into(), d);
                        }
                        Value::Object(opt)
                    })
                    .collect();
                o.insert("options".into(), json!(options));
            }
            _ => {} // progress, sparkline, chart, heatmap: numbers only
        }
        Ok(Value::Object(o))
    }
    let root = walk(root, 0, &mut nodes, &mut rasters, &need, &label)?;
    let keys = json!(keys
        .as_array()
        .into_iter()
        .flatten()
        .map(|k| {
            let mut k = k.clone();
            if let Some(d) = k.get("description").and_then(label) {
                k["description"] = d;
            }
            k
        })
        .collect::<Vec<_>>());
    Ok((root, keys, rasters))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells(n: usize, ch: char) -> String {
        use base64::Engine;
        let mut b = vec![];
        for _ in 0..n {
            b.extend((ch as u32).to_le_bytes());
            b.extend(0u32.to_le_bytes());
            b.extend(0u32.to_le_bytes());
        }
        base64::engine::general_purpose::STANDARD.encode(b)
    }

    #[test]
    fn checks_views() {
        let offered = vec!["go".to_string()];
        let ok = json!({ "type": "box", "title": "\x1b[31mhi", "children": [{ "type": "button", "label": "x", "action": "go" }, { "type": "raster", "key": "r", "columns": 2, "rows": 1, "cells": cells(2, 'a') }] });
        let (root, _, rasters) = check_view(&ok, &json!([]), None, &offered).unwrap();
        assert_eq!(root["title"], "hi");
        assert_eq!(rasters["r"], (2, 1));
        let refused = |v: Value, close: Option<&str>| check_view(&v, &json!([]), close, &offered).unwrap_err().code;
        assert_eq!(refused(json!({ "type": "button", "label": "x", "action": "nope" }), None), "no_such_action");
        assert_eq!(refused(json!({ "type": "text" }), Some("nope")), "no_such_action");
        assert_eq!(refused(json!({ "type": "raster", "key": "r", "columns": 1, "rows": 1, "cells": cells(1, '中') }), None), "invalid_params");
        let mut deep = json!({ "type": "box" });
        for _ in 0..50 {
            deep = json!({ "type": "box", "children": [deep] });
        }
        assert_eq!(refused(deep, None), "invalid_params");
        assert!(check_shape(&json!({ "type": "nope" }), "root").is_err());
        assert!(check_shape(&json!({ "type": "box", "nope": 1 }), "root").is_err());
        assert!(check_shape(&json!({ "type": "box", "width": "50%", "children": [{ "type": "text", "children": ["a", { "type": "span", "bold": true }] }] }), "root").is_ok());
        assert_eq!(clean_block("a\tb\nc\x1b[31m\x07", 100), "a\tb\nc");
    }
}
