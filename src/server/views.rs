// What a plugin's view may hold (UI 3, examples/plugins/VIEWS.md): its shapes (each element's type and fields, with the
// colours, styles, constraints, text and blocks in them read by protocol/ui.rs, so the server takes what clients draw),
// how much of it (elements, depth, bytes), that its ids are unique and every action it names is one the plugin offered,
// its Rasters' and Images' bytes, and its text made safe to draw.
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{json, Map, Value};
use unicode_segmentation::UnicodeSegmentation;

use crate::config::themes::THEMES;
use crate::core::text::{clean_text, grapheme_width, width};
use crate::protocol::conn::{fail, RpcError, RpcResult};
use crate::protocol::schema::invalid;
use crate::protocol::ui::{self, Ink};

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
static SGR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1b\[[0-9;:]*m").unwrap());

// Multi-line text (Markdown, code, a diff, a Text, what a field holds): no escape sequences or control characters, but
// newlines and tabs kept, and at most `chars` of it.
pub fn clean_block(text: &str, chars: usize) -> String {
    let head: String = text.chars().take(chars * 2).collect();
    let s = OSC.replace_all(&head, "");
    let s = CSI.replace_all(&s, "");
    CONTROL.replace_all(&s, "").chars().take(chars).collect()
}

// A program's coloured output (a text's `ansi`): cleaned as clean_block is, but its SGR sequences (colours, bold, …)
// kept.
pub fn clean_ansi(text: &str, chars: usize) -> String {
    let head: String = text.chars().take(chars).collect();
    let mut out = String::new();
    let mut at = 0;
    for m in SGR.find_iter(&head) {
        out += &clean_block(&head[at..m.start()], chars);
        out += m.as_str();
        at = m.end();
    }
    out + &clean_block(&head[at..], chars)
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

// A view's size in cells, or a share of the terminal ("50%").
pub fn check_size(path: &str, v: Option<&Value>) -> RpcResult<()> {
    match v {
        None => Ok(()),
        Some(Value::Number(n)) if n.as_f64().is_some_and(|f| f.fract() == 0.0 && (0.0..=1000.0).contains(&f)) => Ok(()),
        Some(Value::String(s)) if s.strip_suffix('%').and_then(|n| n.parse::<f64>().ok()).is_some_and(|n| (0.0..=100.0).contains(&n)) => Ok(()),
        Some(_) => Err(invalid(path, "Invalid input")),
    }
}

// ---------- shapes ----------

type Check = RpcResult<()>;

const TYPES: &[&str] = &[
    "layout", "text", "block", "list", "table", "tabs", "gauge", "line_gauge", "sparkline", "bar_chart", "chart", "canvas", "calendar", "fill", "clear", "code", "diff", "markdown", "big_text", "image", "input", "textarea", "tree", "button", "spinner", "raster",
];
// a Block's fields: an element's `block`, or a `block` element's own
const BLOCK: &[&str] = &["borders", "border_type", "border_style", "title", "titles", "padding", "style", "shadow", "merge"];
const U16: f64 = 65535.0;
const WHOLE: f64 = 9007199254740991.0; // the most a JSON number holds exactly

fn kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn sub(at: &str, k: impl std::fmt::Display) -> String {
    format!("{at}.{k}")
}

fn expected(at: &str, what: &str, v: &Value) -> RpcError {
    invalid(at, format!("Invalid input: expected {what}, received {}", kind(v)))
}

// a field without which the element means nothing
fn need<'a>(m: &'a Map<String, Value>, at: &str, k: &str, what: &str) -> RpcResult<&'a Value> {
    m.get(k).ok_or_else(|| invalid(&sub(at, k), format!("Invalid input: expected {what}, received undefined")))
}

fn string<'a>(v: &'a Value, at: &str) -> RpcResult<&'a str> {
    v.as_str().ok_or_else(|| expected(at, "string", v))
}

// ids, actions, languages: 1 to max characters
fn name(v: &Value, at: &str, max: usize) -> Check {
    let n = string(v, at)?.chars().count();
    if n == 0 || n > max {
        return Err(invalid(at, format!("Invalid input: expected 1 to {max} characters")));
    }
    Ok(())
}

fn boolean(v: &Value, at: &str) -> Check {
    v.as_bool().map(drop).ok_or_else(|| expected(at, "boolean", v))
}

fn number(v: &Value, at: &str) -> RpcResult<f64> {
    v.as_f64().ok_or_else(|| expected(at, "number", v))
}

// a whole number from lo to hi
fn int(v: &Value, at: &str, lo: f64, hi: f64) -> Check {
    let f = number(v, at)?;
    if f.fract() != 0.0 || !(lo..=hi).contains(&f) {
        return Err(invalid(at, format!("Invalid input: expected a whole number from {lo} to {hi}")));
    }
    Ok(())
}

fn one_of(v: &Value, at: &str, options: &[&str]) -> Check {
    match v.as_str() {
        Some(s) if options.contains(&s) => Ok(()),
        _ => Err(invalid(at, format!("Invalid option: expected one of {}", options.iter().map(|o| format!("\"{o}\"")).collect::<Vec<_>>().join("|")))),
    }
}

fn array<'a>(v: &'a Value, at: &str) -> RpcResult<&'a Vec<Value>> {
    v.as_array().ok_or_else(|| expected(at, "array", v))
}

fn each(v: &Value, at: &str, check: impl Fn(&Value, &str) -> Check) -> Check {
    array(v, at)?.iter().enumerate().try_for_each(|(i, x)| check(x, &sub(at, i)))
}

// exactly n numbers: [x, y], a canvas line's [x1, y1, x2, y2], …
fn numbers(v: &Value, at: &str, n: usize) -> Check {
    let xs = array(v, at)?;
    if xs.len() != n {
        return Err(invalid(at, format!("Invalid input: expected {n} numbers")));
    }
    xs.iter().enumerate().try_for_each(|(i, x)| number(x, &sub(at, i)).map(drop))
}

// an object with only the keys it may have
fn object<'a>(v: &'a Value, at: &str, keys: &[&str]) -> RpcResult<&'a Map<String, Value>> {
    let m = v.as_object().ok_or_else(|| expected(at, "object", v))?;
    if let Some(k) = m.keys().find(|k| !keys.contains(&k.as_str())) {
        return Err(invalid(at, format!("Unrecognized key: \"{k}\"")));
    }
    Ok(m)
}

fn ink() -> Ink<'static> {
    Ink::plain(&THEMES[0].1)
}

// what ui.rs says is wrong with it, here
fn read<T>(r: Result<T, String>, at: &str) -> Check {
    r.map(drop).map_err(|e| invalid(at, e))
}

fn style(v: &Value, at: &str) -> Check {
    if v.is_null() {
        return Err(expected(at, "style", v));
    }
    read(ui::style(v, &THEMES[0].1), at)
}

// a word ui.rs reads (a direction, flex, an alignment, a marker, a colour)
fn word<T>(v: &Value, at: &str, reader: impl Fn(&Value) -> Result<T, String>) -> Check {
    string(v, at)?;
    read(reader(v), at)
}

// one character, one cell wide: a symbol drawn in each cell
fn symbol(v: &Value, at: &str) -> Check {
    let mut g = string(v, at)?.graphemes(true);
    match (g.next(), g.next()) {
        (Some(c), None) if grapheme_width(c) == 1 => Ok(()),
        _ => Err(invalid(at, "Invalid input: expected one character, one cell wide")),
    }
}

