// The dialogs: a centred panel over a dimmed screen that routes keys to itself and resolves with its result (None when
// dismissed). A list to choose from (pick, and menus placed where they were asked for), a question with buttons (ask,
// confirm, permission) and a line of text (prompt). They're all drawn from the same parts — a header, a search field,
// rows, buttons and a footer of key hints — so they look and behave alike.
use std::rc::Rc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Position;
use ratatui::style::{Color, Modifier};
use serde_json::json;
use tokio::sync::oneshot;

use super::design::{fit, floating, mix};
use super::draw::{Canvas, Hit};
use super::{App, Shared};
use crate::config::themes::Theme;
use crate::config::BorderStyle;
use crate::core::layout::Rect;
use crate::core::text::width;

pub struct ListButton {
    pub icon: String,
    pub value: String,
    pub key: String,
    pub title: String,
    pub danger: bool,
}

#[derive(Default)]
pub struct ListItem {
    pub name: String,
    pub description: Option<String>,
    pub value: String,
    pub key: Option<String>,
    pub danger: bool,
    pub buttons: Vec<ListButton>,
    pub context: Option<Rc<dyn Fn(i32, i32)>>, // its right-click menu
}

impl ListItem {
    pub fn new(name: impl Into<String>, description: impl Into<String>, value: impl Into<String>) -> ListItem {
        let d: String = description.into();
        ListItem { name: name.into(), description: (!d.is_empty()).then_some(d), value: value.into(), ..Default::default() }
    }
    pub fn key(mut self, k: impl Into<String>) -> ListItem {
        let k: String = k.into();
        self.key = (!k.is_empty()).then_some(k);
        self
    }
}

#[derive(Default)]
pub struct ListOptions {
    pub title: String,
    pub meta: Option<String>, // on the right of the header
    pub items: Vec<ListItem>,
    pub placeholder: Option<String>,
    pub width: Option<i32>,
    pub rows: Option<usize>, // at most this many at a time
    pub selected: Option<usize>, // the item selected at first
    pub at: Option<(i32, i32)>, // a menu: placed here, and each item's key chooses it while nothing's typed
}

pub struct List {
    o: ListOptions,
    query: String,
    shown: Vec<usize>,
    pub sel: usize,
    first: usize,
}

pub struct Choice {
    pub label: String,
    pub key: String,
    pub value: String,
    pub tone: Option<&'static str>, // "danger" | "primary"
}

pub struct Ask {
    title: String,
    lines: Vec<String>,
    choices: Vec<Choice>,
    sel: usize,
    width: i32,
}

pub struct Prompt {
    title: String,
    value: Vec<char>,
    cursor: usize,
    placeholder: String,
}

pub enum Kind {
    List(List),
    Ask(Ask),
    Prompt(Prompt),
    Settings(Box<super::settings::Settings>),
    Popup(super::plugin_ui::Popup), // a plugin's popup pane: its program gets the keys
    Busy(Busy),                     // something under way, behind a spinner
}

// Work under way: its title, what it's doing, and a spinner (esc hides it; the work carries on).
pub struct Busy {
    pub id: u64,
    pub title: String,
    pub what: String,
}

pub struct Modal {
    pub kind: Kind,
    done: Option<oneshot::Sender<Option<String>>>,
}

// ---------- opening and closing ----------

async fn open(shared: &Shared, kind: Kind) -> Option<String> {
    let (tx, rx) = oneshot::channel();
    {
        let mut app = shared.borrow_mut();
        close(&mut app, None);
        app.modal = Some(Modal { kind, done: Some(tx) });
        app.dirty();
    }
    rx.await.ok().flatten()
}

// A dialog that isn't waited on (a plugin's popup), over anything open.
pub fn show(app: &mut App, kind: Kind) {
    close(app, None);
    app.modal = Some(Modal { kind, done: None });
    app.dirty();
}

// A spinner dialog for work under way; it resolves when the user hides it (esc), or when close_busy takes it away.
pub fn open_busy(shared: &Shared, title: &str, what: &str) -> (u64, oneshot::Receiver<Option<String>>) {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let id = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let (tx, rx) = oneshot::channel();
    let mut app = shared.borrow_mut();
    close(&mut app, None);
    app.modal = Some(Modal { kind: Kind::Busy(Busy { id, title: title.into(), what: what.into() }), done: Some(tx) });
    super::views::animate(&mut app);
    app.dirty();
    (id, rx)
}

pub fn close_busy(app: &mut App, id: u64) {
    if matches!(&app.modal, Some(Modal { kind: Kind::Busy(b), .. }) if b.id == id) {
        close(app, None);
    }
}

pub fn busy(app: &App) -> bool {
    matches!(&app.modal, Some(Modal { kind: Kind::Busy(_), .. }))
}

