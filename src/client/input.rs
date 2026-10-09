// Keys and the pointer. Keys, in priority order: cancel a resize, the open dialog, copy mode, then the prefix (Ctrl+B by
// default) followed by a binding; everything else goes to the focused pane. Clicks go to what the last frame drew under
// them (`app.hits`); a pane's program gets the mouse when it asked for it, otherwise a drag selects its text and the
// wheel scrolls its scrollback.
use std::io::Write;

use alacritty_terminal::grid::Scroll;
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::TermMode;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use serde_json::json;

use super::actions;
use super::views;
use super::design::chrome;
use super::draw::Hit;
use super::modals::{self, key_name};
use super::{App, Resizing, Selecting, Shared};
use crate::core::layout::{display_rects, divider_at, panes as tree_panes, Rect};
use crate::protocol::conn::b64;

pub fn event(shared: &Shared, e: Event) {
    match e {
        Event::Key(k) if k.kind != KeyEventKind::Release => key(shared, k),
        Event::Mouse(m) => mouse(shared, m),
        Event::Paste(text) => paste(shared, &text),
        Event::Resize(w, h) => {
            let mut app = shared.borrow_mut();
            app.width = w as i32;
            app.height = h as i32;
            end_resize(&mut app);
            let area = app.area();
            app.notify_server("area", json!({ "area": area }));
            super::plugin_ui::fit_popup(&app); // a popup's program sizes to it again
            app.dirty();
        }
        Event::FocusGained | Event::FocusLost => {
            let mut app = shared.borrow_mut();
            app.term_focused = e == Event::FocusGained;
            end_resize(&mut app);
            app.dirty();
        }
        _ => {}
    }
}

// The focused pane, if there's one to type into.
fn focused(app: &App) -> Option<String> {
    app.ready().then(|| app.tab().focused.clone())
}

// Programs that asked to hear about focus (CSI ? 1004 h) are told when typing stops reaching them and when it reaches
// them again: this terminal losing focus, a dialog or copy mode over them, another pane focused. Asked every frame.
pub fn report_focus(app: &mut App) {
    let typing = app.term_focused && app.modal.is_none() && !app.copy_mode && !views::has_keyboard(app);
    let now = if typing { focused(app) } else { None };
    if now == app.reported_focus {
        return;
    }
    for (pane, bytes) in [(app.reported_focus.take(), b"\x1b[O"), (now.clone(), b"\x1b[I")] {
        if let Some(p) = pane.filter(|p| app.panes.get(p).is_some_and(|c| c.screen.mode().contains(TermMode::FOCUS_IN_OUT))) {
            app.notify_server("input", json!({ "pane": p, "data": b64(bytes) }));
        }
    }
    app.reported_focus = now;
}

// Input for a pane, as its program expects it. A pane scrolled back into its history comes back down first.
fn send(app: &mut App, pane: &str, bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    if let Some(p) = app.panes.get_mut(pane) {
        p.screen.term.scroll_display(Scroll::Bottom);
    }
    app.notify_server("input", json!({ "pane": pane, "data": b64(bytes) }));
}

fn key(shared: &Shared, k: KeyEvent) {
    let mut app = shared.borrow_mut();
    if app.resizing.is_some() && k.code == KeyCode::Esc {
        end_resize(&mut app);
        return pointer(&mut app, "default");
    }
    if app.modal.is_some() {
        return modals::key(&mut app, &k);
    }
    if views::has_keyboard(&app) {
        return views::key(&mut app, &k);
    }
    let is_prefix = app.prefix.ctrl && k.modifiers.contains(KeyModifiers::CONTROL) && matches!(k.code, KeyCode::Char(c) if c.to_lowercase().to_string() == app.prefix.name);
    if app.copy_mode && !app.prefix_armed && !is_prefix {
        return copy_key(&mut app, &k);
    }
    if app.prefix_armed {
        app.prefix_armed = false;
        app.dirty();
        if is_prefix {
            // prefix twice sends it through
            if let Some(p) = focused(&app) {
                let bytes = encode_for(&app, &p, &k);
                send(&mut app, &p, &bytes);
            }
            return;
        }
        let name = key_name(&k);
        let action = app.bindings.get(&name).copied();
        if action.is_none() {
            return super::plugin_ui::plugin_key(&mut app, &name); // modisa's keys first; a plugin never gets one of them
        }
        drop(app);
        if let Some(a) = action {
            actions::run(shared, a);
        }
        return;
    }
    if is_prefix {
        app.prefix_armed = true;
        app.dirty();
        return;
    }
    if let Some(p) = focused(&app) {
        let bytes = encode_for(&app, &p, &k);
        send(&mut app, &p, &bytes);
    }
}

