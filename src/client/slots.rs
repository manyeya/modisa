// Slots (UI 4, examples/plugins/CHROME.md): plugins' pieces in modisa's own chrome, from View.slots, drawn in the user's
// theme before, after or instead of what modisa draws there: the status row, tab labels, the space chip, pane borders,
// agents' rows and plugins' own sections in the sidebar, menus, the palette and toasts. Everything a piece shows is its
// plugin's: hovering it says whose, menus and the palette name it, a section is headed by it. A click runs the piece's
// action as a status segment's runs, with `ui: { slot, id, pane?, instance?, tab?, space? }` saying where.
//
// The server has decided who replaces what: a "replace" piece is drawn only when it `replaces`.
use std::collections::HashMap;
use std::time::Instant;

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::{json, Map, Value};
use unicode_segmentation::UnicodeSegmentation;

use super::design::{color, fit, mix};
use super::draw::{side_row, Canvas, Hit, Side};
use super::plugin_ui::{run_plugin_action, title_of};
use super::views::build::{self, Ctx, Draw, Elems, Fired};
use super::App;
use crate::core::layout::Rect;
use crate::core::text::{grapheme_width, width};
use crate::protocol::types::{PaneInfo, SlotPiece, SlotPosition};

// ---------- which pieces ----------

fn all(app: &App) -> &[SlotPiece] {
    app.view.as_ref().map(|v| v.slots.as_slice()).unwrap_or(&[])
}

// The pieces drawn in `slot` for what `about` picks, in their order: lower `order` first, then by plugin, then id. A
// replace that lost isn't drawn, nor a piece while the terminal is narrower than its hide_below.
pub fn pieces<'a>(app: &'a App, slot: &str, about: impl Fn(&SlotPiece) -> bool) -> Vec<&'a SlotPiece> {
    let mut ps: Vec<&SlotPiece> = all(app)
        .iter()
        .filter(|p| p.slot == slot && (p.position != SlotPosition::Replace || p.replaces) && p.hide_below.as_ref().is_none_or(|h| app.width >= h.width as i32) && about(p))
        .collect();
    ps.sort_by(|a, b| (a.order, &a.plugin, &a.id).cmp(&(b.order, &b.plugin, &b.id)));
    ps
}

// a piece about this pane's process
pub fn of_pane(p: &SlotPiece, pane: &PaneInfo) -> bool {
    p.pane.as_deref() == Some(pane.id.as_str()) && p.instance.as_deref().is_none_or(|i| i == pane.instance)
}

// What's at one place: plugins' pieces before modisa's own, the one drawn instead of it, and those after it.
pub struct Around<'a> {
    pub before: Vec<&'a SlotPiece>,
    pub instead: Option<&'a SlotPiece>,
    pub after: Vec<&'a SlotPiece>,
}

pub fn around<'a>(app: &'a App, slot: &str, about: impl Fn(&SlotPiece) -> bool) -> Around<'a> {
    let ps = pieces(app, slot, about);
    Around {
        before: ps.iter().copied().filter(|p| p.position == SlotPosition::Before).collect(),
        instead: ps.iter().copied().find(|p| p.position == SlotPosition::Replace),
        after: ps.iter().copied().filter(|p| p.position == SlotPosition::After).collect(),
    }
}

// The plugins that ask to draw `slot` instead of modisa, by name; and the one that does.
pub fn askers(app: &App, slot: &str) -> Vec<String> {
    let mut names: Vec<String> = all(app).iter().filter(|p| p.slot == slot && p.position == SlotPosition::Replace).map(|p| p.plugin.clone()).collect();
    names.sort();
    names.dedup();
    names
}

pub fn holder<'a>(app: &'a App, slot: &str) -> Option<&'a str> {
    all(app).iter().find(|p| p.slot == slot && p.replaces).map(|p| p.plugin.as_str())
}

// ---------- text ----------

// What drawing a piece's text needs: the theme, and how agents' marks (`{ "icon": id }`) are drawn in this terminal.
pub fn ctx(app: &App) -> Ctx<'_> {
    Ctx { th: &app.th, logos: app.logos, cell: app.cell_ems(), tick: super::views::tick() }
}

// A piece's text: its `lines`, or its `line`.
pub fn lines(ctx: &Ctx, p: &SlotPiece) -> Vec<Line<'static>> {
    match (&p.lines, &p.line) {
        (Some(ls), _) => ls.iter().map(|l| ctx.line(l)).collect(),
        (None, Some(l)) => vec![ctx.line(l)],
        (None, None) => vec![],
    }
}

fn first(ctx: &Ctx, p: &SlotPiece) -> Line<'static> {
    lines(ctx, p).into_iter().next().unwrap_or_default()
}

pub fn line_width(l: &Line) -> usize {
    l.spans.iter().map(|s| width(&s.content)).sum()
}

// `l` in at most `w` cells: cut between characters, with … (in the style of what it cuts) where it's cut.
pub fn fit_line(l: Line<'static>, w: usize) -> Line<'static> {
    if line_width(&l) <= w {
        return l;
    }
    let (mut out, mut used, mut last) = (vec![], 0, l.style);
    'spans: for s in &l.spans {
        last = l.style.patch(s.style);
        let mut text = String::new();
        for g in s.content.graphemes(true) {
            let gw = grapheme_width(g);
            if used + gw > w.saturating_sub(1) {
                out.push(Span::styled(text, last));
                break 'spans;
            }
            text.push_str(g);
            used += gw;
        }
        out.push(Span::styled(text, last));
    }
    if w > 0 {
        out.push(Span::styled("…", last));
    }
    Line::from(out)
}

