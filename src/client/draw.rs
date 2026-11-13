// Draw what the server last sent: pane terminals at their layout positions, then the chrome (the tab bar, the sidebar,
// the status row), a dialog over it all, and the toasts over that. Everything clickable is recorded in `app.hits`.
use ratatui::buffer::Buffer;
use ratatui::layout::Position;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::Frame;

use super::design::{agent_graph, agent_mark, agent_task, color, fit, mix, sidebar_budget, sidebar_columns, tab_window, GraphRow};
use super::format::{render as format, Run};
use super::slots::{self, Seg, SlotElem, Strip};
use super::vars;
use super::{look, modals, App};
use crate::config::BorderStyle;
use crate::core::layout::{display_rects, panes as tree_panes, Rect};
use crate::core::text::width;
use crate::protocol::types::{AgentState, PaneInfo, TabView};

// What a click on a spot does.
#[derive(Clone, Debug, PartialEq)]
pub enum Hit {
    Action(&'static str),            // run one of the actions (actions.rs)
    Tab(usize),                      // a tab: select it (or rename it, when it's the one shown); right: its menu
    TabNode(String, String),         // a tab's row in the sidebar's graph: fold it (tab id); right: its focused pane's menu
    FocusPane(String),               // an agent's sidebar row
    Pane(String),                    // a pane's terminal
    Border(String),                  // a pane's border: drag a divider, else focus; right: its menu
    SidebarEdge,                     // drag it to make the sidebar wider or narrower
    Veil,                            // the screen behind a dialog: a click dismisses it
    Modal(String),                   // something in a dialog: what it chooses (modals.rs)
    Inert,                           // part of a dialog that does nothing, but isn't the veil
    PluginFold(String), // a plugin's sidebar heading: fold its section
    Named(String),      // part of a format of the user's (#[click=…]): what it runs, by name (actions.rs run_named)
    View { view: String, key: String, part: i32 }, // an element of a plugin's view or sidebar section; part: a list's row, or what it is (views/)
    Slot(Box<slots::SlotHit>), // a plugin's piece of the chrome: run its action (slots.rs); without one, it only says whose it is
}

impl Hit {
    // what gets the hand pointer
    pub fn clickable(&self) -> bool {
        !matches!(self, Hit::Pane(_) | Hit::Veil | Hit::Inert | Hit::SidebarEdge | Hit::Border(_)) && !matches!(self, Hit::View { part, .. } if *part == super::views::build::SCROLLS) && !matches!(self, Hit::Slot(s) if s.action.is_none())
    }
}

pub struct Canvas<'a> {
    pub buf: &'a mut Buffer,
    pub hits: &'a mut Vec<(Rect, Hit)>,
    pub hover: Option<(i32, i32)>,
    pub w: i32,
    pub h: i32,
}

impl Canvas<'_> {
    // `s` at (x, y) in fg on bg (none: what's there), at most `max` cells; returns the cells it took.
    pub fn text(&mut self, x: i32, y: i32, s: &str, fg: &str, bg: Option<&str>, m: Modifier, max: usize) -> i32 {
        if y < 0 || y >= self.h || x >= self.w {
            return 0;
        }
        let mut style = Style::new().fg(color(fg)).add_modifier(m);
        if let Some(bg) = bg {
            style = style.bg(color(bg));
        }
        let (x0, skip) = if x < 0 { (0, (-x) as usize) } else { (x, 0) };
        let room = ((self.w - x0) as usize).min(max.saturating_sub(skip));
        let shown: String = if skip > 0 { fit_from(s, skip) } else { s.to_string() };
        let end = self.buf.set_stringn(x0 as u16, y as u16, &shown, room, style);
        end.0 as i32 - x0 + skip as i32
    }

    pub fn fill(&mut self, r: Rect, bg: &str) {
        let style = Style::new().bg(color(bg));
        for y in r.y.max(0)..(r.y + r.h).min(self.h) {
            for x in r.x.max(0)..(r.x + r.w).min(self.w) {
                if let Some(c) = self.buf.cell_mut(Position::new(x as u16, y as u16)) {
                    c.reset();
                    c.set_style(style);
                }
            }
        }
    }

    pub fn hit(&mut self, r: Rect, h: Hit) {
        self.hits.push((r, h));
    }

    pub fn hovered(&self, r: Rect) -> bool {
        self.hover.is_some_and(|(x, y)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)
    }

    // A one-line clickable label: under the pointer it takes the border colour behind the text colour.
    pub fn button(&mut self, x: i32, y: i32, text: &str, w: i32, fg: &str, bg: &str, hover: (&str, &str), hit: Hit) -> i32 {
        let r = Rect { x, y, w, h: 1 };
        let (fg, bg) = if self.hovered(r) { hover } else { (fg, bg) };
        self.fill(r, bg);
        self.text(x, y, &fit(text, w.max(0) as usize), fg, Some(bg), Modifier::empty(), w.max(0) as usize);
        self.hit(r, hit);
        w
    }

    // A box's border in `fg`, a title one cell in from its corner (OpenTUI's).
    pub fn border(&mut self, r: Rect, style: BorderStyle, fg: &str, bg: Option<&str>, title: Option<(&str, &str)>) {
        if r.w < 2 || r.h < 2 {
            return;
        }
        let b = style.set();
        let n = (r.w - 2) as usize;
        let line = |l: &str, hz: &str, rr: &str| format!("{l}{}{rr}", hz.repeat(n));
        self.text(r.x, r.y, &line(b.top_left, b.horizontal_top, b.top_right), fg, bg, Modifier::empty(), r.w as usize);
        self.text(r.x, r.y + r.h - 1, &line(b.bottom_left, b.horizontal_bottom, b.bottom_right), fg, bg, Modifier::empty(), r.w as usize);
        for y in r.y + 1..r.y + r.h - 1 {
            self.text(r.x, y, b.vertical_left, fg, bg, Modifier::empty(), 1);
            self.text(r.x + r.w - 1, y, b.vertical_right, fg, bg, Modifier::empty(), 1);
        }
        if let Some((t, c)) = title.filter(|(t, _)| !t.is_empty() && r.w > 4) {
            self.text(r.x + 2, r.y, t, c, bg, Modifier::empty(), (r.w - 4) as usize);
        }
    }

    // Spans on one line: (text, colour, bold); returns where it ended.
    pub fn spans(&mut self, x: i32, y: i32, spans: &[(String, &str, bool)], bg: Option<&str>, max: usize) -> i32 {
        let mut at = x;
        for (t, c, b) in spans {
            let room = max.saturating_sub((at - x) as usize);
            if room == 0 {
                break;
            }
            at += self.text(at, y, t, c, bg, if *b { Modifier::BOLD } else { Modifier::empty() }, room);
        }
        at
    }
}

// what's left of `s` after its first `skip` cells
pub fn fit_from(s: &str, skip: usize) -> String {
    let mut used = 0;
    let mut out = String::new();
    for c in s.chars() {
        if used >= skip {
            out.push(c);
        }
        used += width(c.encode_utf8(&mut [0; 4]));
    }
    out
}