fn encode_for(app: &App, pane: &str, k: &KeyEvent) -> Vec<u8> {
    let mode = app.panes.get(pane).map(|p| p.screen.mode()).unwrap_or_default();
    super::keys::encode(k, mode)
}

fn paste(shared: &Shared, text: &str) {
    let mut app = shared.borrow_mut();
    if app.modal.is_some() {
        return modals::paste(&mut app, text);
    }
    if views::has_keyboard(&app) {
        return views::paste(&mut app, text);
    }
    let Some(p) = focused(&app) else { return };
    let bracketed = app.panes.get(&p).is_some_and(|x| x.screen.mode().contains(TermMode::BRACKETED_PASTE));
    let data = if bracketed { format!("\x1b[200~{text}\x1b[201~") } else { text.to_string() };
    send(&mut app, &p, data.as_bytes());
}

// ---------- copy mode: the focused pane's scrollback from the keyboard, and jumping between search matches ----------

fn scroll(app: &mut App, pane: &str, lines: i32) {
    if let Some(p) = app.panes.get_mut(pane) {
        p.screen.term.scroll_display(Scroll::Delta(lines));
        app.dirty();
    }
}

pub fn jump(app: &mut App) {
    let Some(p) = focused(app) else { return };
    let Some(search) = app.search.as_ref() else { return };
    let line = search.matches[search.i];
    let rows = app.info(&p).map(|i| i.rows as usize).unwrap_or(20);
    let up = search.total.saturating_sub(line + rows.div_ceil(2));
    let msg = format!("match {}/{}", search.i + 1, search.matches.len());
    if let Some(pane) = app.panes.get_mut(&p) {
        pane.screen.term.scroll_display(Scroll::Bottom);
        pane.screen.term.scroll_display(Scroll::Delta(up as i32));
    }
    let a = app.th.accent;
    app.toast(&msg, a);
}

fn copy_key(app: &mut App, k: &KeyEvent) {
    let Some(p) = focused(app) else { return };
    let half = (app.info(&p).map(|i| i.rows as i32).unwrap_or(20) / 2).max(1);
    let name = key_name(k);
    if name == "q" || name == "escape" {
        if let Some(pane) = app.panes.get_mut(&p) {
            pane.screen.term.scroll_display(Scroll::Bottom);
        }
        app.copy_mode = false;
        app.search = None;
        return app.dirty();
    }
    if name == "/" {
        let shared = app.shared();
        return actions::run(&shared, "search");
    }
    if let Some(s) = app.search.as_mut().filter(|_| name == "n" || name == "N") {
        let n = s.matches.len();
        s.i = (s.i + if name == "n" { n - 1 } else { 1 }) % n;
        return jump(app);
    }
    // up the scrollback is positive here
    let moves = match name.as_str() {
        "k" | "up" => 1,
        "j" | "down" => -1,
        "u" => half,
        "pageup" => half * 2,
        "d" => -half,
        "pagedown" => -half * 2,
        "g" => i32::MAX / 2,
        "G" => i32::MIN / 2,
        _ => return,
    };
    scroll(app, &p, moves);
}

// ---------- the pointer ----------

fn hit_at(app: &App, x: i32, y: i32) -> Option<Hit> {
    app.hits.iter().rev().find(|(r, _)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h).map(|(_, h)| h.clone())
}

// One place decides the pointer shape (OSC 22): the move arrows over a pane border or the sidebar's edge (or while
// resizing), a hand over anything clickable, the default arrow elsewhere.
pub fn pointer(app: &mut App, shape: &'static str) {
    if shape == app.pointer_shape {
        return;
    }
    app.pointer_shape = shape;
    let mut out = std::io::stdout();
    let _ = write!(out, "\x1b]22;{shape}\x07");
    let _ = out.flush();
}

