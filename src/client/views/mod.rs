// Plugins' views in this client, drawn from what the server sends (plugin.view, plugin.view.closed, plugin.blit):
// floating over everything (popup) or over the pane they're from (overlay; a popup while that pane isn't on screen),
// the newest on top. The top one has the keyboard: Tab moves between what's in it, its keys run its plugin's actions,
// Escape (or prefix x) closes it. Everything in one is framed and titled with its plugin's name.
//
// Views draw under dialogs and pane popups: a dialog opened over a view has the keyboard until it closes, then the view
// has it back.
pub mod build;
pub mod charts;
pub mod code;
pub mod fields;
pub mod headless;
pub mod highlight;
pub mod image;
pub mod markdown;
pub mod raster;
pub mod tree;

use std::sync::LazyLock;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Position;
use ratatui::style::{Modifier, Style};
use serde_json::{json, Map, Value};

use super::design::{color, fit, floating};
use super::draw::{Canvas, Hit};
use super::App;
use crate::config::BorderStyle;
use crate::core::layout::{display_rects, Rect};
use crate::core::text::width;
use build::{focus_kind, focusables, node_at, Ctx, Draw, Elems, Fired, Focus};

pub struct OpenView {
    pub state: Value,                            // as the server holds it (PluginViewState)
    pub elems: Elems,                            // what the user did in its elements, by key
    pub focus: Option<String>,                   // the key of what has the keyboard, kept across updates; None: nothing
    pub focus_rev: Option<u64>,                  // the rev whose `focus` was applied: a plugin hands the keyboard over once per update
    pub armed: bool,                             // the prefix was pressed: x closes
    pub clicked: Option<(String, i32, Instant)>, // the last click, and on what: a second one there soon is a double click
}

impl OpenView {
    pub fn new(state: Value) -> Self {
        let mut v = OpenView { state: Value::Null, elems: Elems::new(), focus: None, focus_rev: None, armed: false, clicked: None };
        v.update(state);
        v
    }
    fn update(&mut self, state: Value) {
        self.elems = build::reconcile(&mut self.elems, &state["root"]);
        self.state = state;
        refocus(self);
    }
}

const MIN: (i32, i32) = (20, 5);
static START: LazyLock<Instant> = LazyLock::new(Instant::now);

fn id_of(s: &Value) -> String {
    format!("{}/{}", s["plugin"].as_str().unwrap_or(""), s["id"].as_str().unwrap_or(""))
}

// ---------- what the server sends ----------

