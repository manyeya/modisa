// What plugins contribute to the TUI, drawn in the user's theme from the data the server sends with the view, and
// running their actions from a status segment, sidebar row, menu entry or the palette. Everything a plugin shows is
// attributed to it by name, so none of it can pass for modisa's own prompts.
//
// The server's PluginUiView is kept as the JSON it sent (View.plugins); this reads what it draws from it.
use std::sync::LazyLock;

use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Position;
use regex::Regex;
use serde_json::{json, Value};
use unicode_segmentation::UnicodeSegmentation;

use super::design::{agent_mark, fit};
use super::draw::{Canvas, Hit};
use super::modals::{self, key_name, Kind, Modal};
use super::{App, Shared};
use crate::config::keys::{bind_plugin_keys, BoundKey, DeclaredKey, KeyState};
use crate::config::BorderStyle;
use crate::core::layout::Rect;
use crate::core::text::{grapheme_width, width};
use crate::protocol::conn::b64;
use crate::protocol::links::{link_matches, LinkEntry};
use crate::vt::Screen;

pub fn plugin_ui(app: &App) -> &[Value] {
    app.view.as_ref().and_then(|v| v.plugins.as_deref()).unwrap_or(&[])
}

// A plugin's tone (dim, accent, warn, working, blocked, done, idle) as the theme's colour; anything else is the text's.
pub fn tone_color(app: &App, tone: &str) -> &'static str {
    match tone {
        "dim" | "accent" | "warn" | "working" | "blocked" | "done" | "idle" => crate::protocol::ui::token(&app.th, tone).unwrap_or(app.th.fg),
        _ => app.th.fg,
    }
}

fn str_of<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}
pub fn list_of<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v[k].as_array().map(Vec::as_slice).unwrap_or(&[])
}

// A sidebar row's spans as (text, colour, bold), cut to `width` cells with … where it runs out. An icon is the agent's
// glyph in its brand colour, or in the theme's text colour when the brand has none or it would be faint on this theme.
pub fn span_text(app: &App, spans: &[Value], base: &str, w: usize) -> Vec<(String, String, bool)> {
    let mut out = vec![];
    let mut left = w as i64;
    for x in spans {
        if left <= 0 {
            break;
        }
        if let Some(icon) = x["icon"].as_str() {
            // a logo is drawn two cells wide: the cell after it is its own, so what follows doesn't run into it
            let mark = agent_mark(&app.th, icon, app.logos, app.cell_ems());
            out.push((format!("{}{}", mark.glyph, " ".repeat(mark.cells.saturating_sub(2))), mark.color, false));
            left -= mark.cells as i64 - 1;
            continue;
        }
        let text = fit(str_of(x, "text"), left as usize);
        left -= width(&text) as i64;
        out.push((text, tone_color(app, x["tone"].as_str().unwrap_or(base)).to_string(), x["bold"].as_bool() == Some(true)));
    }
    out
}

pub fn title_of(app: &App, plugin: &str, action: &str) -> String {
    plugin_ui(app).iter().find(|p| p["plugin"] == plugin).and_then(|p| list_of(p, "actions").iter().find(|a| a["id"] == action)).and_then(|a| a["title"].as_str()).unwrap_or(action).to_string()
}

fn short(value: &Value) -> String {
    let text = value.as_str().map(String::from).unwrap_or_else(|| value.to_string());
    if text.chars().count() > 80 { format!("{}…", text.chars().take(79).collect::<String>()) } else { text }
}

// Run a plugin's action and say how it went: its result, its error, or that a timeout left the outcome unknown. `run` is
// the run whose UI it was taken from (captured when that was drawn): the server refuses it if that run ended. `ui` says
// where in modisa's chrome it was clicked (slots.rs).
#[allow(clippy::too_many_arguments)]
pub fn run_plugin_action(app: &App, plugin: &str, run: &str, action: &str, params: Value, target: Option<(String, String)>, link: Option<String>, ui: Option<Value>) {
    let Some(conn) = app.conn.clone() else { return };
    let label = format!("{plugin}: {}", title_of(app, plugin, action));
    let mut p = json!({ "plugin": plugin, "action": action, "params": params, "run": run });
    if let Some((pane, instance)) = target {
        p["target"] = json!({ "pane": pane, "instance": instance });
    }
    if let Some(l) = link {
        p["link"] = json!(l);
    }
    if let Some(u) = ui {
        p["ui"] = u;
    }
    let me = app.me.clone();
    tokio::task::spawn_local(async move {
        let r = conn.request("plugin.invoke", p, None).await;
        let Some(a) = me.upgrade() else { return };
        let mut a = a.borrow_mut();
        let (done, warn, blocked) = (a.th.done, a.th.warn, a.th.blocked);
        match r {
            Ok(Value::Null) => a.toast(&format!("{label} ✓"), done),
            Ok(v) => a.toast(&format!("{label} → {}", short(&v)), done),
            Err(e) if e.code == "timeout" => a.toast_from(&format!("{label}: no answer in time, so its outcome is unknown"), warn, Some(8000), None),
            Err(e) => a.toast_from(&format!("{label}: {}", e.message), blocked, Some(8000), None),
        }
    });
}