fn on_divider(app: &App, x: i32, y: i32) -> bool {
    if !app.ready() || app.modal.is_some() {
        return false;
    }
    let t = app.tab();
    // compact view: no borders to drag
    if t.zoomed || display_rects(&t.tree, app.area(), &t.focused, false).len() < tree_panes(&t.tree).len() {
        return false;
    }
    divider_at(&t.tree, app.area(), x, y).is_some()
}

// A pane's box on screen, its border included: a popup's where it floats.
fn pane_rect(app: &App, id: &str) -> Option<Rect> {
    if let Some(p) = super::plugin_ui::popup(app).filter(|p| p.pane == id) {
        return Some(super::plugin_ui::popup_rect(app, p));
    }
    let t = app.tab();
    display_rects(&t.tree, app.area(), &t.focused, t.zoomed).get(id).copied()
}

fn begin_resize(app: &mut App, x: i32, y: i32, sidebar: bool) {
    if app.resizing.is_some() {
        return;
    }
    app.resizing = Some(Resizing { x, y, sidebar, saw_drag: false });
    pointer(app, "move");
    if !sidebar {
        app.call("dragStart", json!({ "x": x, "y": y }));
    }
}

// A pane border's drag is the server's (it owns the layout); the sidebar's is this client's own, saved when it ends.
pub fn end_resize(app: &mut App) {
    let Some(r) = app.resizing.take() else { return };
    if r.sidebar {
        // the panes' terminals take their new size once, here: resizing them on every step of the drag would have each
        // program redraw over and over
        let area = app.area();
        app.notify_server("area", json!({ "area": area }));
        if let Err(e) = crate::config::save_setting(Some("sidebar"), "width", &json!(app.cfg.sidebar.width)) {
            let w = app.th.warn;
            app.toast(&format!("sidebar width not saved: {e}"), w);
        }
    } else {
        app.call("dragEnd", json!({}));
    }
    pointer(app, "default");
}

// The edge follows the pointer, within what the terminal allows the sidebar (as wide as it can be at any preference).
fn sidebar_to(app: &mut App, w: i32) {
    let most = chrome(app.width, app.height, true, 999).side;
    let next = w.min(most).max(20) as i64;
    if next != app.cfg.sidebar.width {
        app.cfg.sidebar.width = next;
        app.dirty();
    }
}

// SGR / X10 mouse reports for a program that asked for the mouse: b is the button code, (x, y) cell coordinates from 1.
fn mouse_report(mode: TermMode, b: u8, x: i32, y: i32, release: bool) -> Vec<u8> {
    if mode.contains(TermMode::SGR_MOUSE) {
        return format!("\x1b[<{b};{x};{y}{}", if release { 'm' } else { 'M' }).into_bytes();
    }
    let b = if release { 3 | (b & 0b0001_1100) } else { b }; // a release names no button
    let mut out = b"\x1b[M".to_vec();
    out.push(32 + b);
    for v in [x, y] {
        if mode.contains(TermMode::UTF8_MOUSE) {
            let c = char::from_u32((32 + v) as u32).unwrap_or(' ');
            out.extend(c.to_string().as_bytes());
        } else {
            out.push((32 + v.min(223)) as u8);
        }
    }
    out
}

fn modifier_bits(m: KeyModifiers) -> u8 {
    (m.contains(KeyModifiers::SHIFT) as u8) * 4 + (m.contains(KeyModifiers::ALT) as u8) * 8 + (m.contains(KeyModifiers::CONTROL) as u8) * 16
}