pub fn render(app: &mut App, f: &mut Frame) {
    let size = f.area();
    look::notice(app);
    app.width = size.width as i32;
    app.height = size.height as i32;
    let mut hits = std::mem::take(&mut app.hits);
    hits.clear();
    let mut elems = std::mem::take(&mut app.slot_elems);
    let mut c = Canvas { buf: f.buffer_mut(), hits: &mut hits, hover: app.hover, w: size.width as i32, h: size.height as i32 };
    c.fill(Rect { x: 0, y: 0, w: c.w, h: c.h }, app.th.bg);
    let mut cursor = None;
    if app.ready() {
        cursor = in_role(app, &["pane.border", "pane.border.focused"], |app| panes(app, &mut c));
        in_role(app, &["tab.inactive"], |app| tabs(app, &mut c));
        in_role(app, &["sidebar"], |app| sidebar(app, &mut c, &mut elems));
        in_role(app, &["status"], |app| status(app, &mut c));
    }
    app.slot_elems = elems;
    // plugins' views over all that; the top one has the keyboard, unless a dialog is open over it
    if !app.views.is_empty() {
        cursor = super::views::draw(app, &mut c);
    }
    // the prefix held a moment with nothing after it: the keys that can come next
    if app.ready() && app.prefix_armed && app.modal.is_none() && app.prefix_at.elapsed() >= look::WHICH_KEY {
        which_key(app, &mut c);
        cursor = None;
    }
    if app.modal.is_some() {
        cursor = in_role(app, &["menu", "menu.selected"], |app| modals::draw(app, &mut c));
    }
    in_role(app, &["toast"], |app| toasts(app, &mut c));
    slots::tooltip(app, &mut c);
    app.hits = hits;
    if let Some(at) = cursor {
        f.set_cursor_position(at);
    }
}

// A part of the chrome drawn in its theme's roles (CUSTOMIZE.md, Themes): while `draw` runs, the tokens it's drawn with
// are the roles' (a region's role: its text and background; pane.border*: the borders; menu.selected: what's chosen),
// and they're the theme's again after.
fn in_role<T>(app: &mut App, roles: &[&str], draw: impl FnOnce(&mut App) -> T) -> T {
    let th = app.th;
    for name in roles {
        let Some(r) = app.roles.get(name).copied() else { continue };
        match *name {
            "pane.border" => app.th.border = r.fg.unwrap_or(th.border),
            "pane.border.focused" => app.th.focus = r.fg.unwrap_or(th.focus),
            "menu.selected" => app.th.focus = r.bg.or(r.fg).unwrap_or(th.focus),
            "tab.inactive" => {
                app.th.dim = r.fg.unwrap_or(th.dim);
                app.th.bar = r.bg.unwrap_or(th.bar);
            }
            _ => {
                app.th.fg = r.fg.unwrap_or(th.fg);
                app.th.bar = r.bg.unwrap_or(th.bar);
            }
        }
    }
    let out = draw(app);
    app.th = th;
    out
}

// ---------- panes ----------

fn panes(app: &App, c: &mut Canvas) -> Option<Position> {
    let th = &app.th;
    let ctx = slots::ctx(app);
    let a = app.area();
    let t = app.tab();
    let rs = display_rects(&t.tree, a, &t.focused, t.zoomed);
    let mut cursor = None;
    for (id, r) in &rs {
        let Some(i) = app.info(id) else { continue };
        let focused = *id == t.focused;
        let st = i.agent.as_ref().map(|a| a.state);
        // quiet hairlines; the focused pane's ring runs from the focus colour round to the accent (and sweeps once when
        // focus arrives); an agent that needs you, elsewhere, pulses
        let hair = look::hair(th);
        let border = if focused {
            th.focus.to_string()
        } else if st == Some(AgentState::Blocked) {
            mix(&hair, th.blocked, 0.3 + 0.55 * look::breath(app))
        } else {
            hair.clone()
        };
        let exited = if i.status == "exited" { format!(" [exited {}]", i.exit_code.map(|c| c.to_string()).unwrap_or("?".into())) } else { String::new() };
        // driven from another terminal (pane attach), at its size: typing here doesn't reach it
        let elsewhere = if i.takeover == Some(true) { " [attached elsewhere]" } else { "" };
        // its title: its name, agent and state, with plugins' pieces around them or a plugin's instead
        let room = (r.w - 4).max(0) as usize;
        let faint = look::faint(th);
        let mut title = Strip::default();
        title.text(format!(" {} ", if focused { "◆" } else { "◇" }), Style::new().fg(color(if focused { th.focus } else { &faint })));
        if app.cfg.panes.title.is_empty() {
            title.append(slots::label_spans(&ctx, &slots::around(app, "pane.title", |p| slots::of_pane(p, i)), &pane_title(app, i, focused), room));
        } else {
            // [panes] title: the user's format for it
            let vars = |n: &str, a: Option<&str>| vars::pane(app, i, n, a);
            for run in format(&app.cfg.panes.title, &vars, th, Style::new()).left {
                title.text(run.text, run.style);
            }
        }
        title.text(format!("{exited}{elsewhere} "), Style::new());
        title.fit(room);
        let title_color = if focused { th.fg } else { st.map(|s| app.state_color(s)).or(app.roles.get("pane.title").and_then(|r| r.fg)).unwrap_or(th.dim) };
        c.border(*r, app.cfg.panes.border, &border, Some(th.bg), None);
        if focused {
            let turn = look::eased(app, app.focus_at, look::SWEEP);
            look::ring(c, *r, |t| look::ring_color(th, t, turn));
        }
        c.hit(*r, Hit::Border(id.clone()));
        if r.w > 4 && r.h >= 2 {
            // [panes] title_position: which edge, and where along it
            let pos = app.cfg.panes.title_position.as_str();
            let ty = if pos.starts_with("bottom") { r.y + r.h - 1 } else { r.y };
            let tw = title.width() as i32;
            let tx = if pos.ends_with("center") { r.x + (r.w - tw) / 2 } else if pos.ends_with("right") { r.x + r.w - 2 - tw } else { r.x + 2 };
            title.draw(c, tx.max(r.x + 1), ty, Style::new().fg(color(title_color)).bg(color(th.bg)), room);
        }
        slots::border(app, &ctx, c, *r, i, title.width(), &border);
        let inner = Rect { x: r.x + 1, y: r.y + 1, w: r.w - 2, h: r.h - 2 };
        if let Some(p) = app.panes.get(id) {
            let area = ratatui::layout::Rect::new(inner.x.max(0) as u16, inner.y.max(0) as u16, inner.w.max(0) as u16, inner.h.max(0) as u16).intersection(c.buf.area);
            let at = super::pane::draw(&p.screen, area, c.buf, color(th.fg), color(th.bg));
            if focused && app.modal.is_none() && app.views.is_empty() && !app.copy_mode {
                cursor = at;
            }
            // [panes] dim_unfocused: the others' text mixed toward the background
            if !focused && app.cfg.panes.dim_unfocused > 0.0 && rs.len() > 1 {
                dim(c.buf, area, color(th.bg), app.cfg.panes.dim_unfocused.min(0.8));
            }
        }
        c.hit(inner, Hit::Pane(id.clone()));
    }
    cursor
}

