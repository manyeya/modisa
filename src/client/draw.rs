// Draw what the server last sent: pane terminals at their layout positions, then the chrome (the tab bar, the sidebar,
// the status row), a dialog over it all, and the toasts over that. Everything clickable is recorded in `app.hits`.
use ratatui::buffer::Buffer;
use ratatui::layout::Position;
use ratatui::style::{Modifier, Style};
use ratatui::Frame;
use serde_json::json;

use super::design::{agent_graph, agent_mark, agent_task, color, fit, mix, sidebar_budget, sidebar_columns, tab_window, GraphRow};
use super::plugin_ui::{list_of, plugin_ui, span_text};
use super::{modals, tone_color, App};
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
    // a plugin's status segment or sidebar row: focus the pane's process it names, or run its action (plugin_ui.rs)
    Plugin { plugin: String, run: String, action: Option<String>, pane: Option<String>, instance: Option<String> },
    PluginFold(String), // a plugin's sidebar heading: fold its section
    View { view: String, key: String, part: i32 }, // an element of a plugin's view; part: a list's row, or what it is (views/)
}

impl Hit {
    // what gets the hand pointer
    pub fn clickable(&self) -> bool {
        !matches!(self, Hit::Pane(_) | Hit::Veil | Hit::Inert | Hit::SidebarEdge | Hit::Border(_)) && !matches!(self, Hit::View { part, .. } if *part == super::views::build::SCROLLS)
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
        let [tl, tr, bl, br, hz, vt] = match style {
            BorderStyle::Single => ["┌", "┐", "└", "┘", "─", "│"],
            BorderStyle::Rounded => ["╭", "╮", "╰", "╯", "─", "│"],
            BorderStyle::Double => ["╔", "╗", "╚", "╝", "═", "║"],
            BorderStyle::Heavy => ["┏", "┓", "┗", "┛", "━", "┃"],
        };
        let n = (r.w - 2) as usize;
        let line = |l: &str, rr: &str| format!("{l}{}{rr}", hz.repeat(n));
        self.text(r.x, r.y, &line(tl, tr), fg, bg, Modifier::empty(), r.w as usize);
        self.text(r.x, r.y + r.h - 1, &line(bl, br), fg, bg, Modifier::empty(), r.w as usize);
        for y in r.y + 1..r.y + r.h - 1 {
            self.text(r.x, y, vt, fg, bg, Modifier::empty(), 1);
            self.text(r.x + r.w - 1, y, vt, fg, bg, Modifier::empty(), 1);
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
    app.width = size.width as i32;
    app.height = size.height as i32;
    let mut hits = std::mem::take(&mut app.hits);
    hits.clear();
    let mut c = Canvas { buf: f.buffer_mut(), hits: &mut hits, hover: app.hover, w: size.width as i32, h: size.height as i32 };
    c.fill(Rect { x: 0, y: 0, w: c.w, h: c.h }, app.th.bg);
    let mut cursor = None;
    if app.ready() {
        cursor = panes(app, &mut c);
        tabs(app, &mut c);
        sidebar(app, &mut c);
        status(app, &mut c);
    }
    // plugins' views over all that; the top one has the keyboard, unless a dialog is open over it
    if !app.views.is_empty() {
        cursor = super::views::draw(app, &mut c);
    }
    if app.modal.is_some() {
        cursor = modals::draw(app, &mut c);
    }
    toasts(app, &mut c);
    app.hits = hits;
    if let Some(at) = cursor {
        f.set_cursor_position(at);
    }
}

// ---------- panes ----------

fn panes(app: &App, c: &mut Canvas) -> Option<Position> {
    let th = &app.th;
    let a = app.area();
    let t = app.tab();
    let view = app.view.as_ref().unwrap();
    let rs = display_rects(&t.tree, a, &t.focused, t.zoomed);
    let mut cursor = None;
    for (id, r) in &rs {
        let Some(i) = app.info(id) else { continue };
        let focused = *id == t.focused;
        let st = i.agent.as_ref().map(|a| a.state);
        let border = if focused { th.focus } else if st == Some(AgentState::Blocked) { th.blocked } else { th.border };
        let agent_tag = match &i.agent {
            Some(ag) => [app.cfg.indicators.pane.then(|| app.icon(ag.state).to_string()), app.cfg.pane_labels.agent.then(|| format!("{} {}", ag.harness, ag.state.as_str()))].into_iter().flatten().map(|s| format!(" {s}")).collect::<String>(),
            None => String::new(),
        };
        let exited = if i.status == "exited" { format!(" [exited {}]", i.exit_code.map(|c| c.to_string()).unwrap_or("?".into())) } else { String::new() };
        // driven from another terminal (pane attach), at its size: typing here doesn't reach it
        let elsewhere = if i.takeover == Some(true) { " [attached elsewhere]" } else { "" };
        let badges = plugin_badges(view, i);
        let name = i.name.as_ref().map(|n| format!("@{n}")).unwrap_or(i.title.clone());
        let title = fit(&format!(" {} {name}{agent_tag}{exited}{elsewhere}{badges} ", if focused { "◆" } else { "◇" }), (r.w - 4).max(0) as usize);
        let title_color = if focused { th.focus } else { st.map(|s| app.state_color(s)).unwrap_or(th.dim) };
        c.border(*r, app.cfg.panes.border, border, Some(th.bg), Some((&title, title_color)));
        c.hit(*r, Hit::Border(id.clone()));
        let inner = Rect { x: r.x + 1, y: r.y + 1, w: r.w - 2, h: r.h - 2 };
        if let Some(p) = app.panes.get(id) {
            let area = ratatui::layout::Rect::new(inner.x.max(0) as u16, inner.y.max(0) as u16, inner.w.max(0) as u16, inner.h.max(0) as u16).intersection(c.buf.area);
            let at = super::pane::draw(&p.screen, area, c.buf, color(th.fg), color(th.bg));
            if focused && app.modal.is_none() && app.views.is_empty() && !app.copy_mode {
                cursor = at;
            }
        }
        c.hit(inner, Hit::Pane(id.clone()));
    }
    cursor
}

// plugins' badges, only for the process they were set for
fn plugin_badges(view: &crate::protocol::types::View, i: &PaneInfo) -> String {
    let mut out = String::new();
    for p in view.plugins.iter().flatten() {
        for b in p["badges"].as_array().into_iter().flatten() {
            if b["pane"] == json!(i.id) && b["instance"] == json!(i.instance) {
                out += &format!(" [{}: {}]", p["plugin"].as_str().unwrap_or(""), b["text"].as_str().unwrap_or(""));
            }
        }
    }
    out
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
    let y = app.metrics().top - 1;
    c.fill(Rect { x: 0, y, w: c.w, h: 1 }, th.bar);
    let ws = app.ws();
    let hover = (th.fg, th.border);
    // The workspace is a compact chip, not a column aligned with the sidebar. Keep its arrow visible even when a long
    // workspace name needs truncation.
    let brand_width = (width(&ws.name) as i32 + 6).min(24).min((c.w - 16).max(1));
    let brand = if brand_width >= 6 { format!(" ◈ {} ▸ ", fit(&ws.name, (brand_width - 6) as usize)) } else { fit(&format!(" {}", ws.name), brand_width as usize) };
    let mut x = c.button(0, y, &brand, brand_width, th.bg, th.focus, hover, Hit::Action("workspace-picker"));
    x += 1;
    let available = (c.w - brand_width - 10).max(1) as usize; // 2 for the active tab's ✕
    let win = tab_window(ws.tabs.len(), ws.active, available);
    for i in win.start..win.end.min(ws.tabs.len()) {
        let t = &ws.tabs[i];
        let on = i == ws.active;
        let blocked = app.cfg.indicators.tab && tree_panes(&t.tree).iter().any(|id| app.info(id).and_then(|p| p.agent.as_ref()).is_some_and(|a| a.state == AgentState::Blocked));
        let unread = app.cfg.notify.unread && t.unread && !on && !blocked; // [notify] unread: marked until looked at
        let suffix = format!("{}{}{}", if t.zoomed { " [Z]" } else { "" }, if blocked { format!(" {}", app.icon(AgentState::Blocked)) } else { String::new() }, if unread { " •" } else { "" });
        let prefix = format!(" {}:", i + 1);
        let name = tab_label(app, t);
        let text = format!("{prefix}{}{suffix} ", fit(&name, win.width.saturating_sub(width(&prefix) + width(&suffix) + 2)));
        // sized to the label so tabs sit side by side; window.width only caps long names. Clicking the tab you're on
        // renames it.
        let w = (win.width as i32 - 1).min(width(&text) as i32);
        let fg = if blocked { th.warn } else if on || unread { th.fg } else { th.dim };
        x += c.button(x, y, &text, w, fg, if on { th.border } else { th.bar }, hover, Hit::Tab(i));
        if on {
            x += c.button(x, y, "✕ ", 2, th.dim, th.border, hover, Hit::Action("close-tab"));
        }
        x += 1;
    }
    x += c.button(x, y, " + ", 3, th.focus, th.bar, hover, Hit::Action("new-tab"));
    let _ = x;
    if win.end - win.start < ws.tabs.len() {
        c.button(c.w - 4, y, " ‹", 2, th.dim, th.bar, hover, Hit::Action("prev-tab"));
        c.button(c.w - 2, y, " ›", 2, th.dim, th.bar, hover, Hit::Action("next-tab"));
    }
}

// ---------- the sidebar: a compact navigator ----------

struct Side {
    x: i32,
    y: i32, // the next free row
    w: i32,
    cw: usize, // its content's width
    end: i32,  // the first row it can't use (the footer's)
}

fn row_bg(app: &App, c: &Canvas, r: Rect, selected: bool) -> String {
    let rest = if selected { mix(app.th.bar, app.th.focus, 0.12) } else { app.th.bar.to_string() };
    if c.hovered(r) { mix(&rest, app.th.fg, 0.07) } else { rest }
}

// a row of the sidebar: its tint, its hit, and where its body starts (after a two-cell gutter)
fn side_row(app: &App, c: &mut Canvas, s: &mut Side, height: i32, selected: bool, hit: Option<Hit>) -> (i32, i32, String) {
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
    let (left, right) = sidebar_columns(label, hint, s.cw);
    let r = Rect { x: s.x, y: s.y, w: s.w - 1, h: 1 };
    let on = c.hovered(r);
    let (x, y, bg) = side_row(app, c, s, 1, false, Some(hit));
    let w = c.text(x, y, &left, if on { app.th.fg } else { app.th.dim }, Some(&bg), Modifier::empty(), s.cw);
    c.text(x + w, y, &right, app.th.dim, Some(&bg), Modifier::empty(), s.cw);
}

fn sidebar(app: &App, c: &mut Canvas) {
    let th = &app.th;
    let m = app.metrics();
    let w = m.side;
    if w == 0 {
        return;
    }
    let height = m.area.h;
    let top = m.top;
    c.fill(Rect { x: 0, y: top, w, h: height }, th.bar);
    // its edge: drag it to make the sidebar wider or narrower
    let edge = mix(th.bar, th.border, 0.65);
    for y in top..top + height {
        c.text(w - 1, y, "│", &edge, Some(th.bar), Modifier::empty(), 1);
    }
    c.hit(Rect { x: w - 1, y: top, w: 1, h: height }, Hit::SidebarEdge);
    let mut s = Side { x: 0, y: top + 1, w, cw: (w - 4).max(0) as usize, end: top + height - 5 };
    let agents = app.sorted_agents();
    let budget = sidebar_budget(height, agents.len());
    // a plugin the config names in [sidebar] agents takes the AGENTS list's place and its room, while it shows a section;
    // otherwise (not installed, stopped, nothing to show) modisa's own list is there as ever
    let plugins = plugin_ui(app);
    let takeover = (!app.cfg.sidebar.agents.is_empty()).then(|| plugins.iter().position(|p| p["plugin"] == app.cfg.sidebar.agents.as_str() && p["sidebar"].is_object())).flatten();
    match takeover {
        Some(i) => plugin_section(app, c, &mut s, &plugins[i], budget.lines),
        None => agent_list(app, c, &mut s, &agents, budget.lines),
    }
    for (i, p) in plugins.iter().enumerate() {
        if !p["sidebar"].is_object() || Some(i) == takeover {
            continue;
        }
        s.y += 1;
        plugin_section(app, c, &mut s, p, 8);
    }
    // the footer: a rule, then the shortcuts; one blank row under it
    let mut f = Side { x: 0, y: top + height - 5, w, cw: s.cw, end: c.h };
    c.text(0, f.y, &format!("  {}", "─".repeat(s.cw)), th.border, Some(th.bar), Modifier::empty(), (w - 1) as usize);
    f.y += 1;
    side_action(app, c, &mut f, "Commands", ":", Hit::Action("palette"));
    side_action(app, c, &mut f, "Keyboard guide", "?", Hit::Action("help"));
    side_action(app, c, &mut f, "Settings", "⚙", Hit::Action("settings"));
}

// modisa's own list of agents, as a git graph of the space's tabs: each tab a node on one trunk, in its own lane colour,
// its agents branching off under it (most pressing first). Click a tab to fold it: its row then counts its agents by
// state. An agent needing you lights its branch.
fn agent_list(app: &App, c: &mut Canvas, s: &mut Side, agents: &[PaneInfo], lines: usize) {
    let th = &app.th;
    let cw = s.cw;
    let (left, right) = sidebar_columns("AGENTS", &agents.len().to_string(), cw);
    c.spans(s.x + 2, s.y, &[(left, th.dim, true), (right, th.dim, false)], Some(th.bar), cw);
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
    let graph = agent_graph(&tabs, ws.active, &app.collapsed_tabs, lines);
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
    let state_color = |s: AgentState| if s == AgentState::Blocked { th.warn } else if s == AgentState::Working { th.focus } else { th.dim };
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
                let name_color = if needs_you { th.warn } else if mine.is_empty() { th.dim } else { th.fg };
                let gap = cw.saturating_sub(2 + number.len() + width(&name) + right_width).max(1);
                let lane_color = lane(i);
                let mut spans: Vec<(String, &str, bool)> = vec![(node.into(), &lane_color, false), (format!(" {number}"), th.dim, false), (name, name_color, on), (" ".repeat(gap), th.fg, false)];
                spans.extend(right);
                c.spans(x, y, &spans, Some(&bg), cw);
            }
            GraphRow::Agent { tab, agent: pane, graph } => {
                // an agent: its branch off the trunk, its mark, its name and state; under them the task its title names
                let ag = pane.agent.as_ref().unwrap();
                let selected = pane.id == *focused;
                let col = state_color(ag.state);
                let branch = if ag.state == AgentState::Blocked { th.warn.to_string() } else { lane(tab) };
                let mark = agent_mark(th, &ag.harness, app.logos, app.cell_ems());
                let wd = cw.saturating_sub(2 + mark.cells);
                let name = pane.name.as_ref().map(|n| format!("@{n}")).unwrap_or(ag.harness.clone());
                let (nl, nr) = sidebar_columns(&name, if app.cfg.indicators.sidebar { app.icon(ag.state) } else { "" }, wd);
                let task = agent_task(pane.terminal_title.as_deref().or(Some(&pane.title)), &[pane.name.as_deref(), Some(&ag.harness)]);
                let meta = if !task.is_empty() { task } else if pane.name.is_some() { ag.harness.clone() } else { String::new() };
                let (ml, mr) = sidebar_columns(&meta, label(ag.state), wd);
                let (x, y, bg) = side_row(app, c, s, 2, selected, Some(Hit::FocusPane(pane.id.clone())));
                let base = if matches!(ag.state, AgentState::Done | AgentState::Idle) { th.dim } else { th.fg };
                let [g1, g2] = if rails { graph } else { ["  ", "  "] };
                let mut g1c = g1.chars();
                let (a, b) = (g1c.next().unwrap_or(' ').to_string(), g1c.next().unwrap_or(' ').to_string());
                let gap = " ".repeat(mark.cells - 1);
                let (top, bottom) = match mark.halves {
                    Some((t, b)) => (t.to_string(), b.to_string()),
                    None => (mark.glyph.clone(), " ".to_string()),
                };
                c.spans(x, y, &[(a, &trunk, false), (b, &branch, false), (top, &mark.color, false), (gap.clone(), base, false), (nl, base, selected), (nr, col, false)], Some(&bg), cw);
                c.spans(x, y + 1, &[(g2.to_string(), &trunk, false), (bottom, &mark.color, false), (gap, base, false), (ml, th.dim, false), (mr, col, false)], Some(&bg), cw);
            }
        }
    }
    if graph.hidden > 0 {
        side_action(app, c, s, &format!("{} more agents", graph.hidden), "›", Hit::Action("pane-picker"));
    }
}

