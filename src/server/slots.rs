// Slots (UI 4, examples/plugins/CHROME.md): plugins' content in modisa's own chrome (the status row, tabs, spaces, pane
// borders, the sidebar, menus, the palette), before or after what modisa draws there, or instead of it. Here: what a
// piece may be (checked and cleaned, so what clients draw is what the server took), the older methods as pieces, who
// draws instead of modisa where, and the one list clients draw. The plugin host (plugins.rs) keeps each run's pieces
// and sends that list.
use std::collections::HashMap;

use indexmap::IndexMap;
use serde_json::{json, Map, Value};

use super::views::{self, check_shape, check_view};
use crate::core::text::{clean_text, width};
use crate::protocol::conn::{fail, RpcResult};
use crate::protocol::schema::{invalid, Params};
use crate::protocol::types::{SlotPiece, SlotPosition, REPLACEABLE};

pub const PIECES: usize = 500; // a plugin's, across every slot
pub const PER_SECOND: u64 = 30; // lists sent to clients, however many updates
const LINE: usize = 200; // cells in a Line
const TITLE: usize = 40; // a menu or palette entry's
const SECTION_TITLE: usize = 30;
const ROWS: usize = 3; // an agent's
const SECTION_ROWS: usize = 30; // a sidebar section's, Lines or an element's height

// What a slot is about: nothing, or one pane's process, tab or space.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Per {
    Nothing,
    Pane,
    Tab,
    Space,
}

// What a slot takes: a Line; an agent's rows (1–3 Lines); a sidebar section (Lines, or an element `height` rows
// tall); the AGENTS list's place (the same, replace only); a menu entry (its title and action); a palette entry (the
// same, and a Line beside it).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Takes {
    Line,
    Rows,
    Section,
    Agents,
    Entry,
    Palette,
}

const KINDS: &[(&str, Per, Takes)] = &[
    ("status.left", Per::Nothing, Takes::Line),
    ("status.right", Per::Nothing, Takes::Line),
    ("status.agents", Per::Nothing, Takes::Line),
    ("status.panes", Per::Nothing, Takes::Line),
    ("status.git", Per::Nothing, Takes::Line),
    ("status.theme", Per::Nothing, Takes::Line),
    ("tab", Per::Tab, Takes::Line),
    ("space", Per::Space, Takes::Line),
    ("pane.title", Per::Pane, Takes::Line),
    ("pane.top_right", Per::Pane, Takes::Line),
    ("pane.bottom_left", Per::Pane, Takes::Line),
    ("pane.bottom_right", Per::Pane, Takes::Line),
    ("agent.row", Per::Pane, Takes::Rows),
    ("sidebar", Per::Nothing, Takes::Section),
    ("sidebar.agents", Per::Nothing, Takes::Agents),
    ("menu.pane", Per::Pane, Takes::Entry),
    ("menu.tab", Per::Tab, Takes::Entry),
    ("menu.space", Per::Space, Takes::Entry),
    ("palette", Per::Nothing, Takes::Palette),
];

// What a piece is for: nothing in particular, a pane's process (id and instance), a tab (its id) or a space (its name).
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Nothing,
    Pane(String, String),
    Tab(String),
    Space(String),
}

