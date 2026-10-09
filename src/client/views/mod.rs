// Plugins' views in this client, drawn from what the server sends (plugin.view, plugin.view.closed, plugin.blit):
// floating over everything (popup) or over the pane they're from (overlay; a popup while that pane isn't on screen),
// the newest on top. The top one has the keyboard: Tab moves between what's in it, its keys run its plugin's actions,
// Escape (or prefix x) closes it. Everything in one is framed and titled with its plugin's name.
//
// Views draw under dialogs and pane popups: a dialog opened over a view has the keyboard until it closes, then the view
// has it back.
pub mod build;
pub mod charts;

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Position;
use ratatui::style::Modifier;
use serde_json::{json, Map, Value};

use super::design::{fit, floating};
use super::draw::{Canvas, Hit};
use super::App;
use crate::config::BorderStyle;
use crate::core::layout::{display_rects, Rect};
use crate::core::text::width;
use build::{focusables, key_of, walk, Ctx, Draw, ElState, Focus, Fired};

pub struct OpenView {
    pub state: Value, // as the server holds it (PluginViewState)
    pub elems: HashMap<String, ElState>, // what the user did in its elements, by key
    pub focus: Option<String>, // the key of what has the keyboard, kept across updates; None: nothing
    pub focus_rev: Option<u64>, // the rev whose `focus` was applied: a plugin hands the keyboard over once per update
    pub armed: bool, // the prefix was pressed: x closes
}

const MIN: (i32, i32) = (20, 5);
static START: LazyLock<Instant> = LazyLock::new(Instant::now);

fn id_of(s: &Value) -> String {
    format!("{}/{}", s["plugin"].as_str().unwrap_or(""), s["id"].as_str().unwrap_or(""))
}

// ---------- what the server sends ----------

pub fn view_set(app: &mut App, state: Value) {
    let id = id_of(&state);
    let i = match app.views.iter().position(|v| id_of(&v.state) == id) {
        Some(i) => i,
        None => {
            app.views.push(OpenView { state: Value::Null, elems: HashMap::new(), focus: None, focus_rev: None, armed: false });
            app.views.len() - 1
        }
    };
    let v = &mut app.views[i];
    v.elems = build::reconcile(&mut v.elems, &state["root"]);
    v.state = state;
    refocus(v);
    animate(app);
    app.dirty();
}

// Focus goes where the plugin says, once per update; else it stays on what had it, by key; else the first field, list,
// button or scroll area.
fn refocus(v: &mut OpenView) {
    let fs = focusables(&v.state["root"]);
    if let Some(f) = v.state["focus"].as_str() {
        let rev = v.state["rev"].as_u64();
        if v.focus_rev != rev {
            v.focus_rev = rev;
            v.focus = Some(format!("#{f}"));
        }
    }
    if v.focus.as_ref().is_some_and(|k| fs.iter().any(|(x, _)| x == k)) {
        return;
    }
    v.focus = [Focus::Field, Focus::List, Focus::Button, Focus::Scroll].iter().find_map(|kind| fs.iter().find(|(_, f)| f == kind)).map(|(k, _)| k.clone());
}

pub fn view_closed(app: &mut App, plugin: &str, id: &str) {
    let gone = format!("{plugin}/{id}");
    app.views.retain(|v| id_of(&v.state) != gone);
    app.dirty();
}

pub fn view_blit(app: &mut App, d: &Value) {
    let id = format!("{}/{}", d["plugin"].as_str().unwrap_or(""), d["view"].as_str().unwrap_or(""));
    let Some(v) = app.views.iter_mut().find(|v| id_of(&v.state) == id) else { return };
    // kept in the state: the next frame draws what's there now
    fn swap(n: &mut Value, key: &str, cells: &Value) {
        if n["type"] == "raster" && n["key"] == key {
            n["cells"] = cells.clone();
        } else if let Some(kids) = n.get_mut("children").and_then(Value::as_array_mut) {
            for k in kids {
                swap(k, key, cells);
            }
        }
    }
    swap(&mut v.state["root"], d["key"].as_str().unwrap_or(""), &d["cells"]);
    app.dirty();
}