// The settings page, at a section (prefix s, or the theme button).
pub async fn settings(shared: &Shared, start: &str) {
    let (tx, rx) = oneshot::channel();
    {
        let mut app = shared.borrow_mut();
        close(&mut app, None);
        let mut s = super::settings::Settings::new(&app, start);
        let at = s.current as i64;
        super::settings::show(&mut app, &mut s, at);
        app.modal = Some(Modal { kind: Kind::Settings(Box::new(s)), done: Some(tx) });
        app.dirty();
    }
    let _ = rx.await;
}

// The settings page's own handling, with the page taken out of the App for it; it closes if a row asked to.
fn with_page(app: &mut App, f: impl FnOnce(&mut App, &mut super::settings::Settings)) -> bool {
    let Some(mut m) = app.modal.take_if(|m| matches!(m.kind, Kind::Settings(_))) else { return false };
    let Kind::Settings(s) = &mut m.kind else { unreachable!() };
    f(app, s);
    let closing = s.close;
    app.modal = Some(m);
    if closing {
        close(app, None);
    }
    app.dirty();
    true
}

// Resolves the open dialog with `v` (None: dismissed) and takes it away.
pub fn close(app: &mut App, v: Option<String>) {
    if let Some(mut m) = app.modal.take() {
        if let Kind::Settings(s) = &m.kind {
            super::settings::leave(app, s); // drop an unapplied theme preview
        }
        if let Kind::Popup(p) = &m.kind {
            super::plugin_ui::popup_closed(app, p);
        }
        if let Some(tx) = m.done.take() {
            let _ = tx.send(v);
        }
        app.dirty();
    }
}

pub async fn list(shared: &Shared, o: ListOptions) -> Option<String> {
    let n = o.items.len();
    let sel = o.selected.unwrap_or(0).min(n.saturating_sub(1));
    open(shared, Kind::List(List { shown: (0..n).collect(), query: String::new(), sel, first: 0, o })).await
}

pub async fn pick(shared: &Shared, title: &str, items: Vec<ListItem>, meta: Option<String>) -> Option<String> {
    list(shared, ListOptions { title: title.into(), meta, items, ..Default::default() }).await
}

// A small menu at (x, y): the searchable list, placed there, whose items also run on their own keys.
pub async fn menu(shared: &Shared, title: &str, options: Vec<(&str, &str, &str, bool)>, x: i32, y: i32) -> Option<String> {
    let items = options.into_iter().map(|(name, key, action, danger)| ListItem { danger, ..ListItem::new(name, "", action).key(key) }).collect();
    list(shared, ListOptions { title: title.into(), at: Some((x, y)), width: Some(44), rows: Some(14), items, ..Default::default() }).await
}

// A question with buttons: yes/no before something destructive, or the answers a permission request can have. Each
// button answers to its key, a click, or ←→ / tab and ↵.
pub async fn ask(shared: &Shared, title: &str, body: &str, choices: Vec<Choice>, start: usize) -> Option<String> {
    ask_width(shared, title, body, choices, start, 60).await
}

pub async fn ask_width(shared: &Shared, title: &str, body: &str, choices: Vec<Choice>, start: usize, width: i32) -> Option<String> {
    open(shared, Kind::Ask(Ask { title: title.into(), lines: body.split('\n').map(String::from).collect(), choices, sel: start, width })).await
}

// Cancel is where the focus starts: ↵ alone never deletes anything.
pub async fn confirm(shared: &Shared, title: &str, text: &str, yes: &str) -> bool {
    let danger = ["delete", "close", "kill", "remove"].iter().any(|w| yes.to_lowercase().contains(w));
    let label = yes[..1].to_uppercase() + &yes[1..];
    let choices = vec![
        Choice { label: "Cancel".into(), key: "n".into(), value: "no".into(), tone: None },
        Choice { label, key: "y".into(), value: "yes".into(), tone: Some(if danger { "danger" } else { "primary" }) },
    ];
    ask(shared, title, text, choices, 0).await.as_deref() == Some("yes")
}

// Ask for a line of text: ↵ or the OK button takes it, esc or Cancel doesn't.
pub async fn prompt(shared: &Shared, title: &str, value: &str, placeholder: &str) -> Option<String> {
    let value: Vec<char> = value.chars().collect();
    open(shared, Kind::Prompt(Prompt { title: title.into(), cursor: value.len(), value, placeholder: placeholder.into() })).await
}

// The server asks whether an agent may act on a pane it didn't create: allow, always, or deny.
pub fn permission(app: &mut App, id: u64, text: String) {
    app.prompt_ids.insert(id);
    let shared = app.shared();
    tokio::task::spawn_local(async move {
        let choices = vec![
            Choice { label: "Deny".into(), key: "n".into(), value: "deny".into(), tone: Some("danger") },
            Choice { label: "Always".into(), key: "a".into(), value: "always".into(), tone: None },
            Choice { label: "Allow".into(), key: "y".into(), value: "allow".into(), tone: Some("primary") },
        ];
        let answer = ask(&shared, "Permission", &text, choices, 2).await.unwrap_or_else(|| "deny".into());
        shared.borrow().notify_server("promptReply", json!({ "id": id, "answer": answer }));
    });
}

// ---------- search ----------