// A plugin's section, at most `rows` of its rows: click its heading to fold it, a row to run its action or focus its pane.
// It's headed by the plugin's name on its own row, so it can't pass for one of modisa's however narrow the sidebar; the
// title the plugin chose goes under it.
fn plugin_section(app: &App, c: &mut Canvas, s: &mut Side, plugin: &serde_json::Value, rows: usize) {
    let th = &app.th;
    let name = plugin["plugin"].as_str().unwrap_or("");
    let run = plugin["run"].as_str().unwrap_or("");
    let section = &plugin["sidebar"];
    let items = list_of(section, "rows");
    let folded = app.collapsed_plugins.contains(name);
    if s.y >= s.end {
        return;
    }
    // the count is of rows that do something (focus a pane, run an action), not a plugin's headers and spacing
    let doing = items.iter().filter(|x| x["pane"].is_string() || x["action"].is_string()).count();
    let (left, right) = sidebar_columns(&format!("{} {name}", if folded { "▸" } else { "▾" }), &(if doing > 0 { doing } else { items.len() }).to_string(), s.cw);
    let (x, y, bg) = side_row(app, c, s, 1, false, Some(Hit::PluginFold(name.to_string())));
    c.spans(x, y, &[(left, th.dim, true), (right, th.dim, false)], Some(&bg), s.cw);
    if folded || s.y >= s.end {
        return;
    }
    c.text(s.x + 2, s.y, &fit(section["title"].as_str().unwrap_or(""), s.cw), th.dim, Some(th.bar), Modifier::empty(), s.cw);
    s.y += 1;
    for item in items.iter().take(rows) {
        if s.y >= s.end {
            return;
        }
        let some = |k: &str| item[k].as_str().map(String::from);
        let hit = Hit::Plugin { plugin: name.into(), run: run.into(), action: some("action"), pane: some("pane"), instance: some("instance") };
        let (x, y, bg) = side_row(app, c, s, 1, false, Some(hit));
        let tone = item["tone"].as_str().unwrap_or("fg");
        let spans = list_of(item, "spans");
        if spans.is_empty() {
            c.text(x, y, &fit(item["text"].as_str().unwrap_or(""), s.cw), tone_color(app, tone), Some(&bg), Modifier::empty(), s.cw);
        } else {
            let st = span_text(app, spans, tone, s.cw);
            let refs: Vec<(String, &str, bool)> = st.iter().map(|(t, col, b)| (t.clone(), col.as_str(), *b)).collect();
            c.spans(x, y, &refs, Some(&bg), s.cw);
        }
    }
}