impl Target {
    // what tells two of a plugin's pieces with the same slot and id apart: a pane by its id, so a new process's piece
    // replaces the last one's
    fn key(&self) -> &str {
        match self {
            Target::Nothing => "",
            Target::Pane(id, _) | Target::Tab(id) | Target::Space(id) => id,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Piece {
    pub slot: &'static str,
    pub id: String,
    pub target: Target,
    pub position: &'static str, // before, after or replace
    pub order: i64,
    pub body: Map<String, Value>, // what's drawn and what a click does: line, lines or element (and height), title, action, hide_below
    pub legacy: Option<Value>, // set by an older method (ui.status.set …): what it set, for UI 3 clients and ui.state
}

// A piece's place among a plugin's: its slot, what it's for (a pane's id, a tab's, a space's name) and its id.
pub fn key(slot: &str, target: &str, id: &str) -> String {
    format!("{slot}\n{target}\n{id}")
}

impl Piece {
    pub fn key(&self) -> String {
        key(self.slot, self.target.key(), &self.id)
    }

    // as clients get it (`replaces` is resolve's to say)
    pub fn wire(&self, plugin: &str, run: &str) -> SlotPiece {
        let mut o = Map::new();
        o.insert("plugin".into(), json!(plugin));
        o.insert("run".into(), json!(run));
        o.insert("slot".into(), json!(self.slot));
        o.insert("id".into(), json!(self.id));
        o.insert("position".into(), json!(self.position));
        o.insert("order".into(), json!(self.order));
        match &self.target {
            Target::Nothing => {}
            Target::Pane(p, i) => {
                o.insert("pane".into(), json!(p));
                o.insert("instance".into(), json!(i));
            }
            Target::Tab(t) => drop(o.insert("tab".into(), json!(t))),
            Target::Space(s) => drop(o.insert("space".into(), json!(s))),
        }
        o.extend(self.body.clone());
        serde_json::from_value(Value::Object(o)).unwrap_or_default() // what check() took always reads back (tested)
    }
}

// What a piece's references are checked against: whether a pane's id and instance are its process now (and it's
// running), a tab's id and a space's name from what names them, and the actions the plugin offered.
pub struct World<'a> {
    pub process: &'a dyn Fn(&str, &str) -> bool,
    pub tab: &'a dyn Fn(&str) -> RpcResult<String>,
    pub space: &'a dyn Fn(&str) -> RpcResult<String>,
    pub actions: &'a [String],
}

impl World<'_> {
    fn offered(&self, action: &str) -> RpcResult<()> {
        offered(self.actions, action)
    }
}

fn offered(actions: &[String], action: &str) -> RpcResult<()> {
    if actions.iter().any(|a| a == action) {
        return Ok(());
    }
    let offers = if actions.is_empty() { "none".to_string() } else { actions.join(", ") };
    Err(fail("no_such_action", format!("this plugin didn't offer action {action} in hello (it offers: {offers})")))
}

const KEYS: &[&str] = &["caller", "slot", "id", "pane", "instance", "tab", "space", "position", "order", "line", "lines", "element", "height", "title", "action", "hide_below"];

// A Line made safe to draw on one line (no escape sequences or control characters) and cut to `cells`: what's past
// them is dropped, and an agent's mark takes two.
pub fn clean_line(v: &Value, cells: usize) -> Value {
    let mut left = cells;
    match v {
        Value::Array(xs) => Value::Array(xs.iter().filter_map(|x| clean_span(x, &mut left)).collect()),
        Value::Object(o) if o.contains_key("spans") => {
            let mut o = o.clone();
            let spans: Vec<Value> = o["spans"].as_array().into_iter().flatten().filter_map(|x| clean_span(x, &mut left)).collect();
            o.insert("spans".into(), Value::Array(spans));
            Value::Object(o)
        }
        other => clean_span(other, &mut left).unwrap_or_else(|| json!("")),
    }
}

fn clean_span(x: &Value, left: &mut usize) -> Option<Value> {
    let mut cut = |s: &str| {
        let t = clean_text(s, *left);
        *left -= width(&t);
        (!t.is_empty()).then_some(t)
    };
    match x {
        Value::String(s) => cut(s).map(Value::String),
        Value::Object(o) if o.contains_key("icon") => (*left >= 2).then(|| {
            *left -= 2;
            json!({ "icon": clean_text(o["icon"].as_str().unwrap_or(""), 40) })
        }),
        Value::Object(o) => {
            let mut o = o.clone();
            o.insert("text".into(), json!(cut(o["text"].as_str().unwrap_or(""))?));
            Some(Value::Object(o))
        }
        _ => None,
    }
}

fn line(v: &Value, at: &str) -> RpcResult<Value> {
    views::line(v, at)?;
    Ok(clean_line(v, LINE))
}

// A sidebar row: a Line, which as { spans, … } can also run an `action` or focus a `pane` (with its `instance`: a
// click reaches that process or nothing).
fn row(v: &Value, at: &str, w: &World) -> RpcResult<Value> {
    let mut v = v.clone();
    let mut extra = Map::new();
    if let Some(o) = v.as_object_mut().filter(|o| o.contains_key("spans")) {
        for k in ["action", "pane", "instance"] {
            if let Some(x) = o.remove(k) {
                extra.insert(k.into(), x);
            }
        }
    }
    let mut out = line(&v, at)?;
    let field = |k: &str| -> RpcResult<Option<&str>> {
        match extra.get(k) {
            None => Ok(None),
            Some(Value::String(s)) if !s.is_empty() => Ok(Some(s)),
            Some(_) => Err(invalid(&format!("{at}.{k}"), "Invalid input: expected a non-empty string")),
        }
    };
    if let Some(a) = field("action")? {
        w.offered(a)?;
    }
    match (field("pane")?, field("instance")?) {
        (None, None) => {}
        (Some(p), Some(i)) if (w.process)(p, i) => {}
        (Some(p), Some(i)) => return Err(fail("pane_gone", format!("no pane {p} with instance {i}: it closed, exited or was restarted"))),
        _ => return Err(invalid(&format!("{at}.instance"), "Invalid input: a row's pane needs its instance")),
    }
    if let Some(o) = out.as_object_mut() {
        o.extend(extra);
    }
    Ok(out)
}

// ui.slot.set's params: one piece, checked and cleaned.
pub fn check(p: &Value, w: &World) -> RpcResult<Piece> {
    let pr = Params::new(p)?;
    pr.opt_str("caller")?;
    let slot = pr.len("slot", 1, None)?;
    if slot == "toast" {
        return Err(invalid("slot", "a toast isn't kept: send it with ui.toast"));
    }
    let Some(&(slot, per, takes)) = KINDS.iter().find(|k| k.0 == slot) else {
        return Err(invalid("slot", format!("Invalid option: expected one of {}", KINDS.iter().map(|k| format!("\"{}\"", k.0)).collect::<Vec<_>>().join("|"))));
    };
    let id = pr.len("id", 1, Some(40))?;
    if let Some(k) = p.as_object().and_then(|m| m.keys().find(|k| !KEYS.contains(&k.as_str()))) {
        return Err(invalid(k, "Unrecognized key"));
    }
    let not = |k: &str| -> RpcResult<()> { if pr.has(k) { Err(invalid(k, format!("Unrecognized key: {slot} doesn't take {k}"))) } else { Ok(()) } };

    // what it's for
    let per_name = |p: Per| match p {
        Per::Pane => "pane",
        Per::Tab => "tab",
        _ => "space",
    };
    for (k, p) in [("pane", Per::Pane), ("instance", Per::Pane), ("tab", Per::Tab), ("space", Per::Space)] {
        if p != per {
            not(k)?;
        }
    }
    let optional = takes == Takes::Entry; // a menu entry for none is for every one
    let need = |k: &str| -> RpcResult<String> {
        if !pr.has(k) {
            return Err(invalid(k, format!("Invalid input: expected string, received undefined ({slot} is per {})", per_name(per))));
        }
        pr.len(k, 1, None)
    };
    let target = match per {
        Per::Nothing => Target::Nothing,
        Per::Pane if optional && !pr.has("pane") && !pr.has("instance") => Target::Nothing,
        Per::Tab if optional && !pr.has("tab") => Target::Nothing,
        Per::Space if optional && !pr.has("space") => Target::Nothing,
        Per::Pane => {
            let (pane, instance) = (need("pane")?, need("instance")?);
            if !(w.process)(&pane, &instance) {
                return Err(fail("pane_gone", format!("no pane {pane} with instance {instance}: it closed, exited or was restarted")));
            }
            Target::Pane(pane, instance)
        }
        Per::Tab => Target::Tab((w.tab)(&need("tab")?)?),
        Per::Space => Target::Space((w.space)(&need("space")?)?),
    };

    // where, among what's there
    let position = match (pr.opt_enum("position", &["before", "after", "replace"])?.as_deref(), takes) {
        (None | Some("replace"), Takes::Agents) => "replace",
        (Some(_), Takes::Agents) => return Err(invalid("position", "sidebar.agents is replace only")),
        (Some("replace"), _) if !REPLACEABLE.contains(&slot) => return Err(invalid("position", format!("there's nothing of modisa's in {slot} to replace: before or after"))),
        (Some("replace"), _) => "replace",
        (Some("before"), _) => "before",
        _ => "after",
    };
    let order = pr.num_or("order", 0.0, Some(-1_000_000.0), Some(1_000_000.0), true)? as i64;

    // what's drawn, and what a click does
    let mut body = Map::new();
    if let Some(h) = pr.raw("hide_below") {
        let hm = views::object(h, "hide_below", &["width"])?;
        views::int(hm.get("width").unwrap_or(&Value::Null), "hide_below.width", 0.0, 65535.0)?;
        body.insert("hide_below".into(), h.clone());
    }
    let action = pr.opt_len("action", 1, None)?;
    if let Some(a) = &action {
        w.offered(a)?;
        body.insert("action".into(), json!(a));
    }
    let entry = matches!(takes, Takes::Entry | Takes::Palette);
    if entry && action.is_none() {
        return Err(invalid("action", format!("Invalid input: expected string, received undefined ({slot} entries run one)")));
    }
    match takes {
        Takes::Entry | Takes::Palette => drop(body.insert("title".into(), json!(clean_text(&pr.str("title")?, TITLE)))),
        Takes::Section | Takes::Agents => {
            if let Some(t) = pr.opt_str("title")? {
                body.insert("title".into(), json!(clean_text(&t, SECTION_TITLE)));
            }
        }
        _ => not("title")?,
    }
    let one = |k: &str| pr.raw(k).map(|v| line(v, k)).transpose();
    let some = |max: usize, each: &dyn Fn(&Value, &str) -> RpcResult<Value>| -> RpcResult<Option<Value>> {
        let Some(v) = pr.raw("lines") else { return Ok(None) };
        let xs = v.as_array().ok_or_else(|| invalid("lines", "Invalid input: expected array"))?;
        if xs.len() > max {
            return Err(invalid("lines", format!("Too big: expected array to have <={max} items")));
        }
        Ok(Some(Value::Array(xs.iter().enumerate().map(|(i, x)| each(x, &format!("lines.{i}"))).collect::<RpcResult<_>>()?)))
    };
    let only_one = |keys: &[&str]| -> RpcResult<()> {
        let given: Vec<&&str> = keys.iter().filter(|k| pr.has(k)).collect();
        match given[..] {
            [_] => Ok(()),
            [] => Err(invalid(keys[0], format!("Invalid input: {slot} takes {}", keys.join(" or ")))),
            _ => Err(invalid(given[1], format!("Invalid input: {} or {}, not both", given[0], given[1]))),
        }
    };
    match takes {
        Takes::Line => {
            for k in ["lines", "element", "height"] {
                not(k)?;
            }
            only_one(&["line"])?;
            body.insert("line".into(), one("line")?.unwrap());
        }
        Takes::Rows => {
            not("element")?;
            not("height")?;
            only_one(&["line", "lines"])?;
            let lines = match one("line")? {
                Some(l) => json!([l]),
                None => some(ROWS, &|x, at| line(x, at))?.unwrap(),
            };
            if lines.as_array().is_some_and(Vec::is_empty) {
                return Err(invalid("lines", "Too small: expected array to have >=1 items"));
            }
            body.insert("lines".into(), lines);
        }
        Takes::Section | Takes::Agents => {
            only_one(&["lines", "line", "element"])?;
            let each = |x: &Value, at: &str| row(x, at, w);
            if let Some(l) = pr.raw("line") {
                body.insert("lines".into(), json!([each(l, "line")?]));
            } else if let Some(lines) = some(SECTION_ROWS, &each)? {
                body.insert("lines".into(), lines);
            }
            match pr.raw("element") {
                Some(e) => {
                    check_shape(e, "element")?;
                    let (root, _, _) = check_view(e, &json!([]), None, w.actions)?;
                    body.insert("element".into(), root);
                    match pr.opt_num("height", Some(1.0), Some(SECTION_ROWS as f64), true)? {
                        Some(h) => drop(body.insert("height".into(), json!(h as u64))),
                        None if takes == Takes::Section => return Err(invalid("height", "Invalid input: expected number, received undefined (the rows a sidebar element takes, 1 to 30)")),
                        None => {}
                    }
                }
                None => not("height")?,
            }
        }
        Takes::Entry => {
            for k in ["line", "lines", "element", "height"] {
                not(k)?;
            }
        }
        Takes::Palette => {
            for k in ["lines", "element", "height"] {
                not(k)?;
            }
            if let Some(l) = one("line")? {
                body.insert("line".into(), l);
            }
        }
    }
    Ok(Piece { slot, id, target, position, order, body, legacy: None })
}

// ui.slot.clear's params, as a test of a piece: each one given must match.
pub fn matcher(p: &Value) -> RpcResult<impl Fn(&Piece) -> bool> {
    let pr = Params::new(p)?;
    pr.opt_str("caller")?;
    let slot = pr.opt_enum("slot", &KINDS.iter().map(|k| k.0).collect::<Vec<_>>())?;
    let id = pr.opt_len("id", 1, Some(40))?;
    let (pane, tab, space) = (pr.opt_len("pane", 1, None)?, pr.opt_len("tab", 1, None)?, pr.opt_len("space", 1, None)?);
    Ok(move |x: &Piece| {
        let is = |want: &Option<String>, have: &str| want.as_ref().is_none_or(|w| w == have);
        let target = |want: &Option<String>, have: Option<&str>| want.is_none() || want.as_deref() == have;
        is(&slot, x.slot)
            && is(&id, &x.id)
            && target(&pane, if let Target::Pane(p, _) = &x.target { Some(p) } else { None })
            && target(&tab, if let Target::Tab(t) = &x.target { Some(t) } else { None })
            && target(&space, if let Target::Space(s) = &x.target { Some(s) } else { None })
    })
}

// What ui.toast takes besides its text, tone and system (CHROME.md's toast): `lines` (Lines, at most 3), `actions`
// (buttons: [{ title, action }], at most 3), `timeout` (ms) and an `id` its buttons' actions say. Checked and cleaned;
// and the text a client that draws only text shows: the lines', when there's no `text`.
pub fn toast(p: &Value, actions: &[String]) -> RpcResult<(Option<String>, Map<String, Value>)> {
    let pr = Params::new(p)?;
    let mut more = Map::new();
    let mut plain = None;
    if let Some(v) = pr.raw("lines") {
        let xs = v.as_array().ok_or_else(|| invalid("lines", "Invalid input: expected array"))?;
        if xs.len() > ROWS {
            return Err(invalid("lines", format!("Too big: expected array to have <={ROWS} items")));
        }
        let lines: Vec<Value> = xs.iter().enumerate().map(|(i, x)| line(x, &format!("lines.{i}"))).collect::<RpcResult<_>>()?;
        plain = Some(lines.iter().map(text_of).collect::<Vec<_>>().join(" · "));
        more.insert("lines".into(), json!(lines));
    }
    if let Some(v) = pr.raw("actions") {
        let xs = v.as_array().ok_or_else(|| invalid("actions", "Invalid input: expected array"))?;
        if xs.len() > 3 {
            return Err(invalid("actions", "Too big: expected array to have <=3 items"));
        }
        let mut buttons = vec![];
        for (i, x) in xs.iter().enumerate() {
            let at = format!("actions.{i}");
            let b = views::object(x, &at, &["title", "action"])?;
            let title = b.get("title").and_then(Value::as_str).ok_or_else(|| invalid(&format!("{at}.title"), "Invalid input: expected string"))?;
            let action = b.get("action").and_then(Value::as_str).filter(|a| !a.is_empty()).ok_or_else(|| invalid(&format!("{at}.action"), "Invalid input: expected a non-empty string"))?;
            offered(actions, action)?;
            buttons.push(json!({ "title": clean_text(title, TITLE), "action": action }));
        }
        more.insert("actions".into(), json!(buttons));
    }
    if let Some(t) = pr.opt_num("timeout", Some(1000.0), Some(60_000.0), true)? {
        more.insert("timeout".into(), json!(t as u64));
    }
    if let Some(id) = pr.opt_len("id", 1, Some(40))? {
        more.insert("id".into(), json!(id));
    }
    Ok((plain, more))
}

// a cleaned Line's text, its marks left out
fn text_of(l: &Value) -> String {
    match l {
        Value::String(s) => s.clone(),
        Value::Array(xs) => xs.iter().map(text_of).collect(),
        Value::Object(o) if o.contains_key("spans") => text_of(&o["spans"]),
        Value::Object(o) => o.get("text").and_then(Value::as_str).unwrap_or("").to_string(),
        _ => String::new(),
    }
}

// ---------- the older methods, as pieces ----------
// ui.status.set is status.right, ui.badge.set pane.title after, ui.sidebar.set a sidebar section, ui.menu.set menu.pane
// entries (CHROME.md). Their text and tone are a one-span Line, named the way they were always drawn so they can't
// pass for modisa's own: "radar: 3 waiting", "[radar: blocked]". What they set is kept as it was (`legacy`).

fn tone_line(text: String, tone: &str) -> Value {
    json!([{ "text": text, "style": format!("${tone}") }])
}

fn old(slot: &'static str, id: String, target: Target, body: Map<String, Value>, legacy: Value) -> Piece {
    Piece { slot, id, target, position: "after", order: 0, body, legacy: Some(legacy) }
}

// a status segment: { id, text, tone, action? }
pub fn status(plugin: &str, seg: Value) -> Piece {
    let mut body = Map::new();
    body.insert("line".into(), tone_line(format!("{plugin}: {}", seg["text"].as_str().unwrap_or("")), seg["tone"].as_str().unwrap_or("fg")));
    if let Some(a) = seg.get("action") {
        body.insert("action".into(), a.clone());
    }
    let id = seg["id"].as_str().unwrap_or("").to_string();
    old("status.right", id, Target::Nothing, body, seg)
}

// a badge: { pane, instance, text, tone }
pub fn badge(plugin: &str, b: Value) -> Piece {
    let mut body = Map::new();
    body.insert("line".into(), tone_line(format!("[{plugin}: {}]", b["text"].as_str().unwrap_or("")), b["tone"].as_str().unwrap_or("accent")));
    let target = Target::Pane(b["pane"].as_str().unwrap_or("").into(), b["instance"].as_str().unwrap_or("").into());
    old("pane.title", "badge".into(), target, body, b)
}

// a sidebar section: { title, rows: [{ text, tone, spans?, action?, pane?, instance? }] }, its rows as Lines with
// their action or pane
pub fn section(s: Value) -> Piece {
    let rows: Vec<Value> = s["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| {
            let tone = r["tone"].as_str().unwrap_or("fg");
            let spans: Vec<Value> = match r["spans"].as_array() {
                Some(spans) => spans
                    .iter()
                    .map(|x| match x.get("icon") {
                        Some(i) => json!({ "icon": i }),
                        None => {
                            let t = x["tone"].as_str().unwrap_or(tone);
                            json!({ "text": x["text"], "style": if x["bold"] == true { format!("bold ${t}") } else { format!("${t}") } })
                        }
                    })
                    .collect(),
                None => vec![json!({ "text": r["text"], "style": format!("${tone}") })],
            };
            let mut o = json!({ "spans": spans });
            for k in ["action", "pane", "instance"] {
                if let Some(x) = r.get(k) {
                    o[k] = x.clone();
                }
            }
            o
        })
        .collect();
    let mut body = Map::new();
    body.insert("title".into(), s["title"].clone());
    body.insert("lines".into(), json!(rows));
    old("sidebar", "sidebar".into(), Target::Nothing, body, s)
}