// The connection went: what plugins showed goes with it (the next attach sends what's open then).
pub fn clear_views(app: &mut App) {
    app.views.clear();
    app.dirty();
}

// A view has the keyboard: the top one, unless a dialog or a popup is open over it.
pub fn has_keyboard(app: &App) -> bool {
    !app.views.is_empty() && app.modal.is_none()
}

// Spinners move: a frame every 80ms while one shows (in a view, or a dialog waiting on something).
pub fn animate(app: &mut App) {
    if app.animating || !spinning(app) {
        return;
    }
    app.animating = true;
    let me = app.me.clone();
    tokio::task::spawn_local(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(80)).await;
            let Some(a) = me.upgrade() else { return };
            let mut a = a.borrow_mut();
            if !spinning(&a) || a.quitting.is_some() {
                a.animating = false;
                return;
            }
            a.dirty();
        }
    });
}

fn spinning(app: &App) -> bool {
    super::modals::busy(app) || app.views.iter().any(|v| {
        let mut any = false;
        walk(&v.state["root"], "0".into(), &mut |n, _| any |= n["type"] == "spinner");
        any
    })
}

pub fn tick() -> u64 {
    (START.elapsed().as_millis() / 80) as u64
}

// ---------- placing and drawing ----------

// cells from a view's size: a number of cells, or a percentage of the terminal
pub fn cells(size: &Value, total: i32, fallback: i32) -> i32 {
    if let Some(n) = size.as_f64() {
        return n as i32;
    }
    match size.as_str().and_then(|s| s.strip_suffix('%')).and_then(|p| p.trim().parse::<f64>().ok()) {
        Some(p) => ((total as f64 * p) / 100.0).floor() as i32,
        None => fallback,
    }
}

fn rect_of(app: &App, s: &Value) -> Rect {
    let (w, h) = (app.width, app.height);
    if s["placement"] == "overlay" && app.ready() {
        if let Some(pane) = s["from"]["pane"].as_str() {
            let t = app.tab();
            let rs = display_rects(&t.tree, app.area(), &t.focused, t.zoomed);
            if let Some(r) = rs.get(pane).filter(|_| app.info(pane).is_some_and(|p| s["from"]["instance"] == p.instance.as_str())) {
                return *r;
            }
        }
    }
    let vw = (w - 2).min(cells(&s["width"], w, (w as f64 * 0.7).floor() as i32).max(MIN.0)).max(1);
    let vh = (h - 2).min(cells(&s["height"], h, (h as f64 * 0.6).floor() as i32).max(MIN.1)).max(1);
    floating(w, h, vw, vh, Some((w - vw) / 2), Some((h - vh) / 3))
}