// `l` at (x, y) over `base` (the colours of where it goes), in at most `max` cells; returns the cells it took.
pub fn put(c: &mut Canvas, x: i32, y: i32, l: &Line, max: usize, base: Style) -> i32 {
    let w = line_width(l).min(max).min((c.w - x).max(0) as usize);
    if w == 0 || x < 0 || y < 0 || y >= c.h {
        return 0;
    }
    c.buf.set_style(ratatui::layout::Rect::new(x as u16, y as u16, w as u16, 1), base);
    c.buf.set_line(x as u16, y as u16, l, w as u16);
    w as i32
}

// One line of chrome: modisa's own text and plugins' pieces, cut to fit as one. Where each piece went is kept: it takes
// clicks there (when it has an action), and says whose it is.
#[derive(Default)]
pub struct Strip<'a> {
    spans: Vec<Span<'static>>,
    marks: Vec<(usize, usize, &'a SlotPiece)>, // a piece's first cell and its width
}

impl<'a> Strip<'a> {
    pub fn text(&mut self, s: impl Into<String>, style: Style) {
        self.spans.push(Span::styled(s.into(), style));
    }
    // one of a piece's lines
    pub fn piece_line(&mut self, l: Line<'static>, p: &'a SlotPiece) {
        let (at, st) = (self.width(), l.style);
        self.spans.extend(l.spans.into_iter().map(|s| Span::styled(s.content, st.patch(s.style))));
        self.marks.push((at, self.width() - at, p));
    }
    pub fn piece(&mut self, ctx: &Ctx, p: &'a SlotPiece) {
        self.piece_line(first(ctx, p), p);
    }
    pub fn append(&mut self, other: Strip<'a>) {
        let at = self.width();
        self.marks.extend(other.marks.into_iter().map(|(x, w, p)| (at + x, w, p)));
        self.spans.extend(other.spans);
    }
    pub fn width(&self) -> usize {
        self.spans.iter().map(|s| width(&s.content)).sum()
    }
    // spaces to `w` cells
    pub fn pad(&mut self, w: usize) {
        let n = w.saturating_sub(self.width());
        if n > 0 {
            self.text(" ".repeat(n), Style::new());
        }
    }
    pub fn acts(&self) -> bool {
        self.marks.iter().any(|(_, _, p)| p.action.is_some())
    }
    pub fn fit(&mut self, w: usize) {
        if self.width() > w {
            self.spans = fit_line(Line::from(std::mem::take(&mut self.spans)), w).spans;
        }
    }
    // ponytail: what doesn't fit is cut, and left out when under three cells are left for it
    pub fn fit_or_drop(&mut self, w: usize) {
        if self.width() > w && w < 3 {
            *self = Strip::default();
        }
        self.fit(w);
    }
    // at (x, y) over `base`, in at most `max` cells: returns the cells it took
    pub fn draw(&self, c: &mut Canvas, x: i32, y: i32, base: Style, max: usize) -> i32 {
        let w = put(c, x, y, &Line::from(self.spans.clone()), max, base);
        for (at, pw, p) in &self.marks {
            let pw = (*pw as i32).min(w - *at as i32);
            if pw > 0 {
                c.hit(Rect { x: x + *at as i32, y, w: pw, h: 1 }, hit_of(p, Map::new()));
            }
        }
        w
    }
}

// modisa's own text (`own`, cut to fit) with plugins' pieces before and after it, or a plugin's instead, in `room`
// cells. The pieces come first: modisa's text keeps a few cells, however much they take.
pub fn label<'a>(ctx: &Ctx, a: &Around<'a>, own: &str, room: usize) -> Strip<'a> {
    let mut s = Strip::default();
    for p in &a.before {
        s.piece(ctx, p);
        s.text(" ", Style::new());
    }
    let after: Vec<Line> = a.after.iter().map(|p| first(ctx, p)).collect();
    match a.instead {
        Some(p) => s.piece(ctx, p),
        None => {
            let pieces = s.width() + after.iter().map(|l| line_width(l) + 1).sum::<usize>();
            s.text(fit(own, room.saturating_sub(pieces).max(room.min(4))), Style::new());
        }
    }
    for (l, p) in after.into_iter().zip(&a.after) {
        s.text(" ", Style::new());
        s.piece_line(l, p);
    }
    s.fit(room);
    s
}

// one of a piece's lines at (x, y), cut to `max` cells
pub fn draw_line(c: &mut Canvas, x: i32, y: i32, l: &Line<'static>, p: &SlotPiece, max: usize, base: Style) -> i32 {
    let mut s = Strip::default();
    s.piece_line(l.clone(), p);
    s.fit(max);
    s.draw(c, x, y, base, max)
}

// ---------- clicks ----------

// What a click on a piece does: its plugin's action, told where it was.
#[derive(Clone, Debug, PartialEq)]
pub struct SlotHit {
    pub plugin: String,
    pub run: String,
    pub action: Option<String>,
    pub ui: Value,
    pub target: Option<(String, String)>, // the pane's process it's about
    pub toast: Option<u64>,               // a toast's button: the toast goes once it's pressed
}

// `ui` for a piece's action: its slot and id, and what it's about
pub fn ui_of(p: &SlotPiece) -> Map<String, Value> {
    let mut ui = Map::from_iter([("slot".to_string(), json!(p.slot)), ("id".to_string(), json!(p.id))]);
    for (k, v) in [("pane", &p.pane), ("instance", &p.instance), ("tab", &p.tab), ("space", &p.space)] {
        if let Some(v) = v {
            ui.insert(k.into(), json!(v));
        }
    }
    ui
}

pub fn hit_of(p: &SlotPiece, more: Map<String, Value>) -> Hit {
    let mut ui = ui_of(p);
    ui.extend(more);
    Hit::Slot(Box::new(SlotHit { plugin: p.plugin.clone(), run: p.run.clone(), action: p.action.clone(), ui: Value::Object(ui), target: p.pane.clone().zip(p.instance.clone()), toast: None }))
}