// Search: every word of the query somewhere in the text, case aside. Ranks what starts with the query first, then what
// has it in its name, then what only has it in its description.
pub fn matches(items: &[ListItem], query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return (0..items.len()).collect();
    }
    let words: Vec<&str> = q.split_whitespace().collect();
    let mut ranked: Vec<(usize, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, t)| {
            let n = t.name.to_lowercase();
            let all = format!("{n} {}", t.description.as_deref().unwrap_or("").to_lowercase());
            if !words.iter().all(|w| all.contains(w)) {
                return None;
            }
            Some((if n.starts_with(&q) { 0 } else if words.iter().all(|w| n.contains(w)) { 1 } else { 2 }, i))
        })
        .collect();
    ranked.sort();
    ranked.into_iter().map(|(_, i)| i).collect()
}

// Typing into a search: printable keys add, backspace takes one back, ctrl+w a word, ctrl+u everything. The new query,
// or None when the key isn't one for the search.
pub fn typed(query: &str, k: &KeyEvent) -> Option<String> {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    match k.code {
        KeyCode::Backspace => {
            let mut q = query.to_string();
            q.pop();
            Some(q)
        }
        KeyCode::Char('u') if ctrl => Some(String::new()),
        KeyCode::Char('w') if ctrl => Some(query.trim_end().trim_end_matches(|c: char| !c.is_whitespace()).to_string()),
        KeyCode::Char(c) if !ctrl && !k.modifiers.contains(KeyModifiers::ALT) && c >= ' ' => Some(format!("{query}{c}")),
        _ => None,
    }
}