// a pane menu entry: { id, title, action }
pub fn menu_entry(item: Value) -> Piece {
    let mut body = Map::new();
    body.insert("title".into(), item["title"].clone());
    body.insert("action".into(), item["action"].clone());
    let id = item["id"].as_str().unwrap_or("").to_string();
    old("menu.pane", id, Target::Nothing, body, item)
}

// What a UI 3 client draws, and ui.state's older fields: what the older methods set, as they set it.
pub struct Legacy {
    pub status: Vec<Value>,
    pub sidebar: Option<Value>,
    pub badges: Vec<Value>,
    pub menu: Vec<Value>,
}

pub fn legacy(pieces: &IndexMap<String, Piece>) -> Legacy {
    let of = |slot: &'static str| pieces.values().filter(move |p| p.slot == slot).filter_map(|p| p.legacy.clone());
    Legacy { status: of("status.right").collect(), sidebar: of("sidebar").next(), badges: of("pane.title").collect(), menu: of("menu.pane").collect() }
}

// ---------- who draws what ----------

// A running plugin's pieces.
pub struct Shown<'a> {
    pub plugin: &'a str,
    pub run: &'a str,
    pub pieces: &'a IndexMap<String, Piece>,
}

fn asked(s: &Shown, slot: &str) -> bool {
    s.pieces.values().any(|p| p.slot == slot && p.position == "replace")
}