// modisa's own pane title: its @name (or title), then its agent: the state's mark, the agent's logo (or its name), what
// it's doing (shimmering while it works, in the focused pane), how long for, and what it has printed lately.
fn pane_title(app: &App, i: &PaneInfo, focused: bool) -> Vec<(String, Style)> {
    let th = &app.th;
    let name = i.name.as_ref().map(|n| format!("@{n}")).unwrap_or(i.title.clone());
    let st = |c: &str| Style::new().fg(color(c));
    let mut out = vec![(name, if focused { st(th.fg).add_modifier(Modifier::BOLD) } else { st(th.dim) })];
    let Some(ag) = &i.agent else { return out };
    let col = app.state_color(ag.state);
    if app.cfg.indicators.pane {
        let icon = if ag.state == AgentState::Working { look::working_icon(app) } else { app.icon(ag.state) };
        out.push((format!(" {icon}"), st(col)));
    }
    if app.cfg.pane_labels.agent {
        let mark = agent_mark(th, &ag.harness, app.logos, app.cell_ems());
        if mark.cells == 3 {
            out.push((format!(" {}  ", mark.glyph), st(&mark.color))); // a logo: two cells wide over one
        } else {
            out.push((format!(" {} ", ag.harness), st(th.dim)));
        }
        let word = if ag.state == AgentState::Blocked { "needs you" } else { ag.state.as_str() };
        if ag.state == AgentState::Working && focused {
            out.extend(look::shimmer(app, word, col, th.fg).into_iter().map(|(ch, c)| (ch, st(&c))));
        } else {
            out.push((word.to_string(), st(col).add_modifier(if ag.state == AgentState::Blocked { Modifier::BOLD } else { Modifier::empty() })));
        }
        let faint = look::faint(th);
        if let Some(t) = look::since(app, &i.id) {
            out.push((format!("  {t}"), st(&faint)));
        }
    }
    out
}

// Text in `area` mixed `by` toward `to`; a cell coloured from the terminal's palette (not RGB) is drawn dim instead.
fn dim(buf: &mut Buffer, area: ratatui::layout::Rect, to: ratatui::style::Color, by: f64) {
    use ratatui::style::Color;
    let Color::Rgb(tr, tg, tb) = to else { return };
    let blend = |a: u8, b: u8| (a as f64 + (b as f64 - a as f64) * by).round() as u8;
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let Some(cell) = buf.cell_mut((x, y)) else { continue };
            match cell.fg {
                Color::Rgb(r, g, b) => {
                    cell.fg = Color::Rgb(blend(r, tr), blend(g, tg), blend(b, tb));
                }
                _ => {
                    cell.modifier |= Modifier::DIM;
                }
            }
        }
    }
}

// ---------- the top row: the space button, the current space's tabs, and new-tab / overflow controls ----------

// A tab's name: the one it was given, else its focused pane's @name or title
pub fn tab_label(app: &App, t: &TabView) -> String {
    if let Some(n) = &t.name {
        return n.clone();
    }
    match app.info(&t.focused) {
        Some(p) => p.name.as_ref().map(|n| format!("@{n}")).unwrap_or(p.title.clone()),
        None => "shell".into(),
    }
}

fn tabs(app: &App, c: &mut Canvas) {
    let th = &app.th;
    let ctx = slots::ctx(app);
    let Some(y) = app.metrics().tabs_y else { return }; // [tabs] position = "hidden"
    c.fill(Rect { x: 0, y, w: c.w, h: 1 }, th.bar);
    let ws = app.ws();
    let hover = (th.fg, th.border);
    let faint = look::faint(th);
    // The workspace is a compact pill, not a column aligned with the sidebar. Keep its arrow visible even when a long
    // workspace name needs truncation. Plugins' pieces sit around the name, or one in its place.
    let space = slots::around(app, "space", |p| p.space.as_deref() == Some(ws.name.as_str()));
    let most = if space.before.is_empty() && space.after.is_empty() && space.instead.is_none() { 24 } else { 40 };
    let brand_width = (slots::label(&ctx, &space, &ws.name, usize::MAX).width() as i32 + 6).min(most).min((c.w - 18).max(1));
    let mut brand = Strip::default();
    if brand_width >= 6 {
        brand.text(" ◈ ", Style::new().fg(color(th.focus)));
        brand.append(slots::label_spans(&ctx, &space, &[(ws.name.clone(), Style::new().add_modifier(Modifier::BOLD))], (brand_width - 6) as usize));
        brand.text(" ▸ ", Style::new().fg(color(&faint)));
    } else {
        brand.text(fit(&format!(" {}", ws.name), brand_width as usize), Style::new());
    }
    brand.pad(brand_width as usize);
    let raised = look::raised(th);
    let mut x = slots::button(app, c, 0, y, &Seg { strip: brand, fg: th.fg.into(), bg: raised.clone(), hit: Some(Hit::Action("workspace-picker")), plugin: false, pill: true }) + 1;
    // who needs you, at the right: a pulse and their name, a click away ([indicators] tab)
    let focused = &app.tab().focused;
    let calling = app.cfg.indicators.tab.then(|| app.sorted_agents().into_iter().find(|p| p.agent.as_ref().is_some_and(|a| a.state == AgentState::Blocked) && p.id != *focused)).flatten();
    let call = calling.as_ref().map(|p| {
        let b = look::breath(app);
        let fill = mix(th.bar, th.blocked, if look::light(th) { 0.12 } else { 0.14 } + 0.06 * b);
        let mut strip = Strip::default();
        let name = p.name.as_ref().map(|n| format!("@{n}")).unwrap_or(p.title.clone());
        strip.text(format!(" {} ", app.icon(AgentState::Blocked)), Style::new().fg(color(&mix(th.blocked, th.fg, 0.25 * b))).add_modifier(Modifier::BOLD));
        strip.text(format!("{} needs you ", fit(&name, 20)), Style::new().fg(color(th.blocked)).add_modifier(Modifier::BOLD));
        Seg { strip, fg: th.blocked.into(), bg: fill, hit: Some(Hit::FocusPane(p.id.clone())), plugin: false, pill: true }
    });
    let call_w = call.as_ref().map_or(0, |s| s.width() as i32 + 1);
    let available = (c.w - x - 11 - call_w).max(1) as usize; // 4 for the active tab's ✕ and ends, 3 for +, 4 for ‹ ›
    let win = tab_window(ws.tabs.len(), ws.active, available);
    for i in win.start..win.end.min(ws.tabs.len()) {
        let t = &ws.tabs[i];
        let on = i == ws.active;
        let blocked = app.cfg.indicators.tab && tree_panes(&t.tree).iter().any(|id| app.info(id).and_then(|p| p.agent.as_ref()).is_some_and(|a| a.state == AgentState::Blocked));
        let unread = app.cfg.notify.unread && t.unread && !on && !blocked; // [notify] unread: marked until looked at
        let badge = format!("{}{}", if blocked { format!(" {}", app.icon(AgentState::Blocked)) } else { String::new() }, if unread { " •" } else { "" });
        let zoom = if t.zoomed { " [Z]" } else { "" };
        let prefix = format!(" {}:", i + 1);
        // its name, with plugins' pieces around it or a plugin's instead
        let pieces = slots::around(app, "tab", |p| p.tab.as_deref() == Some(t.id.as_str()));
        let active = app.roles.get("tab.active").copied().unwrap_or_default();
        let fg = if on { active.fg.unwrap_or(th.fg) } else if unread { th.fg } else { th.dim };
        let mut text = Strip::default();
        if app.cfg.tabs.format.is_empty() {
            text.text(prefix.clone(), Style::new().fg(color(&faint)));
            let name_style = if on { Style::new().add_modifier(Modifier::BOLD) } else { Style::new() };
            text.append(slots::label_spans(&ctx, &pieces, &[(tab_label(app, t), name_style)], win.width.saturating_sub(width(&prefix) + width(&badge) + width(zoom) + 2)));
            text.text(zoom, Style::new().fg(color(&faint)));
            text.text(badge, Style::new().fg(color(&mix(th.blocked, th.fg, 0.2 * look::breath(app)))).add_modifier(Modifier::BOLD));
            text.text(" ", Style::new());
        } else {
            // [tabs] format: the whole label, as the user's format draws it
            let vars = |n: &str, a: Option<&str>| vars::tab(app, t, i, n, a);
            text.text(" ", Style::new());
            for r in format(&app.cfg.tabs.format, &vars, th, Style::new()).left {
                text.text(r.text, r.style);
            }
            text.text(" ", Style::new());
        }
        // sized to the label so tabs sit side by side; window.width only caps long names. Clicking the tab you're on
        // renames it. The one you're on is a raised pill, its ✕ inside it.
        let w = (win.width as i32).min(text.width() as i32).max(0) as usize;
        text.fit(w);
        text.pad(w);
        if on {
            let fill = active.bg.map(String::from).unwrap_or_else(|| look::raised2(th));
            look::cap(app, c, x, y, true, &fill, th.bar);
            x += 1 + slots::button(app, c, x + 1, y, &Seg { strip: text, fg: fg.into(), bg: fill.clone(), hit: Some(Hit::Tab(i)), plugin: false, pill: false });
            x += c.button(x, y, "✕ ", 2, &faint, &fill, hover, Hit::Action("close-tab"));
            look::cap(app, c, x, y, false, &fill, th.bar);
            x += 1;
        } else {
            x += slots::button(app, c, x, y, &Seg { strip: text, fg: fg.into(), bg: th.bar.into(), hit: Some(Hit::Tab(i)), plugin: false, pill: false });
        }
    }
    x += c.button(x, y, " + ", 3, &faint, th.bar, hover, Hit::Action("new-tab"));
    let overflow = win.end - win.start < ws.tabs.len();
    if let Some(seg) = call {
        let at = c.w - if overflow { 5 } else { 1 } - seg.width() as i32;
        if at > x {
            slots::button(app, c, at, y, &seg);
        }
    }
    if overflow {
        c.button(c.w - 4, y, " ‹", 2, th.dim, th.bar, hover, Hit::Action("prev-tab"));
        c.button(c.w - 2, y, " ›", 2, th.dim, th.bar, hover, Hit::Action("next-tab"));
    }
}