// A click on a piece: its action runs as a status segment's does; a toast's button takes its toast away.
pub fn clicked(app: &mut App, h: &SlotHit) {
    if let Some(id) = h.toast {
        app.toasts.retain(|t| t.id != id);
        app.dirty();
    }
    if let Some(a) = &h.action {
        run_plugin_action(app, &h.plugin, &h.run, a, json!({}), h.target.clone(), None, Some(h.ui.clone()));
    }
}

// A menu's or the palette's entry chosen: its action, for what `more` says it was used on (the pane, tab or space).
pub fn run(app: &App, p: &SlotPiece, more: Map<String, Value>, target: Option<(String, String)>) {
    let Some(a) = &p.action else { return };
    let mut ui = ui_of(p);
    ui.extend(more);
    run_plugin_action(app, &p.plugin, &p.run, a, json!({}), target.or_else(|| p.pane.clone().zip(p.instance.clone())), None, Some(Value::Object(ui)));
}

// Whose a piece is: the pointer resting on one shows its plugin by it, and what a click does.
pub fn tooltip(app: &App, c: &mut Canvas) {
    let Some((x, y)) = app.hover else { return };
    let under = c.hits.iter().rev().find(|(r, _)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h).cloned();
    let Some((_, Hit::Slot(h))) = under else { return };
    let what = match &h.action {
        Some(a) => format!(" {} · {} ", h.plugin, title_of(app, &h.plugin, a)),
        None => format!(" {} ", h.plugin),
    };
    let text = fit(&what, c.w.max(0) as usize);
    let w = width(&text) as i32;
    let at = (x - 1).min(c.w - w).max(0);
    c.text(at, if y > 0 { y - 1 } else { y + 1 }, &text, app.th.fg, Some(&mix(app.th.bar, app.th.fg, 0.18)), Modifier::empty(), w as usize);
}

// ---------- the status row ----------

// A segment of the status row: modisa's own (a button, its counts, git) or a plugin's piece. Plugins' share the room
// modisa's leave, in order, each cut to fit.
pub struct Seg<'a> {
    pub strip: Strip<'a>,
    pub fg: String,
    pub bg: String,
    pub hit: Option<Hit>,
    pub plugin: bool,
}

impl Seg<'_> {
    pub fn own(text: &str, fg: &str, bg: &str, hit: Option<Hit>) -> Self {
        let mut strip = Strip::default();
        strip.text(text, Style::new());
        Seg { strip, fg: fg.into(), bg: bg.into(), hit, plugin: false }
    }
}

fn seg<'a>(app: &App, ctx: &Ctx, p: &'a SlotPiece) -> Seg<'a> {
    let mut strip = Strip::default();
    strip.text(" ", Style::new());
    strip.piece(ctx, p);
    strip.text(" ", Style::new());
    Seg { strip, fg: app.th.fg.into(), bg: app.th.bar.into(), hit: None, plugin: true }
}

// the pieces in `slot`, as segments
pub fn segs<'a>(app: &'a App, ctx: &Ctx, slot: &str) -> Vec<Seg<'a>> {
    pieces(app, slot, |_| true).into_iter().map(|p| seg(app, ctx, p)).collect()
}

// modisa's own segments for `slot`, with plugins' pieces before and after them, or a plugin's instead
pub fn wrap<'a>(app: &'a App, ctx: &Ctx, slot: &str, own: Vec<Seg<'a>>) -> Vec<Seg<'a>> {
    let a = around(app, slot, |_| true);
    let mut out: Vec<Seg> = a.before.iter().map(|p| seg(app, ctx, p)).collect();
    match a.instead {
        Some(p) => out.push(seg(app, ctx, p)),
        None => out.extend(own),
    }
    out.extend(a.after.iter().map(|p| seg(app, ctx, p)));
    out
}

// The row at `y`: `left` from the left edge, `right` against the right one.
pub fn row<'a>(app: &App, c: &mut Canvas, y: i32, mut left: Vec<Seg<'a>>, mut right: Vec<Seg<'a>>) {
    let own: usize = left.iter().chain(&right).filter(|s| !s.plugin).map(|s| s.strip.width()).sum();
    let mut room = (c.w.max(0) as usize).saturating_sub(own);
    for s in left.iter_mut().chain(right.iter_mut()).filter(|s| s.plugin) {
        s.strip.fit_or_drop(room);
        room -= s.strip.width();
    }
    let mut x = 0;
    for s in &left {
        x += button(app, c, x, y, s);
    }
    let total: i32 = right.iter().map(|s| s.strip.width() as i32).sum();
    let mut rx = (c.w - total).max(x);
    for s in &right {
        rx += button(app, c, rx, y, s);
    }
}

// A segment drawn; under the pointer, one that does something takes the border colour behind the text colour.
pub fn button(app: &App, c: &mut Canvas, x: i32, y: i32, s: &Seg) -> i32 {
    let w = s.strip.width() as i32;
    if w == 0 {
        return 0;
    }
    let r = Rect { x, y, w, h: 1 };
    let (fg, bg) = if (s.hit.is_some() || s.strip.acts()) && c.hovered(r) { (app.th.fg, app.th.border) } else { (s.fg.as_str(), s.bg.as_str()) };
    c.fill(r, bg);
    if let Some(h) = &s.hit {
        c.hit(r, h.clone());
    }
    s.strip.draw(c, x, y, Style::new().fg(color(fg)).bg(color(bg)), (c.w - x).max(0) as usize);
    w
}

// ---------- pane borders ----------