// The plugin drawing instead of modisa in a slot, and for sidebar.agents maybe the key of its sidebar section that does.
struct Held<'a> {
    plugin: &'a str,
    section: Option<String>,
}

// Who holds each slot: the plugin config.toml's [slots] names ("builtin": nobody), else the first plugin by name that
// asked to replace it. [sidebar] agents names sidebar.agents's, as it did before slots, and the plugin it names gives
// its sidebar section that place when it hasn't asked for it with a piece of its own. A plugin holds a slot only while
// it has something for it.
fn held<'a>(shown: &[Shown<'a>], slots: &IndexMap<String, String>, sidebar_agents: &str) -> HashMap<&'static str, Held<'a>> {
    let mut out = HashMap::new();
    for &slot in REPLACEABLE {
        let named = slots.get(slot).map(String::as_str).filter(|s| !s.is_empty()).or_else(|| Some(sidebar_agents).filter(|s| slot == "sidebar.agents" && !s.is_empty()));
        let who = match named {
            Some("builtin") => continue,
            Some(name) => shown.iter().find(|s| s.plugin == name),
            None => shown.iter().filter(|s| asked(s, slot)).min_by_key(|s| s.plugin),
        };
        let Some(s) = who else { continue };
        if asked(s, slot) {
            out.insert(slot, Held { plugin: s.plugin, section: None });
        } else if let Some(p) = s.pieces.values().find(|p| slot == "sidebar.agents" && p.slot == "sidebar") {
            out.insert(slot, Held { plugin: s.plugin, section: Some(p.key()) });
        }
    }
    out
}