// ---------- the sidebar: a compact navigator ----------

pub struct Side {
    pub x: i32,
    pub y: i32, // the next free row
    pub w: i32,
    pub cw: usize, // its content's width
    pub end: i32,  // the first row it can't use (the footer's)
}

fn row_bg(app: &App, c: &Canvas, r: Rect, selected: bool) -> String {
    let picked = app.roles.get("sidebar.selected").and_then(|r| r.bg).map(String::from);
    let rest = if selected { picked.unwrap_or_else(|| mix(app.th.bar, app.th.focus, 0.12)) } else { app.th.bar.to_string() };
    if c.hovered(r) { mix(&rest, app.th.fg, 0.07) } else { rest }
}

// a row of the sidebar: its tint, its hit, and where its body starts (after a two-cell gutter)
pub fn side_row(app: &App, c: &mut Canvas, s: &mut Side, height: i32, selected: bool, hit: Option<Hit>) -> (i32, i32, String) {
    let r = Rect { x: s.x, y: s.y, w: s.w - 1, h: height };
    let bg = row_bg(app, c, r, selected);
    c.fill(r, &bg);
    if let Some(h) = hit {
        c.hit(r, h);
    }
    s.y += height;
    (s.x + 2, r.y, bg)
}

fn side_action(app: &App, c: &mut Canvas, s: &mut Side, label: &str, hint: &str, hit: Hit) {
    let (left, right) = sidebar_columns(label, &format!(" {hint} "), s.cw);
    let r = Rect { x: s.x, y: s.y, w: s.w - 1, h: 1 };
    let on = c.hovered(r);
    let (x, y, bg) = side_row(app, c, s, 1, false, Some(hit));
    let w = c.text(x, y, &left, if on { app.th.fg } else { app.th.dim }, Some(&bg), Modifier::empty(), s.cw);
    c.text(x + w, y, &right, if on { app.th.fg } else { app.th.dim }, Some(&look::raised2(&app.th)), Modifier::empty(), s.cw);
}

fn sidebar(app: &App, c: &mut Canvas, elems: &mut std::collections::HashMap<String, SlotElem>) {
    let th = &app.th;
    let m = app.metrics();
    let w = m.side;
    if w == 0 {
        return;
    }
    let height = m.area.h;
    let top = m.top;
    let x0 = m.side_x;
    c.fill(Rect { x: x0, y: top, w, h: height }, th.bar);
    // its edge, on the panes' side: drag it to make the sidebar wider or narrower
    let edge = mix(th.bar, th.border, 0.5);
    let ex = if x0 == 0 { w - 1 } else { x0 };
    for y in top..top + height {
        c.text(ex, y, "│", &edge, Some(th.bar), Modifier::empty(), 1);
    }
    c.hit(Rect { x: ex, y: top, w: 1, h: height }, Hit::SidebarEdge);
    let sx = if x0 == 0 { 0 } else { x0 + 1 };
    // [sidebar] sections: their order, and which show; commands is the footer, and without it the rest have its room
    let sections = &app.cfg.sidebar.sections;
    let footer = sections.iter().any(|x| x == "commands");
    let mut s = Side { x: sx, y: top + 1, w, cw: (w - 4).max(0) as usize, end: if footer { top + height - 5 } else { top + height } };
    let agents = app.sorted_agents();
    let budget = sidebar_budget(if footer { height } else { height + 5 }, agents.len());
    for section in sections {
        match section.as_str() {
            // a plugin that replaces the AGENTS list (sidebar.agents: the server says who, [sidebar] agents included)
            // takes the list's place and its room; otherwise modisa's own list is there as ever
            "agents" => match slots::pieces(app, "sidebar.agents", |p| p.replaces).into_iter().next() {
                Some(p) => slots::agents_instead(app, c, &mut s, p, budget.lines, elems),
                None if !app.cfg.sidebar.row.is_empty() => agent_rows(app, c, &mut s, &agents),
                None => agent_list(app, c, &mut s, &agents, budget.lines),
            },
            "plugins" => slots::sections(app, c, &mut s, elems, None),
            other => {
                if let Some(name) = other.strip_prefix("plugin:") {
                    slots::sections(app, c, &mut s, elems, Some(name));
                }
            }
        }
    }
    if !footer {
        return;
    }
    // the footer: a rule, then the shortcuts; one blank row under it
    let mut f = Side { x: sx, y: top + height - 5, w, cw: s.cw, end: c.h };
    c.text(sx, f.y, &format!("  {}", "─".repeat(s.cw)), &look::hair(th), Some(th.bar), Modifier::empty(), (w - 1) as usize);
    f.y += 1;
    side_action(app, c, &mut f, "Commands", ":", Hit::Action("palette"));
    side_action(app, c, &mut f, "Keyboard guide", "?", Hit::Action("help"));
    side_action(app, c, &mut f, "Settings", "⚙", Hit::Action("settings"));
}