// A Span, Line or Text: ui.rs reads them; this adds what it lets pass (keys it doesn't know, nulls) and says where.
fn span(v: &Value, at: &str) -> Check {
    match v {
        Value::String(_) => Ok(()),
        Value::Object(m) if m.contains_key("icon") => {
            object(v, at, &["icon"])?;
            name(&m["icon"], &sub(at, "icon"), 40)
        }
        Value::Object(m) => {
            object(v, at, &["text", "style"])?;
            string(need(m, at, "text", "string")?, &sub(at, "text"))?;
            m.get("style").map_or(Ok(()), |s| style(s, &sub(at, "style")))
        }
        _ => Err(invalid(at, format!("Invalid input: a span is a string, {{ text, style }} or {{ icon }}, not {}", kind(v)))),
    }
}

fn line(v: &Value, at: &str) -> Check {
    match v {
        Value::Array(_) => each(v, at, span)?,
        Value::Object(m) if m.contains_key("spans") => {
            object(v, at, &["spans", "style", "align"])?;
            each(&m["spans"], &sub(at, "spans"), span)?;
            if let Some(s) = m.get("style") {
                style(s, &sub(at, "style"))?;
            }
            if let Some(a) = m.get("align") {
                word(a, &sub(at, "align"), ui::align)?;
            }
        }
        _ => span(v, at)?,
    }
    read(ui::line(v, &ink()), at)
}

fn text(v: &Value, at: &str) -> Check {
    match v {
        Value::Array(_) => each(v, at, line)?,
        Value::Object(m) if m.contains_key("lines") => {
            object(v, at, &["lines", "style", "align"])?;
            each(&m["lines"], &sub(at, "lines"), line)?;
            if let Some(s) = m.get("style") {
                style(s, &sub(at, "style"))?;
            }
            if let Some(a) = m.get("align") {
                word(a, &sub(at, "align"), ui::align)?;
            }
        }
        _ => line(v, at)?,
    }
    read(ui::text(v, &ink()), at)
}

// A Block's fields (an element's `block`, or a `block` element), as ui.rs reads them into ratatui's Block.
fn frame(m: &Map<String, Value>, v: &Value, at: &str) -> Check {
    for k in ["border_type", "merge"] {
        if let Some(x) = m.get(k) {
            string(x, &sub(at, k))?;
        }
    }
    if let Some(t) = m.get("title") {
        line(t, &sub(at, "title"))?;
    }
    if let Some(ts) = m.get("titles") {
        each(ts, &sub(at, "titles"), |t, at| {
            let tm = object(t, at, &["content", "position", "align"])?;
            line(need(tm, at, "content", "line")?, &sub(at, "content"))?;
            if let Some(p) = tm.get("position") {
                one_of(p, &sub(at, "position"), &["top", "bottom"])?;
            }
            tm.get("align").map_or(Ok(()), |a| word(a, &sub(at, "align"), ui::align))
        })?;
    }
    if let Some(s @ Value::Object(sm)) = m.get("shadow") {
        let at = sub(at, "shadow");
        object(s, &at, &["kind", "offset", "style"])?;
        if let Some(k) = sm.get("kind") {
            one_of(k, &sub(&at, "kind"), &["dark_shade", "medium_shade", "light_shade", "block", "overlay"])?;
        }
        if let Some(o) = sm.get("offset") {
            each(o, &sub(&at, "offset"), |x, at| int(x, at, -U16, U16))?; // ui.rs keeps it within 4 cells
        }
    }
    for k in ["style", "border_style"] {
        if let Some(s) = m.get(k) {
            style(s, &sub(at, k))?;
        }
    }
    read(ui::block(v, &ink()), at)
}

fn block(v: &Value, at: &str) -> Check {
    frame(object(v, at, BLOCK)?, v, at)
}

// a table's selection: a row, or [row, column]
fn cursor(v: &Value, at: &str) -> Check {
    match v {
        Value::Array(xs) if xs.len() == 2 => xs.iter().enumerate().try_for_each(|(i, x)| int(x, &sub(at, i), 0.0, WHOLE)),
        Value::Array(_) => Err(invalid(at, "Invalid input: expected a row, or [row, column]")),
        _ => int(v, at, 0.0, WHOLE),
    }
}

// a tree node's path: the ids from the root
fn path(v: &Value, at: &str) -> Check {
    if array(v, at)?.is_empty() {
        return Err(invalid(at, "Invalid input: a path names at least one node"));
    }
    each(v, at, |x, at| name(x, at, 80))
}

fn row(v: &Value, at: &str) -> Check {
    let Value::Object(m) = v else { return each(v, at, cell) };
    object(v, at, &["cells", "style", "height", "top_margin", "bottom_margin"])?;
    each(need(m, at, "cells", "array")?, &sub(at, "cells"), cell)?;
    for (k, x) in m {
        match k.as_str() {
            "style" => style(x, &sub(at, k))?,
            "height" | "top_margin" | "bottom_margin" => int(x, &sub(at, k), 0.0, U16)?,
            _ => {}
        }
    }
    Ok(())
}

fn cell(v: &Value, at: &str) -> Check {
    match v {
        Value::Object(m) if m.contains_key("content") => {
            object(v, at, &["content", "style", "span"])?;
            text(&m["content"], &sub(at, "content"))?;
            if let Some(s) = m.get("style") {
                style(s, &sub(at, "style"))?;
            }
            m.get("span").map_or(Ok(()), |n| int(n, &sub(at, "span"), 1.0, U16))
        }
        _ => text(v, at),
    }
}

// a list's item: a Line, or { content: Text, style }
fn item(v: &Value, at: &str) -> Check {
    match v {
        Value::Object(m) if m.contains_key("content") => {
            object(v, at, &["content", "style"])?;
            text(&m["content"], &sub(at, "content"))?;
            m.get("style").map_or(Ok(()), |s| style(s, &sub(at, "style")))
        }
        _ => line(v, at),
    }
}

fn tree(v: &Value, at: &str, depth: usize) -> Check {
    if depth > VIEW_LIMIT.depth {
        return Err(fail("invalid_params", format!("a view nests at most {} deep", VIEW_LIMIT.depth)));
    }
    let mut ids = HashSet::new();
    for (i, x) in array(v, at)?.iter().enumerate() {
        let at = sub(at, i);
        let m = object(x, &at, &["id", "text", "children"])?;
        let id = need(m, &at, "id", "string")?;
        name(id, &sub(&at, "id"), 80)?;
        if !ids.insert(id.as_str()) {
            return Err(invalid(&sub(&at, "id"), format!("Invalid input: two items here have the id {id}")));
        }
        line(need(m, &at, "text", "line")?, &sub(&at, "text"))?;
        if let Some(c) = m.get("children") {
            tree(c, &sub(&at, "children"), depth + 1)?;
        }
    }
    Ok(())
}

fn bars(v: &Value, at: &str) -> Check {
    each(v, at, |g, at| {
        let gm = object(g, at, &["label", "bars"])?;
        if let Some(l) = gm.get("label") {
            line(l, &sub(at, "label"))?;
        }
        each(need(gm, at, "bars", "array")?, &sub(at, "bars"), |b, at| {
            let bm = object(b, at, &["value", "label", "text_value", "style", "value_style"])?;
            int(need(bm, at, "value", "number")?, &sub(at, "value"), 0.0, WHOLE)?;
            for (k, x) in bm {
                let at = sub(at, k);
                match k.as_str() {
                    "label" => line(x, &at)?,
                    "text_value" => string(x, &at).map(drop)?,
                    "style" | "value_style" => style(x, &at)?,
                    _ => {}
                }
            }
            Ok(())
        })
    })
}