// Every piece clients draw (View.slots), in order: by `order`, then plugin name, then id. A replacing piece `replaces`
// only when its plugin holds the slot; one that doesn't is there to say who asked, and isn't drawn.
pub fn resolve(shown: &[Shown], slots: &IndexMap<String, String>, sidebar_agents: &str) -> Vec<SlotPiece> {
    let held = held(shown, slots, sidebar_agents);
    let mut out: Vec<SlotPiece> = vec![];
    for s in shown {
        let holds = |slot: &str| held.get(slot).is_some_and(|h| h.plugin == s.plugin);
        let moved = held.get("sidebar.agents").filter(|h| h.plugin == s.plugin).and_then(|h| h.section.clone());
        for p in s.pieces.values() {
            let mut w = p.wire(s.plugin, s.run);
            w.replaces = p.position == "replace" && holds(p.slot);
            if moved.as_ref() == Some(&p.key()) {
                w.slot = "sidebar.agents".into();
                (w.position, w.replaces) = (SlotPosition::Replace, true);
            }
            out.push(w);
        }
    }
    out.sort_by(|a, b| (a.order, &a.plugin, &a.id).cmp(&(b.order, &b.plugin, &b.id)));
    out
}

// The slots a plugin holds, and those it asked to replace that the user, or a plugin before it by name, has.
pub fn replacing(shown: &[Shown], slots: &IndexMap<String, String>, sidebar_agents: &str, plugin: &str) -> (Vec<&'static str>, Vec<&'static str>) {
    let held = held(shown, slots, sidebar_agents);
    let holds: Vec<&'static str> = REPLACEABLE.iter().copied().filter(|s| held.get(s).is_some_and(|h| h.plugin == plugin)).collect();
    let mine = shown.iter().find(|s| s.plugin == plugin);
    let asks = REPLACEABLE.iter().copied().filter(|s| !holds.contains(s) && mine.is_some_and(|m| asked(m, s))).collect();
    (holds, asks)
}