// modisa's own list of agents, as a git graph of the space's tabs: each tab a node on one trunk, in its own lane colour,
// its agents branching off under it (most pressing first). Click a tab to fold it: its row then counts its agents by
// state. An agent needing you lights its branch.
// [sidebar] row: the agents as a flat list, each agent its rows of the user's formats; a click focuses its pane.
fn agent_rows(app: &App, c: &mut Canvas, s: &mut Side, agents: &[PaneInfo]) {
    let th = &app.th;
    let (left, right) = sidebar_columns("AGENTS", &agents.len().to_string(), s.cw);
    c.spans(s.x + 2, s.y, &[(left, th.dim, true), (right, th.dim, false)], Some(th.bar), s.cw);
    s.y += 2;
    let formats: Vec<&String> = app.cfg.sidebar.row.iter().take(3).collect();
    for p in agents {
        if s.y + formats.len() as i32 > s.end {
            return;
        }
        let selected = app.tab().focused == p.id;
        let (x, y, bg) = side_row(app, c, s, formats.len() as i32, selected, Some(Hit::FocusPane(p.id.clone())));
        let base = Style::new().fg(color(th.fg)).bg(color(&bg));
        let vars = |n: &str, a: Option<&str>| vars::pane(app, p, n, a);
        for (k, f) in formats.iter().enumerate() {
            let out = format(f, &vars, th, base);
            let runs: Vec<Run> = out.left.into_iter().chain(out.right).map(|r| Run { click: None, ..r }).collect();
            draw_runs(c, &runs, x, y + k as i32, base, s.cw);
        }
    }
}

fn agent_list(app: &App, c: &mut Canvas, s: &mut Side, agents: &[PaneInfo], lines: usize) {
    let th = &app.th;
    let cw = s.cw;
    let faint = look::faint(th);
    let (left, right) = sidebar_columns("AGENTS", &agents.len().to_string(), cw);
    c.spans(s.x + 2, s.y, &[(left, th.dim, true), (right, &faint, false)], Some(th.bar), cw);
    meter(app, c, s.x + 2, s.y + 1, cw, agents);
    s.y += 2;
    if agents.is_empty() {
        c.text(s.x + 2, s.y, &fit("No agents here", cw), th.dim, Some(th.bar), Modifier::empty(), cw);
        s.y += 1;
        side_action(app, c, s, "Launch an agent", "+", Hit::Action("new-agent"));
        return;
    }
    let ws = app.ws();
    let tabs: Vec<(String, Vec<PaneInfo>)> = ws
        .tabs
        .iter()
        .map(|t| {
            let mine = tree_panes(&t.tree);
            (t.id.clone(), agents.iter().filter(|p| mine.contains(&p.id)).cloned().collect())
        })
        .collect();
    let ctx = slots::ctx(app);
    let graph = agent_graph(&tabs, ws.active, &app.collapsed_tabs, lines, |p: &PaneInfo| slots::agent_rows(app, &ctx, p).height());
    let focused = &app.tab().focused;
    let trunk = mix(th.border, th.dim, 0.35);
    // a lane per tab, git-graph style; the active tab's at full strength, the others' quieter
    let lanes = [th.focus, th.accent, th.done, th.working];
    let lane = |i: usize| if i == ws.active { lanes[i % 4].to_string() } else { mix(lanes[i % 4], th.bar, 0.4) };
    let label = |s: AgentState| match s {
        AgentState::Blocked => "Needs you",
        AgentState::Working => "Working",
        AgentState::Done => "Done",
        AgentState::Idle => "Idle",
    };
    let state_color = |s: AgentState| if matches!(s, AgentState::Idle) { th.dim } else { app.state_color(s) };
    let rails = app.cfg.sidebar.graph;
    for g in graph.rows {
        match g {
            GraphRow::Rail => {
                if rails {
                    c.text(s.x + 2, s.y, "│", &trunk, Some(th.bar), Modifier::empty(), 1);
                    s.y += 1;
                }
            }
            GraphRow::Tab { tab: i, node, open } => {
                let tab = &ws.tabs[i];
                let mine = &tabs[i].1;
                let on = i == ws.active;
                let (x, y, bg) = side_row(app, c, s, 1, false, Some(Hit::TabNode(tab.id.clone(), tab.focused.clone())));
                // on the right: open, how many agents and ▾; folded, a count per state, what needs you first, and ▸
                let counts: Vec<(AgentState, usize)> = [AgentState::Blocked, AgentState::Working, AgentState::Done, AgentState::Idle].into_iter().map(|st| (st, mine.iter().filter(|p| p.agent.as_ref().unwrap().state == st).count())).filter(|(_, n)| *n > 0).collect();
                let mut right: Vec<(String, &str, bool)> = vec![];
                if !mine.is_empty() {
                    if open {
                        right.push((format!("{} ▾", mine.len()), th.dim, false));
                    } else {
                        for (st, n) in &counts {
                            right.push((format!("{} {n}", app.icon(*st)), state_color(*st), false));
                            right.push(("  ".into(), th.dim, false));
                        }
                        right.push(("▸".into(), th.dim, false));
                    }
                }
                let right_width: usize = right.iter().map(|(t, _, _)| width(t)).sum();
                let number = format!("{} ", i + 1);
                let name = fit(&tab_label(app, tab), cw.saturating_sub(2 + number.len() + right_width + 1).max(1));
                let needs_you = !open && counts.iter().any(|(st, _)| *st == AgentState::Blocked);
                let name_color = if needs_you { th.blocked } else if mine.is_empty() { th.dim } else { th.fg };
                let gap = cw.saturating_sub(2 + number.len() + width(&name) + right_width).max(1);
                let lane_color = lane(i);
                let mut spans: Vec<(String, &str, bool)> = vec![(node.into(), &lane_color, false), (format!(" {number}"), th.dim, false), (name, name_color, on), (" ".repeat(gap), th.fg, false)];
                spans.extend(right);
                c.spans(x, y, &spans, Some(&bg), cw);
            }
            GraphRow::Agent { tab, agent: pane, graph } => {
                // an agent: its branch off the trunk, its mark, its name and state; under them the task its title names.
                // Plugins' lines go above and below them, by the trunk, or a plugin's lines in their place.
                let ag = pane.agent.as_ref().unwrap();
                let selected = pane.id == *focused;
                let col = state_color(ag.state);
                let branch = if ag.state == AgentState::Blocked { th.blocked.to_string() } else { lane(tab) };
                let mark = agent_mark(th, &ag.harness, app.logos, app.cell_ems());
                let wd = cw.saturating_sub(2 + mark.cells);
                let name = pane.name.as_ref().map(|n| format!("@{n}")).unwrap_or(ag.harness.clone());
                // the state's mark on the right (a spinner while it works), and when there's room what it printed in
                // the last seconds after it; under them the task, and its state and how long for
                let icon = if !app.cfg.indicators.sidebar { "" } else if ag.state == AgentState::Working { look::working_icon(app) } else { app.icon(ag.state) };
                let spark = (wd >= width(&name) + 12 && !icon.is_empty() && look::active(app, &pane.id, 6)).then(|| {
                    let mut act = look::spark(app, &pane.id, 6);
                    act.insert(0, ' ');
                    act
                });
                let (nl, nr) = sidebar_columns(&name, &format!("{icon}{}", spark.as_deref().unwrap_or("")), wd);
                let nr_icon: String = nr.chars().take(icon.chars().count()).collect();
                let nr_spark: String = nr.chars().skip(icon.chars().count()).collect();
                let task = agent_task(pane.terminal_title.as_deref().or(Some(&pane.title)), &[pane.name.as_deref(), Some(&ag.harness)]);
                let meta = if !task.is_empty() { task } else if pane.name.is_some() { ag.harness.clone() } else { String::new() };
                let when = look::since(app, &pane.id).filter(|t| wd >= width(&meta) + width(label(ag.state)) + width(t) + 3);
                let state_text = match &when {
                    Some(t) => format!("{} {t}", label(ag.state)),
                    None => label(ag.state).to_string(),
                };
                let (ml, mr) = sidebar_columns(&meta, &state_text, wd);
                let rows = slots::agent_rows(app, &ctx, &pane);
                let blocked_row = ag.state == AgentState::Blocked && !selected;
                let (x, mut y, bg) = side_row(app, c, s, rows.height() as i32, selected, Some(Hit::FocusPane(pane.id.clone())));
                let bg = if blocked_row {
                    let tint = mix(&bg, th.blocked, if look::light(th) { 0.06 } else { 0.08 } + 0.05 * look::breath(app));
                    c.fill(Rect { x: s.x, y, w: s.w - 1, h: rows.height() as i32 }, &tint);
                    tint
                } else {
                    bg
                };
                let base = if matches!(ag.state, AgentState::Done | AgentState::Idle) { th.dim } else { th.fg };
                let [g1, g2] = if rails { graph } else { ["  ", "  "] };
                let mut g1c = g1.chars();
                let (a, b) = (g1c.next().unwrap_or(' ').to_string(), g1c.next().unwrap_or(' ').to_string());
                let gap = " ".repeat(mark.cells - 1);
                let (top, bottom) = match mark.halves {
                    Some((t, b)) => (t.to_string(), b.to_string()),
                    None => (mark.glyph.clone(), " ".to_string()),
                };
                let spark_color = if ag.state == AgentState::Working { mix(col, &bg, 0.2) } else { faint.clone() };
                let style = Style::new().fg(color(base)).bg(color(&bg));
                let under = 2 + mark.cells; // a plugin's line beside an agent's starts under its name
                for (l, p) in &rows.before {
                    c.text(x, y, if rails { "│ " } else { "  " }, &trunk, Some(&bg), Modifier::empty(), 2);
                    slots::draw_line(c, x + under as i32, y, l, p, cw.saturating_sub(under), style);
                    y += 1;
                }
                match &rows.instead {
                    Some(lines) => {
                        for (k, (l, p)) in lines.iter().enumerate() {
                            let g = if k == 0 { vec![(a.clone(), trunk.as_str(), false), (b.clone(), branch.as_str(), false)] } else { vec![(g2.to_string(), trunk.as_str(), false)] };
                            c.spans(x, y, &g, Some(&bg), 2);
                            slots::draw_line(c, x + 2, y, l, p, cw.saturating_sub(2), style);
                            y += 1;
                        }
                    }
                    None => {
                        c.spans(x, y, &[(a, &trunk, false), (b, &branch, false), (top, &mark.color, false), (gap.clone(), base, false), (nl, base, selected || blocked_row), (nr_icon, col, false), (nr_spark, &spark_color, false)], Some(&bg), cw);
                        let at = c.spans(x, y + 1, &[(g2.to_string(), &trunk, false), (bottom, &mark.color, false), (gap, base, false), (ml, th.dim, false)], Some(&bg), cw);
                        // its state: shimmering while it works; how long for after it, quieter
                        let word: String = mr.chars().take(label(ag.state).chars().count()).collect();
                        let rest: String = mr.chars().skip(word.chars().count()).collect();
                        let room = cw.saturating_sub((at - x) as usize);
                        let shine: Vec<(String, String)> = if ag.state == AgentState::Working { look::shimmer(app, &word, col, th.fg) } else { vec![(word.clone(), col.to_string())] };
                        let mut spans: Vec<(String, &str, bool)> = shine.iter().map(|(ch, cl)| (ch.clone(), cl.as_str(), ag.state == AgentState::Blocked)).collect();
                        spans.push((rest, &faint, false));
                        c.spans(at, y + 1, &spans, Some(&bg), room);
                        y += 2;
                    }
                }
                for (l, p) in &rows.after {
                    c.text(x, y, g2, &trunk, Some(&bg), Modifier::empty(), 2);
                    slots::draw_line(c, x + under as i32, y, l, p, cw.saturating_sub(under), style);
                    y += 1;
                }
            }
        }
    }
    if graph.hidden > 0 {
        side_action(app, c, s, &format!("{} more agents", graph.hidden), "›", Hit::Action("pane-picker"));
    }
}