fn datasets(v: &Value, at: &str) -> Check {
    each(v, at, |d, at| {
        let dm = object(d, at, &["name", "data", "graph_type", "marker", "style", "fill_to"])?;
        each(need(dm, at, "data", "array")?, &sub(at, "data"), |p, at| numbers(p, at, 2))?;
        for (k, x) in dm {
            let at = sub(at, k);
            match k.as_str() {
                "name" => line(x, &at)?,
                "graph_type" => one_of(x, &at, &["line", "scatter", "bar", "area"])?,
                "marker" => word(x, &at, ui::marker)?,
                "style" => style(x, &at)?,
                "fill_to" => number(x, &at).map(drop)?,
                _ => {}
            }
        }
        Ok(())
    })
}

fn axis(v: &Value, at: &str) -> Check {
    for (k, x) in object(v, at, &["title", "bounds", "labels", "labels_align", "style"])? {
        let at = sub(at, k);
        match k.as_str() {
            "title" => line(x, &at)?,
            "bounds" => numbers(x, &at, 2)?,
            "labels" => each(x, &at, span)?,
            "labels_align" => word(x, &at, ui::align)?,
            _ => style(x, &at)?,
        }
    }
    Ok(())
}

// what a canvas draws: one of { line }, { rectangle }, { circle }, { points }, { map }, { text, at }, or { layer }
fn shape(v: &Value, at: &str) -> Check {
    let m = v.as_object().ok_or_else(|| expected(at, "object", v))?;
    let kinds = ["line", "rectangle", "circle", "points", "map", "text", "layer"];
    let found: Vec<&str> = kinds.into_iter().filter(|k| m.contains_key(*k)).collect();
    let [k] = found[..] else { return Err(invalid(at, "Invalid input: a shape is one of { line }, { rectangle }, { circle }, { points }, { map }, { text, at } or { layer }")) };
    let x = &m[k];
    let here = sub(at, k);
    match k {
        "text" => {
            object(v, at, &["text", "at"])?;
            line(x, &here)?;
            return numbers(need(m, at, "at", "array")?, &sub(at, "at"), 2);
        }
        "layer" => {
            object(v, at, &["layer"])?;
            return if *x == Value::Bool(true) { Ok(()) } else { Err(invalid(&here, "Invalid input: expected true")) };
        }
        "line" | "rectangle" => numbers(x, &here, 4)?,
        "circle" => numbers(x, &here, 3)?,
        "points" => each(x, &here, |p, at| numbers(p, at, 2))?,
        _ => one_of(x, &here, &["low", "high"])?,
    }
    object(v, at, &[k, "color"])?;
    m.get("color").map_or(Ok(()), |c| word(c, &sub(at, "color"), |c| ui::color(c.as_str().unwrap_or(""), &THEMES[0].1)))
}

// a calendar's day: YYYY-MM-DD, a real one
fn day(s: &str) -> bool {
    let p: Vec<&str> = s.split('-').collect();
    let [y, m, d] = p[..] else { return false };
    if [(y, 4), (m, 2), (d, 2)].iter().any(|(x, n)| x.len() != *n || !x.bytes().all(|b| b.is_ascii_digit())) {
        return false;
    }
    let (y, m, d): (u32, u32, u32) = (y.parse().unwrap_or(0), m.parse().unwrap_or(0), d.parse().unwrap_or(0));
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => 0,
    };
    y >= 1 && (1..=days).contains(&d)
}