// Plugins' pieces on the rest of a pane's border (`r`): the top right, a cell clear of the title (`title` cells of the
// top), and the bottom left and right.
pub fn border(app: &App, ctx: &Ctx, c: &mut Canvas, r: Rect, pane: &PaneInfo, title: usize, fg: &str) {
    if r.w < 8 || r.h < 2 {
        return;
    }
    let base = Style::new().fg(color(fg)).bg(color(app.th.bg));
    let strip = |slot: &str| {
        let mut s = Strip::default();
        for p in pieces(app, slot, |p| of_pane(p, pane)) {
            s.text(" ", Style::new());
            s.piece(ctx, p);
        }
        if s.width() > 0 {
            s.text(" ", Style::new());
        }
        s
    };
    let room = (r.w - 4) as usize;
    let right = |s: &Strip| r.x + r.w - 2 - s.width() as i32;
    let mut top = strip("pane.top_right");
    top.fit_or_drop(room.saturating_sub(title + 1));
    top.draw(c, right(&top), r.y, base, room);
    let mut br = strip("pane.bottom_right");
    br.fit_or_drop(room);
    let mut bl = strip("pane.bottom_left");
    bl.fit_or_drop(room.saturating_sub(br.width() + (br.width() > 0) as usize));
    let bottom = r.y + r.h - 1;
    bl.draw(c, r.x + 2, bottom, base, room);
    br.draw(c, right(&br), bottom, base, room);
}

// ---------- the sidebar ----------

// An agent's rows in the sidebar's AGENTS list: plugins' lines before and after modisa's two, or a plugin's instead.
pub struct AgentRows<'a> {
    pub before: Vec<(Line<'static>, &'a SlotPiece)>,
    pub instead: Option<Vec<(Line<'static>, &'a SlotPiece)>>,
    pub after: Vec<(Line<'static>, &'a SlotPiece)>,
}

impl AgentRows<'_> {
    pub fn height(&self) -> usize {
        self.before.len() + self.instead.as_ref().map_or(2, Vec::len) + self.after.len()
    }
}

pub fn agent_rows<'a>(app: &'a App, ctx: &Ctx, pane: &PaneInfo) -> AgentRows<'a> {
    let a = around(app, "agent.row", |p| of_pane(p, pane));
    let of = |ps: &[&'a SlotPiece]| ps.iter().flat_map(|p| lines(ctx, p).into_iter().map(move |l| (l, *p))).collect::<Vec<_>>();
    // ponytail: three of plugins' lines at most around an agent (the ones before it first), and three instead of it
    let mut before = of(&a.before);
    before.truncate(3);
    let mut after = of(&a.after);
    after.truncate(3 - before.len());
    let instead = a.instead.map(|p| of(&[p]).into_iter().take(3).collect::<Vec<_>>()).filter(|ls| !ls.is_empty());
    AgentRows { before, instead, after }
}

// What a sidebar element keeps between frames: what the user did in it, and the last click (for a double click).
#[derive(Default)]
pub struct SlotElem {
    pub elems: Elems,
    pub clicked: Option<(String, i32, Instant)>,
}

// a piece's element, as its hits name it
fn elem_key(p: &SlotPiece) -> String {
    format!("slot:{}/{}/{}", p.slot, p.plugin, p.id)
}

// The elements still shown: the rest's state goes.
pub fn keep_elems(app: &mut App) {
    let shown: Vec<String> = all(app).iter().filter(|p| p.element.is_some()).map(elem_key).collect();
    app.slot_elems.retain(|k, _| shown.contains(k));
}

// A piece's element in `r`, in the sidebar's colours: drawn as a view's are, its interactive elements taking clicks and
// the wheel (no keyboard: the sidebar has none).
fn element(app: &App, c: &mut Canvas, p: &SlotPiece, r: Rect, elems: &mut HashMap<String, SlotElem>) {
    let Some(root) = &p.element else { return };
    let key = elem_key(p);
    let el = elems.entry(key.clone()).or_default();
    el.elems = build::reconcile(&mut el.elems, root);
    let (x, y) = (r.x.max(0), r.y.max(0));
    let area = ratatui::layout::Rect::new(x as u16, y as u16, (r.x + r.w - x).max(0) as u16, (r.y + r.h - y).max(0) as u16).intersection(c.buf.area);
    c.buf.set_style(area, Style::new().fg(color(app.th.fg)).bg(color(app.th.bar)));
    let mut d = Draw { view: &key, focus: None, active: false, cursor: None };
    build::draw(&ctx(app), c, root, &build::root_key(root), area, &mut el.elems, &mut d);
}

// A piece's title, then its lines (a click on one runs its action, with `ui.row`) or its element (`height` rows, else
// `fallback`), in at most `rows` rows.
fn body(app: &App, c: &mut Canvas, s: &mut Side, p: &SlotPiece, rows: i32, fallback: i32, elems: &mut HashMap<String, SlotElem>) {
    let th = &app.th;
    let end = s.end.min(s.y + rows);
    if let Some(t) = p.title.as_deref().filter(|t| !t.is_empty() && s.y < end) {
        c.text(s.x + 2, s.y, &fit(t, s.cw), th.dim, Some(th.bar), Modifier::empty(), s.cw);
        s.y += 1;
    }
    if p.element.is_some() {
        let h = p.height.map_or(fallback, i32::from).min(end - s.y);
        if h > 0 {
            element(app, c, p, Rect { x: s.x + 2, y: s.y, w: s.cw as i32, h }, elems);
            s.y += h;
        }
        return;
    }
    for (i, l) in lines(&ctx(app), p).into_iter().enumerate() {
        if s.y >= end {
            return;
        }
        // a row's own action, else the pane it names (while that's still its process), else the piece's action
        let row = p.lines.as_ref().and_then(|ls| ls.get(i));
        let own = row.and_then(|r| r["action"].as_str());
        let pane = row.and_then(|r| r["pane"].as_str().zip(r["instance"].as_str())).filter(|(id, inst)| app.info(id).is_some_and(|i| i.instance == *inst));
        let hit = match (own, pane) {
            (Some(a), _) => {
                let mut h = hit_of(p, Map::from_iter([("row".to_string(), json!(i))]));
                if let Hit::Slot(sh) = &mut h {
                    sh.action = Some(a.to_string());
                }
                Some(h)
            }
            (None, Some((id, _))) => Some(Hit::FocusPane(id.to_string())),
            (None, None) => p.action.is_some().then(|| hit_of(p, Map::from_iter([("row".to_string(), json!(i))]))),
        };
        let (x, y, bg) = side_row(app, c, s, 1, false, hit);
        put(c, x, y, &fit_line(l, s.cw), s.cw, Style::new().fg(color(th.fg)).bg(color(&bg)));
    }
}