// The flock at a glance, under the AGENTS heading: a bar in each state's colour, as long as its share of the agents
// (what needs you first, pulsing).
fn meter(app: &App, c: &mut Canvas, x: i32, y: i32, w: usize, agents: &[PaneInfo]) {
    let th = &app.th;
    if agents.is_empty() || w < 8 {
        return;
    }
    let states = [AgentState::Blocked, AgentState::Working, AgentState::Done, AgentState::Idle];
    let counts: Vec<(AgentState, usize)> = states.iter().map(|&st| (st, agents.iter().filter(|p| p.agent.as_ref().is_some_and(|a| a.state == st)).count())).filter(|(_, n)| *n > 0).collect();
    let room = w.saturating_sub(counts.len() - 1); // a cell between bars
    let mut at = x;
    let mut left = room;
    for (k, (st, n)) in counts.iter().enumerate() {
        let len = if k + 1 == counts.len() { left } else { ((room as f64 * *n as f64 / agents.len() as f64).round() as usize).clamp(1, left) };
        left -= len;
        let col = if *st == AgentState::Blocked { mix(th.blocked, th.bar, 0.25 * (1.0 - look::breath(app))) } else { mix(app.state_color(*st), th.bar, if *st == AgentState::Idle { 0.45 } else { 0.1 }) };
        c.text(at, y, &"━".repeat(len), &col, Some(th.bar), Modifier::empty(), len);
        at += len as i32 + 1;
    }
}

// ---------- the bottom row ----------