// One field of an element of type t, checked; None: t has no such field.
fn field(t: &str, k: &str, v: &Value, at: &str) -> Option<Check> {
    Some(match (t, k) {
        (_, "id") => name(v, at, 80),
        (_, "size") => read(ui::constraint(v), at),
        (_, "block") => block(v, at),
        (_, "style") => style(v, at),
        (_, "hide_below") => object(v, at, &["width", "height"]).and_then(|m| m.iter().try_for_each(|(k, x)| int(x, &sub(at, k), 0.0, U16))),
        ("list" | "table" | "tabs" | "diff" | "input" | "textarea" | "tree", "action" | "change") | ("button", "action") | ("tree", "toggle") => name(v, at, usize::MAX),

        ("layout", "direction") | ("bar_chart", "direction") => word(v, at, ui::direction),
        ("layout", "constraints") | ("table", "widths") => each(v, at, |x, at| read(ui::constraint(x), at)),
        ("layout" | "table", "flex") => word(v, at, ui::flex),
        ("layout", "spacing") => int(v, at, -U16, U16),
        ("layout", "margin") => match v {
            Value::Array(xs) if xs.len() == 2 => xs.iter().enumerate().try_for_each(|(i, x)| int(x, &sub(at, i), 0.0, U16)),
            Value::Array(_) => Err(invalid(at, "Invalid input: expected n, or [vertical, horizontal]")),
            _ => int(v, at, 0.0, U16),
        },
        ("layout", "children") => each(v, at, check_shape),

        ("text" | "big_text", "text") => text(v, at),
        ("text", "ansi") | ("code" | "markdown", "content") | ("diff", "diff") | ("input" | "textarea", "value" | "placeholder") | ("image", "alt") | ("list" | "table" | "tree", "highlight_symbol") => string(v, at).map(drop),
        ("text" | "big_text", "align") => word(v, at, ui::align),
        ("text", "wrap") if matches!(v, Value::Bool(_)) || v == "trim" => Ok(()),
        ("text", "wrap") => Err(invalid(at, "Invalid input: expected true, false or \"trim\"")),
        ("text", "scroll") if matches!(v, Value::Bool(_)) || v == "bottom" => Ok(()),
        ("text", "scroll") => Err(invalid(at, "Invalid input: expected true, false or \"bottom\"")),
        ("text", "scrollbar") | ("gauge", "unicode") | ("code", "wrap") | ("diff", "cursor") | ("diff" | "textarea", "line_numbers") => boolean(v, at),

        ("block", "child") => check_shape(v, at),
        ("block", k) if BLOCK.contains(&k) => Ok(()), // frame() checks them together

        ("list", "items") => each(v, at, item),
        ("list" | "tabs", "selected") => int(v, at, 0.0, WHOLE),
        ("list" | "tabs" | "tree", "highlight_style") | ("table", "row_highlight_style" | "column_highlight_style" | "cell_highlight_style") => style(v, at),
        ("list" | "table", "highlight_spacing") => one_of(v, at, &["always", "when_selected", "never"]),
        ("list", "direction") => one_of(v, at, &["top_to_bottom", "bottom_to_top"]),
        ("list", "scroll_padding") | ("table", "column_spacing") | ("bar_chart", "bar_width" | "bar_gap" | "group_gap") => int(v, at, 0.0, U16),

        ("table", "header" | "footer") => row(v, at),
        ("table", "rows") => each(v, at, row),
        ("table", "select") => one_of(v, at, &["row", "cell", "column", "none"]),
        ("table", "selected") => cursor(v, at),

        ("tabs", "titles") => each(v, at, line),
        ("tabs", "divider") => span(v, at),
        ("tabs", "padding") => match v.as_array() {
            Some(xs) if xs.len() == 2 => each(v, at, span),
            _ => Err(invalid(at, "Invalid input: expected [left, right]")),
        },

        ("gauge" | "line_gauge", "ratio") => match number(v, at) {
            Ok(r) if !(0.0..=1.0).contains(&r) => Err(invalid(at, "Invalid input: expected 0 to 1")),
            r => r.map(drop),
        },
        ("gauge" | "line_gauge", "percent") => int(v, at, 0.0, 100.0),
        ("gauge" | "line_gauge", "label") => span(v, at),
        ("gauge", "gauge_style") | ("line_gauge", "filled_style" | "unfilled_style") => style(v, at),
        ("line_gauge", "filled_symbol" | "unfilled_symbol") | ("sparkline", "absent_symbol") | ("fill", "symbol") | ("input", "mask") => symbol(v, at),

        ("sparkline", "data") => each(v, at, |x, at| if x.is_null() { Ok(()) } else { int(x, at, 0.0, WHOLE) }),
        ("sparkline" | "bar_chart", "max") => int(v, at, 0.0, WHOLE),
        ("sparkline", "direction") => one_of(v, at, &["left_to_right", "right_to_left"]),
        ("sparkline", "bar_set") => one_of(v, at, &["nine_levels", "three_levels"]),
        ("sparkline", "absent_style") | ("bar_chart", "bar_style" | "value_style" | "label_style") => style(v, at),

        ("bar_chart", "groups") => bars(v, at),
        ("bar_chart", "data") => each(v, at, |p, at| match p.as_array().map(Vec::as_slice) {
            Some([l, n]) => string(l, &sub(at, 0)).and_then(|_| int(n, &sub(at, 1), 0.0, WHOLE)),
            _ => Err(invalid(at, "Invalid input: expected [label, value]")),
        }),

        ("chart", "datasets") => datasets(v, at),
        ("chart", "x_axis" | "y_axis") => axis(v, at),
        ("chart", "legend") => one_of(v, at, &["top_right", "top_left", "top", "left", "right", "bottom", "bottom_left", "bottom_right", "none"]),

        ("canvas", "x_bounds" | "y_bounds") => numbers(v, at, 2),
        ("canvas", "marker") => word(v, at, ui::marker),
        ("canvas", "background") => word(v, at, |c| ui::color(c.as_str().unwrap_or(""), &THEMES[0].1)),
        ("canvas", "shapes") => each(v, at, shape),

        ("calendar", "year") => int(v, at, 1.0, 9999.0),
        ("calendar", "month") => int(v, at, 1.0, 12.0),
        ("calendar", "events") => v.as_object().ok_or_else(|| expected(at, "object", v)).and_then(|m| {
            m.iter().try_for_each(|(d, s)| if day(d) { style(s, &sub(at, d)) } else { Err(invalid(&sub(at, d), "Invalid input: expected a day, YYYY-MM-DD")) })
        }),
        ("calendar", "month_header" | "weekday_header" | "surrounding") if *v == Value::Bool(false) => Ok(()),
        ("calendar", "month_header" | "weekday_header" | "surrounding" | "default_style") => style(v, at),

        ("code" | "diff", "language") | ("code", "syntax_theme") => name(v, at, 40),
        ("code", "line_numbers") if v.is_boolean() => Ok(()),
        ("code", "line_numbers") => int(v, at, 0.0, WHOLE),
        ("code", "highlight") | ("diff", "marks") => each(v, at, |x, at| int(x, at, 0.0, WHOLE)),
        ("diff", "view") => one_of(v, at, &["unified", "split"]),

        ("big_text", "pixel_size") => one_of(v, at, &["full", "half_height", "half_width", "quadrant", "third_height", "sextant", "quarter_height", "octant"]),
        ("image", "data") | ("raster", "cells") => string(v, at).map(drop), // their bytes: check_view
        ("image", "resize") => one_of(v, at, &["fit", "crop", "scale"]),

        ("tree", "items") => tree(v, at, 0),
        ("tree", "open") => each(v, at, path),
        ("tree", "selected") => path(v, at),

        ("button" | "spinner", "label") => line(v, at),
        ("button", "focus_style") => style(v, at),
        ("spinner", "set") => one_of(v, at, &["braille", "dots", "ascii", "arrows", "clock", "circle", "box", "bounce", "pulse"]),
        ("raster", "columns") => int(v, at, 1.0, 512.0),
        ("raster", "rows") => int(v, at, 1.0, 256.0),
        _ => return None,
    })
}

// The element tree's shapes, as VIEWS.md says them.
pub fn check_shape(v: &Value, path: &str) -> RpcResult<()> {
    let Value::Object(m) = v else { return Err(expected(path, "object", v)) };
    let t = m.get("type").and_then(Value::as_str).unwrap_or("");
    if !TYPES.contains(&t) {
        return Err(invalid(&sub(path, "type"), format!("Invalid option: expected one of {}", TYPES.iter().map(|o| format!("\"{o}\"")).collect::<Vec<_>>().join("|"))));
    }
    for (k, x) in m {
        if k == "type" {
            continue;
        }
        let at = sub(path, k);
        if x.is_null() {
            return Err(expected(&at, "a value", x));
        }
        field(t, k, x, &at).unwrap_or_else(|| Err(invalid(path, format!("Unrecognized key: \"{k}\""))))?;
    }
    // what no one field says alone
    let both = |a: &str, b: &str| -> Check {
        if m.contains_key(a) && m.contains_key(b) {
            return Err(invalid(path, format!("Invalid input: {a} or {b}, not both")));
        }
        Ok(())
    };
    match t {
        "layout" => {
            let (c, n) = (m.get("constraints").and_then(Value::as_array).map_or(0, Vec::len), m.get("children").and_then(Value::as_array).map_or(0, Vec::len));
            if c > n {
                return Err(invalid(&sub(path, "constraints"), format!("Invalid input: {c} constraints for {n} children")));
            }
        }
        "text" => {
            both("text", "ansi")?;
            if m.get("scroll").is_some_and(|s| *s != Value::Bool(false)) && !m.contains_key("id") {
                return Err(invalid(&sub(path, "id"), "Invalid input: a scrolling text needs an id (what's scrolled is kept by it)"));
            }
        }
        "block" => frame(m, v, path)?,
        "list" => drop(need(m, path, "items", "array")?),
        "table" => drop(need(m, path, "rows", "array")?),
        "tabs" => drop(need(m, path, "titles", "array")?),
        "gauge" | "line_gauge" => both("ratio", "percent")?,
        "sparkline" => drop(need(m, path, "data", "array")?),
        "bar_chart" => {
            both("groups", "data")?;
            if !m.contains_key("groups") {
                need(m, path, "data", "array")?;
            }
        }
        "chart" => drop(need(m, path, "datasets", "array")?),
        "calendar" => {
            need(m, path, "year", "number")?;
            need(m, path, "month", "number")?;
        }
        "code" | "markdown" => drop(need(m, path, "content", "string")?),
        "diff" => drop(need(m, path, "diff", "string")?),
        "big_text" => drop(need(m, path, "text", "text")?),
        "image" => drop(need(m, path, "data", "string")?),
        "tree" => drop(need(m, path, "items", "array")?),
        "raster" => {
            for k in ["id", "columns", "rows", "cells"] {
                need(m, path, k, if k == "id" || k == "cells" { "string" } else { "number" })?;
            }
        }
        _ => {}
    }
    if !m.contains_key("id") && ["action", "change", "toggle"].iter().any(|k| m.contains_key(*k)) {
        return Err(invalid(&sub(path, "id"), "Invalid input: an element that runs actions needs an id (they say which element they came from)"));
    }
    Ok(())
}