// Plugins' own sidebar sections, under each plugin's name (a click on it folds them): each its title, then its lines or
// its element.
pub fn sections(app: &App, c: &mut Canvas, s: &mut Side, elems: &mut HashMap<String, SlotElem>, only: Option<&str>) {
    let ps = pieces(app, "sidebar", |p| only.is_none_or(|o| p.plugin == o));
    let mut names: Vec<&str> = vec![];
    for p in &ps {
        if !names.contains(&p.plugin.as_str()) {
            names.push(&p.plugin);
        }
    }
    for name in names {
        if s.y + 1 >= s.end {
            return;
        }
        s.y += 1;
        let folded = app.collapsed_plugins.contains(name);
        let (x, y, bg) = side_row(app, c, s, 1, false, Some(Hit::PluginFold(name.to_string())));
        c.text(x, y, &fit(&format!("{} {name}", if folded { "▸" } else { "▾" }), s.cw), app.th.dim, Some(&bg), Modifier::BOLD, s.cw);
        if !folded {
            for p in ps.iter().filter(|p| p.plugin == name) {
                body(app, c, s, p, 30, 8, elems);
            }
        }
    }
}

// The AGENTS list's place, a plugin's: headed with the plugin's name, as its own sections are (so it can't pass for
// modisa's list), then its lines or its element, in `rows`.
pub fn agents_instead(app: &App, c: &mut Canvas, s: &mut Side, p: &SlotPiece, rows: usize, elems: &mut HashMap<String, SlotElem>) {
    let th = &app.th;
    c.text(s.x + 2, s.y, &fit(&format!("▾ {}", p.plugin), s.cw), th.dim, Some(th.bar), Modifier::BOLD, s.cw);
    s.y += 2;
    body(app, c, s, p, rows as i32, rows as i32, elems);
}

// A click on part of a sidebar element: what a view's element does with it, its actions telling `ui.slot` and `ui.piece`.
pub fn element_click(app: &mut App, view: &str, key: &str, part: i32) {
    let Some(p) = all(app).iter().find(|p| elem_key(p) == view).cloned() else { return };
    let Some(n) = p.element.as_ref().and_then(|root| build::node_at(root, key)).cloned() else { return };
    let el = app.slot_elems.entry(view.to_string()).or_default();
    let now = Instant::now();
    let double = part >= 0 && el.clicked.as_ref().is_some_and(|(k, q, t)| k == key && *q == part && now.duration_since(*t) < super::views::DOUBLE);
    el.clicked = (!double).then(|| (key.to_string(), part, now));
    let fired = build::click(&n, el.elems.entry(key.to_string()).or_default(), part, double);
    fire(app, &p, fired);
    app.dirty();
}

// The wheel over a sidebar element: the innermost part under the pointer that scrolls (or moves a choice).
pub fn element_wheel(app: &mut App, under: &[Hit], rows: i32) {
    for h in under {
        let Hit::View { view, key, .. } = h else { continue };
        let Some(p) = all(app).iter().find(|p| elem_key(p) == *view).cloned() else { continue };
        let Some(n) = p.element.as_ref().and_then(|root| build::node_at(root, key)).cloned() else { continue };
        let st = app.slot_elems.entry(view.clone()).or_default().elems.entry(key.clone()).or_default();
        if let Some(fired) = build::wheel(&n, st, rows) {
            fire(app, &p, fired);
            return app.dirty();
        }
    }
}

fn fire(app: &App, p: &SlotPiece, fired: Vec<Fired>) {
    let base = Map::from_iter([("slot".to_string(), json!(p.slot)), ("piece".to_string(), json!(p.id))]);
    super::views::send(app, &p.plugin, &json!(p.run), base, fired);
}

// a spinner in a sidebar element: frames come on their own
pub fn spinning(app: &App) -> bool {
    all(app).iter().filter_map(|p| p.element.as_ref()).any(|root| {
        let mut any = false;
        build::walk(root, build::root_key(root), &mut |n, _| any |= n["type"] == "spinner");
        any
    })
}

// ---------- menus, the palette and toasts ----------

// A menu's entries from plugins (menu.pane, menu.tab, menu.space; or the palette's): those for what it's opened on, and
// those for every one. Each is named after its plugin; chosen, it's `slot:<i>`.
pub struct Entries(pub Vec<SlotPiece>);

pub fn entries(app: &App, slot: &str, about: impl Fn(&SlotPiece) -> bool) -> Entries {
    let every = |p: &SlotPiece| p.pane.is_none() && p.tab.is_none() && p.space.is_none();
    Entries(pieces(app, slot, |p| every(p) || about(p)).into_iter().cloned().collect())
}

impl Entries {
    // (name, value) of those `before` modisa's own, or the rest
    pub fn named(&self, before: bool) -> Vec<(String, String)> {
        let at = |p: &SlotPiece| (p.position == SlotPosition::Before) == before;
        self.0.iter().enumerate().filter(|(_, p)| at(p)).map(|(i, p)| (format!("{}: {}", p.plugin, p.title.as_deref().unwrap_or(&p.id)), format!("slot:{i}"))).collect()
    }
    pub fn chosen(&self, value: &str) -> Option<&SlotPiece> {
        self.0.get(value.strip_prefix("slot:")?.parse::<usize>().ok()?)
    }
}