// The focused pane's process, as an action's target: the one focused now, not when the action finishes.
pub fn focused_target(app: &App) -> Option<(String, String)> {
    if !app.ready() {
        return None;
    }
    let pane = app.tab().focused.clone();
    let instance = app.info(&pane).map(|p| p.instance.clone()).filter(|i| !i.is_empty())?;
    Some((pane, instance))
}

// Plugins' keys as this client's own config binds them: the session's server sends what plugin.json declares, and
// [plugin_keys] here, not on the server, decides which key runs what, so each attached client can differ.
pub fn plugin_keys(app: &App) -> Vec<BoundKey> {
    let declared: Vec<DeclaredKey> = plugin_ui(app)
        .iter()
        .flat_map(|p| {
            list_of(p, "keys").iter().map(|k| DeclaredKey {
                plugin: str_of(p, "plugin").to_string(),
                key: str_of(k, "key").to_string(),
                action: k["action"].as_str().map(String::from),
                pane: k["pane"].as_str().map(String::from),
                description: str_of(k, "description").to_string(),
            })
        })
        .collect();
    bind_plugin_keys(&declared, &app.cfg.plugin_keys, &app.bindings)
}

// Prefix + a plugin's key: its action, or its pane, for the pane focused now (not when the action finishes).
pub fn plugin_key(app: &mut App, key: &str) {
    if app.view.is_none() {
        return;
    }
    let Some(bound) = plugin_keys(app).into_iter().find(|k| k.declared.key == key && k.state == KeyState::Active) else { return };
    let Some(run) = plugin_ui(app).iter().find(|p| p["plugin"] == bound.declared.plugin.as_str()).map(|p| str_of(p, "run").to_string()) else { return };
    let target = focused_target(app);
    let plugin = bound.declared.plugin.clone();
    if let Some(action) = &bound.declared.action {
        return run_plugin_action(app, &plugin, &run, action, json!({}), target, None, None);
    }
    if let Some(pane) = &bound.declared.pane {
        open_plugin_pane(app, &plugin, &run, pane, json!({}), target);
    }
}

// Ctrl+click on a URL: the plugin actions whose link pattern matches it, by plugin name then manifest order. One runs;
// several ask which; none says so. The URL goes as the invocation's link, never as params.
pub fn plugin_link(app: &mut App, pane: &str, url: &str, x: i32, y: i32) {
    if app.modal.is_some() {
        return;
    }
    let mut plugins: Vec<&Value> = plugin_ui(app).iter().collect();
    plugins.sort_by(|a, b| str_of(a, "plugin").cmp(str_of(b, "plugin")));
    let mut handlers: Vec<(String, String, String)> = vec![]; // plugin, run, action
    for p in plugins {
        let mut actions: Vec<String> = vec![]; // an action once, however many of its links match
        for l in list_of(p, "links") {
            let entry: Option<LinkEntry> = serde_json::from_value(l.clone()).ok();
            if entry.is_some_and(|e| link_matches(&e, url)) && !actions.contains(&str_of(l, "action").to_string()) {
                actions.push(str_of(l, "action").to_string());
            }
        }
        handlers.extend(actions.into_iter().map(|a| (str_of(p, "plugin").to_string(), str_of(p, "run").to_string(), a)));
    }
    let target = app.info(pane).map(|p| (pane.to_string(), p.instance.clone())).filter(|(_, i)| !i.is_empty());
    if handlers.is_empty() {
        let d = app.th.dim;
        return app.toast(&format!("No plugin handles {}", short(&json!(url))), d);
    }
    if handlers.len() == 1 {
        let (p, r, a) = &handlers[0];
        return run_plugin_action(app, p, r, a, json!({}), target, Some(url.to_string()), None);
    }
    // the title says what's being chosen, with the URL cut to fit the menu (fit also blanks control characters)
    let title = format!("Open {} with", fit(url, 18));
    let names: Vec<(String, String)> = handlers.iter().enumerate().map(|(i, (p, _, a))| (format!("{p}: {}", title_of(app, p, a)), i.to_string())).collect();
    let (shared, url) = (app.shared(), url.to_string());
    tokio::task::spawn_local(async move {
        let options = names.iter().map(|(n, v)| (n.as_str(), "", v.as_str(), false)).collect();
        let chosen = modals::menu(&shared, &title, options, x, y).await;
        if let Some((p, r, a)) = chosen.and_then(|i| i.parse::<usize>().ok()).and_then(|i| handlers.get(i)) {
            run_plugin_action(&shared.borrow(), p, r, a, json!({}), target, Some(url), None);
        }
    });
}