// Keys a view binds while it has focus: at most 40, each a key and the action it runs.
pub fn check_keys(keys: &Value) -> RpcResult<()> {
    let Value::Array(a) = keys else { return Err(invalid("keys", "Invalid input: expected array")) };
    if a.len() > 40 {
        return Err(invalid("keys", "Too big: expected array to have <=40 items"));
    }
    for (i, k) in a.iter().enumerate() {
        let at = format!("keys.{i}");
        let m = object(k, &at, &["key", "action", "params", "description"])?;
        name(need(m, &at, "key", "string")?, &sub(&at, "key"), 20)?;
        name(need(m, &at, "action", "string")?, &sub(&at, "action"), usize::MAX)?;
        if m.get("params").is_some_and(|p| !p.is_object()) {
            return Err(invalid(&sub(&at, "params"), "Invalid input: expected record"));
        }
        if let Some(d) = m.get("description") {
            string(d, &sub(&at, "description"))?;
        }
    }
    Ok(())
}

const PNG: &[u8] = &[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
const JPEG: &[u8] = &[0xff, 0xd8, 0xff];

// Every string a client may draw, cleaned. Ids and actions are names, never drawn; `ansi` keeps its colours.
fn clean(v: &mut Value) {
    match v {
        Value::String(s) => *s = clean_block(s, VIEW_LIMIT.block),
        Value::Array(xs) => xs.iter_mut().for_each(clean),
        Value::Object(m) => m.values_mut().for_each(clean),
        _ => {}
    }
}

struct Walk<'a> {
    nodes: usize,
    ids: HashSet<String>,
    rasters: HashMap<String, (u64, u64)>,
    need: &'a dyn Fn(Option<&str>) -> RpcResult<()>,
}

fn walk(n: &Value, depth: usize, w: &mut Walk) -> RpcResult<Value> {
    w.nodes += 1;
    if w.nodes > VIEW_LIMIT.nodes {
        return Err(fail("invalid_params", format!("a view has at most {} elements", VIEW_LIMIT.nodes)));
    }
    if depth > VIEW_LIMIT.depth {
        return Err(fail("invalid_params", format!("a view nests at most {} deep", VIEW_LIMIT.depth)));
    }
    let mut o = n.as_object().cloned().unwrap_or_default();
    let t = n["type"].as_str().unwrap_or("");
    if let Some(id) = o.get("id").and_then(Value::as_str) {
        if !w.ids.insert(id.to_string()) {
            return Err(fail("invalid_params", format!("two elements in one view have the id {id}")));
        }
    }
    for k in ["action", "change", "toggle"] {
        (w.need)(o.get(k).and_then(Value::as_str))?;
    }
    for (k, v) in o.iter_mut() {
        match (t, k.as_str()) {
            (_, "children") => *v = json!(v.as_array().into_iter().flatten().map(|c| walk(c, depth + 1, w)).collect::<RpcResult<Vec<_>>>()?),
            (_, "child") => *v = walk(v, depth + 1, w)?,
            (_, "ansi") => *v = json!(clean_ansi(v.as_str().unwrap_or(""), VIEW_LIMIT.block)),
            (_, "type" | "id" | "action" | "change" | "toggle") | ("image", "data") | ("raster", "cells") => {}
            _ => clean(v),
        }
    }
    match t {
        "raster" => {
            let (cols, rows) = (n["columns"].as_u64().unwrap_or(0), n["rows"].as_u64().unwrap_or(0));
            check_cells(n["cells"].as_str().unwrap_or(""), cols, rows)?;
            w.rasters.insert(n["id"].as_str().unwrap_or("").to_string(), (cols, rows));
        }
        "image" => {
            let data = n["data"].as_str().unwrap_or("");
            if data.len() > VIEW_LIMIT.image {
                return Err(fail("invalid_params", format!("an image is at most {} MB of base64", VIEW_LIMIT.image / 1024 / 1024)));
            }
            let head = unb64(&data.chars().take(12).collect::<String>()).unwrap_or_default();
            if !(head.starts_with(PNG) || head.starts_with(JPEG) || head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a")) {
                return Err(fail("invalid_params", "an image's data isn't base64 of a PNG, JPEG or GIF"));
            }
        }
        _ => {}
    }
    Ok(Value::Object(o))
}