// A key as prefix bindings name it: one character as typed (H is shift+h), or a key with a name.
pub fn key_name(k: &KeyEvent) -> String {
    match k.code {
        KeyCode::Char(' ') => "space".into(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "return".into(),
        KeyCode::Esc => "escape".into(),
        KeyCode::Tab => "tab".into(),
        KeyCode::BackTab => "tab".into(),
        KeyCode::Backspace => "backspace".into(),
        KeyCode::Left => "left".into(),
        KeyCode::Right => "right".into(),
        KeyCode::Up => "up".into(),
        KeyCode::Down => "down".into(),
        KeyCode::Home => "home".into(),
        KeyCode::End => "end".into(),
        KeyCode::PageUp => "pageup".into(),
        KeyCode::PageDown => "pagedown".into(),
        KeyCode::Delete => "delete".into(),
        KeyCode::F(n) => format!("f{n}"),
        _ => String::new(),
    }
}

// ---------- keys and the pointer ----------

// A key while a dialog is open; it never reaches anything else.
pub fn key(app: &mut App, k: &KeyEvent) {
    if matches!(app.modal, Some(Modal { kind: Kind::Popup(_), .. })) {
        return super::plugin_ui::popup_key(app, k); // Escape too: it's the popup program's
    }
    if k.code == KeyCode::Esc {
        return close(app, None);
    }
    if with_page(app, |app, s| super::settings::key(app, s, k)) {
        return;
    }
    let Some(m) = app.modal.as_mut() else { return };
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let shift = k.modifiers.contains(KeyModifiers::SHIFT) || k.code == KeyCode::BackTab;
    let mut return_value: Option<String> = None; // ↵ on a question or a prompt: its answer
    let result: Option<Option<String>> = match &mut m.kind {
        Kind::List(l) => {
            let n = l.shown.len();
            let step = |l: &mut List, by: i64| {
                if n > 0 {
                    l.sel = ((l.sel as i64 + by).rem_euclid(n as i64)) as usize;
                }
            };
            match k.code {
                KeyCode::Up => step(l, -1),
                KeyCode::Char('p') if ctrl => step(l, -1),
                KeyCode::BackTab => step(l, -1),
                KeyCode::Tab if shift => step(l, -1),
                KeyCode::Down | KeyCode::Tab => step(l, 1),
                KeyCode::Char('n') if ctrl => step(l, 1),
                KeyCode::PageUp => l.sel = l.sel.saturating_sub(8),
                KeyCode::PageDown => l.sel = (l.sel + 8).min(n.saturating_sub(1)),
                KeyCode::Enter => {
                    if let Some(&i) = l.shown.get(l.sel) {
                        let v = l.o.items[i].value.clone();
                        return close(app, Some(v));
                    }
                }
                _ => {
                    let item = l.shown.get(l.sel).map(|&i| &l.o.items[i]);
                    if let Some(b) = item.filter(|_| ctrl).and_then(|it| it.buttons.iter().find(|b| KeyCode::Char(b.key.chars().next().unwrap_or('\0')) == k.code)) {
                        let v = b.value.clone();
                        return close(app, Some(v));
                    }
                    // a menu item's own key, while nothing's been typed
                    let name = key_name(k);
                    if l.o.at.is_some() && l.query.is_empty() && !ctrl {
                        if let Some(own) = l.o.items.iter().find(|i| i.key.as_ref().is_some_and(|key| key.chars().count() == 1 && *key == name)) {
                            let v = own.value.clone();
                            return close(app, Some(v));
                        }
                    }
                    if let Some(q) = typed(&l.query, k) {
                        l.query = q;
                        l.shown = matches(&l.o.items, &l.query);
                        l.sel = 0;
                        l.first = 0;
                    }
                }
            }
            None
        }
        Kind::Ask(a) => {
            let n = a.choices.len();
            if let Some(c) = a.choices.iter().find(|c| key_name(k) == c.key) {
                Some(Some(c.value.clone()))
            } else {
                match k.code {
                    KeyCode::Left | KeyCode::BackTab => a.sel = (a.sel + n - 1) % n,
                    KeyCode::Tab if shift => a.sel = (a.sel + n - 1) % n,
                    KeyCode::Right | KeyCode::Tab => a.sel = (a.sel + 1) % n,
                    KeyCode::Enter => return_value = Some(a.choices[a.sel].value.clone()),
                    _ => {}
                }
                None
            }
        }
        Kind::Prompt(p) => {
            match k.code {
                KeyCode::Enter => return_value = Some(p.value.iter().collect()),
                KeyCode::Left => p.cursor = p.cursor.saturating_sub(1),
                KeyCode::Right => p.cursor = (p.cursor + 1).min(p.value.len()),
                KeyCode::Home => p.cursor = 0,
                KeyCode::Char('a') if ctrl => p.cursor = 0,
                KeyCode::End => p.cursor = p.value.len(),
                KeyCode::Char('e') if ctrl => p.cursor = p.value.len(),
                KeyCode::Char('u') if ctrl => {
                    p.value.drain(..p.cursor);
                    p.cursor = 0;
                }
                KeyCode::Char('k') if ctrl => p.value.truncate(p.cursor),
                KeyCode::Char('w') if ctrl => {
                    let mut i = p.cursor;
                    while i > 0 && p.value[i - 1].is_whitespace() {
                        i -= 1;
                    }
                    while i > 0 && !p.value[i - 1].is_whitespace() {
                        i -= 1;
                    }
                    p.value.drain(i..p.cursor);
                    p.cursor = i;
                }
                KeyCode::Backspace if p.cursor > 0 => {
                    p.cursor -= 1;
                    p.value.remove(p.cursor);
                }
                KeyCode::Delete if p.cursor < p.value.len() => {
                    p.value.remove(p.cursor);
                }
                KeyCode::Char(c) if !ctrl => {
                    p.value.insert(p.cursor, c);
                    p.cursor += 1;
                }
                _ => {}
            }
            None
        }
        Kind::Settings(_) | Kind::Popup(_) | Kind::Busy(_) => None, // handled above, or takes no keys
    };
    if let Some(v) = result {
        return close(app, v);
    }
    if return_value.is_some() {
        return close(app, return_value);
    }
    app.dirty();
}

// Text pasted while a dialog is open goes into what's being typed.
pub fn paste(app: &mut App, text: &str) {
    let Some(m) = app.modal.as_mut() else { return };
    match &mut m.kind {
        Kind::Prompt(p) => {
            for c in text.chars().filter(|c| !c.is_control()) {
                p.value.insert(p.cursor, c);
                p.cursor += 1;
            }
        }
        Kind::List(l) => {
            l.query.extend(text.chars().filter(|c| !c.is_control()));
            l.shown = matches(&l.o.items, &l.query);
            l.sel = 0;
        }
        Kind::Popup(p) => {
            let (pane, text) = (p.pane.clone(), text.to_string());
            let bracketed = app.panes.get(&pane).is_some_and(|x| x.screen.mode().contains(alacritty_terminal::term::TermMode::BRACKETED_PASTE));
            let data = if bracketed { format!("\x1b[200~{text}\x1b[201~") } else { text };
            super::plugin_ui::to_pane(app, &pane, data.as_bytes());
        }
        Kind::Ask(_) | Kind::Settings(_) | Kind::Busy(_) => {}
    }
    app.dirty();
}

// What a click in a dialog chose: "row:<i>" a list's row, "value:<v>" a value, "context:<i>" a row's own menu.
pub fn click(app: &mut App, what: &str, right: bool, x: i32, y: i32) {
    if !right && with_page(app, |app, s| super::settings::click(app, s, what)) {
        return;
    }
    let Some(m) = app.modal.as_ref() else { return };
    if let Some(i) = what.strip_prefix("row:").and_then(|i| i.parse::<usize>().ok()) {
        let Kind::List(l) = &m.kind else { return };
        let item = &l.o.items[l.shown[i]];
        if right {
            if let Some(ctx) = item.context.clone() {
                close(app, None);
                ctx(x, y);
            }
            return;
        }
        let v = item.value.clone();
        return close(app, Some(v));
    }
    if right {
        return;
    }
    if let Some(v) = what.strip_prefix("value:") {
        return close(app, Some(v.to_string()));
    }
    if what == "cancel" {
        return close(app, None);
    }
    if what == "ok" {
        if let Kind::Prompt(p) = &m.kind {
            let v: String = p.value.iter().collect();
            return close(app, Some(v));
        }
    }
}

// The pointer resting on a list's row selects it, when [mouse] hover says so.
pub fn hover(app: &mut App, what: &str) {
    if with_page(app, |app, s| super::settings::hover(app, s, what)) {
        return;
    }
    if !app.cfg.mouse.hover {
        return;
    }
    let Some(i) = what.strip_prefix("row:").and_then(|i| i.parse::<usize>().ok()) else { return };
    if let Some(Modal { kind: Kind::List(l), .. }) = app.modal.as_mut() {
        if l.sel != i {
            l.sel = i;
            app.dirty();
        }
    }
}

pub fn scroll(app: &mut App, by: i64) {
    if with_page(app, |app, s| super::settings::scroll(app, s, by)) {
        return;
    }
    if let Some(Modal { kind: Kind::List(l), .. }) = app.modal.as_mut() {
        l.sel = (l.sel as i64 + by).clamp(0, l.shown.len().saturating_sub(1) as i64) as usize;
        app.dirty();
    }
}

// ---------- drawing ----------

// The screen behind, dimmed: what was drawn takes a black veil.
fn veil(c: &mut Canvas) {
    let shade = |col: Color| match col {
        Color::Rgb(r, g, b) => Color::Rgb((r as f64 * 0.45) as u8, (g as f64 * 0.45) as u8, (b as f64 * 0.45) as u8),
        other => other,
    };
    for cell in c.buf.content.iter_mut() {
        cell.fg = shade(cell.fg);
        cell.bg = shade(cell.bg);
        if !matches!(cell.fg, Color::Rgb(..)) {
            cell.modifier |= Modifier::DIM;
        }
    }
    c.hit(Rect { x: 0, y: 0, w: c.w, h: c.h }, Hit::Veil);
}

struct Frame {
    x: i32,
    y: i32, // the next row
    w: i32, // inside the border and padding
    h: i32,
    bottom: i32,
}

// A panel `w` wide and `h` tall (both clamped to the terminal), centred unless placed at (x, y).
fn frame(th: &Theme, c: &mut Canvas, h: i32, w: i32, at: Option<(i32, i32)>) -> Frame {
    let r = floating(c.w, c.h, w, h, at.map(|a| a.0), at.map(|a| a.1));
    c.fill(r, th.bar);
    c.border(r, BorderStyle::Rounded, &mix(th.border, th.focus, 0.45), Some(th.bar), None);
    c.hit(r, Hit::Inert);
    Frame { x: r.x + 2, y: r.y + 1, w: (r.w - 4).max(1), h: (r.h - 2).max(1), bottom: r.y + r.h - 1 }
}

// The title in bold, and on the right something about what's below (a count, the prefix key) in the dim colour.
fn header(th: &Theme, c: &mut Canvas, f: &mut Frame, title: &str, meta: &str) {
    let right = fit(meta, (f.w / 2).max(0) as usize);
    let t = fit(title, (f.w - width(&right) as i32 - 1).max(1) as usize);
    c.text(f.x, f.y, &t, th.fg, Some(th.bar), Modifier::BOLD, f.w as usize);
    if !right.is_empty() {
        c.text(f.x + f.w - width(&right) as i32, f.y, &right, th.dim, Some(th.bar), Modifier::empty(), right.len());
    }
    f.y += 1;
}

// The search field: what's typed so far and a caret, or the placeholder, on an inset strip.
fn search_field(th: &Theme, c: &mut Canvas, f: &mut Frame, query: &str, placeholder: &str) {
    c.fill(Rect { x: f.x, y: f.y, w: f.w, h: 1 }, th.bg);
    let mut x = f.x + c.text(f.x, f.y, " ⌕ ", if query.is_empty() { th.dim } else { th.accent }, Some(th.bg), Modifier::empty(), 3);
    let room = (f.w - 4).max(1) as usize;
    if !query.is_empty() {
        let shown = if width(query) > room - 1 { format!("…{}", query.chars().rev().take(room.saturating_sub(2)).collect::<Vec<_>>().into_iter().rev().collect::<String>()) } else { query.to_string() };
        x += c.text(x, f.y, &shown, th.fg, Some(th.bg), Modifier::empty(), room);
        c.text(x, f.y, "▏", th.focus, Some(th.bg), Modifier::empty(), 1);
    } else {
        x += c.text(x, f.y, "▏", th.focus, Some(th.bg), Modifier::empty(), 1);
        c.text(x, f.y, &fit(placeholder, room - 1), th.dim, Some(th.bg), Modifier::empty(), room);
    }
    f.y += 1;
}

// Key hints along the bottom: each key in the text colour, what it does dim.
fn footer(th: &Theme, c: &mut Canvas, f: &Frame, y: i32, hints: &[(String, String)], right: &str) {
    let mut used = width(right) as i32 + 1;
    let mut x = f.x;
    for (key, what) in hints {
        let w = (width(key) + width(what) + 3) as i32;
        if used + w > f.w {
            break;
        }
        used += w;
        c.text(x, y, key, th.fg, Some(th.bar), Modifier::empty(), w as usize);
        c.text(x + width(key) as i32, y, &format!(" {what}   "), th.dim, Some(th.bar), Modifier::empty(), w as usize);
        x += w;
    }
    if !right.is_empty() {
        c.text(f.x + f.w - width(right) as i32, y, right, th.dim, Some(th.bar), Modifier::empty(), width(right));
    }
}

// A button: filled in `tone` when it's the primary (or focused) one, else a quiet outline of the text colour. Returns
// its width.
fn button(th: &Theme, c: &mut Canvas, x: i32, y: i32, label: &str, key: &str, tone: &str, filled: bool, hit: Hit) -> i32 {
    let name = format!(" {label}  ");
    let w = (width(&name) + width(key) + 1) as i32;
    let r = Rect { x, y, w, h: 1 };
    let (mut fg, mut bg) = if filled { (th.bg.to_string(), tone.to_string()) } else { (tone.to_string(), mix(th.bar, th.fg, 0.08)) };
    if c.hovered(r) {
        (fg, bg) = if filled { (th.bg.to_string(), mix(tone, th.fg, 0.2)) } else { (th.fg.to_string(), mix(th.bar, th.fg, 0.16)) };
    }
    c.fill(r, &bg);
    let used = c.text(x, y, &name, &fg, Some(&bg), if filled { Modifier::BOLD } else { Modifier::empty() }, w as usize);
    c.text(x + used, y, &format!("{key} "), &if filled { mix(tone, th.bg, 0.5) } else { th.dim.to_string() }, Some(&bg), Modifier::empty(), (w - used) as usize);
    c.hit(r, hit);
    w
}

// A scrollbar for `total` rows, `shown` of them from `first`, `height` cells tall: the thumb's cells.
fn thumb(total: usize, shown: usize, first: usize, height: usize) -> Option<(usize, usize)> {
    if total <= shown {
        return None;
    }
    let size = ((shown as f64 / total as f64) * height as f64).round().max(1.0) as usize;
    let top = ((first as f64 / (total - shown).max(1) as f64) * (height - size) as f64).round() as usize;
    Some((top, size))
}

// `s` with the query's words picked out in `hit`: (text, colour, bold) chunks.
fn highlight<'a>(s: &str, query: &str, color: &'a str, hit: &'a str, bold: bool) -> Vec<(String, &'a str, bool)> {
    let chars: Vec<char> = s.chars().collect();
    let lower: Vec<char> = s.to_lowercase().chars().collect();
    let mut on = vec![false; chars.len()];
    if lower.len() == chars.len() {
        for w in query.trim().to_lowercase().split_whitespace() {
            let w: Vec<char> = w.chars().collect();
            if let Some(at) = (0..lower.len().saturating_sub(w.len() - 1)).find(|&i| lower[i..].starts_with(&w)) {
                on[at..at + w.len()].iter_mut().for_each(|x| *x = true);
            }
        }
    }
    let mut out = vec![];
    let mut i = 0;
    while i < chars.len() {
        let mut j = i;
        while j < chars.len() && on[j] == on[i] {
            j += 1;
        }
        out.push((chars[i..j].iter().collect(), if on[i] { hit } else { color }, bold || on[i]));
        i = j;
    }
    out
}