// A plugin's toast with rich text (plugin.toast with `lines`, `actions` and `timeout`): its lines, and its buttons, each
// running its action with `ui: { slot: "toast", id }`.
pub fn toast(app: &App, d: &Value) -> (Vec<Line<'static>>, Vec<(String, Hit)>, Option<u64>) {
    let p: SlotPiece = serde_json::from_value(d.clone()).unwrap_or_default();
    let buttons = p
        .actions
        .iter()
        .map(|b| {
            let ui = json!({ "slot": "toast", "id": p.id });
            (b.title.clone(), Hit::Slot(Box::new(SlotHit { plugin: p.plugin.clone(), run: p.run.clone(), action: Some(b.action.clone()), ui, target: None, toast: None })))
        })
        .collect();
    (lines(&ctx(app), &p), buttons, p.timeout)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;
    use tokio::sync::Notify;

    use super::*;
    use crate::cli::sessions::{ClientOptions, Connect};
    use crate::client::views::headless;
    use crate::protocol::types::View;

    // A client that never connects, w×h, showing the space "main" with one tab ("dev", t1): an agent (p1, focused) beside
    // a shell (p2), and `slots`.
    pub(crate) fn app(w: u16, h: u16, slots: Value) -> Rc<RefCell<App>> {
        let connect: Connect = Rc::new(|_, _| Box::pin(async { Err(crate::protocol::conn::fail("unreachable", "a test")) }));
        let opts = ClientOptions { session: "test".into(), connect, remote: false };
        let app = Rc::new_cyclic(|me| RefCell::new(App::new(opts, crate::config::defaults(), (w as i32, h as i32), me.clone(), Rc::new(Notify::new()), Rc::new(Notify::new()))));
        let pane = |id: &str, agent: Value| json!({ "id": id, "instance": format!("{id}i"), "title": "zsh", "cwd": "/", "cols": 40, "rows": 20, "status": "running", "agent": agent });
        let view: View = serde_json::from_value(json!({
            "active": 0,
            "workspaces": [{ "id": "w1", "name": "main", "cwd": "/", "active": 0, "tabs": [{ "id": "t1", "name": "dev", "tree": { "dir": "row", "ratio": 0.5, "a": { "pane": "p1" }, "b": { "pane": "p2" } }, "focused": "p1", "zoomed": false }] }],
            "panes": [pane("p1", json!({ "harness": "claude-code", "state": "working", "source": "screen" })), pane("p2", Value::Null)],
            "slots": slots,
        }))
        .unwrap();
        crate::client::set_view(&mut app.borrow_mut(), view);
        app
    }

    // a piece of radar's in `slot`
    fn radar(slot: &str, id: &str, more: Value) -> Value {
        let mut p = json!({ "plugin": "radar", "run": "r1", "slot": slot, "id": id });
        p.as_object_mut().unwrap().extend(more.as_object().unwrap().clone());
        p
    }

    fn frame(app: &Rc<RefCell<App>>) -> Buffer {
        let (w, h) = (app.borrow().width as u16, app.borrow().height as u16);
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| crate::client::draw::render(&mut app.borrow_mut(), f)).unwrap();
        t.backend().buffer().clone()
    }

    fn rows(app: &Rc<RefCell<App>>) -> Vec<String> {
        headless::text(&frame(app)).lines().map(String::from).collect()
    }

    // the sidebar's part of each row
    fn side(app: &Rc<RefCell<App>>) -> Vec<String> {
        rows(app).iter().map(|l| l.chars().take(25).collect()).collect()
    }

    // what the last frame put where for plugins' pieces
    fn slot_hits(app: &Rc<RefCell<App>>) -> Vec<(Rect, SlotHit)> {
        app.borrow().hits.iter().filter_map(|(r, h)| if let Hit::Slot(s) = h { Some((*r, (**s).clone())) } else { None }).collect()
    }

    fn at(rows: &[String], s: &str) -> usize {
        rows.iter().position(|l| l.contains(s)).unwrap_or_else(|| panic!("{s} isn't in {rows:#?}"))
    }

    #[test]
    fn status_pieces_go_after_modisas_buttons_and_before_its_right_hand_segments() {
        let a = app(120, 30, json!([radar("status.left", "l", json!({ "line": [{ "text": "LEFT", "style": "$warn" }], "action": "open" })), radar("status.right", "r", json!({ "line": "RIGHT" }))]));
        let status = rows(&a).pop().unwrap();
        let (l, need, right, panes) = (status.find("LEFT").unwrap(), status.find("need you").unwrap(), status.find("RIGHT").unwrap(), status.find("2 panes").unwrap());
        assert!(need < l && l < right && right < panes, "{status}");
        let buf = frame(&a);
        let x = status[..l].chars().count() as u16;
        assert_eq!(buf[(x, 29)].fg, color(a.borrow().th.warn)); // $warn, in this theme
        let hits = slot_hits(&a);
        let (r, left) = hits.iter().find(|(_, h)| h.ui["id"] == "l").unwrap();
        assert_eq!((r.x, r.y, r.w, left.action.as_deref()), (x as i32, 29, 4, Some("open")));
        assert_eq!(left.ui, json!({ "slot": "status.left", "id": "l" }));
        assert!(hits.iter().any(|(_, h)| h.ui["id"] == "r" && h.action.is_none())); // no action: it only says whose it is
    }

    #[test]
    fn modisas_segments_take_pieces_around_them_or_one_instead() {
        let a = app(
            120,
            30,
            json!([
                radar("status.panes", "b", json!({ "position": "before", "line": "BEFORE" })),
                radar("status.panes", "a", json!({ "line": "AFTER" })),
                radar("status.agents", "x", json!({ "position": "replace", "replaces": true, "line": "MINE" })),
                radar("status.panes", "lost", json!({ "position": "replace", "line": "LOST" })), // asked, but doesn't replace it
                radar("status.git", "g", json!({ "position": "replace", "replaces": true, "line": "@ jj" })), // outside a repository too
            ]),
        );
        let status = rows(&a).pop().unwrap();
        assert!(status.contains(" BEFORE  2 panes  AFTER  @ jj"), "{status}");
        assert!(status.contains(" MINE ") && !status.contains("working") && !status.contains("need you"), "{status}");
        assert!(!status.contains("LOST"), "{status}");
    }

    #[test]
    fn pieces_get_the_room_modisas_own_leave_cut_to_it() {
        let a = app(60, 20, json!([radar("status.left", "l", json!({ "line": "x".repeat(80) })), radar("status.right", "r", json!({ "line": "never" }))]));
        let status = rows(&a).pop().unwrap();
        assert!(status.ends_with(" 2 panes"), "{status}"); // modisa's own keep their place
        assert!(status.contains("xxx… 2 panes"), "{status}"); // the piece is cut where its room ends, and the next left out
        assert!(!status.contains("never"), "{status}");
    }

    #[test]
    fn pieces_go_by_order_and_hide_below_a_width() {
        let pieces = json!([
            radar("status.left", "b", json!({ "line": "SECOND", "order": 1 })),
            radar("status.left", "a", json!({ "line": "FIRST" })),
            radar("status.left", "c", json!({ "line": "WIDE", "hide_below": { "width": 150 } })),
        ]);
        let status = rows(&app(120, 30, pieces.clone())).pop().unwrap();
        assert!(status.find("FIRST").unwrap() < status.find("SECOND").unwrap() && !status.contains("WIDE"), "{status}");
        assert!(rows(&app(160, 30, pieces)).pop().unwrap().contains("WIDE"));
    }

    #[test]
    fn tab_labels_and_the_space_chip_take_pieces() {
        let a = app(
            120,
            30,
            json!([
                radar("tab", "n", json!({ "tab": "t1", "line": [{ "text": "●3", "style": "$accent" }] })),
                radar("space", "s", json!({ "space": "main", "position": "before", "line": "★" })),
                radar("tab", "other", json!({ "tab": "t9", "line": "NOPE" })),
            ]),
        );
        let top = rows(&a)[0].clone();
        assert!(top.starts_with(" ◈ ★ main ▸  1:dev ●3 ✕"), "{top}");
        assert!(!top.contains("NOPE"), "{top}");
        let b = app(120, 30, json!([radar("tab", "n", json!({ "tab": "t1", "position": "replace", "replaces": true, "line": "MINE" }))]));
        let top = rows(&b)[0].clone();
        assert!(top.contains(" 1:MINE ") && !top.contains("dev"), "{top}");
    }

    #[test]
    fn pane_borders_take_pieces_in_all_four_places() {
        let on = |slot: &str, id: &str, text: &str| radar(slot, id, json!({ "pane": "p1", "instance": "p1i", "line": text }));
        let a = app(
            120,
            30,
            json!([
                on("pane.title", "t", "43%"),
                on("pane.top_right", "tr", "TR"),
                on("pane.bottom_left", "bl", "BL"),
                on("pane.bottom_right", "br", "BR"),
                radar("pane.title", "gone", json!({ "pane": "p1", "instance": "old", "line": "STALE" })), // another process's
                radar("pane.title", "mine", json!({ "pane": "p2", "instance": "p2i", "position": "replace", "replaces": true, "line": "SHELL" })),
            ]),
        );
        let r = rows(&a);
        let (top, bottom) = (&r[1], &r[28]);
        assert!(top.contains("◆ zsh ◆ claude-code working 43% ─"), "{top}");
        assert!(top.contains("─ TR ─┐"), "{top}");
        assert!(!top.contains("STALE"), "{top}");
        assert!(top.contains("◇ SHELL ─") && top.matches("zsh").count() == 1, "{top}"); // p2's title is the plugin's
        assert!(bottom.contains("└─ BL ─") && bottom.contains("─ BR ─┘"), "{bottom}");
        let hits = slot_hits(&a);
        let (_, t) = hits.iter().find(|(_, h)| h.ui["id"] == "t").unwrap();
        assert_eq!(t.target, Some(("p1".into(), "p1i".into())));
        assert_eq!(t.ui, json!({ "slot": "pane.title", "id": "t", "pane": "p1", "instance": "p1i" }));
    }

    #[test]
    fn a_long_pane_title_is_cut_inside_its_border() {
        let a = app(120, 30, json!([radar("pane.title", "t", json!({ "pane": "p1", "instance": "p1i", "line": "y".repeat(200) }))]));
        let top = rows(&a)[1].clone();
        let p1: String = top.chars().skip(26).take(47).collect(); // p1's box
        assert!(p1.ends_with("y…─┐"), "{p1}");
    }

    #[test]
    fn agent_rows_take_lines_around_them_or_a_plugins_instead() {
        let row = |position: &str, replaces: bool, lines: Value| radar("agent.row", position, json!({ "pane": "p1", "instance": "p1i", "position": position, "replaces": replaces, "lines": lines, "action": "details" }));
        let a = app(120, 30, json!([row("after", false, json!(["43% context", "2 files"])), row("before", false, json!(["ABOVE"]))]));
        let s = side(&a);
        assert!(at(&s, "ABOVE") < at(&s, "claude-code") && at(&s, "claude-code") + 2 == at(&s, "43% context") && at(&s, "43% context") + 1 == at(&s, "2 files"), "{s:#?}");
        let (_, after) = slot_hits(&a).into_iter().find(|(_, h)| h.ui["id"] == "after").unwrap();
        assert_eq!(after.action.as_deref(), Some("details"));
        let b = app(120, 30, json!([row("replace", true, json!(["MINE"]))]));
        let s = side(&b);
        assert!(s.iter().any(|l| l.contains("MINE")) && !s.iter().any(|l| l.contains("claude-code")), "{s:#?}");
    }

    #[test]
    fn sidebar_sections_have_lines_and_elements_that_take_clicks() {
        let list = json!({ "type": "list", "id": "l", "items": ["alpha", "beta"], "change": "pick" });
        let a = app(
            120,
            30,
            json!([
                radar("sidebar", "rows", json!({ "title": "Watching", "lines": ["one", [{ "text": "two", "style": "bold" }]], "action": "open" })),
                radar("sidebar", "tree", json!({ "element": list, "height": 2 })),
            ]),
        );
        let s = side(&a);
        assert!(at(&s, "▾ radar") + 1 == at(&s, "Watching") && at(&s, "Watching") + 1 == at(&s, "one") && at(&s, "one") + 2 == at(&s, "alpha") && at(&s, "alpha") + 1 == at(&s, "beta"), "{s:#?}");
        let (_, two) = slot_hits(&a).into_iter().find(|(_, h)| h.ui["row"] == 1).unwrap();
        assert_eq!(two.ui, json!({ "slot": "sidebar", "id": "rows", "row": 1 }));
        // a click on the list's second item chooses it, as in a view
        let item = a.borrow().hits.iter().find_map(|(r, h)| match h {
            Hit::View { view, key, part: 1 } => Some((*r, view.clone(), key.clone())),
            _ => None,
        });
        let (r, view, key) = item.unwrap();
        assert_eq!(r.y as usize, at(&s, "beta"));
        element_click(&mut a.borrow_mut(), &view, &key, 1);
        assert_eq!(a.borrow().slot_elems[&view].elems["#l"].list.selected(), Some(1));
        // folded under its plugin's name
        a.borrow_mut().collapsed_plugins.insert("radar".into());
        assert!(!rows(&a).iter().any(|l| l.contains("Watching")));
    }

    #[test]
    fn a_plugin_can_draw_the_agents_list_instead() {
        let a = app(120, 30, json!([radar("sidebar.agents", "list", json!({ "position": "replace", "replaces": true, "lines": ["my agents"] }))]));
        let s = side(&a);
        assert_eq!(s[2].trim_end(), "  ▾ radar", "{s:#?}"); // named as its sections are, never AGENTS
        assert!(!s.iter().any(|l| l.contains("AGENTS")), "{s:#?}");
        assert_eq!(s[4].trim(), "my agents");
        assert!(!s.iter().any(|l| l.contains("claude-code")), "{s:#?}");
    }

    #[test]
    fn hovering_a_piece_says_whose_it_is() {
        let a = app(120, 30, json!([radar("status.left", "l", json!({ "line": "LEFT", "action": "open" }))]));
        let status = rows(&a).pop().unwrap();
        let x = status[..status.find("LEFT").unwrap()].chars().count() as i32;
        a.borrow_mut().hover = Some((x, 29));
        let above = rows(&a)[28].clone();
        assert!(above.contains(" radar · open "), "{above}");
    }

    #[test]
    fn toasts_have_rich_text_and_buttons_that_take_them_away() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        tokio::task::LocalSet::new().block_on(&rt, async {
            let a = app(120, 30, json!([]));
            let lines = json!([[{ "text": "Build ", "style": "bold" }, { "text": "failed", "style": "$blocked" }], "3 tests"]);
            let sent = json!({ "plugin": "radar", "run": "r1", "id": "t", "text": "plain", "tone": "warn", "lines": lines, "actions": [{ "title": "Open", "action": "open" }], "timeout": 60000 });
            crate::client::sent_toast(&mut a.borrow_mut(), &sent);
            let r = rows(&a);
            let at = at(&r, "Build failed");
            assert!(r[at - 1].contains(" radar ") && r[at + 1].contains("3 tests") && r[at + 2].contains(" Open "), "{r:#?}");
            let (_, open) = slot_hits(&a).into_iter().find(|(_, h)| h.action.as_deref() == Some("open")).unwrap();
            assert_eq!(open.ui, json!({ "slot": "toast", "id": "t" }));
            clicked(&mut a.borrow_mut(), &open);
            assert!(a.borrow().toasts.is_empty());
        });
    }

    #[test]
    fn menus_have_entries_for_what_theyre_opened_on_and_for_every_one() {
        let a = app(
            120,
            30,
            json!([
                radar("menu.pane", "here", json!({ "pane": "p1", "instance": "p1i", "title": "Inspect", "action": "inspect" })),
                radar("menu.pane", "every", json!({ "title": "Note", "action": "note", "position": "before" })),
                radar("menu.pane", "there", json!({ "pane": "p2", "instance": "p2i", "title": "Other", "action": "other" })),
            ]),
        );
        let app = a.borrow();
        let p1 = app.info("p1").unwrap().clone();
        let e = entries(&app, "menu.pane", |p| of_pane(p, &p1));
        assert_eq!(e.named(true), [("radar: Note".to_string(), "slot:0".to_string())]);
        assert_eq!(e.named(false), [("radar: Inspect".to_string(), "slot:1".to_string())]);
        assert_eq!(e.chosen("slot:1").map(|p| p.id.as_str()), Some("here"));
    }

    #[test]
    fn lines_are_cut_with_an_ellipsis_in_the_style_of_what_they_cut() {
        let red = Style::new().fg(ratatui::style::Color::Red);
        let l = Line::from(vec![Span::raw("ab"), Span::styled("日本語", red)]);
        let text = |l: Line| l.spans.iter().map(|s| s.content.to_string()).collect::<String>();
        let cut = fit_line(l.clone(), 5);
        assert_eq!((text(cut.clone()), line_width(&cut)), ("ab日…".to_string(), 5));
        assert_eq!(cut.spans.last().unwrap().style, red);
        assert_eq!(text(fit_line(l.clone(), 4)), "ab…"); // a wide character doesn't half fit
        assert_eq!(text(fit_line(l, 0)), "");
    }
}