// Every view, the newest on top, each framed and titled; returns where the caret of the field with the keyboard is.
pub fn draw(app: &mut App, c: &mut Canvas) -> Option<Position> {
    if app.views.is_empty() {
        return None;
    }
    let mut views = std::mem::take(&mut app.views);
    let mut cursor = None;
    {
        let app = &*app;
        let th = app.th;
        let ctx = Ctx { th: &th, logos: app.logos, cell: app.cell_ems(), tick: tick() };
        let n = views.len();
        for (i, v) in views.iter_mut().enumerate() {
            let top = i + 1 == n;
            let r = rect_of(app, &v.state);
            if top {
                c.hit(Rect { x: 0, y: 0, w: c.w, h: c.h }, Hit::Inert); // under the top one, nothing takes a click
            }
            c.fill(r, th.bg);
            c.hit(r, Hit::Inert);
            let s = &v.state;
            let fs = focusables(&s["root"]);
            let keys: Vec<String> = s["keys"].as_array().into_iter().flatten().filter_map(|k| k["description"].as_str().filter(|d| !d.is_empty()).map(|d| format!("{} {d}", k["key"].as_str().unwrap_or("")))).collect();
            let mut bottom = keys;
            if fs.len() > 1 {
                bottom.push("tab moves".into());
            }
            bottom.push("esc closes".into());
            let room = (r.w - 4).max(0) as usize;
            let title = fit(&format!(" {} · {} ", s["plugin"].as_str().unwrap_or(""), s["title"].as_str().unwrap_or("")), room);
            let (edge, tc) = if top { (th.focus, th.focus) } else { (th.border, th.dim) };
            c.border(r, BorderStyle::Rounded, edge, Some(th.bg), Some((&title, tc)));
            let bt = fit(&format!(" {} ", bottom.join(" · ")), room);
            if r.h >= 2 && !bt.is_empty() {
                c.text(r.x + r.w - 2 - width(&bt) as i32, r.y + r.h - 1, &bt, tc, Some(th.bg), Modifier::empty(), room);
            }
            // inside the border, a cell of padding each side
            let inner = Rect { x: r.x + 2, y: r.y + 1, w: (r.w - 4).max(0), h: (r.h - 2).max(0) };
            let id = id_of(s);
            let el = build::layout(&ctx, &s["root"], inner, &mut v.elems);
            let mut d = Draw { view: &id, focus: v.focus.as_deref(), active: top && app.modal.is_none(), cursor: None };
            build::draw(&ctx, c, &el, inner, &mut v.elems, &mut d);
            if d.active {
                cursor = d.cursor;
            }
        }
    }
    app.views = views;
    cursor
}

// ---------- the keyboard ----------

// A key as views name it: "j", "J", "enter", "S-tab", "C-s", "M-x".
pub fn view_key_name(k: &KeyEvent) -> String {
    let m = k.modifiers;
    let (ctrl, meta, shift) = (m.contains(KeyModifiers::CONTROL), m.contains(KeyModifiers::ALT), m.contains(KeyModifiers::SHIFT));
    let base = match k.code {
        KeyCode::Char(' ') => "space".to_string(),
        KeyCode::Char(c) if ctrl || meta => c.to_lowercase().to_string(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::BackTab => return "S-tab".into(),
        other => super::modals::key_name(&KeyEvent::new(other, KeyModifiers::NONE)).replace("return", "enter"),
    };
    let letter = matches!(k.code, KeyCode::Char(c) if c.is_ascii_alphabetic());
    let shifted = shift && !matches!(k.code, KeyCode::Char(_)); // a character already says it was shifted
    let base = if letter && shift && (ctrl || meta) { base.to_uppercase() } else { base };
    format!("{}{}{}{base}", if ctrl { "C-" } else { "" }, if meta { "M-" } else { "" }, if shifted { "S-" } else { "" })
}

fn node_at<'a>(root: &'a Value, key: &str) -> Option<&'a Value> {
    let mut found = None;
    walk(root, key_of(root, "0"), &mut |n, k| {
        if found.is_none() && k == key {
            found = Some(n);
        }
    });
    found
}

const LIST_KEYS: [&str; 11] = ["up", "down", "j", "k", "S-up", "S-down", "enter", "left", "right", "[", "]"];

fn scroll_step(name: &str) -> Option<i32> {
    Some(match name {
        "up" | "k" => -1,
        "down" | "j" => 1,
        "pageup" | "C-u" => -10,
        "pagedown" | "C-d" => 10,
        _ => return None,
    })
}