// The pointer over a pane whose program asked for the mouse: its report, if the program wants this kind of event.
fn forward(app: &mut App, pane: &str, m: &MouseEvent) -> bool {
    let Some(p) = app.panes.get(pane) else { return false };
    let mode = p.screen.mode();
    if !mode.intersects(TermMode::MOUSE_MODE) {
        return false;
    }
    let Some(r) = pane_rect(app, pane) else { return false };
    let (x, y) = (m.column as i32 - r.x, m.row as i32 - r.y); // from 1: the border is column 0
    if x < 1 || y < 1 || x > r.w - 2 || y > r.h - 2 {
        return false;
    }
    let button = |b: MouseButton| match b {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    };
    let mods = modifier_bits(m.modifiers);
    let report = match m.kind {
        MouseEventKind::Down(b) => mouse_report(mode, button(b) | mods, x, y, false),
        MouseEventKind::Up(b) => mouse_report(mode, button(b) | mods, x, y, true),
        MouseEventKind::Drag(b) if mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION) => mouse_report(mode, (button(b) + 32) | mods, x, y, false),
        MouseEventKind::Moved if mode.contains(TermMode::MOUSE_MOTION) => mouse_report(mode, (3 + 32) | mods, x, y, false),
        MouseEventKind::ScrollUp => mouse_report(mode, 64 | mods, x, y, false),
        MouseEventKind::ScrollDown => mouse_report(mode, 65 | mods, x, y, false),
        _ => return true, // its program has the mouse: nothing else does anything with this
    };
    let pane = pane.to_string();
    app.notify_server("input", json!({ "pane": pane, "data": b64(&report) }));
    true
}

// Where (x, y) is in a pane's grid, counting the scrollback it's showing.
fn grid_point(app: &App, pane: &str, x: i32, y: i32) -> Option<Point> {
    let r = pane_rect(app, pane)?;
    let p = app.panes.get(pane)?;
    let offset = p.screen.term.grid().display_offset() as i32;
    let cols = r.w - 2;
    let (col, row) = ((x - r.x - 1).clamp(0, cols.max(1) - 1), (y - r.y - 1).clamp(0, (r.h - 2).max(1) - 1));
    Some(Point::new(Line(row - offset), Column(col as usize)))
}

// The URL drawn under (x, y) in a pane, if there's one.
fn url_under(app: &App, pane: &str, x: i32, y: i32) -> Option<String> {
    let r = pane_rect(app, pane)?;
    let p = app.panes.get(pane)?;
    let (col, row) = (x - r.x - 1, y - r.y - 1);
    if col < 0 {
        return None;
    }
    super::plugin_ui::url_at(&super::plugin_ui::line_text(&p.screen, row), col as usize)
}

fn clear_selections(app: &mut App) {
    for p in app.panes.values_mut() {
        p.screen.term.selection = None;
    }
}