pub fn draw(app: &mut App, c: &mut Canvas) -> Option<Position> {
    if matches!(app.modal, Some(Modal { kind: Kind::Popup(_), .. })) {
        return super::plugin_ui::draw_popup(app, c); // no veil: the popup's frame is what stands out
    }
    veil(c);
    let modal = app.modal.as_mut()?;
    match &mut modal.kind {
        Kind::Popup(_) => None,
        Kind::Busy(_) => {
            draw_busy(app, c);
            None
        }
        Kind::List(_) => {
            draw_list(app, c);
            None
        }
        Kind::Ask(_) => {
            draw_ask(app, c);
            None
        }
        Kind::Prompt(_) => draw_prompt(app, c),
        Kind::Settings(_) => {
            let mut m = app.modal.take().unwrap();
            if let Kind::Settings(s) = &mut m.kind {
                super::settings::draw(app, s, c);
            }
            app.modal = Some(m);
            None
        }
    }
}

fn draw_list(app: &mut App, c: &mut Canvas) {
    let Some(Modal { kind: Kind::List(l), .. }) = app.modal.as_mut() else { return };
    let most = l.o.rows.unwrap_or(12).min(l.o.items.len()).max(1) as i32;
    let height = most + 7; // border, header, search, gap, rows, gap, footer
    let th = app.th;
    let cfg_w = l.o.width.unwrap_or(72);
    let mut f = frame(&th, c, height, cfg_w, l.o.at);
    let capacity = (f.h - 5).max(1) as usize; // header, search, gap … gap, footer
    l.first = l.first.min(l.sel).min(l.shown.len().saturating_sub(capacity));
    if l.sel >= l.first + capacity {
        l.first = l.sel + 1 - capacity;
    }
    let meta = l.o.meta.clone().unwrap_or_else(|| if !l.query.is_empty() { format!("{} of {}", l.shown.len(), l.o.items.len()) } else if l.o.at.is_some() { String::new() } else { l.o.items.len().to_string() });
    let placeholder = l.o.placeholder.clone().unwrap_or_else(|| if l.o.at.is_some() { "Filter, or press a key".into() } else { "Type to search".into() });
    let (query, title) = (l.query.clone(), l.o.title.clone());
    let key_width = l.o.items.iter().map(|i| i.key.as_deref().map_or(0, width)).max().unwrap_or(0); // keycaps line up
    let buttons_width = l.o.items.iter().map(|i| i.buttons.len() * 3).max().unwrap_or(0); // so do buttons: " ✎ " each
    header(&th, c, &mut f, &title, &meta);
    search_field(&th, c, &mut f, &query, &placeholder);
    f.y += 1;
    let bar = thumb(l.shown.len(), capacity, l.first, capacity);
    let mut drawn = 0;
    for (k, &index) in l.shown.iter().enumerate().skip(l.first).take(capacity) {
        let item = &l.o.items[index];
        let on = k == l.sel;
        let r = Rect { x: f.x, y: f.y, w: f.w, h: 1 };
        let bg = if on { mix(th.bar, th.focus, 0.16) } else if !app.cfg.mouse.hover && c.hovered(r) { mix(th.bar, th.fg, 0.07) } else { th.bar.to_string() };
        c.fill(r, &bg);
        c.hit(r, Hit::Modal(format!("row:{k}")));
        let right = item.key.clone().unwrap_or_default();
        let right_width = if key_width > 0 { key_width + 2 } else { 0 } + buttons_width;
        let room = (f.w as usize).saturating_sub(3 + right_width); // the marker, a space before it, and the scrollbar
        let name_color = if item.danger { th.blocked } else { th.fg };
        let desc_w = item.description.as_deref().map_or(0, width);
        let name = fit(&item.name, room.min(((room as f64 * 0.55).ceil() as usize).max(room.saturating_sub(desc_w + 2))).max(1));
        let mut x = f.x + 1;
        x = c.spans(x, f.y, &[(" ".into(), th.fg, false)], Some(&bg), 1);
        x = c.spans(x, f.y, &highlight(&name, &query, name_color, th.accent, on), Some(&bg), room);
        let desc_room = room as i64 - width(&name) as i64 - 3;
        if let Some(d) = item.description.as_ref().filter(|_| desc_room >= 4) {
            x = c.spans(x, f.y, &[("  ".into(), th.dim, false)], Some(&bg), 2);
            let dim_hit = mix(th.dim, th.accent, 0.6);
            let spans = highlight(&fit(d, desc_room as usize), &query, th.dim, &dim_hit, false);
            let owned: Vec<(String, &str, bool)> = spans.iter().map(|(t, col, b)| (t.clone(), *col, *b)).collect();
            c.spans(x, f.y, &owned, Some(&bg), desc_room as usize);
        }
        let mut rx = f.x + f.w - 1 - buttons_width as i32 - if key_width > 0 { key_width as i32 + 2 } else { 0 };
        if !right.is_empty() {
            let cap = format!(" {right:>key_width$} ");
            c.text(rx, f.y, &cap, if on { th.fg } else { th.dim }, Some(&mix(th.bar, th.fg, if on { 0.14 } else { 0.07 })), Modifier::empty(), cap.len());
        }
        if key_width > 0 {
            rx += key_width as i32 + 2;
        }
        rx += (buttons_width - item.buttons.len() * 3) as i32;
        for b in &item.buttons {
            let fg = if on { if b.danger { th.blocked } else { th.fg } } else { th.dim };
            c.text(rx, f.y, &format!(" {} ", b.icon), fg, Some(&bg), Modifier::empty(), 3);
            c.hit(Rect { x: rx, y: f.y, w: 3, h: 1 }, Hit::Modal(format!("value:{}", b.value)));
            rx += 3;
        }
        let thumb_cell = bar.is_some_and(|(top, size)| k - l.first >= top && k - l.first < top + size);
        c.text(f.x + f.w - 1, f.y, if thumb_cell { "▐" } else { " " }, th.border, Some(&bg), Modifier::empty(), 1);
        f.y += 1;
        drawn += 1;
    }
    if l.shown.is_empty() {
        c.text(f.x, f.y, &fit(&format!("  No matches for “{query}”"), f.w as usize), th.dim, Some(th.bar), Modifier::empty(), f.w as usize);
    }
    let _ = drawn;
    let mut hints: Vec<(String, String)> = vec![("↑↓".into(), "move".into()), ("↵".into(), if l.o.at.is_some() { "run".into() } else { "choose".into() })];
    if let Some(&i) = l.shown.get(l.sel) {
        hints.extend(l.o.items[i].buttons.iter().map(|b| (format!("^{}", b.key), b.title.clone())));
    }
    hints.push(("esc".into(), "close".into()));
    let bottom = f.bottom - 1;
    footer(&th, c, &f, bottom, &hints, if query.is_empty() { "" } else { "^u clear" });
}