// A key while the top view has the keyboard. Nothing it gets reaches the panes under it.
pub fn key(app: &mut App, k: &KeyEvent) {
    let Some(i) = app.views.len().checked_sub(1) else { return };
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let is_prefix = app.prefix.ctrl && ctrl && matches!(k.code, KeyCode::Char(c) if c.to_lowercase().to_string() == app.prefix.name);
    app.dirty();
    // prefix x closes it, as it closes a popup
    if is_prefix && !app.views[i].armed {
        app.views[i].armed = true;
        return;
    }
    let name = view_key_name(k);
    if app.views[i].armed {
        app.views[i].armed = false;
        if name == "x" {
            return close_by_user(app, i);
        }
    }
    let printable = match k.code {
        KeyCode::Char(c) if !ctrl && !k.modifiers.contains(KeyModifiers::ALT) => Some(c),
        _ => None,
    };
    let v = &mut app.views[i];
    let fs = focusables(&v.state["root"]);
    let at = v.focus.as_ref().and_then(|f| fs.iter().position(|(x, _)| x == f));
    let kind = at.map(|a| fs[a].1);
    let typing = kind == Some(Focus::Field);
    if name == "escape" {
        if typing {
            v.focus = None; // out of the field first; Escape again closes
            return;
        }
        return close_by_user(app, i);
    }
    if name == "tab" || name == "S-tab" {
        let n = fs.len();
        if n > 0 {
            let to = match at {
                None if name == "tab" => 0,
                None => n - 1,
                Some(a) if name == "tab" => (a + 1) % n,
                Some(a) => (a + n - 1) % n,
            };
            v.focus = Some(fs[to].0.clone());
        }
        return;
    }
    let bound = v.state["keys"].as_array().into_iter().flatten().find(|x| x["key"] == name.as_str()).cloned();
    let bound_fire = |b: &Value| Fired { action: b["action"].as_str().map(String::from), params: b.get("params").cloned().unwrap_or(json!({})), ui: Map::new() };
    let focused = v.focus.clone();
    let node = focused.as_deref().and_then(|f| node_at(&v.state["root"], f)).cloned();
    if let (Some(f), Some(n)) = (focused.as_deref(), node.as_ref()) {
        let st = v.elems.entry(f.to_string()).or_default();
        let took = if typing {
            // a field takes what's typed; only a view key with Ctrl or Alt gets past it
            Some(match bound.as_ref().filter(|_| name.starts_with("C-") || name.starts_with("M-")) {
                Some(b) => Some(bound_fire(b)),
                None => build::field_key(n, st, &name, printable).flatten(),
            })
        } else if kind == Some(Focus::List) && LIST_KEYS.contains(&name.as_str()) && bound.is_none() {
            Some(build::list_key(n, st, &name).flatten()) // the list moves and chooses
        } else if n["type"] == "diff" {
            build::diff_key(n, st, &name) // an element with keys of its own (a diff's cursor) took it
        } else {
            None
        };
        if let Some(fired) = took {
            return fire(app, i, fired);
        }
    }
    if let Some(b) = bound {
        return fire(app, i, Some(bound_fire(&b)));
    }
    if kind == Some(Focus::Button) && (name == "enter" || name == "space") {
        return fire(app, i, node.as_ref().map(build::press));
    }
    if let Some(delta) = scroll_step(&name) {
        // what has the keyboard, if it scrolls; else the first thing that does
        let target = focused.filter(|_| kind == Some(Focus::Scroll)).or_else(|| fs.iter().find(|(_, f)| *f == Focus::Scroll).map(|(k, _)| k.clone()));
        if let Some(t) = target {
            if let Some(n) = node_at(&v.state["root"], &t).cloned() {
                build::scroll_by(&n, v.elems.entry(t).or_default(), delta);
            }
        }
    }
}

// Text pasted while a view has the keyboard goes into the field that has it.
pub fn paste(app: &mut App, text: &str) {
    let Some(v) = app.views.last_mut() else { return };
    let Some(f) = v.focus.clone() else { return };
    let Some(n) = node_at(&v.state["root"], &f).cloned() else { return };
    if build::focus_kind(n["type"].as_str().unwrap_or("")) != Some(Focus::Field) {
        return;
    }
    let area = n["type"] == "textarea";
    let st = v.elems.entry(f).or_default();
    for ch in text.chars().filter(|c| !c.is_control() || (area && *c == '\n')) {
        build::field_key(&n, st, "", Some(ch));
    }
    app.dirty();
}

fn close_by_user(app: &mut App, i: usize) {
    let v = app.views.remove(i);
    app.dirty();
    let Some(conn) = app.conn.clone() else { return };
    let params = json!({ "plugin": v.state["plugin"], "id": v.state["id"] });
    tokio::task::spawn_local(async move {
        let _ = conn.request("plugin.view.close", params, None).await; // already gone is fine
    });
}