pub fn view_set(app: &mut App, state: Value) {
    let id = id_of(&state);
    match app.views.iter_mut().find(|v| id_of(&v.state) == id) {
        Some(v) => v.update(state),
        None => app.views.push(OpenView::new(state)),
    }
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
    v.focus = [Focus::Field, Focus::Pick, Focus::Button, Focus::Scroll].iter().find_map(|kind| fs.iter().find(|(_, f)| f == kind)).map(|(k, _)| k.clone());
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
    fn swap(n: &mut Value, id: &str, cells: &Value) {
        if n["type"] == "raster" && n["id"] == id {
            n["cells"] = cells.clone();
        } else if let Some(kids) = n.get_mut("children").and_then(Value::as_array_mut) {
            kids.iter_mut().for_each(|k| swap(k, id, cells));
        } else if let Some(child) = n.get_mut("child") {
            swap(child, id, cells);
        }
    }
    swap(&mut v.state["root"], d["id"].as_str().unwrap_or(""), &d["cells"]);
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
    super::modals::busy(app)
        || super::slots::spinning(app)
        || app.views.iter().any(|v| {
            let mut any = false;
            build::walk(&v.state["root"], "0".into(), &mut |n, _| any |= n["type"] == "spinner");
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

// Every view, the newest on top; returns where the caret of the field with the keyboard is.
pub fn draw(app: &mut App, c: &mut Canvas) -> Option<Position> {
    if app.views.is_empty() {
        return None;
    }
    let mut views = std::mem::take(&mut app.views);
    let mut cursor = None;
    {
        let app = &*app;
        let ctx = Ctx { th: &app.th, logos: app.logos, cell: app.cell_ems(), tick: tick() };
        let n = views.len();
        for (i, v) in views.iter_mut().enumerate() {
            let top = i + 1 == n;
            if top {
                c.hit(Rect { x: 0, y: 0, w: c.w, h: c.h }, Hit::Inert); // under the top one, nothing takes a click
            }
            let active = top && app.modal.is_none();
            let at = draw_view(&ctx, c, v, rect_of(app, &v.state), top, active);
            if active {
                cursor = at;
            }
        }
    }
    app.views = views;
    cursor
}

fn cells_of(r: Rect) -> ratatui::layout::Rect {
    let (x, y) = (r.x.max(0), r.y.max(0));
    ratatui::layout::Rect::new(x as u16, y as u16, (r.x + r.w - x).max(0) as u16, (r.y + r.h - y).max(0) as u16)
}

// A view in `r`: its frame (its plugin and title on top; its keys, Tab and Escape at the bottom), and its elements
// inside, a cell in from the sides. Returns where the caret goes.
pub fn draw_view(ctx: &Ctx, c: &mut Canvas, v: &mut OpenView, r: Rect, top: bool, active: bool) -> Option<Position> {
    let th = ctx.th;
    c.fill(r, th.bg);
    let area = cells_of(r).intersection(c.buf.area);
    c.buf.set_style(area, Style::new().fg(color(th.fg)));
    c.hit(r, Hit::Inert);
    let s = &v.state;
    let fs = focusables(&s["root"]);
    let keys: Vec<String> = s["keys"].as_array().into_iter().flatten().filter_map(|k| k["description"].as_str().filter(|d| !d.is_empty()).map(|d| format!("{} {d}", k["key"].as_str().unwrap_or("")))).collect();
    let mut bottom = keys;
    if !bottom.is_empty() {
        bottom.push("? keys".into());
    }
    if fs.len() > 1 {
        bottom.push("tab moves".into());
    }
    bottom.push("esc closes".into());
    let room = (r.w - 4).max(0) as usize;
    let (plugin, name) = (s["plugin"].as_str().unwrap_or(""), s["title"].as_str().unwrap_or(""));
    let title = fit(&if plugin.is_empty() { format!(" {name} ") } else { format!(" {plugin} · {name} ") }, room);
    let (edge, tc) = if top { (th.focus, th.focus) } else { (th.border, th.dim) };
    c.border(r, BorderStyle::Rounded, edge, Some(th.bg), Some((&title, tc)));
    let bt = fit(&format!(" {} ", bottom.join(" · ")), room);
    if r.h >= 2 && !bt.is_empty() {
        c.text(r.x + r.w - 2 - width(&bt) as i32, r.y + r.h - 1, &bt, tc, Some(th.bg), Modifier::empty(), room);
    }
    // inside the border, a cell of padding each side
    let inner = Rect { x: r.x + 2, y: r.y + 1, w: (r.w - 4).max(0), h: (r.h - 2).max(0) };
    let id = id_of(s);
    let mut d = Draw { view: &id, focus: v.focus.as_deref(), active, cursor: None };
    let root = &v.state["root"];
    build::draw(ctx, c, root, &build::root_key(root), cells_of(inner), &mut v.elems, &mut d);
    d.cursor
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
    let focused = v.focus.clone().and_then(|f| node_at(&v.state["root"], &f).cloned().map(|n| (f, n)));
    if let Some((f, n)) = focused {
        let st = v.elems.entry(f.clone()).or_default();
        if typing {
            // a field takes what's typed; only a view key with Ctrl or Alt gets past it
            if bound.is_none() || !(name.starts_with("C-") || name.starts_with("M-")) {
                let (fired, edited) = fields::key(&n, st, k, &name);
                fire(app, i, fired);
                if edited {
                    changed(app, i, &f);
                }
                return;
            }
        } else if let Some(fired) = build::element_key(&n, st, &name) {
            return fire(app, i, fired); // what has the keyboard uses the key
        }
    }
    if let Some(b) = bound {
        return fire(app, i, bound_fire(&b).into_iter().collect());
    }
    if name == "?" {
        return help(app, i);
    }
    // nothing with the keyboard scrolls: the scroll keys reach the first thing that does
    if !matches!(kind, Some(Focus::Scroll | Focus::Pick)) {
        if let Some((t, n)) = fs.iter().find(|(_, f)| *f == Focus::Scroll).and_then(|(t, _)| node_at(&v.state["root"], t).cloned().map(|n| (t.clone(), n))) {
            build::scroll_key(&n, v.elems.entry(t).or_default(), &name);
        }
    }
}

// one of the view's keys, as it runs
fn bound_fire(b: &Value) -> Option<Fired> {
    b["action"].as_str().map(|a| Fired { action: a.to_string(), params: b.get("params").cloned().unwrap_or(json!({})), ui: Map::new() })
}

// `?`: the view's keys in a list; choosing one runs it.
fn help(app: &mut App, i: usize) {
    let v = &app.views[i];
    let keys: Vec<Value> = v.state["keys"].as_array().cloned().unwrap_or_default();
    if keys.is_empty() {
        return;
    }
    let items: Vec<super::modals::ListItem> = keys.iter().enumerate().map(|(k, b)| super::modals::ListItem::new(b["description"].as_str().or(b["action"].as_str()).unwrap_or(""), "", k.to_string()).key(b["key"].as_str().unwrap_or(""))).collect();
    let (title, view, shared) = (format!("{} · keys", v.state["plugin"].as_str().unwrap_or("")), id_of(&v.state), app.shared());
    tokio::task::spawn_local(async move {
        let chosen = super::modals::pick(&shared, &title, items, Some("tab moves · esc closes".into())).await;
        let Some(b) = chosen.and_then(|k| k.parse::<usize>().ok()).and_then(|k| keys.get(k)) else { return };
        let mut a = shared.borrow_mut();
        if let Some(i) = a.views.iter().position(|v| id_of(&v.state) == view) {
            fire(&mut a, i, bound_fire(b).into_iter().collect());
        }
    });
}

// Text pasted while a view has the keyboard goes into the field that has it.
pub fn paste(app: &mut App, text: &str) {
    let Some(i) = app.views.len().checked_sub(1) else { return };
    let v = &mut app.views[i];
    let Some(f) = v.focus.clone() else { return };
    let Some(n) = node_at(&v.state["root"], &f).filter(|n| focus_kind(n) == Some(Focus::Field)).cloned() else { return };
    fields::paste(&n, v.elems.entry(f.clone()).or_default(), text);
    changed(app, i, &f);
    app.dirty();
}

// A field's text changed: its `change` runs, at most every 150ms; the last edit's always arrives.
const CHANGE_EVERY: Duration = Duration::from_millis(150);

fn changed(app: &mut App, i: usize, key: &str) {
    let v = &mut app.views[i];
    let Some(n) = node_at(&v.state["root"], key).filter(|n| !n["change"].is_null()).cloned() else { return };
    let st = v.elems.entry(key.to_string()).or_default();
    if st.pending {
        return; // the one that's due says what's there then
    }
    let wait = st.sent.map_or(Duration::ZERO, |t| CHANGE_EVERY.saturating_sub(t.elapsed()));
    if wait.is_zero() {
        st.sent = Some(Instant::now());
        let f = fields::change(&n, st);
        return fire(app, i, f.into_iter().collect());
    }
    st.pending = true;
    let (me, view, key) = (app.me.clone(), id_of(&v.state), key.to_string());
    tokio::task::spawn_local(async move {
        tokio::time::sleep(wait).await;
        let Some(a) = me.upgrade() else { return };
        let mut a = a.borrow_mut();
        let Some(i) = a.views.iter().position(|v| id_of(&v.state) == view) else { return };
        let v = &mut a.views[i];
        let (Some(n), Some(st)) = (node_at(&v.state["root"], &key).cloned(), v.elems.get_mut(&key)) else { return };
        (st.pending, st.sent) = (false, Some(Instant::now()));
        let f = fields::change(&n, st);
        fire(&mut a, i, f.into_iter().collect());
    });
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

// An element was used: its plugin's actions, with what it holds.
fn fire(app: &mut App, i: usize, fired: Vec<Fired>) {
    let Some(v) = app.views.get(i) else { return };
    let view = Map::from_iter([("view".to_string(), v.state["id"].clone())]);
    send(app, v.state["plugin"].as_str().unwrap_or(""), &v.state["run"], view, fired);
}

// Elements' actions to their plugin (a view's, or a sidebar section's), each with what it holds as `ui` over `base`. Only
// a failure is shown; what an action does, its plugin shows.
pub fn send(app: &App, plugin: &str, run: &Value, base: Map<String, Value>, fired: Vec<Fired>) {
    let Some(conn) = app.conn.clone() else { return };
    for f in fired {
        let mut ui = base.clone();
        ui.extend(f.ui);
        let (conn, me, plugin, action) = (conn.clone(), app.me.clone(), plugin.to_string(), f.action);
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
}

// ---------- the pointer ----------

pub const DOUBLE: Duration = Duration::from_millis(400);

// A click on an element of a view: it takes the keyboard, and what's clicked in it is chosen or pressed.
pub fn click(app: &mut App, view: &str, key: &str, part: i32) {
    let Some(i) = app.views.iter().position(|v| id_of(&v.state) == view) else { return };
    app.dirty();
    let v = &mut app.views[i];
    let Some(n) = node_at(&v.state["root"], key).cloned() else { return };
    if focus_kind(&n).is_some() {
        v.focus = Some(key.to_string());
    }
    let now = Instant::now();
    let double = part >= 0 && v.clicked.as_ref().is_some_and(|(k, p, t)| k == key && *p == part && now.duration_since(*t) < DOUBLE);
    v.clicked = (!double).then(|| (key.to_string(), part, now));
    let fired = build::click(&n, v.elems.entry(key.to_string()).or_default(), part, double);
    fire(app, i, fired);
}

// The wheel over a view: the innermost element under the pointer that scrolls (or moves a choice).
pub fn wheel(app: &mut App, under: &[Hit], rows: i32) {
    for h in under {
        let Hit::View { view, key, .. } = h else { continue };
        let Some(i) = app.views.iter().position(|v| id_of(&v.state) == *view) else { continue };
        let v = &mut app.views[i];
        let Some(n) = node_at(&v.state["root"], key).cloned() else { continue };
        if let Some(fired) = build::wheel(&n, v.elems.entry(key.clone()).or_default(), rows) {
            app.dirty();
            return fire(app, i, fired);
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

    #[test]
    fn focus_goes_where_the_plugin_says_once_per_update() {
        let root = json!({ "type": "layout", "children": [{ "type": "button", "id": "ok" }, { "type": "input", "id": "name" }] });
        let mut v = OpenView::new(json!({ "root": root, "rev": 1 }));
        assert_eq!(v.focus.as_deref(), Some("#name")); // the field first
        v.update(json!({ "root": root, "rev": 2, "focus": "ok" }));
        assert_eq!(v.focus.as_deref(), Some("#ok"));
        v.focus = Some("#name".into());
        v.update(json!({ "root": root, "rev": 2, "focus": "ok" }));
        assert_eq!(v.focus.as_deref(), Some("#name")); // the same update doesn't take it back
    }
}