const SPIN: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

fn draw_busy(app: &App, c: &mut Canvas) {
    let Some(Modal { kind: Kind::Busy(b), .. }) = app.modal.as_ref() else { return };
    let th = &app.th;
    let mut f = frame(th, c, 6, 64, None);
    header(th, c, &mut f, &b.title, "");
    f.y += 1;
    let frame_no = super::views::tick() * 80 / 100; // a frame every 100ms
    c.text(f.x, f.y, &fit(&format!("{} {}", SPIN[(frame_no % 10) as usize], b.what), f.w as usize), th.fg, Some(th.bar), Modifier::empty(), f.w as usize);
    let bottom = f.bottom - 1;
    footer(th, c, &f, bottom, &[("esc".into(), "hide: it carries on".into())], "");
}

fn draw_ask(app: &App, c: &mut Canvas) {
    let Some(Modal { kind: Kind::Ask(a), .. }) = app.modal.as_ref() else { return };
    let th = &app.th;
    let mut f = frame(th, c, a.lines.len() as i32 + 6, a.width, None);
    header(th, c, &mut f, &a.title, "");
    f.y += 1;
    for l in &a.lines {
        c.text(f.x, f.y, &fit(l, f.w as usize), th.fg, Some(th.bar), Modifier::empty(), f.w as usize);
        f.y += 1;
    }
    f.y += 1;
    let widths: Vec<i32> = a.choices.iter().map(|ch| (width(&ch.label) + 3 + width(&ch.key) + 1) as i32).collect();
    let total: i32 = widths.iter().sum::<i32>() + (a.choices.len() as i32 - 1).max(0);
    let mut x = f.x + f.w - total;
    for (i, ch) in a.choices.iter().enumerate() {
        let tone = match ch.tone {
            Some("danger") => th.blocked,
            Some("primary") => th.accent,
            _ => th.focus,
        };
        x += button(th, c, x, f.y, &ch.label, &ch.key, tone, i == a.sel, Hit::Modal(format!("value:{}", ch.value))) + 1;
    }
}