// Pieces for what's gone (a pane's process, a tab, a space) go with it. Whether any did.
pub fn prune(pieces: &mut IndexMap<String, Piece>, alive: impl Fn(&Target) -> bool) -> bool {
    let before = pieces.len();
    pieces.retain(|_, p| alive(&p.target));
    pieces.len() != before
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world_with<'a>(actions: &'a [String]) -> World<'a> {
        World {
            process: &|p, i| p == "p1" && i == "aaaa",
            tab: &|t| if t == "t1" || t == "work" { Ok("t1".into()) } else { Err(fail("error", format!("no such tab: {t}"))) },
            space: &|s| if s == "api" || s == "w1" { Ok("api".into()) } else { Err(fail("error", format!("no such space: {s}"))) },
            actions,
        }
    }

    fn offered() -> Vec<String> {
        vec!["open".into(), "details".into()]
    }

    fn ok(p: Value) -> Piece {
        let a = offered();
        check(&p, &world_with(&a)).unwrap_or_else(|e| panic!("{p}: {}", e.message))
    }

    fn refused(p: Value) -> (String, String) {
        let a = offered();
        let e = check(&p, &world_with(&a)).err().unwrap_or_else(|| panic!("{p} was taken"));
        (e.code, e.message)
    }

    #[test]
    fn takes_a_piece_in_every_slot() {
        let line = json!([{ "text": "43%", "style": "$warn" }]);
        for (slot, extra) in [
            ("status.left", json!({ "line": line })),
            ("status.right", json!({ "line": "x", "action": "open", "hide_below": { "width": 100 } })),
            ("status.git", json!({ "line": line, "position": "replace" })),
            ("tab", json!({ "tab": "work", "line": "x", "position": "before" })),
            ("space", json!({ "space": "w1", "line": "x" })),
            ("pane.title", json!({ "pane": "p1", "instance": "aaaa", "line": line, "position": "replace" })),
            ("pane.bottom_right", json!({ "pane": "p1", "instance": "aaaa", "line": { "spans": ["a", { "icon": "codex" }], "align": "right" } })),
            ("agent.row", json!({ "pane": "p1", "instance": "aaaa", "lines": [line, "second"], "order": -2 })),
            ("sidebar", json!({ "title": "Usage", "lines": ["a", { "spans": ["b"], "action": "open" }, { "spans": ["c"], "pane": "p1", "instance": "aaaa" }] })),
            ("sidebar", json!({ "element": { "type": "sparkline", "data": [1, 2, 3] }, "height": 3 })),
            ("sidebar.agents", json!({ "element": { "type": "list", "id": "l", "items": ["a"], "action": "open" } })),
            ("menu.pane", json!({ "title": "Mark seen", "action": "open" })),
            ("menu.tab", json!({ "tab": "t1", "title": "Rename…", "action": "open" })),
            ("palette", json!({ "title": "Refresh", "action": "details", "line": "⌘" })),
        ] {
            let mut p = extra.clone();
            p["slot"] = json!(slot);
            p["id"] = json!("x");
            let piece = ok(p);
            assert_eq!(piece.slot, slot);
            let wire = piece.wire("demo", "r1"); // reads back as clients get it
            assert_eq!((wire.slot.as_str(), wire.id.as_str(), wire.plugin.as_str()), (slot, "x", "demo"));
            assert_eq!(serde_json::to_value(&wire).unwrap()["lines"], piece.body.get("lines").cloned().unwrap_or(Value::Null));
        }
        // targets are what the session calls them now: a tab's id, a space's name
        assert_eq!(ok(json!({ "slot": "tab", "id": "x", "tab": "work", "line": "x" })).target, Target::Tab("t1".into()));
        assert_eq!(ok(json!({ "slot": "space", "id": "x", "space": "w1", "line": "x" })).target, Target::Space("api".into()));
        // defaults: after, order 0; sidebar.agents is replace; a menu entry with no target is for every pane
        let p = ok(json!({ "slot": "status.right", "id": "x", "line": "x" }));
        assert_eq!((p.position, p.order), ("after", 0));
        assert_eq!(ok(json!({ "slot": "sidebar.agents", "id": "x", "lines": [] })).position, "replace");
        assert_eq!(ok(json!({ "slot": "menu.pane", "id": "x", "title": "t", "action": "open" })).target, Target::Nothing);
        // one line is lines where a slot takes several
        assert_eq!(ok(json!({ "slot": "agent.row", "id": "x", "pane": "p1", "instance": "aaaa", "line": "43%" })).body["lines"], json!(["43%"]));
    }

    #[test]
    fn refuses_what_chrome_md_doesnt_say() {
        let p1 = |mut v: Value| {
            v["pane"] = json!("p1");
            v["instance"] = json!("aaaa");
            v
        };
        let says = |v: Value, code: &str, text: &str| {
            let (c, m) = refused(v.clone());
            assert_eq!(c, code, "{v}: {m}");
            assert!(m.contains(text), "{v}: {m:?} doesn't say {text:?}");
        };
        says(json!({ "slot": "statusbar", "id": "x", "line": "x" }), "invalid_params", "slot Invalid option");
        says(json!({ "slot": "toast", "id": "x" }), "invalid_params", "ui.toast");
        says(json!({ "slot": "status.right", "line": "x" }), "invalid_params", "id Invalid input");
        says(json!({ "slot": "status.right", "id": "x".repeat(41), "line": "x" }), "invalid_params", "id Too big");
        says(json!({ "slot": "status.right", "id": "x", "line": "x", "colour": "red" }), "invalid_params", "colour Unrecognized key");
        says(json!({ "slot": "status.right", "id": "x", "line": "x", "title": "t" }), "invalid_params", "doesn't take title");
        says(json!({ "slot": "status.right", "id": "x" }), "invalid_params", "takes line");
        says(json!({ "slot": "status.right", "id": "x", "lines": ["x"] }), "invalid_params", "doesn't take lines");
        says(json!({ "slot": "status.right", "id": "x", "line": [{ "text": "a", "colour": "red" }] }), "invalid_params", "line.0 Unrecognized key");
        says(json!({ "slot": "status.right", "id": "x", "line": [{ "text": "a", "style": "$nope" }] }), "invalid_params", "theme colour");
        says(json!({ "slot": "status.right", "id": "x", "line": "x", "position": "replace" }), "invalid_params", "nothing of modisa's");
        says(json!({ "slot": "status.right", "id": "x", "line": "x", "position": "middle" }), "invalid_params", "position Invalid option");
        says(json!({ "slot": "status.right", "id": "x", "line": "x", "order": 1.5 }), "invalid_params", "order");
        says(json!({ "slot": "status.right", "id": "x", "line": "x", "action": "nope" }), "no_such_action", "offers: open, details");
        says(json!({ "slot": "status.right", "id": "x", "line": "x", "hide_below": { "height": 3 } }), "invalid_params", "hide_below");
        says(json!({ "slot": "status.right", "id": "x", "line": "x", "pane": "p1" }), "invalid_params", "doesn't take pane");
        says(json!({ "slot": "pane.title", "id": "x", "line": "x" }), "invalid_params", "pane Invalid input: expected string, received undefined (pane.title is per pane)");
        says(json!({ "slot": "pane.title", "id": "x", "line": "x", "pane": "p1" }), "invalid_params", "instance");
        says(json!({ "slot": "pane.title", "id": "x", "line": "x", "pane": "p1", "instance": "bbbb" }), "pane_gone", "no pane p1 with instance bbbb");
        says(p1(json!({ "slot": "pane.top_right", "id": "x", "line": "x", "position": "replace" })), "invalid_params", "nothing of modisa's");
        says(json!({ "slot": "tab", "id": "x", "tab": "t9", "line": "x" }), "error", "no such tab: t9");
        says(json!({ "slot": "space", "id": "x", "space": "nope", "line": "x" }), "error", "no such space: nope");
        says(p1(json!({ "slot": "agent.row", "id": "x", "lines": ["1", "2", "3", "4"] })), "invalid_params", "<=3");
        says(p1(json!({ "slot": "agent.row", "id": "x", "lines": [] })), "invalid_params", ">=1");
        says(p1(json!({ "slot": "agent.row", "id": "x", "line": "a", "lines": ["b"] })), "invalid_params", "not both");
        says(json!({ "slot": "sidebar", "id": "x", "lines": vec!["r"; 31] }), "invalid_params", "<=30");
        says(json!({ "slot": "sidebar", "id": "x", "element": { "type": "clear" } }), "invalid_params", "height");
        says(json!({ "slot": "sidebar", "id": "x", "element": { "type": "clear" }, "height": 31 }), "invalid_params", "height Too big");
        says(json!({ "slot": "sidebar", "id": "x", "lines": [], "height": 3 }), "invalid_params", "doesn't take height");
        says(json!({ "slot": "sidebar", "id": "x", "element": { "type": "box" }, "height": 3 }), "invalid_params", "element.type");
        says(json!({ "slot": "sidebar", "id": "x", "element": { "type": "button", "id": "b", "action": "nope" }, "height": 3 }), "no_such_action", "nope");
        says(json!({ "slot": "sidebar", "id": "x", "lines": [{ "spans": ["a"], "action": "nope" }] }), "no_such_action", "nope");
        says(json!({ "slot": "sidebar", "id": "x", "lines": [{ "spans": ["a"], "pane": "p1" }] }), "invalid_params", "lines.0.instance");
        says(json!({ "slot": "sidebar", "id": "x", "lines": [{ "spans": ["a"], "pane": "p1", "instance": "old" }] }), "pane_gone", "p1");
        says(json!({ "slot": "sidebar", "id": "x", "lines": ["a"], "element": { "type": "clear" } }), "invalid_params", "not both");
        says(json!({ "slot": "sidebar", "id": "x", "lines": ["a"], "position": "replace" }), "invalid_params", "nothing of modisa's");
        says(json!({ "slot": "sidebar.agents", "id": "x", "lines": ["a"], "position": "after" }), "invalid_params", "replace only");
        says(json!({ "slot": "menu.pane", "id": "x", "title": "t" }), "invalid_params", "action");
        says(json!({ "slot": "menu.pane", "id": "x", "action": "open" }), "invalid_params", "title");
        says(json!({ "slot": "menu.pane", "id": "x", "title": "t", "action": "open", "line": "x" }), "invalid_params", "doesn't take line");
        says(json!({ "slot": "menu.tab", "id": "x", "title": "t", "action": "open", "pane": "p1", "instance": "aaaa" }), "invalid_params", "doesn't take pane");
        says(json!({ "slot": "palette", "id": "x", "title": "t", "action": "open", "lines": ["x"] }), "invalid_params", "doesn't take lines");
    }

    #[test]
    fn cleans_text_and_cuts_a_line_to_200_cells() {
        let p = ok(json!({ "slot": "status.right", "id": "x", "line": [{ "text": "\x1b[31mred\x07", "style": "bold" }, "\x1b]0;title\x07ok"] }));
        assert_eq!(p.body["line"], json!([{ "text": "red", "style": "bold" }, "ok"]));
        let long = ok(json!({ "slot": "status.right", "id": "x", "line": { "spans": ["x".repeat(150), { "icon": "codex" }, "y".repeat(100), "dropped"], "style": "$dim" } }));
        let spans = long.body["line"]["spans"].as_array().unwrap().clone();
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[2].as_str().unwrap().len(), 48); // 150 + an icon's 2 + 48 = 200
        assert_eq!(long.body["line"]["style"], "$dim");
        assert_eq!(clean_line(&json!("日本語"), 5), json!("日本")); // cells, not characters
        let title = ok(json!({ "slot": "palette", "id": "x", "title": format!("\x1b[1m{}", "t".repeat(60)), "action": "open" }));
        assert_eq!(title.body["title"], json!("t".repeat(40)));
        let el = ok(json!({ "slot": "sidebar", "id": "x", "element": { "type": "text", "text": "a\x1b[2Jb" }, "height": 1 }));
        assert_eq!(el.body["element"]["text"], "ab");
    }

    #[test]
    fn clear_matches_what_it_names() {
        let a = ok(json!({ "slot": "agent.row", "id": "ctx", "pane": "p1", "instance": "aaaa", "line": "x" }));
        let b = ok(json!({ "slot": "status.right", "id": "ctx", "line": "x" }));
        let m = |p: Value| matcher(&p).unwrap();
        assert!(m(json!({}))(&a) && m(json!({}))(&b));
        assert!(m(json!({ "id": "ctx" }))(&a) && m(json!({ "id": "ctx" }))(&b));
        assert!(m(json!({ "pane": "p1" }))(&a) && !m(json!({ "pane": "p1" }))(&b));
        assert!(!m(json!({ "slot": "agent.row", "id": "other" }))(&a));
        assert!(matcher(&json!({ "slot": "nope" })).is_err());
    }

    #[test]
    fn the_older_methods_are_pieces() {
        let s = status("radar", json!({ "id": "count", "text": "3 waiting", "tone": "warn", "action": "open" }));
        assert_eq!((s.slot, s.id.as_str(), s.position), ("status.right", "count", "after"));
        assert_eq!(s.body["line"], json!([{ "text": "radar: 3 waiting", "style": "$warn" }]));
        assert_eq!(s.body["action"], "open");
        let b = badge("radar", json!({ "pane": "p1", "instance": "aaaa", "text": "blocked", "tone": "warn" }));
        assert_eq!((b.slot, b.target.clone()), ("pane.title", Target::Pane("p1".into(), "aaaa".into())));
        assert_eq!(b.body["line"], json!([{ "text": "[radar: blocked]", "style": "$warn" }]));
        let section = section(json!({ "title": "Agents", "rows": [
            { "text": "@w blocked", "tone": "warn", "pane": "p1", "instance": "aaaa" },
            { "text": "✳ claude", "tone": "fg", "spans": [{ "icon": "claude-code" }, { "text": " claude", "tone": "done", "bold": true }, { "text": " · x" }], "action": "open" },
        ] }));
        assert_eq!(section.body["title"], "Agents");
        assert_eq!(section.body["lines"], json!([
            { "spans": [{ "text": "@w blocked", "style": "$warn" }], "pane": "p1", "instance": "aaaa" },
            { "spans": [{ "icon": "claude-code" }, { "text": " claude", "style": "bold $done" }, { "text": " · x", "style": "$fg" }], "action": "open" },
        ]));
        let m = menu_entry(json!({ "id": "seen", "title": "Mark seen", "action": "open" }));
        assert_eq!((m.slot, m.body["title"].clone(), m.target.clone()), ("menu.pane", json!("Mark seen"), Target::Nothing));
        // each kept as it was set, for UI 3 clients
        let pieces: IndexMap<String, Piece> = [s, b, section, m, ok(json!({ "slot": "status.right", "id": "new", "line": "x" }))].into_iter().map(|p| (p.key(), p)).collect();
        let l = legacy(&pieces);
        assert_eq!(l.status, [json!({ "id": "count", "text": "3 waiting", "tone": "warn", "action": "open" })]);
        assert_eq!(l.badges.len(), 1);
        assert_eq!(l.sidebar.unwrap()["title"], "Agents");
        assert_eq!(l.menu, [json!({ "id": "seen", "title": "Mark seen", "action": "open" })]);
    }

    fn pieces(list: Vec<Value>) -> IndexMap<String, Piece> {
        list.into_iter().map(ok).map(|p| (p.key(), p)).collect()
    }

    // what's drawn: (plugin, slot, id, position), a replace that lost left out
    fn drawn(shown: &[Shown], slots: &[(&str, &str)], sidebar_agents: &str) -> Vec<(String, String, String, String)> {
        let slots: IndexMap<String, String> = slots.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let pos = |p: &SlotPiece| serde_json::to_value(p.position).unwrap().as_str().unwrap().to_string();
        resolve(shown, &slots, sidebar_agents).iter().filter(|p| p.position != SlotPosition::Replace || p.replaces).map(|p| (p.plugin.clone(), p.slot.clone(), p.id.clone(), pos(p))).collect()
    }

    #[test]
    fn who_replaces_what() {
        let row = |id: &str| json!({ "slot": "agent.row", "id": id, "pane": "p1", "instance": "aaaa", "line": "x", "position": "replace" });
        let radar = pieces(vec![row("r"), json!({ "slot": "status.right", "id": "s", "line": "x" })]);
        let usage = pieces(vec![row("u"), json!({ "slot": "agent.row", "id": "after", "pane": "p1", "instance": "aaaa", "line": "x" })]);
        let shown = [Shown { plugin: "usage", run: "r2", pieces: &usage }, Shown { plugin: "radar", run: "r1", pieces: &radar }];
        let has = |d: &[(String, String, String, String)], plugin: &str, id: &str| d.iter().any(|x| x.0 == plugin && x.2 == id);
        // nothing in [slots]: the first by name that asked
        let d = drawn(&shown, &[], "");
        assert!(has(&d, "radar", "r") && !has(&d, "usage", "u"));
        assert!(has(&d, "usage", "after")); // what only adds is always there
        assert_eq!(replacing(&shown, &IndexMap::new(), "", "radar"), (vec!["agent.row"], vec![]));
        assert_eq!(replacing(&shown, &IndexMap::new(), "", "usage"), (vec![], vec!["agent.row"]));
        // the user's choice
        let d = drawn(&shown, &[("agent.row", "usage")], "");
        assert!(!has(&d, "radar", "r") && has(&d, "usage", "u"));
        // builtin: nobody
        let d = drawn(&shown, &[("agent.row", "builtin")], "");
        assert!(!has(&d, "radar", "r") && !has(&d, "usage", "u"));
        // a plugin named but not asking: modisa draws its own
        assert!(drawn(&shown, &[("agent.row", "jev")], "").iter().all(|x| x.3 != "replace"));
        // a replace that lost is still sent, to say who asked
        let asked: Vec<(String, bool)> = resolve(&shown, &IndexMap::new(), "").into_iter().filter(|p| p.position == SlotPosition::Replace).map(|p| (p.plugin, p.replaces)).collect();
        assert_eq!(asked, [("radar".to_string(), true), ("usage".to_string(), false)]);
    }

    #[test]
    fn sidebar_agents_is_the_old_setting_too() {
        let radar = pieces(vec![json!({ "slot": "sidebar", "id": "s", "title": "radar", "lines": ["a"] })]);
        let other = pieces(vec![json!({ "slot": "sidebar.agents", "id": "mine", "lines": ["b"] })]);
        let shown = [Shown { plugin: "radar", run: "r1", pieces: &radar }, Shown { plugin: "zed", run: "r2", pieces: &other }];
        // [sidebar] agents = "radar": its section takes the AGENTS list's place, and isn't a section too
        let d = drawn(&shown, &[], "radar");
        assert!(d.contains(&("radar".into(), "sidebar.agents".into(), "s".into(), "replace".into())));
        assert!(!d.iter().any(|x| x.1 == "sidebar"));
        assert!(!d.iter().any(|x| x.0 == "zed"));
        assert_eq!(replacing(&shown, &IndexMap::new(), "radar", "radar").0, vec!["sidebar.agents"]);
        // [slots] outranks it
        let d = drawn(&shown, &[("sidebar.agents", "zed")], "radar");
        assert!(d.contains(&("zed".into(), "sidebar.agents".into(), "mine".into(), "replace".into())));
        assert!(d.contains(&("radar".into(), "sidebar".into(), "s".into(), "after".into())));
        // unnamed, a plain section never asked: only the plugin that did replaces
        let d = drawn(&shown, &[], "");
        assert!(d.contains(&("zed".into(), "sidebar.agents".into(), "mine".into(), "replace".into())));
        assert!(d.contains(&("radar".into(), "sidebar".into(), "s".into(), "after".into())));
    }

    #[test]
    fn ordered_by_order_then_plugin_then_id() {
        let a = pieces(vec![json!({ "slot": "status.right", "id": "z", "line": "x" }), json!({ "slot": "status.right", "id": "a", "line": "x" }), json!({ "slot": "status.right", "id": "first", "line": "x", "order": -1 })]);
        let b = pieces(vec![json!({ "slot": "status.right", "id": "b", "line": "x" })]);
        let shown = [Shown { plugin: "beta", run: "r", pieces: &b }, Shown { plugin: "alpha", run: "r", pieces: &a }];
        let ids: Vec<String> = drawn(&shown, &[], "").into_iter().map(|x| x.2).collect();
        assert_eq!(ids, ["first", "a", "z", "b"]);
        let wire = serde_json::to_value(&resolve(&shown, &IndexMap::new(), "")[0]).unwrap();
        assert_eq!(wire, json!({ "plugin": "alpha", "run": "r", "slot": "status.right", "id": "first", "position": "after", "order": -1, "line": "x" }));
    }

    #[test]
    fn pieces_for_whats_gone_go() {
        let mut p = pieces(vec![
            json!({ "slot": "agent.row", "id": "x", "pane": "p1", "instance": "aaaa", "line": "x" }),
            json!({ "slot": "tab", "id": "x", "tab": "t1", "line": "x" }),
            json!({ "slot": "space", "id": "x", "space": "api", "line": "x" }),
            json!({ "slot": "status.right", "id": "x", "line": "x" }),
        ]);
        assert!(!prune(&mut p, |_| true));
        assert!(prune(&mut p, |t| !matches!(t, Target::Pane(..))));
        assert!(prune(&mut p, |t| !matches!(t, Target::Tab(_) | Target::Space(_))));
        assert_eq!(p.values().map(|x| x.slot).collect::<Vec<_>>(), ["status.right"]);
    }
}