// The bottom row: sidebar toggle, new agent and agent counts on the left; pane count, theme and the active space's git
// on the right. [status] and [git] choose which of them show. Plugins' pieces go after modisa's left-hand segments
// (status.left), before its right-hand ones (status.right), and around or instead of its counts, pane count, theme and
// git (status.agents, status.panes, status.theme, status.git).
fn status(app: &App, c: &mut Canvas) {
    let th = &app.th;
    let ctx = slots::ctx(app);
    let y = c.h - 1;
    c.fill(Rect { x: 0, y, w: c.w, h: 1 }, th.bar);
    if !app.cfg.status.left.is_empty() || !app.cfg.status.right.is_empty() {
        return status_formatted(app, c, y);
    }
    let count = tree_panes(&app.tab().tree).len();
    let agents = app.sorted_agents();
    let blocked = agents.iter().filter(|p| p.agent.as_ref().unwrap().state == AgentState::Blocked).count();
    let running = agents.iter().filter(|p| p.agent.as_ref().unwrap().state == AgentState::Working).count();
    let side = app.side_width() > 0;
    let own = |text: &str, fg: &str, bg: &str, hit: Hit| Seg::own(text, fg, bg, Some(hit));
    let (faint, raised) = (look::faint(th), look::raised(th));
    let mut left = vec![
        own(if side { "◧ sidebar " } else { "◨ sidebar " }, th.fg, &raised, Hit::Action("toggle-sidebar")).pill(),
        own("  + agent ", th.dim, th.bar, Hit::Action("new-agent")),
    ];
    if let Some(mode) = &app.mode {
        left.insert(0, Seg::own(&format!(" {mode} "), th.bg, th.accent, None).pill()); // the key mode it's in, until escape
    }
    if app.prefix_armed && app.mode.is_none() {
        left.insert(0, Seg::own(&format!(" {} ", prefix_label(&app.cfg.prefix)), th.bg, th.focus, None).pill()); // waiting for the key after it
    }
    if c.w >= 100 && app.cfg.status.agents {
        let b = look::breath(app);
        let need = if blocked > 0 { mix(th.blocked, th.fg, 0.2 * b) } else { faint.clone() };
        let icon = if running > 0 { look::working_icon(app) } else { app.icon(AgentState::Working) };
        let counts = vec![
            own(&format!("  {icon} {running} working "), if running > 0 { th.working } else { &faint }, th.bar, Hit::Action("working-agents")),
            own(&format!("  {} {blocked} need you ", app.icon(AgentState::Blocked)), &need, th.bar, Hit::Action("blocked-agents")),
        ];
        left.extend(slots::wrap(app, &ctx, "status.agents", counts));
    }
    left.extend(slots::segs(app, &ctx, "status.left"));
    // right: plugins' pieces, a newer release, the pane count, the theme, then the active space's git
    let view = app.view.as_ref().unwrap();
    let mut right = slots::segs(app, &ctx, "status.right");
    if view.paused && c.w >= 120 {
        right.push(own(" PAUSED ", th.warn, th.bar, Hit::Action("toggle-messaging")));
    }
    if let Some(u) = &app.update {
        right.push(own(&format!(" ↑ {} ", u.version), th.bg, th.focus, Hit::Action("update-modisa")).pill());
        right.push(Seg::own(" ", th.fg, th.bar, None));
    }
    if c.w >= 50 && app.cfg.status.panes {
        let panes = own(&format!(" {count} {} ", if count == 1 { "pane" } else { "panes" }), &faint, th.bar, Hit::Action("pane-picker"));
        right.extend(slots::wrap(app, &ctx, "status.panes", vec![panes]));
    }
    if c.w >= 60 && app.cfg.status.theme {
        let theme = own(&format!(" ◐ {} ", app.cfg.theme), &faint, th.bar, Hit::Action("theme-picker"));
        right.extend(slots::wrap(app, &ctx, "status.theme", vec![theme]));
    }
    // Where the repository stands: its name, the branch (in the done colour when clean and in step with its upstream),
    // ↑ commits to push, ↓ commits to pull, ● files changed. Plugins' pieces there show outside a repository too.
    if app.cfg.git.status && c.w >= 60 {
        let git = app.ws().git.as_ref().map(|g| {
            let clean = g.changes == 0 && g.ahead.unwrap_or(0) == 0 && g.behind.unwrap_or(0) == 0 && g.ahead.is_some();
            let mut git = Strip::default();
            if app.cfg.git.repo {
                git.text(fit(&g.repo, 24), Style::new().fg(color(th.fg)).add_modifier(Modifier::BOLD));
            }
            let mut put = |text: String, fg: &str| git.text(text, Style::new().fg(color(fg)));
            put(format!(" ⎇ {}", fit(&g.branch, 24)), if clean { th.done } else { th.fg });
            if let Some(a) = g.ahead.filter(|a| *a > 0 && app.cfg.git.counts) {
                put(format!(" ↑{a}"), th.working);
            }
            if let Some(b) = g.behind.filter(|b| *b > 0 && app.cfg.git.counts) {
                put(format!(" ↓{b}"), th.warn);
            }
            if g.changes > 0 && app.cfg.git.changes {
                put(format!(" ●{}", g.changes), th.accent);
            }
            Seg { strip: git, fg: th.dim.into(), bg: raised.clone(), hit: None, plugin: false, pill: true }
        });
        right.extend(slots::wrap(app, &ctx, "status.git", git.into_iter().collect()));
    }
    slots::row(app, c, y, left, right);
}

// [status] left and right: the row as the user's formats say, plugins' status pieces after the left and before the right
fn status_formatted(app: &App, c: &mut Canvas, y: i32) {
    let th = &app.th;
    let ctx = slots::ctx(app);
    let base = Style::new().fg(color(th.fg)).bg(color(th.bar));
    let vars = |n: &str, a: Option<&str>| vars::common(app, n, a);
    let left = format(&app.cfg.status.left, &vars, th, base);
    let right = format(&app.cfg.status.right, &vars, th, base);
    let mut x = draw_runs(c, &left.left, 0, y, base, c.w.max(0) as usize);
    for s in slots::segs(app, &ctx, "status.left") {
        x += slots::button(app, c, x, y, &s);
    }
    let rights: Vec<&Run> = left.right.iter().chain(&right.left).chain(&right.right).collect();
    let w: i32 = rights.iter().map(|r| width(&r.text) as i32).sum();
    let mut segs_w = 0;
    let pieces = slots::segs(app, &ctx, "status.right");
    for s in &pieces {
        segs_w += s.strip.width() as i32;
    }
    let mut at = (c.w - w - segs_w).max(x);
    for s in &pieces {
        at += slots::button(app, c, at, y, s);
    }
    let owned: Vec<Run> = rights.into_iter().cloned().collect();
    draw_runs(c, &owned, at, y, base, (c.w - at).max(0) as usize);
}

// A format's runs from x, each part with a click its own hit; how wide they came out.
fn draw_runs(c: &mut Canvas, runs: &[Run], x: i32, y: i32, base: Style, max: usize) -> i32 {
    let mut at = x;
    let mut left = max as i32;
    for r in runs {
        if left <= 0 {
            break;
        }
        let text = fit(&r.text, left as usize);
        let w = slots::put(c, at, y, &Line::from(Span::styled(text, r.style)), left as usize, base);
        if let Some(a) = &r.click {
            c.hit(Rect { x: at, y, w, h: 1 }, Hit::Named(a.clone()));
        }
        at += w;
        left -= w;
    }
    at - x
}

// The terminal's title as [window] title says (OSC 2), for the focused pane; None without one.
pub fn window_title(app: &App) -> Option<String> {
    if app.cfg.window.title.is_empty() || !app.ready() {
        return None;
    }
    let focused = app.info(&app.tab().focused).cloned();
    let vars = |n: &str, a: Option<&str>| match &focused {
        Some(p) => vars::pane(app, p, n, a),
        None => vars::common(app, n, a),
    };
    Some(crate::core::text::clean_text(&format(&app.cfg.window.title, &vars, &app.th, Style::new()).text(), 200))
}

// ---------- toasts: a stack of cards at the top right, the newest on top ----------