// ---------- panes and popups ----------

// The popup's box: its manifest size, at least POPUP_MIN, but never past the terminal (a cell of margin each side), even
// when the terminal shrinks below the minimum while the popup is open. Below the minimum, a popup doesn't open.
pub const POPUP_MIN: (i32, i32) = (20, 5);

pub struct Popup {
    pub pane: String,
    pub title: String,
    pub width: Value,
    pub height: Value,
    pub armed: bool, // the prefix was pressed: x closes
}

pub fn popup(app: &App) -> Option<&Popup> {
    match &app.modal {
        Some(Modal { kind: Kind::Popup(p), .. }) => Some(p),
        _ => None,
    }
}

pub fn popup_rect(app: &App, p: &Popup) -> Rect {
    let (w, h) = (app.width, app.height);
    let cells = super::views::cells;
    let pw = (w - 2).min(cells(&p.width, w, (w as f64 * 0.7).floor() as i32).max(POPUP_MIN.0)).max(1);
    let ph = (h - 2).min(cells(&p.height, h, (h as f64 * 0.6).floor() as i32).max(POPUP_MIN.1)).max(1);
    Rect { x: (w - pw) / 2, y: (h - ph) / 3, w: pw, h: ph }
}

// Open one of a plugin's panes. A popup is shown only here, by the client that asked, over everything else; if a dialog
// is already open it waits for nothing and says so (ui_busy).
pub fn open_plugin_pane(app: &mut App, plugin: &str, run: &str, pane: &str, params: Value, origin: Option<(String, String)>) {
    let warn = app.th.warn;
    if app.modal.is_some() || !app.views.is_empty() {
        return app.toast(&format!("{plugin}: can't open {pane} while a dialog is open"), warn);
    }
    let placement = plugin_ui(app).iter().find(|p| p["plugin"] == plugin).and_then(|p| list_of(p, "panes").iter().find(|x| x["id"] == pane)).map(|x| str_of(x, "placement").to_string());
    if placement.as_deref() == Some("popup") && (app.width < POPUP_MIN.0 + 2 || app.height < POPUP_MIN.1 + 2) {
        return app.toast(&format!("{plugin}: the terminal is too small for {pane}"), warn);
    }
    let Some(conn) = app.conn.clone() else { return };
    let mut p = json!({ "plugin": plugin, "pane": pane, "params": params, "run": run });
    if let Some((pane, instance)) = origin {
        p["from"] = json!({ "pane": pane, "instance": instance });
    }
    let (me, plugin) = (app.me.clone(), plugin.to_string());
    tokio::task::spawn_local(async move {
        let r = conn.request("plugin.pane.open", p, None).await;
        let Some(a) = me.upgrade() else { return };
        let mut a = a.borrow_mut();
        match r {
            Ok(opened) if opened["placement"] == "popup" => show_popup(&mut a, &opened),
            Ok(_) => {}
            Err(e) if e.code == "ui_busy" => {
                let w = a.th.warn;
                a.toast(&format!("{plugin}: a popup is already open"), w);
            }
            Err(e) => {
                let b = a.th.blocked;
                a.toast(&format!("{plugin}: {}", e.message), b);
            }
        }
    });
}

// The popup is a modal: the popup's terminal over everything, which takes no click outside it. Its program gets every
// key, Escape included; prefix x closes it.
fn show_popup(app: &mut App, opened: &Value) {
    let p = Popup { pane: str_of(opened, "pane").to_string(), title: str_of(opened, "title").to_string(), width: opened["width"].clone(), height: opened["height"].clone(), armed: false };
    modals::show(app, Kind::Popup(p));
    fit_popup(app);
}

// its program sizes to the popup, again whenever the terminal does
pub fn fit_popup(app: &App) {
    let Some(p) = popup(app) else { return };
    let Some(conn) = app.conn.clone() else { return };
    let r = popup_rect(app, p);
    let params = json!({ "pane": p.pane, "cols": (r.w - 2).max(10), "rows": (r.h - 2).max(3) });
    tokio::task::spawn_local(async move {
        let _ = conn.request("plugin.popup.resize", params, None).await;
    });
}