// A view as the plugin sent it (its shapes already checked), checked and cleaned: what's drawn, its keys, and its
// Rasters' sizes by id (for blits).
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
    let mut w = Walk { nodes: 0, ids: HashSet::new(), rasters: HashMap::new(), need: &need };
    let root = walk(root, 0, &mut w)?;
    let keys = json!(keys
        .as_array()
        .into_iter()
        .flatten()
        .map(|k| {
            let mut k = k.clone();
            if let Some(d) = k.get("description").and_then(Value::as_str) {
                k["description"] = json!(clean_text(d, VIEW_LIMIT.label));
            }
            k
        })
        .collect::<Vec<_>>());
    Ok((root, keys, w.rasters))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(bytes: &[u8]) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    fn cells(n: usize, ch: char) -> String {
        let mut b = vec![];
        for _ in 0..n {
            b.extend((ch as u32).to_le_bytes());
            b.extend(0u32.to_le_bytes());
            b.extend(0u32.to_le_bytes());
        }
        b64(&b)
    }

    fn offered() -> Vec<String> {
        ["go", "pick", "open"].map(String::from).to_vec()
    }

    // every element, with every field it has
    fn every() -> Vec<Value> {
        vec![
            json!({ "type": "layout", "id": "l", "size": "*", "style": "on $bar", "hide_below": { "width": 20, "height": 3 }, "direction": "horizontal", "constraints": [34, "*"], "flex": "space_between", "spacing": -1, "margin": [1, 2],
                "block": { "borders": ["top", "left"], "border_type": "rounded", "border_style": "$border", "title": [{ "text": "x", "style": "bold" }, { "icon": "claude-code" }], "titles": [{ "content": "y", "position": "bottom", "align": "right" }], "padding": [0, 1], "style": "$fg", "shadow": { "kind": "overlay", "offset": [1, 1], "style": "$dim" }, "merge": "exact" },
                "children": [{ "type": "text" }, { "type": "text" }] }),
            json!({ "type": "fill", "symbol": "·", "style": "$dim" }),
            json!({ "type": "clear" }),
            json!({ "type": "text", "id": "t", "text": ["plain", [{ "text": "a", "style": { "fg": "$accent", "bold": true } }, "b"], { "spans": ["c"], "style": "dim", "align": "right" }], "align": "center", "wrap": "trim", "scroll": "bottom", "scrollbar": true }),
            json!({ "type": "text", "ansi": "\x1b[31mred\x1b[0m", "wrap": false }),
            json!({ "type": "text", "text": { "lines": ["a", "b"], "style": "italic", "align": "left" } }),
            json!({ "type": "text", "text": { "lines": [{ "spans": ["no align"] }] } }),
            json!({ "type": "block", "title": "box", "borders": "all", "border_type": "double", "padding": 1, "shadow": true, "merge": "fuzzy", "child": { "type": "text", "text": "inside" } }),
            json!({ "type": "list", "id": "files", "items": ["a", [{ "text": "b", "style": "bold" }], { "content": "two\nlines", "style": "$warn" }], "selected": 1, "highlight_style": "reversed", "highlight_symbol": "▶ ", "highlight_spacing": "always", "direction": "bottom_to_top", "scroll_padding": 2, "action": "go", "change": "pick" }),
            json!({ "type": "table", "id": "tb", "header": ["Name", "Size"], "footer": { "cells": ["total", { "content": "3", "style": "bold", "span": 1 }], "style": "dim", "height": 1, "top_margin": 1, "bottom_margin": 0 },
                "rows": [["a", "1"], { "cells": [{ "content": ["b", "c"] }, "2"], "height": 2 }], "widths": ["50%", ">=4"], "column_spacing": 2, "flex": "legacy", "select": "cell", "selected": [1, 0],
                "row_highlight_style": "reversed", "column_highlight_style": "bold", "cell_highlight_style": "on $accent", "highlight_symbol": "> ", "highlight_spacing": "never", "action": "go", "change": "pick" }),
            json!({ "type": "table", "rows": [], "selected": 3 }),
            json!({ "type": "tabs", "id": "tabs", "titles": ["One", [{ "text": "Two", "style": "$warn" }]], "selected": 0, "divider": { "text": "|", "style": "dim" }, "padding": [" ", " "], "highlight_style": "bold", "action": "go", "change": "pick" }),
            json!({ "type": "gauge", "ratio": 0.42, "label": "42%", "gauge_style": "$accent on $bar", "unicode": false }),
            json!({ "type": "line_gauge", "percent": 80, "label": { "text": "80%", "style": "bold" }, "filled_style": "$warn", "unfilled_style": "$dim", "filled_symbol": "━", "unfilled_symbol": "─" }),
            json!({ "type": "sparkline", "data": [1, 5, null, 3], "max": 10, "direction": "right_to_left", "bar_set": "three_levels", "absent_symbol": "·", "absent_style": "dim" }),
            json!({ "type": "bar_chart", "groups": [{ "label": "Mon", "bars": [{ "value": 3, "label": "a", "text_value": "3!", "style": "$accent", "value_style": "bold" }] }], "direction": "horizontal", "bar_width": 2, "bar_gap": 1, "group_gap": 2, "max": 10, "bar_style": "$fg", "value_style": "bold", "label_style": "dim" }),
            json!({ "type": "bar_chart", "data": [["a", 1], ["b", 2]] }),
            json!({ "type": "chart", "datasets": [{ "name": "cpu", "data": [[0, 1], [1.5, 2]], "graph_type": "area", "marker": "half_block", "style": "$accent", "fill_to": 0 }],
                "x_axis": { "title": "t", "bounds": [0, 10], "labels": ["0", { "text": "10", "style": "dim" }], "labels_align": "center", "style": "dim" }, "y_axis": { "bounds": [-1, 1] }, "legend": "none" }),
            json!({ "type": "canvas", "x_bounds": [0, 100], "y_bounds": [0, 50], "marker": "x", "background": "#101010", "shapes": [
                { "line": [0, 0, 10, 10], "color": "$accent" }, { "rectangle": [1, 1, 5, 5] }, { "circle": [50, 25, 10], "color": "red" }, { "points": [[1, 2], [3, 4]], "color": "42" },
                { "layer": true }, { "map": "low", "color": "$dim" }, { "text": [{ "text": "hi", "style": "bold" }], "at": [5, 5] }] }),
            json!({ "type": "calendar", "year": 2026, "month": 10, "events": { "2026-10-09": "bold $accent", "2024-02-29": { "fg": "red" } }, "month_header": "bold", "weekday_header": false, "surrounding": "dim", "default_style": "$fg" }),
            json!({ "type": "code", "id": "c", "content": "fn main() {}", "language": "rust", "line_numbers": 10, "highlight": [10], "wrap": true, "syntax_theme": "nord" }),
            json!({ "type": "code", "content": "x", "line_numbers": true }),
            json!({ "type": "diff", "id": "d", "diff": "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n", "language": "ts", "view": "split", "line_numbers": false, "cursor": true, "marks": [0, 1], "action": "go", "change": "pick" }),
            json!({ "type": "markdown", "id": "md", "content": "# Hi\n\n- [x] done" }),
            json!({ "type": "big_text", "text": "12:30", "pixel_size": "quadrant", "align": "center", "style": "$accent" }),
            json!({ "type": "image", "data": b64(&[PNG, &[0; 8]].concat()), "alt": "logo", "resize": "crop" }),
            json!({ "type": "image", "data": b64(&[0xff, 0xd8, 0xff, 0xe0, 0, 0, 0, 0, 0]) }),
            json!({ "type": "image", "data": b64(b"GIF89a\x01\x00\x01\x00") }),
            json!({ "type": "input", "id": "in", "value": "x", "placeholder": "comment…", "mask": "•", "action": "go", "change": "pick" }),
            json!({ "type": "textarea", "id": "ta", "value": "a\nb", "placeholder": "notes", "line_numbers": true, "action": "go", "change": "pick" }),
            json!({ "type": "tree", "id": "tr", "items": [{ "id": "src", "text": "src", "children": [{ "id": "main.rs", "text": [{ "text": "main.rs", "style": "bold" }] }] }, { "id": "README.md", "text": "README.md" }],
                "open": [["src"]], "selected": ["src", "main.rs"], "highlight_style": "reversed", "highlight_symbol": "> ", "action": "go", "change": "pick", "toggle": "open" }),
            json!({ "type": "button", "id": "b", "label": "Send", "action": "go", "focus_style": "reversed", "block": { "border_type": "rounded" } }),
            json!({ "type": "spinner", "label": "working", "set": "dots", "style": "$working" }),
            json!({ "type": "raster", "id": "r", "columns": 2, "rows": 1, "cells": cells(2, 'a') }),
        ]
    }

    #[test]
    fn takes_every_element_with_every_field() {
        let all = every();
        let mut types: Vec<&str> = all.iter().map(|e| e["type"].as_str().unwrap()).collect();
        types.sort();
        types.dedup();
        assert_eq!(types.len(), TYPES.len(), "a valid example of each type");
        for e in &all {
            check_shape(e, "root").unwrap_or_else(|err| panic!("{e}: {}", err.message));
        }
        let root = json!({ "type": "layout", "children": all });
        check_shape(&root, "root").unwrap();
        let (_, _, rasters) = check_view(&root, &json!([]), None, &offered()).unwrap();
        assert_eq!(rasters["r"], (2, 1));
    }

    #[test]
    fn refuses_what_views_md_doesnt_say() {
        let refused = |e: Value, says: &str| {
            let err = check_shape(&e, "root").err().unwrap_or_else(|| panic!("{e} was taken"));
            assert_eq!(err.code, "invalid_params");
            assert!(err.message.contains(says), "{e}: {:?} doesn't say {says:?}", err.message);
        };
        refused(json!({ "type": "box" }), "root.type Invalid option");
        refused(json!([]), "expected object, received array");
        refused(json!({ "type": "layout", "gap": 1 }), "Unrecognized key: \"gap\"");
        refused(json!({ "type": "layout", "direction": "row" }), "isn't vertical or horizontal");
        refused(json!({ "type": "layout", "direction": 1 }), "root.direction Invalid input: expected string");
        refused(json!({ "type": "layout", "constraints": ["*", "*"], "children": [{ "type": "clear" }] }), "2 constraints for 1 children");
        refused(json!({ "type": "layout", "constraints": ["x"], "children": [{ "type": "clear" }] }), "isn't a constraint");
        refused(json!({ "type": "layout", "flex": "between" }), "flex \"between\"");
        refused(json!({ "type": "layout", "margin": [1, 2, 3] }), "[vertical, horizontal]");
        refused(json!({ "type": "layout", "children": [{ "type": "text", "nope": 1 }] }), "root.children.0 Unrecognized key");
        refused(json!({ "type": "layout", "children": {} }), "root.children Invalid input: expected array");
        refused(json!({ "type": "clear", "size": "101%" }), "isn't a constraint");
        refused(json!({ "type": "clear", "style": null }), "root.style Invalid input: expected a value, received null");
        refused(json!({ "type": "clear", "style": "red blue" }), "one foreground colour");
        refused(json!({ "type": "clear", "hide_below": { "width": -1 } }), "root.hide_below.width");
        refused(json!({ "type": "clear", "hide_below": { "depth": 1 } }), "Unrecognized key: \"depth\"");
        refused(json!({ "type": "text", "text": 3 }), "a span is");
        refused(json!({ "type": "text", "text": [[{ "text": "a", "colour": "red" }]] }), "root.text.0.0 Unrecognized key: \"colour\"");
        refused(json!({ "type": "text", "text": { "spans": ["a"], "style": "$nope" } }), "theme colour");
        refused(json!({ "type": "text", "text": { "lines": ["a"], "align": "middle" } }), "align \"middle\"");
        refused(json!({ "type": "text", "text": [{ "icon": "" }] }), "root.text.0.icon");
        refused(json!({ "type": "text", "text": "a", "ansi": "b" }), "text or ansi, not both");
        refused(json!({ "type": "text", "scroll": true }), "root.id Invalid input: a scrolling text needs an id");
        refused(json!({ "type": "text", "wrap": "word" }), "expected true, false or \"trim\"");
        refused(json!({ "type": "text", "id": "t", "scroll": "top" }), "expected true, false or \"bottom\"");
        refused(json!({ "type": "text", "block": { "title": 1 } }), "root.block.title");
        refused(json!({ "type": "text", "block": { "border_type": "wavy" } }), "isn't one ratatui draws");
        refused(json!({ "type": "text", "block": { "titles": [{ "content": "x", "position": "left" }] } }), "root.block.titles.0.position");
        refused(json!({ "type": "text", "block": { "titles": [{ "align": "left" }] } }), "root.block.titles.0.content");
        refused(json!({ "type": "text", "block": { "shadow": { "kind": "x" } } }), "root.block.shadow.kind");
        refused(json!({ "type": "text", "block": { "shadow": { "offset": [1] } } }), "shadow offset");
        refused(json!({ "type": "text", "block": { "padding": [1, 2, 3] } }), "padding is n");
        refused(json!({ "type": "text", "block": { "borders": ["middle"] } }), "isn't top, right, bottom or left");
        refused(json!({ "type": "text", "block": { "nope": 1 } }), "root.block Unrecognized key: \"nope\"");
        refused(json!({ "type": "block", "child": { "type": "nope" } }), "root.child.type");
        refused(json!({ "type": "block", "titles": "x" }), "root.titles Invalid input: expected array");
        refused(json!({ "type": "block", "children": [] }), "Unrecognized key: \"children\"");
        refused(json!({ "type": "list" }), "root.items Invalid input: expected array, received undefined");
        refused(json!({ "type": "list", "items": [], "action": "go" }), "root.id Invalid input: an element that runs actions needs an id");
        refused(json!({ "type": "list", "items": [{ "content": "a", "nope": 1 }] }), "root.items.0 Unrecognized key");
        refused(json!({ "type": "list", "items": [], "highlight_spacing": "sometimes" }), "Invalid option");
        refused(json!({ "type": "list", "items": [], "selected": -1 }), "root.selected Invalid input: expected a whole number");
        refused(json!({ "type": "list", "items": [], "highlight_symbol": 1 }), "expected string");
        refused(json!({ "type": "list", "id": "l", "items": [], "action": "" }), "root.action Invalid input: expected 1 to");
        refused(json!({ "type": "table", "rows": [{ "style": "bold" }] }), "root.rows.0.cells");
        refused(json!({ "type": "table", "rows": [[{ "content": "a", "span": 0 }]] }), "root.rows.0.0.span");
        refused(json!({ "type": "table", "rows": [], "selected": [1] }), "a row, or [row, column]");
        refused(json!({ "type": "table", "rows": [], "select": "rows" }), "Invalid option");
        refused(json!({ "type": "table", "rows": [], "widths": ["1/0"] }), "root.widths.0");
        refused(json!({ "type": "table", "rows": [], "highlight_style": "bold" }), "Unrecognized key: \"highlight_style\"");
        refused(json!({ "type": "tabs" }), "root.titles");
        refused(json!({ "type": "tabs", "titles": [], "padding": [" "] }), "[left, right]");
        refused(json!({ "type": "tabs", "titles": [], "divider": [] }), "a span is");
        refused(json!({ "type": "gauge", "ratio": 1.5 }), "expected 0 to 1");
        refused(json!({ "type": "gauge", "ratio": 0.5, "percent": 50 }), "ratio or percent, not both");
        refused(json!({ "type": "gauge", "filled_symbol": "x" }), "Unrecognized key");
        refused(json!({ "type": "line_gauge", "filled_symbol": "ab" }), "one character");
        refused(json!({ "type": "line_gauge", "percent": 101 }), "from 0 to 100");
        refused(json!({ "type": "line_gauge", "unicode": true }), "Unrecognized key");
        refused(json!({ "type": "sparkline", "data": [1.5] }), "root.data.0");
        refused(json!({ "type": "sparkline", "data": [-1] }), "root.data.0");
        refused(json!({ "type": "sparkline", "data": [], "bar_set": "x" }), "Invalid option");
        refused(json!({ "type": "bar_chart" }), "root.data");
        refused(json!({ "type": "bar_chart", "groups": [], "data": [] }), "groups or data, not both");
        refused(json!({ "type": "bar_chart", "data": [["a"]] }), "[label, value]");
        refused(json!({ "type": "bar_chart", "groups": [{ "bars": [{ "label": "x" }] }] }), "root.groups.0.bars.0.value");
        refused(json!({ "type": "chart" }), "root.datasets");
        refused(json!({ "type": "chart", "datasets": [{ "data": [[1]] }] }), "root.datasets.0.data.0 Invalid input: expected 2 numbers");
        refused(json!({ "type": "chart", "datasets": [{ "data": [], "graph_type": "pie" }] }), "root.datasets.0.graph_type");
        refused(json!({ "type": "chart", "datasets": [], "legend": "middle" }), "Invalid option");
        refused(json!({ "type": "chart", "datasets": [], "x_axis": { "bounds": [0] } }), "root.x_axis.bounds");
        refused(json!({ "type": "canvas", "shapes": [{ "line": [0, 0, 1] }] }), "root.shapes.0.line Invalid input: expected 4 numbers");
        refused(json!({ "type": "canvas", "shapes": [{ "line": [0, 0, 1, 1], "circle": [1, 1, 1] }] }), "a shape is one of");
        refused(json!({ "type": "canvas", "shapes": [{ "map": "medium" }] }), "root.shapes.0.map");
        refused(json!({ "type": "canvas", "shapes": [{ "text": "x" }] }), "root.shapes.0.at");
        refused(json!({ "type": "canvas", "shapes": [{ "layer": false }] }), "expected true");
        refused(json!({ "type": "canvas", "shapes": [{ "line": [0, 0, 1, 1], "color": "$nope" }] }), "root.shapes.0.color");
        refused(json!({ "type": "canvas", "shapes": [{ "circle": [0, 0, 1], "at": [1, 1] }] }), "Unrecognized key: \"at\"");
        refused(json!({ "type": "canvas", "marker": "xy" }), "marker \"xy\"");
        refused(json!({ "type": "calendar", "year": 2026 }), "root.month");
        refused(json!({ "type": "calendar", "year": 2026, "month": 13 }), "root.month");
        refused(json!({ "type": "calendar", "year": 2026, "month": 2, "events": { "2026-02-30": "bold" } }), "YYYY-MM-DD");
        refused(json!({ "type": "calendar", "year": 2026, "month": 2, "weekday_header": true }), "a style is");
        refused(json!({ "type": "fill", "symbol": "ab" }), "one character");
        refused(json!({ "type": "clear", "symbol": "x" }), "Unrecognized key");
        refused(json!({ "type": "code" }), "root.content");
        refused(json!({ "type": "code", "content": "", "language": "" }), "root.language");
        refused(json!({ "type": "code", "content": "", "line_numbers": -1 }), "root.line_numbers");
        refused(json!({ "type": "diff" }), "root.diff");
        refused(json!({ "type": "diff", "diff": "", "marks": [-1] }), "root.marks.0");
        refused(json!({ "type": "diff", "diff": "", "view": "side" }), "Invalid option");
        refused(json!({ "type": "diff", "diff": "", "syntax_theme": "nord" }), "Unrecognized key");
        refused(json!({ "type": "markdown", "content": 1 }), "expected string");
        refused(json!({ "type": "big_text" }), "root.text");
        refused(json!({ "type": "big_text", "text": "x", "pixel_size": "huge" }), "Invalid option");
        refused(json!({ "type": "image" }), "root.data");
        refused(json!({ "type": "image", "data": "x", "resize": "cover" }), "Invalid option");
        refused(json!({ "type": "input", "mask": "**" }), "one character");
        refused(json!({ "type": "input", "line_numbers": true }), "Unrecognized key");
        refused(json!({ "type": "textarea", "mask": "*" }), "Unrecognized key");
        refused(json!({ "type": "tree" }), "root.items");
        refused(json!({ "type": "tree", "items": [{ "id": "a", "text": "a" }, { "id": "a", "text": "b" }] }), "root.items.1.id Invalid input: two items here have the id");
        refused(json!({ "type": "tree", "items": [{ "id": "a" }] }), "root.items.0.text");
        refused(json!({ "type": "tree", "items": [], "selected": [] }), "a path names at least one node");
        refused(json!({ "type": "tree", "items": [], "toggle": "x" }), "needs an id");
        refused(json!({ "type": "button", "label": 1 }), "a span is");
        refused(json!({ "type": "button", "id": "b", "change": "go" }), "Unrecognized key: \"change\"");
        refused(json!({ "type": "spinner", "set": "moon" }), "Invalid option");
        refused(json!({ "type": "raster", "columns": 2, "rows": 1, "cells": "" }), "root.id");
        refused(json!({ "type": "raster", "id": "r", "columns": 0, "rows": 1, "cells": "" }), "root.columns");
        let mut deep = json!({ "id": "a", "text": "a" });
        for _ in 0..50 {
            deep = json!({ "id": "a", "text": "a", "children": [deep] });
        }
        assert!(check_shape(&json!({ "type": "tree", "items": [deep] }), "root").is_err());
    }

    #[test]
    fn checks_views() {
        let ok = |v: Value| check_view(&v, &json!([]), None, &offered());
        let refused = |v: Value, close: Option<&str>| check_view(&v, &json!([]), close, &offered()).unwrap_err().code;
        // ids are unique in a view, wherever they are
        assert_eq!(refused(json!({ "type": "layout", "children": [{ "type": "text", "id": "a" }, { "type": "block", "child": { "type": "list", "id": "a", "items": [] } }] }), None), "invalid_params");
        // every action named is one offered
        assert_eq!(refused(json!({ "type": "button", "id": "b", "action": "nope" }), None), "no_such_action");
        assert_eq!(refused(json!({ "type": "tree", "id": "t", "items": [], "toggle": "nope" }), None), "no_such_action");
        assert_eq!(refused(json!({ "type": "text" }), Some("nope")), "no_such_action");
        assert_eq!(check_view(&json!({ "type": "text" }), &json!([{ "key": "s", "action": "nope" }]), None, &offered()).unwrap_err().code, "no_such_action");
        // bytes: a raster's cells, an image's
        assert_eq!(refused(json!({ "type": "raster", "id": "r", "columns": 1, "rows": 1, "cells": cells(1, '中') }), None), "invalid_params");
        assert_eq!(refused(json!({ "type": "raster", "id": "r", "columns": 2, "rows": 1, "cells": cells(1, 'a') }), None), "invalid_params");
        assert_eq!(refused(json!({ "type": "image", "data": b64(b"not an image") }), None), "invalid_params");
        assert_eq!(refused(json!({ "type": "image", "data": "A".repeat(VIEW_LIMIT.image + 4) }), None), "invalid_params");
        // how much
        let mut deep = json!({ "type": "clear" });
        for _ in 0..50 {
            deep = json!({ "type": "block", "child": deep });
        }
        assert_eq!(refused(deep, None), "invalid_params");
        let many: Vec<Value> = (0..VIEW_LIMIT.nodes).map(|_| json!({ "type": "clear" })).collect();
        assert_eq!(refused(json!({ "type": "layout", "children": many }), None), "invalid_params");
        assert_eq!(refused(json!({ "type": "markdown", "content": "x".repeat(VIEW_LIMIT.bytes) }), None), "invalid_params");
        // a raster under a block, by its id
        let (_, _, rasters) = ok(json!({ "type": "block", "child": { "type": "raster", "id": "art", "columns": 1, "rows": 1, "cells": cells(1, '█') } })).unwrap();
        assert_eq!(rasters["art"], (1, 1));
        // text made safe to draw, wherever it is; ids and actions as they were; ansi keeps its colours only
        let (root, keys, _) = check_view(
            &json!({ "type": "layout", "children": [
                { "type": "text", "text": [{ "text": "\x1b[31mhi\x07", "style": "bold" }], "block": { "title": "a\x1b]0;x\x07b" } },
                { "type": "tree", "id": "t\u{1}", "items": [{ "id": "a", "text": "x\x1b[2Jy" }], "action": "go" },
                { "type": "text", "ansi": "\x1b[1;31mred\x1b[0m\x1b[2J\x1b]8;;http://x\x07link\x1b[?25l" },
            ] }),
            &json!([{ "key": "s", "action": "go", "description": "send\x1b[31m it" }]),
            None,
            &offered(),
        )
        .unwrap();
        assert_eq!(root["children"][0]["text"][0]["text"], "hi");
        assert_eq!(root["children"][0]["block"]["title"], "ab");
        assert_eq!(root["children"][1]["items"][0]["text"], "xy");
        assert_eq!(root["children"][1]["id"], "t\u{1}");
        assert_eq!(root["children"][2]["ansi"], "\x1b[1;31mred\x1b[0mlink");
        assert_eq!(keys[0]["description"], "send it");
        assert_eq!(clean_block("a\tb\nc\x1b[31m\x07", 100), "a\tb\nc");
    }

    #[test]
    fn checks_keys() {
        assert!(check_keys(&json!([{ "key": "s", "action": "go", "params": { "x": 1 }, "description": "send" }])).is_ok());
        assert!(check_keys(&json!([{ "key": "", "action": "go" }])).is_err());
        assert!(check_keys(&json!([{ "key": "s" }])).is_err());
        assert!(check_keys(&json!([{ "key": "s", "action": "go", "nope": 1 }])).is_err());
        assert!(check_keys(&json!((0..41).map(|i| json!({ "key": i.to_string(), "action": "go" })).collect::<Vec<_>>())).is_err());
    }
}