// An element was used: its plugin's action, with what it holds. Only a failure is shown; what an action does, its
// plugin shows.
fn fire(app: &mut App, i: usize, f: Option<Fired>) {
    let Some(f) = f else { return };
    let Some(action) = f.action else { return };
    let Some(v) = app.views.get(i) else { return };
    let (plugin, run, view) = (v.state["plugin"].as_str().unwrap_or("").to_string(), v.state["run"].clone(), v.state["id"].clone());
    let mut ui = Map::new();
    ui.insert("view".into(), view);
    ui.extend(f.ui);
    let Some(conn) = app.conn.clone() else { return };
    let me = app.me.clone();
    let params = json!({ "plugin": plugin, "action": action, "params": f.params, "run": run, "ui": ui });
    tokio::task::spawn_local(async move {
        if let Err(e) = conn.request("plugin.invoke", params, None).await {
            if let Some(a) = me.upgrade() {
                let mut a = a.borrow_mut();
                let c = if e.code == "timeout" { a.th.warn } else { a.th.blocked };
                a.toast(&format!("{plugin}: {action}: {}", e.message), c);
            }
        }
    });
}

// ---------- the pointer ----------

// A click on an element of a view: it takes the keyboard; a button presses, a list's row or a tab is chosen.
pub fn click(app: &mut App, view: &str, key: &str, part: i32) {
    let Some(i) = app.views.iter().position(|v| id_of(&v.state) == view) else { return };
    app.dirty();
    let v = &mut app.views[i];
    let Some(n) = node_at(&v.state["root"], key).cloned() else { return };
    if build::focus_kind(n["type"].as_str().unwrap_or("")).is_some() {
        v.focus = Some(key.to_string());
    }
    let st = v.elems.entry(key.to_string()).or_default();
    let fired = match n["type"].as_str() {
        Some("button") => Some(build::press(&n)),
        Some("select" | "tabs") if part >= 0 => build::pick(&n, st, part as usize),
        _ => None,
    };
    fire(app, i, fired);
}

// The wheel over a view: the innermost area under the pointer that scrolls.
pub fn wheel(app: &mut App, under: &[Hit], rows: i32) {
    for h in under {
        let Hit::View { view, key, .. } = h else { continue };
        let Some(v) = app.views.iter_mut().find(|v| id_of(&v.state) == *view) else { continue };
        let Some(n) = node_at(&v.state["root"], key).cloned() else { continue };
        if matches!(n["type"].as_str(), Some("scroll" | "code" | "diff" | "markdown")) {
            build::scroll_by(&n, v.elems.entry(key.clone()).or_default(), rows);
            app.dirty();
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_keys_as_views_bind_them() {
        let k = |c: KeyCode, m: KeyModifiers| view_key_name(&KeyEvent::new(c, m));
        assert_eq!(k(KeyCode::Char('j'), KeyModifiers::NONE), "j");
        assert_eq!(k(KeyCode::Char('J'), KeyModifiers::SHIFT), "J");
        assert_eq!(k(KeyCode::Enter, KeyModifiers::NONE), "enter");
        assert_eq!(k(KeyCode::BackTab, KeyModifiers::SHIFT), "S-tab");
        assert_eq!(k(KeyCode::Char('s'), KeyModifiers::CONTROL), "C-s");
        assert_eq!(k(KeyCode::Char('x'), KeyModifiers::ALT), "M-x");
        assert_eq!(k(KeyCode::Up, KeyModifiers::SHIFT), "S-up");
        assert_eq!(k(KeyCode::Char(' '), KeyModifiers::NONE), "space");
        assert_eq!(k(KeyCode::Esc, KeyModifiers::NONE), "escape");
        assert_eq!(cells(&json!("50%"), 100, 3), 50);
        assert_eq!(cells(&json!(12), 100, 3), 12);
        assert_eq!(cells(&Value::Null, 100, 3), 3);
    }
}