fn draw_prompt(app: &App, c: &mut Canvas) -> Option<Position> {
    let Some(Modal { kind: Kind::Prompt(p), .. }) = app.modal.as_ref() else { return None };
    let th = &app.th;
    let mut f = frame(th, c, 7, 60, None);
    header(th, c, &mut f, &p.title, "");
    f.y += 1;
    c.fill(Rect { x: f.x, y: f.y, w: f.w, h: 1 }, th.bg);
    let x = f.x + c.text(f.x, f.y, " › ", th.accent, Some(th.bg), Modifier::empty(), 3);
    let room = (f.w - 4).max(1) as usize;
    let value: String = p.value.iter().collect();
    let before: String = p.value[..p.cursor].iter().collect();
    // keep the caret in view: what's typed scrolls left as it grows past the field
    let skip = width(&before).saturating_sub(room - 1);
    let shown: String = {
        let mut used = 0;
        value.chars().filter(|ch| {
            let w = width(ch.encode_utf8(&mut [0; 4]));
            used += w;
            used > skip
        }).collect()
    };
    if value.is_empty() {
        c.text(x, f.y, &fit(&p.placeholder, room), th.dim, Some(th.bg), Modifier::empty(), room);
    } else {
        c.text(x, f.y, &shown, th.fg, Some(th.bg), Modifier::empty(), room);
    }
    c.hit(Rect { x: f.x, y: f.y, w: f.w, h: 1 }, Hit::Inert);
    let cursor = Position::new((x + (width(&before) - skip) as i32) as u16, f.y as u16);
    f.y += 2;
    let (cancel, ok) = ((width("Cancel") + 3 + 3 + 1) as i32, (width("OK") + 3 + width("↵") + 1) as i32);
    let mut bx = f.x + f.w - cancel - 1 - ok;
    bx += button(th, c, bx, f.y, "Cancel", "esc", th.fg, false, Hit::Modal("cancel".into())) + 1;
    button(th, c, bx, f.y, "OK", "↵", th.accent, true, Hit::Modal("ok".into()));
    Some(cursor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_matches() {
        let items = vec![ListItem::new("Split right", "", "a"), ListItem::new("Rename pane", "split it", "b"), ListItem::new("Zoom split", "", "c")];
        assert_eq!(matches(&items, "split"), vec![0, 2, 1]);
        assert_eq!(matches(&items, ""), vec![0, 1, 2]);
        let k = |c: KeyCode, m: KeyModifiers| KeyEvent::new(c, m);
        assert_eq!(typed("ab cd", &k(KeyCode::Char('w'), KeyModifiers::CONTROL)).as_deref(), Some("ab "));
        assert_eq!(typed("ab", &k(KeyCode::Backspace, KeyModifiers::NONE)).as_deref(), Some("a"));
        assert_eq!(typed("ab", &k(KeyCode::Char('x'), KeyModifiers::NONE)).as_deref(), Some("abx"));
    }
}