// The popup is closed (prefix x, its process ending, the connection going): the server takes its pane away.
pub fn popup_closed(app: &App, p: &Popup) {
    let Some(conn) = app.conn.clone().filter(|c| !c.closed()) else { return };
    let params = json!({ "pane": p.pane });
    tokio::task::spawn_local(async move {
        let _ = conn.request("plugin.popup.close", params, None).await; // already gone is fine
    });
}

pub fn popup_key(app: &mut App, k: &KeyEvent) {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let is_prefix = app.prefix.ctrl && ctrl && matches!(k.code, KeyCode::Char(c) if c.to_lowercase().to_string() == app.prefix.name);
    let Some(Modal { kind: Kind::Popup(p), .. }) = app.modal.as_mut() else { return };
    if is_prefix && !p.armed {
        p.armed = true;
        return;
    }
    if p.armed && key_name(k) == "x" {
        p.armed = false;
        return modals::close(app, None);
    }
    p.armed = false;
    // everything else is the popup program's
    let pane = p.pane.clone();
    let mode = app.panes.get(&pane).map(|x| x.screen.mode()).unwrap_or_default();
    to_pane(app, &pane, &super::keys::encode(k, mode));
}

pub fn to_pane(app: &mut App, pane: &str, bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    if let Some(p) = app.panes.get_mut(pane) {
        p.screen.term.scroll_display(Scroll::Bottom);
    }
    app.notify_server("input", json!({ "pane": pane, "data": b64(bytes) }));
}

// The popup's terminal in its box, titled with its pane's title; returns where its cursor is.
pub fn draw_popup(app: &App, c: &mut Canvas) -> Option<Position> {
    let p = popup(app)?;
    let th = &app.th;
    let r = popup_rect(app, p);
    c.hit(Rect { x: 0, y: 0, w: c.w, h: c.h }, Hit::Inert);
    // an opaque background: every cell it covers is the popup's
    c.fill(r, th.bg);
    let title = app.info(&p.pane).map(|i| i.title.clone()).unwrap_or(p.title.clone());
    let title = fit(&format!(" {title} · prefix x closes "), (r.w - 4).max(0) as usize);
    c.border(r, BorderStyle::Single, th.focus, Some(th.bg), Some((&title, th.focus)));
    let inner = Rect { x: r.x + 1, y: r.y + 1, w: (r.w - 2).max(0), h: (r.h - 2).max(0) };
    c.hit(inner, Hit::Pane(p.pane.clone()));
    let pane = app.panes.get(&p.pane)?;
    let area = ratatui::layout::Rect::new(inner.x.max(0) as u16, inner.y.max(0) as u16, inner.w.max(0) as u16, inner.h.max(0) as u16).intersection(c.buf.area);
    super::pane::draw(&pane.screen, area, c.buf, super::design::color(th.fg), super::design::color(th.bg))
}

// ---------- links ----------

static URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"https?://[^\s<>"'`\p{Cc}\x{200E}\x{200F}\x{202A}-\x{202E}\x{2066}-\x{2069}]+"#).unwrap());

// The http(s) URL drawn over a terminal cell of a screen line, if any. Cells aren't string offsets: a wide character
// takes two. Trailing punctuation isn't part of the URL; a closing bracket is, when it closes one opened in the URL.
// Control and bidirectional-override characters end a URL: they could disguise what the action receives.
pub fn url_at(line: &str, cell: usize) -> Option<String> {
    let mut at = None;
    let mut used = 0;
    for (i, g) in line.grapheme_indices(true) {
        let w = grapheme_width(g);
        if cell < used + w {
            at = Some(i);
            break;
        }
        used += w;
    }
    let at = at?;
    for m in URL.find_iter(line) {
        let mut url = m.as_str();
        while let Some(end) = url.chars().last().filter(|c| ".,;:!?)]}".contains(*c)) {
            let opener = match end {
                ')' => Some('('),
                ']' => Some('['),
                '}' => Some('{'),
                _ => None,
            };
            if opener.is_some_and(|o| url.matches(o).count() >= url.matches(end).count()) {
                break;
            }
            url = &url[..url.len() - end.len_utf8()];
        }
        let url: String = url.chars().take(2048).collect();
        if at >= m.start() && at < m.start() + url.len() {
            return Some(url);
        }
    }
    None
}

// A pane's row as it shows now, as text (scrolled back or not).
pub fn line_text(screen: &Screen, row: i32) -> String {
    let grid = screen.term.grid();
    if row < 0 || row >= screen.rows() as i32 {
        return String::new();
    }
    let r = &grid[Line(row - grid.display_offset() as i32)];
    let mut out = String::new();
    for x in 0..grid.columns() {
        let cell = &r[Column(x)];
        if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
            continue;
        }
        out.push(cell.c);
        if let Some(z) = cell.zerowidth() {
            out.extend(z);
        }
    }
    out
}