fn mouse(shared: &Shared, m: MouseEvent) {
    let mut app = shared.borrow_mut();
    let (x, y) = (m.column as i32, m.row as i32);
    app.hover = Some((x, y));
    // A resize under way takes every pointer event until it ends: a release, a fresh press (which then does what it
    // does), Escape or the terminal losing focus. VS Code can report a held drag as plain motion (SGR 35): that's taken
    // as a missed release only once this drag has reported motion with the button held.
    if let Some(r) = app.resizing.as_ref() {
        let held = matches!(m.kind, MouseEventKind::Drag(MouseButton::Left)) || (m.kind == MouseEventKind::Moved && !r.saw_drag);
        match m.kind {
            MouseEventKind::Moved if !held => return end_resize(&mut app),
            MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Moved => {
                let saw_drag = r.saw_drag || matches!(m.kind, MouseEventKind::Drag(_));
                if let Some(r) = app.resizing.as_mut() {
                    r.saw_drag = saw_drag;
                }
                let r = app.resizing.as_ref().unwrap();
                if (x, y) != (r.x, r.y) {
                    let sidebar = r.sidebar;
                    app.resizing = Some(Resizing { x, y, sidebar, saw_drag });
                    if sidebar {
                        sidebar_to(&mut app, x + 1);
                    } else {
                        app.call("dragMove", json!({ "x": x, "y": y }));
                    }
                }
                return;
            }
            MouseEventKind::Up(_) => return end_resize(&mut app),
            MouseEventKind::Down(_) => end_resize(&mut app),
            _ => return,
        }
    }
    let hit = hit_at(&app, x, y);
    // a plugin's popup: its program gets the mouse over its terminal; nothing outside it takes a click
    if super::plugin_ui::popup(&app).is_some() {
        if let Some(Hit::Pane(id)) = &hit {
            let id = id.clone();
            forward(&mut app, &id, &m);
        }
        return pointer(&mut app, "default");
    }
    if app.modal.is_some() {
        match (m.kind, &hit) {
            (MouseEventKind::Down(b), Some(Hit::Modal(v))) => {
                let v = v.clone();
                modals::click(&mut app, &v, b == MouseButton::Right, x, y);
            }
            (MouseEventKind::Down(_), Some(Hit::Veil)) => modals::close(&mut app, None),
            (MouseEventKind::Moved, Some(Hit::Modal(v))) => {
                let v = v.clone();
                modals::hover(&mut app, &v);
            }
            (MouseEventKind::ScrollUp, _) => modals::scroll(&mut app, -3),
            (MouseEventKind::ScrollDown, _) => modals::scroll(&mut app, 3),
            _ => {}
        }
        let shape = if hit.as_ref().is_some_and(|h| matches!(h, Hit::Modal(_))) { "pointer" } else { "default" };
        pointer(&mut app, shape);
        return app.dirty();
    }
    // plugins' views: their elements take clicks and the wheel; nothing under the top one does
    if !app.views.is_empty() {
        match (m.kind, &hit) {
            (MouseEventKind::Down(MouseButton::Left), Some(Hit::View { view, key, part })) => {
                let (view, key, part) = (view.clone(), key.clone(), *part);
                views::click(&mut app, &view, &key, part);
            }
            (MouseEventKind::ScrollUp | MouseEventKind::ScrollDown, _) => {
                let under: Vec<Hit> = app.hits.iter().rev().filter(|(r, _)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h).map(|(_, h)| h.clone()).collect();
                views::wheel(&mut app, &under, if m.kind == MouseEventKind::ScrollUp { -3 } else { 3 });
            }
            (MouseEventKind::Moved, _) => app.dirty(),
            _ => {}
        }
        let shape = if hit.as_ref().is_some_and(Hit::clickable) { "pointer" } else { "default" };
        return pointer(&mut app, shape);
    }
    if !app.ready() {
        return;
    }
    // Ctrl+click on a URL in a pane hands it to the plugins that take such links, before its program gets the click
    if let Some(Hit::Pane(id)) = &hit {
        let ctrl_click = m.modifiers.contains(KeyModifiers::CONTROL) && matches!(m.kind, MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left));
        if let Some(url) = ctrl_click.then(|| url_under(&app, id, x, y)).flatten() {
            if matches!(m.kind, MouseEventKind::Down(_)) {
                let id = id.clone();
                super::plugin_ui::plugin_link(&mut app, &id, &url, x, y);
            }
            return;
        }
    }
    // a pane's program that asked for the mouse gets what happens over its terminal
    if let Some(Hit::Pane(id)) = &hit {
        let id = id.clone();
        if matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) && id != app.tab().focused {
            app.call("focusPane", json!({ "pane": id }));
        }
        let right_click = matches!(m.kind, MouseEventKind::Down(MouseButton::Right) | MouseEventKind::Up(MouseButton::Right));
        if !right_click && app.selecting.is_none() && forward(&mut app, &id, &m) {
            return pointer(&mut app, "default");
        }
    }
    match m.kind {
        MouseEventKind::Moved => {
            let shape = if on_divider(&app, x, y) || matches!(hit, Some(Hit::SidebarEdge)) { "move" } else if hit.as_ref().is_some_and(Hit::clickable) { "pointer" } else { "default" };
            pointer(&mut app, shape);
            app.dirty(); // hover tints
        }
        MouseEventKind::Down(MouseButton::Left) => {
            clear_selections(&mut app);
            match hit {
                Some(Hit::Action(a)) => {
                    drop(app);
                    actions::run(shared, a);
                }
                Some(Hit::Tab(i)) => {
                    if i == app.ws().active {
                        drop(app);
                        actions::run(shared, "rename-tab");
                    } else {
                        app.call("selectTab", json!({ "index": i }));
                    }
                }
                Some(Hit::TabNode(tab, _)) => {
                    if !app.collapsed_tabs.remove(&tab) {
                        app.collapsed_tabs.insert(tab);
                    }
                    app.dirty();
                }
                Some(Hit::FocusPane(p)) => app.call("focusPane", json!({ "pane": p })),
                Some(Hit::Plugin { plugin, run, action, pane, instance }) => {
                    drop(app);
                    super::plugin_ui::clicked(shared, &plugin, &run, action.as_deref(), pane.as_deref(), instance.as_deref());
                }
                Some(Hit::PluginFold(name)) => {
                    if !app.collapsed_plugins.remove(&name) {
                        app.collapsed_plugins.insert(name);
                    }
                    app.dirty();
                }
                Some(Hit::SidebarEdge) => begin_resize(&mut app, x, y, true),
                Some(Hit::Border(id)) => {
                    // the border: press on a divider to resize, anywhere else to focus
                    if on_divider(&app, x, y) {
                        begin_resize(&mut app, x, y, false);
                    } else if id != app.tab().focused {
                        app.call("focusPane", json!({ "pane": id }));
                    }
                }
                Some(Hit::Pane(id)) => {
                    // a drag from here selects the pane's text
                    if let Some(at) = grid_point(&app, &id, x, y) {
                        if let Some(p) = app.panes.get_mut(&id) {
                            p.screen.term.selection = Some(Selection::new(SelectionType::Simple, at, Side::Left));
                        }
                        app.selecting = Some(Selecting { pane: id });
                    }
                }
                _ => {}
            }
        }
        MouseEventKind::Down(MouseButton::Right) => match hit {
            Some(Hit::Tab(i)) => {
                drop(app);
                actions::tab_menu(shared, i, x, y);
            }
            Some(Hit::Pane(id)) | Some(Hit::Border(id)) | Some(Hit::TabNode(_, id)) => {
                drop(app);
                actions::context_menu(shared, &id, x, y);
            }
            _ => {}
        },
        MouseEventKind::Drag(MouseButton::Left) => {
            if let Some(pane) = app.selecting.as_ref().map(|s| s.pane.clone()) {
                if let Some(at) = grid_point(&app, &pane, x, y) {
                    if let Some(sel) = app.panes.get_mut(&pane).and_then(|p| p.screen.term.selection.as_mut()) {
                        sel.update(at, Side::Right);
                    }
                    app.dirty();
                }
            }
        }
        MouseEventKind::Up(MouseButton::Left) => {
            // a selection made is copied to the clipboard
            if let Some(s) = app.selecting.take() {
                let term = app.panes.get(&s.pane).map(|p| &p.screen.term);
                let made = term.and_then(|t| t.selection.as_ref()).is_some_and(|sel| !sel.is_empty());
                match term.and_then(|t| t.selection_to_string()).filter(|t| made && !t.is_empty()) {
                    Some(t) => app.copy(&t),
                    None => clear_selections(&mut app),
                }
                app.dirty();
            }
        }
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
            let up = matches!(m.kind, MouseEventKind::ScrollUp);
            let (Some(Hit::Pane(id)) | Some(Hit::Border(id))) = hit else { return };
            let alt = app.panes.get(&id).map(|p| p.screen.mode()).filter(|m| m.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL));
            if let Some(mode) = alt {
                // a full-screen program without the mouse: the wheel is its arrow keys
                let k = if mode.contains(TermMode::APP_CURSOR) { if up { "\x1bOA" } else { "\x1bOB" } } else if up { "\x1b[A" } else { "\x1b[B" };
                send(&mut app, &id, k.repeat(3).as_bytes());
            } else {
                scroll(&mut app, &id, if up { 3 } else { -3 });
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_the_mouse() {
        assert_eq!(mouse_report(TermMode::SGR_MOUSE, 0, 3, 4, false), b"\x1b[<0;3;4M");
        assert_eq!(mouse_report(TermMode::SGR_MOUSE, 0, 3, 4, true), b"\x1b[<0;3;4m");
        assert_eq!(mouse_report(TermMode::MOUSE_REPORT_CLICK, 0, 1, 1, false), b"\x1b[M !!");
        assert_eq!(mouse_report(TermMode::MOUSE_REPORT_CLICK, 64, 1, 1, false), b"\x1b[M`!!");
    }
}