// ---------- the bottom row ----------

// The bottom row: sidebar toggle, new agent and agent counts on the left; pane count, theme and the active space's git
// on the right. [status] and [git] choose which of them show.
fn status(app: &App, c: &mut Canvas) {
    let th = &app.th;
    let y = c.h - 1;
    c.fill(Rect { x: 0, y, w: c.w, h: 1 }, th.bar);
    let hover = (th.fg, th.border);
    let count = tree_panes(&app.tab().tree).len();
    let agents = app.sorted_agents();
    let blocked = agents.iter().filter(|p| p.agent.as_ref().unwrap().state == AgentState::Blocked).count();
    let running = agents.iter().filter(|p| p.agent.as_ref().unwrap().state == AgentState::Working).count();
    let mut x = 0;
    if let Some(mode) = &app.mode {
        // the key mode it's in: its keys work alone until escape
        let text = format!(" {mode} ");
        x += c.text(x, y, &text, th.bg, Some(th.accent), Modifier::BOLD, width(&text));
    }
    let side = app.side_width() > 0;
    let seg = |c: &mut Canvas, x: &mut i32, text: &str, fg: &str, bg: &str, hit: Hit| *x += c.button(*x, y, text, width(text) as i32, fg, bg, hover, hit);
    seg(c, &mut x, if side { " ◧ sidebar " } else { " ◨ sidebar " }, if side { th.bg } else { th.fg }, if side { th.focus } else { th.border }, Hit::Action("toggle-sidebar"));
    seg(c, &mut x, " + agent ", th.focus, th.border, Hit::Action("new-agent"));
    if c.w >= 100 && app.cfg.status.agents {
        seg(c, &mut x, &format!(" {} {running} working ", app.icon(AgentState::Working)), th.focus, th.bar, Hit::Action("working-agents"));
        seg(c, &mut x, &format!(" {} {blocked} need you ", app.icon(AgentState::Blocked)), if blocked > 0 { th.warn } else { th.dim }, th.bar, Hit::Action("blocked-agents"));
    }
    // plugins' segments while there's room; modisa's own come first, and the right side keeps its space
    if c.w >= 110 {
        let mut room = c.w - 100;
        for p in plugin_ui(app) {
            let (name, run) = (p["plugin"].as_str().unwrap_or(""), p["run"].as_str().unwrap_or(""));
            for st in list_of(p, "status") {
                let text = format!(" {name}: {} ", fit(st["text"].as_str().unwrap_or(""), 24)); // named, so it can't pass for modisa's own
                let w = width(&text) as i32;
                if w > room {
                    break;
                }
                room -= w;
                let hit = Hit::Plugin { plugin: name.into(), run: run.into(), action: st["action"].as_str().map(String::from), pane: None, instance: None };
                seg(c, &mut x, &text, tone_color(app, st["tone"].as_str().unwrap_or("fg")), th.bar, hit);
            }
        }
    }
    // right: a newer release, the pane count, the theme, then the active space's git
    let view = app.view.as_ref().unwrap();
    let mut right: Vec<(String, &str, &str, Hit)> = vec![];
    if view.paused && c.w >= 120 {
        right.push((" PAUSED ".into(), th.warn, th.bar, Hit::Action("toggle-messaging")));
    }
    if let Some(u) = &app.update {
        right.push((format!(" ↑ {} ", u.version), th.bg, th.warn, Hit::Action("update-modisa")));
    }
    if c.w >= 50 && app.cfg.status.panes {
        right.push((format!(" {count} {} ", if count == 1 { "pane" } else { "panes" }), th.fg, th.bar, Hit::Action("pane-picker")));
    }
    if c.w >= 60 && app.cfg.status.theme {
        right.push((format!(" ◐ {} ", app.cfg.theme), th.dim, th.bar, Hit::Action("theme-picker")));
    }
    // Where the repository stands: its name, the branch (in the done colour when clean and in step with its upstream),
    // ↑ commits to push, ↓ commits to pull, ● files changed.
    let mut git: Vec<(String, &str, bool)> = vec![];
    if let Some(g) = app.cfg.git.status.then(|| app.ws().git.as_ref()).flatten().filter(|_| c.w >= 60) {
        let clean = g.changes == 0 && g.ahead.unwrap_or(0) == 0 && g.behind.unwrap_or(0) == 0 && g.ahead.is_some();
        if app.cfg.git.repo {
            git.push((format!(" {}", fit(&g.repo, 24)), th.fg, false));
        }
        git.push((format!(" ⎇ {}", fit(&g.branch, 24)), if clean { th.done } else { th.dim }, false));
        if let Some(a) = g.ahead.filter(|a| *a > 0 && app.cfg.git.counts) {
            git.push((format!(" ↑{a}"), th.working, false));
        }
        if let Some(b) = g.behind.filter(|b| *b > 0 && app.cfg.git.counts) {
            git.push((format!(" ↓{b}"), th.warn, false));
        }
        if g.changes > 0 && app.cfg.git.changes {
            git.push((format!(" ●{}", g.changes), th.accent, false));
        }
        git.push((" ".into(), th.dim, false));
    }
    let total: i32 = right.iter().map(|(t, ..)| width(t) as i32).sum::<i32>() + git.iter().map(|(t, ..)| width(t) as i32).sum::<i32>();
    let mut rx = (c.w - total).max(x);
    for (t, fg, bg, hit) in right {
        rx += c.button(rx, y, &t, width(&t) as i32, fg, bg, hover, hit);
    }
    c.spans(rx, y, &git, Some(th.bar), (c.w - rx).max(0) as usize);
}

// ---------- toasts: a stack of cards at the top right, the newest on top ----------

fn toasts(app: &App, c: &mut Canvas) {
    let mut top = app.metrics().top;
    for t in &app.toasts {
        let content = fit(&t.text, (c.w - 6).max(1) as usize);
        let w = (width(&content).max(t.title.as_ref().map_or(0, |s| width(s) + 2)) + 4) as i32;
        let r = Rect { x: c.w - 1 - w, y: top, w, h: 3 };
        c.fill(r, app.th.bar);
        let title = t.title.as_ref().map(|s| format!(" {s} "));
        c.border(r, BorderStyle::Rounded, t.color, Some(app.th.bar), title.as_deref().map(|s| (s, t.color)));
        c.text(r.x + 2, r.y + 1, &content, app.th.fg, Some(app.th.bar), Modifier::empty(), (w - 4) as usize);
        c.hit(r, Hit::Inert);
        top += 3;
    }
}