// A click on a plugin's sidebar row or status segment: focus the process the row was set for (the server checks the
// instance), never another pane given its id; or run its action.
pub fn clicked(shared: &Shared, plugin: &str, run: &str, action: Option<&str>, pane: Option<&str>, instance: Option<&str>) {
    let app = shared.borrow();
    if let Some(p) = pane {
        let Some(conn) = app.conn.clone() else { return };
        let (me, plugin) = (app.me.clone(), plugin.to_string());
        let target = format!("{p}:{}", instance.unwrap_or(""));
        tokio::task::spawn_local(async move {
            if conn.request("pane.focus", json!({ "target": target }), None).await.is_err() {
                if let Some(a) = me.upgrade() {
                    let mut a = a.borrow_mut();
                    let w = a.th.warn;
                    a.toast(&format!("{plugin}: that pane is gone"), w);
                }
            }
        });
    } else if let Some(a) = action {
        run_plugin_action(&app, plugin, run, a, json!({}), None, None, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_url_under_a_column() {
        let line = "see https://example.com/a?b=1, and (http://x.io/y).";
        assert_eq!(url_at(line, 4).as_deref(), Some("https://example.com/a?b=1"));
        assert_eq!(url_at(line, 28).as_deref(), Some("https://example.com/a?b=1"));
        assert_eq!(url_at(line, 29), None); // the comma
        assert_eq!(url_at(line, 40).as_deref(), Some("http://x.io/y"));
        assert_eq!(url_at(line, 0), None);
        assert_eq!(url_at(&format!("https://e.com/{}", "a".repeat(5000)), 3).map(|u| u.len()), Some(2048));
    }

    #[test]
    fn a_closing_bracket_stays_when_the_url_opened_it() {
        assert_eq!(url_at("see https://en.wikipedia.org/wiki/Foo_(bar) now", 10).as_deref(), Some("https://en.wikipedia.org/wiki/Foo_(bar)"));
        assert_eq!(url_at("(see https://x.dev/a)", 8).as_deref(), Some("https://x.dev/a"));
        assert_eq!(url_at("[https://x.dev/a[1]].", 3).as_deref(), Some("https://x.dev/a[1]"));
        assert_eq!(url_at("https://x.dev/(a)).", 3).as_deref(), Some("https://x.dev/(a)"));
        assert_eq!(url_at("https://x.dev/a_(b)_c", 3).as_deref(), Some("https://x.dev/a_(b)_c"));
        assert_eq!(url_at("https://x.dev/a).", 3).as_deref(), Some("https://x.dev/a"));
        assert_eq!(url_at("https://x.dev/a],", 3).as_deref(), Some("https://x.dev/a"));
        assert_eq!(url_at("https://x.dev/a\u{202e}gpj.exe", 3).as_deref(), Some("https://x.dev/a")); // a bidi override ends it
    }

    #[test]
    fn the_column_is_a_terminal_cell() {
        let cjk = "日本語 https://example.com/x";
        assert_eq!(url_at(cjk, 7).as_deref(), Some("https://example.com/x"));
        assert_eq!(url_at(cjk, 27).as_deref(), Some("https://example.com/x"));
        assert_eq!(url_at(cjk, 28), None);
        assert_eq!(url_at(cjk, 5), None);
        let emoji = "\u{1F642} https://e.dev/a";
        assert_eq!(url_at(emoji, 3).as_deref(), Some("https://e.dev/a"));
        assert_eq!(url_at(emoji, 2), None);
        let cafe = "cafe\u{301} https://e.dev/cafe\u{301}";
        assert_eq!(url_at(cafe, 5).as_deref(), Some("https://e.dev/cafe\u{301}"));
        assert_eq!(url_at(cafe, 22).as_deref(), Some("https://e.dev/cafe\u{301}"));
        assert_eq!(url_at(cafe, 23), None);
        assert_eq!(url_at(cafe, 4), None);
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        let zwj = format!("{family} https://e.dev/{family}/p");
        let want = format!("https://e.dev/{family}/p");
        assert_eq!(url_at(&zwj, 3), Some(want.clone()));
        assert_eq!(url_at(&zwj, 18), Some(want.clone()));
        assert_eq!(url_at(&zwj, 20), Some(want));
        assert_eq!(url_at(&zwj, 21), None);
        assert_eq!(url_at(&zwj, 2), None);
    }
}