// A card: its text (a plugin's lines, when it sent rich text), under its title, and its buttons along the bottom. It
// slides in from the right, floats on a shadow, and the line along its foot runs out as its time does.
fn toasts(app: &App, c: &mut Canvas) {
    let th = &app.th;
    let (panel, line) = (look::panel(th), look::panel_line(th));
    let mut top = app.metrics().top + 1;
    for t in &app.toasts {
        let room = (c.w - 8).max(1) as usize;
        let body: Vec<Line> = if t.lines.is_empty() { vec![Line::raw(fit(&t.text, room))] } else { t.lines.iter().map(|l| slots::fit_line(l.clone(), room)).collect() };
        let buttons: Vec<(String, &Hit)> = t.buttons.iter().map(|(b, h)| (format!(" {} ", fit(b, 20)), h)).collect();
        let row = buttons.iter().map(|(b, _)| width(b) + 1).sum::<usize>().saturating_sub(1);
        let w = (body.iter().map(slots::line_width).max().unwrap_or(0).max(row.min(room)).max(t.title.as_ref().map_or(0, |s| width(s) + 4)) + 4) as i32;
        let h = body.len() as i32 + 2 + !buttons.is_empty() as i32;
        let born = t.born.unwrap_or_else(std::time::Instant::now);
        let slide = ((1.0 - look::eased(app, born, look::SLIDE)) * (w + 3) as f64).round() as i32;
        let r = Rect { x: c.w - 2 - w + slide, y: top, w, h };
        c.fill(r, &panel);
        c.border(r, BorderStyle::Rounded, &line, Some(&panel), None);
        // its kind's colour as a dot on the border, its title (a plugin's name) after it
        let mut tx = r.x + 2;
        tx += c.text(tx, r.y, " ● ", t.color, Some(&panel), Modifier::empty(), 3);
        if let Some(title) = &t.title {
            c.text(tx, r.y, &format!("{} ", fit(title, (w - 8).max(1) as usize)), th.fg, Some(&panel), Modifier::BOLD, (w - 6).max(0) as usize);
        }
        for (k, l) in body.iter().enumerate() {
            slots::put(c, r.x + 2, r.y + 1 + k as i32, l, (w - 4) as usize, Style::new().fg(color(th.fg)).bg(color(&panel)));
        }
        c.hit(r, Hit::Inert);
        let mut x = r.x + 2;
        for (b, hit) in buttons {
            let cell = Rect { x, y: r.y + h - 2, w: width(&b) as i32, h: 1 };
            let bg = if c.hovered(cell) { th.border.to_string() } else { look::raised2(th) };
            let drawn = c.text(x, cell.y, &b, th.fg, Some(&bg), Modifier::empty(), (r.x + w - 2 - x).max(0) as usize);
            if drawn > 0 {
                c.hit(Rect { w: drawn, ..cell }, hit.clone());
            }
            x += drawn + 1;
        }
        // what's left of its time, along its foot
        if look::on(app) && t.ms > 0 {
            let left = 1.0 - (born.elapsed().as_millis() as f64 / t.ms as f64).min(1.0);
            let n = ((w - 4) as f64 * left).round() as usize;
            c.text(r.x + 2, r.y + h - 1, &"─".repeat(n), &mix(t.color, &line, 0.35), Some(&panel), Modifier::empty(), n);
        }
        look::shadow(c, r, th);
        top += h + 1;
    }
}

// The prefix as a keycap: C-b is ⌃B
pub fn prefix_label(p: &str) -> String {
    match p.strip_prefix("C-") {
        Some(k) => format!("⌃{}", k.to_uppercase()),
        None => p.to_string(),
    }
}

// ---------- the keys after the prefix ----------

// What can come after the prefix, grouped the way the keyboard guide groups it, from the keys as bound now (yours, from
// [keys], included): it shows when the prefix is pressed and nothing follows for a moment.
const WHICH: [(&str, &[&str]); 3] = [
    ("PANES", &["split-right", "split-down", "focus", "resize", "zoom", "close-pane", "pane-menu", "pane-picker", "rename-pane", "copy-mode", "search"]),
    ("TABS · SPACES", &["new-tab", "next-tab", "prev-tab", "rename-tab", "close-tab", "workspace-picker", "new-workspace", "rename-workspace", "delete-workspace", "toggle-sidebar", "detach"]),
    ("AGENTS · MODISA", &["new-agent", "agent", "palette", "help", "settings", "theme-picker", "plugins", "send-message"]),
];

fn which_key(app: &App, c: &mut Canvas) {
    let th = &app.th;
    let keys_of = |pred: &dyn Fn(&str) -> bool| -> Vec<String> { app.bindings.iter().filter(|(_, a)| pred(a)).map(|(k, _)| k.clone()).collect() };
    let cols: Vec<(&str, Vec<(String, String)>)> = WHICH
        .iter()
        .map(|(head, ids)| {
            let rows = ids
                .iter()
                .filter_map(|id| {
                    let (keys, what) = match *id {
                        "focus" => (keys_of(&|a| a.starts_with("focus-")), "focus".to_string()),
                        "resize" => (keys_of(&|a| a.starts_with("resize-")), "resize".to_string()),
                        "agent" => {
                            let k = keys_of(&|a| a.starts_with("agent-"));
                            (if k.len() > 2 { vec![format!("{}–{}", k[0], k[k.len() - 1])] } else { k }, "jump to agent".to_string())
                        }
                        a => (keys_of(&|b| b == a), super::actions::label(a).to_lowercase()),
                    };
                    (!keys.is_empty()).then(|| (keys.join(" "), what))
                })
                .take(9)
                .collect();
            (*head, rows)
        })
        .collect();
    let (kw, lw) = (9usize, 22usize);
    let col_w = (kw + 2 + lw) as i32;
    let tall = cols.iter().map(|(_, r)| r.len()).max().unwrap_or(0) as i32;
    let w = (col_w * 3 + 4 + 4).min(c.w - 2);
    let h = tall + 6;
    let bottom = c.h - 1 - (app.cfg.tabs.position == "bottom") as i32;
    let r = Rect { x: (c.w - w - 2).max(0), y: (bottom - h - 1).max(0), w, h };
    let (panel, line) = (look::panel(th), look::panel_line(th));
    c.fill(r, &panel);
    let ring = mix(&line, th.focus, 0.35 * look::eased(app, app.prefix_at + look::WHICH_KEY, look::FADE));
    c.border(r, BorderStyle::Rounded, &ring, Some(&panel), None);
    c.hit(r, Hit::Inert);
    let faint = look::faint(th);
    let x0 = r.x + 3;
    c.spans(x0, r.y + 1, &[(prefix_label(&app.cfg.prefix), th.focus, true), ("  then".into(), th.dim, false)], Some(&panel), (w - 6).max(0) as usize);
    let tail = "every key: ? · esc cancels";
    if w > 60 {
        c.text(r.x + w - 3 - width(tail) as i32, r.y + 1, tail, &faint, Some(&panel), Modifier::empty(), width(tail));
    }
    for (k, (head, rows)) in cols.iter().enumerate() {
        let cx = x0 + k as i32 * (col_w + 2);
        if cx + col_w > r.x + w {
            break;
        }
        c.text(cx, r.y + 3, head, &faint, Some(&panel), Modifier::BOLD, col_w as usize);
        for (j, (keys, what)) in rows.iter().enumerate() {
            let ky = r.y + 4 + j as i32;
            let keys = fit(keys, kw);
            c.text(cx + (kw - width(&keys)) as i32, ky, &keys, th.focus, Some(&panel), Modifier::BOLD, kw);
            c.text(cx + kw as i32 + 2, ky, &fit(what, lw), th.fg, Some(&panel), Modifier::empty(), lw);
        }
    }
    look::shadow(c, r, th);
}
