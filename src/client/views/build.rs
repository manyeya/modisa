// A plugin view's elements (UI 3, examples/plugins/VIEWS.md), drawn with ratatui's own widgets into the frame's buffer.
// The tree is drawn whole every frame; what the user does in an element (its selection, scroll, text, open nodes) is
// kept in an `ElState` by the element's key across updates, until the plugin changes the prop it came from.
//
// Layout, blocks, text, lists, tables, tabs, buttons, spinners, big text, fill and clear are here; the rest each have a
// module: charts.rs (gauges, sparklines, bar charts, charts, canvases, calendars), code.rs (code and diffs, highlighted
// by highlight.rs), markdown.rs, image.rs, fields.rs (input, textarea), tree.rs and raster.rs.
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::LazyLock;
use std::time::Instant;

use ansi_to_tui::IntoText;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Flex, HorizontalAlignment, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Cell, Clear, HighlightSpacing, List, ListDirection, ListItem, ListState, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, StatefulWidget, Table, TableState, Tabs, Widget, Wrap};
use regex::Regex;
use serde_json::{json, Map, Value};
use throbber_widgets_tui::{Throbber, ThrobberState, WhichUse};
use tui_big_text::{BigText, PixelSize};
use tui_input::Input;
use tui_tree_widget::TreeState;

use super::{charts, code, fields, image, markdown, raster, tree};
use crate::client::design::{agent_mark, color, mix};
use crate::client::draw::{Canvas, Hit};
use crate::config::themes::Theme;
use crate::core::text::width;
use crate::platform::logos::Loaded;
use crate::protocol::ui::{self, Ink};

// What drawing needs from the client: the theme, how agents' marks are drawn, and the animation clock.
pub struct Ctx<'a> {
    pub th: &'a Theme,
    pub logos: Option<Loaded>,
    pub cell: f64,
    pub tick: u64, // spinner frames since the client started
}

impl Ctx<'_> {
    // an agent's mark, as `{ "icon": id }` puts it in text: its glyph or logo in its colour, and the room it takes
    fn mark(&self, agent: &str) -> Vec<Span<'static>> {
        let m = agent_mark(self.th, agent, self.logos, self.cell);
        vec![Span::styled(format!("{}{}", m.glyph, " ".repeat(m.cells.saturating_sub(2))), Style::new().fg(color(&m.color)))]
    }
    // What the plugin sent, read as protocol/ui.rs reads it; the server has checked it, so a mistake draws nothing.
    fn read<T: Default>(&self, f: impl FnOnce(&Ink) -> ui::Res<T>) -> T {
        let icon = |a: &str| self.mark(a);
        f(&Ink { th: self.th, icon: &icon }).unwrap_or_default()
    }
    pub fn text(&self, v: &Value) -> Text<'static> {
        self.read(|i| ui::text(v, i))
    }
    pub fn line(&self, v: &Value) -> Line<'static> {
        self.read(|i| ui::line(v, i))
    }
    // one Span: an icon's pieces joined
    pub fn span(&self, v: &Value) -> Span<'static> {
        let parts = self.read(|i| ui::spans(v, i));
        let style = parts.first().map(|s| s.style).unwrap_or_default();
        Span::styled(parts.iter().map(|s| s.content.as_ref()).collect::<String>(), style)
    }
    pub fn style(&self, v: &Value) -> Style {
        ui::style(v, self.th).unwrap_or_default()
    }
    // `field`'s Style, or `default` when the plugin gave none
    pub fn style_or(&self, n: &Value, field: &str, default: Style) -> Style {
        if n[field].is_null() { default } else { self.style(&n[field]) }
    }
    pub fn color(&self, v: &Value) -> Option<Color> {
        v.as_str().and_then(|s| ui::color(s, self.th).ok())
    }
    // a theme colour by its token's name
    pub fn tone(&self, name: &str) -> Color {
        color(ui::token(self.th, name).unwrap_or(self.th.fg))
    }
    // the theme's background tinted toward a token's colour
    pub fn tint(&self, name: &str, t: f64) -> Color {
        color(&mix(self.th.bg, ui::token(self.th, name).unwrap_or(self.th.fg), t))
    }
    // what's chosen in a list, a table or a tree: brighter where the keyboard is
    pub fn selection(&self, on: bool) -> Style {
        Style::new().bg(self.tint("focus", if on { 0.28 } else { 0.14 }))
    }
}

// ---------- the tree ----------

pub type Elems = HashMap<String, ElState>;

// What the user did in an element, kept across updates. One struct for every kind: each uses its own fields.
#[derive(Default)]
pub struct ElState {
    pub ty: String,  // the element type it's for: another type at the same key starts afresh
    pub prop: Value, // the plugin's prop it started from (selected, value, open…): when that changes, the user's goes
    pub list: ListState,
    pub table: TableState,
    pub tab: usize,
    pub tree: TreeState<String>,
    pub drawn: Vec<Vec<String>>, // a tree's paths as the last frame drew them, a row each (for clicks)
    pub input: Input,
    pub area: ratatui_textarea::TextArea<'static>,
    pub scroll: usize,  // rows (or lines) scrolled down
    pub hscroll: usize, // columns scrolled right (code and diffs that don't wrap)
    pub follow: bool,   // a "bottom" text at its end: it stays there as it grows
    pub line: usize,    // a diff's cursor, a body line
    pub reveal: bool,   // the cursor moved: the next frame scrolls it into view
    pub rows: usize,    // the last frame's rows there are…
    pub shown: usize,   // …and rows shown
    pub image: Option<image::Slot>,
    pub sent: Option<Instant>, // a field's last `change`…
    pub pending: bool,         // …and one that's due
}

pub fn ty(n: &Value) -> &str {
    n["type"].as_str().unwrap_or("")
}

// An element's key: its own id (`#id`), else its place under its parent.
pub fn key_of(n: &Value, path: &str) -> String {
    n["id"].as_str().map(|k| format!("#{k}")).unwrap_or_else(|| path.to_string())
}

// what's inside an element: a layout's children, a block's child
pub fn children(n: &Value) -> Vec<&Value> {
    match ty(n) {
        "layout" => n["children"].as_array().map(|a| a.iter().collect()).unwrap_or_default(),
        "block" => n.get("child").filter(|c| c.is_object()).into_iter().collect(),
        _ => vec![],
    }
}

// every element, depth first as drawn, with its key
pub fn walk<'a>(n: &'a Value, key: String, f: &mut impl FnMut(&'a Value, &str)) {
    f(n, &key);
    for (i, c) in children(n).into_iter().enumerate() {
        walk(c, key_of(c, &format!("{key}.{i}")), f);
    }
}

pub fn root_key(root: &Value) -> String {
    key_of(root, "0")
}

pub fn node_at<'a>(root: &'a Value, key: &str) -> Option<&'a Value> {
    let mut found = None;
    walk(root, root_key(root), &mut |n, k| {
        if found.is_none() && k == key {
            found = Some(n);
        }
    });
    found
}

// What Tab moves between: a field, something to choose in, a button, or something to scroll.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Focus {
    Field,
    Pick,
    Button,
    Scroll,
}

pub fn focus_kind(n: &Value) -> Option<Focus> {
    match ty(n) {
        "input" | "textarea" => Some(Focus::Field),
        "list" | "table" | "tabs" | "tree" => Some(Focus::Pick),
        "button" => Some(Focus::Button),
        "code" | "diff" | "markdown" => Some(Focus::Scroll),
        "text" if scrolls(n) => Some(Focus::Scroll),
        _ => None,
    }
}

// a text the user scrolls
fn scrolls(n: &Value) -> bool {
    matches!(&n["scroll"], Value::Bool(true) | Value::String(_))
}

pub fn focusables(root: &Value) -> Vec<(String, Focus)> {
    let mut out = vec![];
    walk(root, root_key(root), &mut |n, k| {
        if let Some(f) = focus_kind(n) {
            out.push((k.to_string(), f));
        }
    });
    out
}

pub fn hash_of(x: impl Hash) -> u64 {
    let mut h = DefaultHasher::new();
    x.hash(&mut h);
    h.finish()
}

fn len(v: &Value) -> usize {
    v.as_array().map_or(0, Vec::len)
}
fn num(v: &Value) -> Option<usize> {
    v.as_u64().map(|n| n as usize)
}

// The states for a new tree: an element keeps the user's, unless the plugin changed what that came from.
pub fn reconcile(old: &mut Elems, root: &Value) -> Elems {
    let mut next = HashMap::new();
    walk(root, root_key(root), &mut |n, k| {
        let t = ty(n);
        let prop = match t {
            "list" | "tabs" | "table" => n["selected"].clone(),
            "tree" => json!([n["open"], n["selected"]]),
            "input" | "textarea" => n["value"].clone(),
            "diff" => json!(hash_of(n["diff"].as_str().unwrap_or(""))),
            "text" | "code" | "markdown" | "image" => Value::Null,
            _ => return,
        };
        let kept = old.remove(k).filter(|s| s.ty == t);
        let fresh = kept.is_none();
        let mut st = kept.unwrap_or_else(|| ElState { ty: t.to_string(), ..Default::default() });
        if fresh || st.prop != prop {
            seed(n, &mut st);
        }
        st.prop = prop;
        next.insert(k.to_string(), st);
    });
    next
}

// An element's state from what the plugin says it starts with.
fn seed(n: &Value, st: &mut ElState) {
    let acts = !n["action"].is_null() || !n["change"].is_null();
    match ty(n) {
        // nothing chosen until the user moves, unless it was given a choice or does something with one
        "list" => st.list.select(num(&n["selected"]).or(acts.then_some(0)).filter(|_| len(&n["items"]) > 0)),
        "tabs" => st.tab = num(&n["selected"]).unwrap_or(0),
        "table" => {
            let sel = &n["selected"];
            let (r, c) = match sel.as_array() {
                Some(a) => (a.first().and_then(num), a.get(1).and_then(num)),
                None => (num(sel), None),
            };
            let (r, c) = (r.or(acts.then_some(0)), c.or(acts.then_some(0)));
            match n["select"].as_str().unwrap_or("row") {
                "cell" => st.table.select_cell(r.zip(c)),
                "column" => st.table.select_column(c),
                "none" => {}
                _ => st.table.select(r),
            }
        }
        "tree" => tree::seed(n, st),
        "input" => st.input = Input::new(n["value"].as_str().unwrap_or("").to_string()),
        "textarea" => fields::seed(n, st),
        "diff" => {
            (st.line, st.scroll) = (0, 0);
        }
        "text" => st.follow = n["scroll"] == "bottom",
        _ => {}
    }
}

// ---------- drawing ----------

// What a frame's drawing needs to know and tells back: whose view it is, what has the keyboard, and where the caret is.
pub struct Draw<'a> {
    pub view: &'a str,
    pub focus: Option<&'a str>,
    pub active: bool, // this view has the keyboard
    pub cursor: Option<Position>,
}

// a part of a hit that's the element itself, and one that's an area to scroll
pub const WHOLE: i32 = -1;
pub const SCROLLS: i32 = -3;

pub fn hit(c: &mut Canvas, d: &Draw, key: &str, r: Rect, part: i32) {
    let r = r.intersection(c.buf.area);
    if !r.is_empty() {
        let at = crate::core::layout::Rect { x: r.x as i32, y: r.y as i32, w: r.width as i32, h: r.height as i32 };
        c.hit(at, Hit::View { view: d.view.to_string(), key: key.to_string(), part });
    }
}

// An element at `area`: its base style, its block, then itself inside the block.
pub fn draw(ctx: &Ctx, c: &mut Canvas, n: &Value, key: &str, area: Rect, elems: &mut Elems, d: &mut Draw) {
    let area = area.intersection(c.buf.area);
    if area.is_empty() {
        return;
    }
    let on = d.active && d.focus == Some(key);
    if ty(n) == "clear" {
        Clear.render(area, c.buf);
        c.buf.set_style(area, Style::new().fg(ctx.tone("fg")).bg(ctx.tone("bg")));
    }
    if !n["style"].is_null() {
        c.buf.set_style(area, ctx.style(&n["style"]));
    }
    let mut inner = area;
    if n["block"].is_object() {
        let b = block(ctx, &n["block"], on);
        inner = b.inner(area);
        b.render(area, c.buf);
    }
    if let Some(f) = focus_kind(n) {
        hit(c, d, key, area, if f == Focus::Scroll { SCROLLS } else { WHOLE });
    }
    if inner.is_empty() {
        return;
    }
    match ty(n) {
        "layout" => layout(ctx, c, n, key, inner, elems, d),
        "block" => {
            let b = block(ctx, n, false);
            let i = b.inner(inner);
            b.render(inner, c.buf);
            if let Some(child) = children(n).first() {
                draw(ctx, c, child, &key_of(child, &format!("{key}.0")), i, elems, d);
            }
        }
        "text" => text(ctx, c, n, key, inner, elems),
        "list" => list(ctx, c, n, key, inner, elems, d, on),
        "table" => table(ctx, c, n, key, inner, elems, d, on),
        "tabs" => tabs(ctx, c, n, key, inner, elems, d, on),
        "gauge" | "line_gauge" | "sparkline" | "bar_chart" | "chart" | "canvas" | "calendar" => charts::draw(ctx, c.buf, n, inner),
        // its `style` is already on its area
        "fill" => {
            let symbol = n["symbol"].as_str().filter(|s| width(s) == 1).unwrap_or(" ");
            for pos in inner.positions() {
                c.buf[pos].set_symbol(symbol);
            }
        }
        "code" => code::draw_code(ctx, c, n, key, inner, elems),
        "diff" => code::draw_diff(ctx, c, n, key, inner, elems, d, on),
        "markdown" => markdown::draw(ctx, c, n, key, inner, elems),
        "big_text" => big_text(ctx, c.buf, n, inner),
        "image" => image::draw(ctx, c, n, key, inner, elems, d),
        "input" => fields::draw_input(ctx, c, n, key, inner, elems, d, on),
        "textarea" => fields::draw_textarea(ctx, c, n, key, inner, elems, on),
        "tree" => tree::draw(ctx, c, n, key, inner, elems, d, on),
        "button" => button(ctx, c, n, inner, on),
        "spinner" => spinner(ctx, c.buf, n, inner),
        "raster" => raster::draw(ctx, c.buf, n, inner),
        _ => {}
    }
}

// A Block from its fields; its border takes the focus colour while what's in it has the keyboard, and the theme's border
// colour when the plugin didn't choose one.
pub fn block(ctx: &Ctx, v: &Value, on: bool) -> Block<'static> {
    let b = ctx.read(|i| ui::block(v, i));
    if !v["border_style"].is_null() {
        return b;
    }
    b.border_style(Style::new().fg(ctx.tone(if on { "focus" } else { "border" })))
}

fn layout(ctx: &Ctx, c: &mut Canvas, n: &Value, key: &str, area: Rect, elems: &mut Elems, d: &mut Draw) {
    let kids = n["children"].as_array().map(Vec::as_slice).unwrap_or_default();
    if kids.is_empty() {
        return;
    }
    let given = n["constraints"].as_array();
    let mut cs: Vec<Constraint> = kids
        .iter()
        .enumerate()
        .map(|(i, k)| given.and_then(|g| g.get(i)).or(k.get("size")).and_then(|v| ui::constraint(v).ok()).unwrap_or(Constraint::Fill(1)))
        .collect();
    let mut l = Layout::default()
        .direction(ui::direction(&n["direction"]).unwrap_or(Direction::Vertical))
        .flex(ui::flex(&n["flex"]).unwrap_or(Flex::Start))
        .spacing(n["spacing"].as_i64().unwrap_or(0).clamp(i16::MIN as i64, i16::MAX as i64) as i16);
    let cells = |v: &Value| v.as_u64().unwrap_or(0).min(u16::MAX as u64) as u16;
    match &n["margin"] {
        Value::Array(m) => l = l.vertical_margin(m.first().map_or(0, cells)).horizontal_margin(m.get(1).map_or(0, cells)),
        m if m.is_u64() => l = l.margin(cells(m)),
        _ => {}
    }
    let mut rects = l.clone().constraints(cs.clone()).split(area);
    // hide_below: a child whose area would be smaller isn't drawn, and its constraint gets no room
    let small = |k: &Value, r: Rect| k["hide_below"].is_object() && (r.width < cells(&k["hide_below"]["width"]) || r.height < cells(&k["hide_below"]["height"]));
    let hidden: Vec<bool> = kids.iter().zip(rects.iter()).map(|(k, r)| small(k, *r)).collect();
    if hidden.contains(&true) {
        for (c, h) in cs.iter_mut().zip(&hidden) {
            if *h {
                *c = Constraint::Length(0);
            }
        }
        rects = l.constraints(cs).split(area);
    }
    for (i, (k, r)) in kids.iter().zip(rects.iter()).enumerate() {
        if !hidden[i] {
            draw(ctx, c, k, &key_of(k, &format!("{key}.{i}")), *r, elems, d);
        }
    }
}

// ---------- scrolling ----------

// Where a scrolled element starts showing, kept within what there is: `total` rows, `shown` at a time.
pub fn scrolled(st: &mut ElState, total: usize, shown: usize) -> usize {
    let most = total.saturating_sub(shown);
    if st.follow {
        st.scroll = most;
    }
    st.scroll = st.scroll.min(most);
    (st.rows, st.shown) = (total, shown);
    st.scroll
}

// The area less a column for a scrollbar, when `total` rows won't all show and the element has one.
pub fn bar_room(n: &Value, area: Rect, total: usize, default: bool) -> (Rect, bool) {
    let want = n["scrollbar"].as_bool().unwrap_or(default);
    if want && total > area.height as usize && area.width > 1 {
        (Rect { width: area.width - 1, ..area }, true)
    } else {
        (area, false)
    }
}

// A scrollbar down the right of `area`: `total` rows, `shown` of them from `top`.
pub fn scrollbar(ctx: &Ctx, buf: &mut Buffer, area: Rect, total: usize, shown: usize, top: usize) {
    let mut s = ScrollbarState::new(total.saturating_sub(shown)).position(top).viewport_content_length(shown);
    Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(None)
        .end_symbol(None)
        .track_symbol(Some(" "))
        .thumb_symbol("▐")
        .thumb_style(Style::new().fg(ctx.tone("border")))
        .render(area, buf, &mut s);
}

// ---------- text ----------

// Escape sequences other than SGR (colours and attributes), and control characters but newlines and tabs.
static NOT_SGR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1b\[[0-9;:?<=>]*[ -/]*[@-ln-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)?|\x1b[PX^_][^\x1b]*(?:\x1b\\)?|\x1b[ -/]+[0-~]|\x1b[0-Z\\-~]|[\x00-\x08\x0b-\x1a\x1c-\x1f\x7f]").unwrap());

// A program's coloured output as Text: its SGR codes kept, anything else dropped.
pub fn ansi(s: &str) -> Text<'static> {
    let clean = NOT_SGR.replace_all(s, "").replace('\t', "    ");
    clean.as_bytes().into_text().unwrap_or_else(|_| Text::raw(clean))
}

fn paragraph(n: &Value, text: Text<'static>) -> Paragraph<'static> {
    let mut p = Paragraph::new(text);
    if let Ok(Some(a)) = ui::align(&n["align"]) {
        p = p.alignment(a);
    }
    match &n["wrap"] {
        Value::Bool(false) => p,
        Value::String(s) if s == "trim" => p.wrap(Wrap { trim: true }),
        _ => p.wrap(Wrap { trim: false }),
    }
}

fn text(ctx: &Ctx, c: &mut Canvas, n: &Value, key: &str, area: Rect, elems: &mut Elems) {
    let content = match n["ansi"].as_str() {
        Some(a) => ansi(a),
        None => ctx.text(&n["text"]),
    };
    let p = paragraph(n, content);
    if !scrolls(n) {
        return p.render(area, c.buf);
    }
    let st = elems.entry(key.to_string()).or_default();
    let (body, bar) = bar_room(n, area, p.line_count(area.width), false);
    let total = p.line_count(body.width);
    let top = scrolled(st, total, body.height as usize);
    p.scroll((top.min(u16::MAX as usize) as u16, 0)).render(body, c.buf);
    if bar {
        scrollbar(ctx, c.buf, area, total, body.height as usize, top);
    }
}

fn big_text(ctx: &Ctx, buf: &mut Buffer, n: &Value, area: Rect) {
    let pixels = match n["pixel_size"].as_str() {
        Some("half_height") => PixelSize::HalfHeight,
        Some("half_width") => PixelSize::HalfWidth,
        Some("quadrant") => PixelSize::Quadrant,
        Some("third_height") => PixelSize::ThirdHeight,
        Some("sextant") => PixelSize::Sextant,
        Some("quarter_height") => PixelSize::QuarterHeight,
        Some("octant") => PixelSize::Octant,
        _ => PixelSize::Full,
    };
    BigText::builder()
        .lines(ctx.text(&n["text"]).lines)
        .pixel_size(pixels)
        .style(ctx.style_or(n, "style", Style::new().fg(ctx.tone("accent"))))
        .alignment(ui::align(&n["align"]).ok().flatten().unwrap_or(HorizontalAlignment::Left))
        .build()
        .render(area, buf);
}

// A button: its label in the middle of it, in its style, or its focus style while it has the keyboard.
fn button(ctx: &Ctx, c: &mut Canvas, n: &Value, area: Rect, on: bool) {
    let look = if on {
        ctx.style_or(n, "focus_style", Style::new().fg(ctx.tone("bg")).bg(ctx.tone("accent")).add_modifier(Modifier::BOLD))
    } else if c.hovered(crate::core::layout::Rect { x: area.x as i32, y: area.y as i32, w: area.width as i32, h: area.height as i32 }) {
        ctx.style_or(n, "style", Style::new().fg(ctx.tone("accent")).bg(ctx.tint("accent", 0.28)))
    } else {
        ctx.style_or(n, "style", Style::new().fg(ctx.tone("accent")).bg(ctx.tint("accent", 0.15)))
    };
    c.buf.set_style(area, look);
    let y = area.y + area.height.saturating_sub(1) / 2;
    Paragraph::new(ctx.line(&n["label"])).alignment(HorizontalAlignment::Center).render(Rect { y, height: 1, ..area }, c.buf);
}

// The spinner sets a view may ask for, by name.
fn spinner_set(name: Option<&str>) -> throbber_widgets_tui::Set {
    use throbber_widgets_tui as t;
    let own = |symbols: &'static [&'static str]| t::Set { full: symbols[0], empty: " ", symbols };
    match name {
        Some("dots") => own(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
        Some("ascii") => t::ASCII,
        Some("arrows") => t::ARROW,
        Some("clock") => t::CLOCK,
        Some("circle") => t::BLACK_CIRCLE,
        Some("box") => t::BOX_DRAWING,
        Some("bounce") => own(&["⠁", "⠂", "⠄", "⡀", "⠄", "⠂"]),
        Some("pulse") => own(&["·", "•", "●", "•"]),
        _ => t::BRAILLE_EIGHT,
    }
}

fn spinner(ctx: &Ctx, buf: &mut Buffer, n: &Value, area: Rect) {
    let set = spinner_set(n["set"].as_str());
    let mut st = ThrobberState::default();
    let step = (ctx.tick % set.symbols.len() as u64) as i8;
    if step > 0 {
        st.calc_step(step); // (a step of 0 picks one at random)
    }
    // its symbol and a space, then its label (a Line: the throbber's own takes a Span); `style` is both's
    let own = if n["style"].is_null() { Style::new().fg(ctx.tone("working")) } else { Style::new() };
    let mut spans = vec![Throbber::default().throbber_style(own).throbber_set(set).use_type(WhichUse::Spin).to_symbol_span(&st)];
    spans.extend(ctx.line(&n["label"]).spans);
    Line::from(spans).style(ctx.style_or(n, "style", Style::new().fg(ctx.tone("dim")))).render(area, buf);
}

// ---------- lists ----------

fn spacing(v: &Value) -> HighlightSpacing {
    match v.as_str() {
        Some("always") => HighlightSpacing::Always,
        Some("never") => HighlightSpacing::Never,
        _ => HighlightSpacing::WhenSelected,
    }
}

// What a list shows: Lines, or `{ "content": Text, "style": Style }`.
fn items(ctx: &Ctx, n: &Value) -> Vec<ListItem<'static>> {
    let items = n["items"].as_array().map(Vec::as_slice).unwrap_or_default();
    items
        .iter()
        .map(|it| match it.get("content") {
            Some(content) => ListItem::new(ctx.text(content)).style(ctx.style(&it["style"])),
            None => ListItem::new(ctx.line(it)),
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn list(ctx: &Ctx, c: &mut Canvas, n: &Value, key: &str, area: Rect, elems: &mut Elems, d: &mut Draw, on: bool) {
    let items = items(ctx, n);
    let heights: Vec<usize> = items.iter().map(ListItem::height).collect();
    let up = n["direction"] == "bottom_to_top";
    let mut l = List::new(items)
        .highlight_style(ctx.style_or(n, "highlight_style", ctx.selection(on)))
        .highlight_spacing(spacing(&n["highlight_spacing"]))
        .direction(if up { ListDirection::BottomToTop } else { ListDirection::TopToBottom })
        .scroll_padding(num(&n["scroll_padding"]).unwrap_or(0));
    if let Some(s) = n["highlight_symbol"].as_str() {
        l = l.highlight_symbol(s.to_string());
    }
    let total: usize = heights.iter().sum();
    let (body, bar) = bar_room(n, area, total, true);
    let st = elems.entry(key.to_string()).or_default();
    StatefulWidget::render(l, body, c.buf, &mut st.list);
    st.shown = body.height as usize;
    // each item that shows takes clicks on its rows
    let first = st.list.offset();
    let mut y = 0;
    for (i, h) in heights.iter().enumerate().skip(first) {
        if y >= body.height as usize {
            break;
        }
        let h = (*h).min(body.height as usize - y) as u16;
        let row = if up { body.bottom() - y as u16 - h } else { body.y + y as u16 };
        hit(c, d, key, Rect { y: row, height: h, ..body }, i as i32);
        y += h as usize;
    }
    if bar {
        scrollbar(ctx, c.buf, area, total, body.height as usize, heights[..first.min(heights.len())].iter().sum());
    }
}

// Where a cursor over `count` things moves for a key: up, down, j, k, a page (`page` things), home, end.
pub fn moved(name: &str, at: Option<usize>, count: usize, page: usize) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let last = count - 1;
    let at = at.map(|a| a.min(last));
    Some(match name {
        "up" | "k" => at.map_or(last, |a| a.saturating_sub(1)),
        "down" | "j" => at.map_or(0, |a| (a + 1).min(last)),
        "pageup" => at.map_or(0, |a| a.saturating_sub(page.max(1))),
        "pagedown" => at.map_or(0, |a| (a + page.max(1)).min(last)),
        "home" => 0,
        "end" => last,
        _ => return None,
    })
}

fn list_key(n: &Value, st: &mut ElState, name: &str) -> Took {
    if name == "enter" {
        let i = st.list.selected()?;
        return Some(fired(n, "action", json!({ "index": i })).into_iter().collect());
    }
    // a list that runs bottom to top moves the other way
    let name = match (n["direction"] == "bottom_to_top", name) {
        (true, "up" | "k") => "down",
        (true, "down" | "j") => "up",
        (_, other) => other,
    };
    let to = moved(name, st.list.selected(), len(&n["items"]), st.list_page())?;
    Some(choose_item(n, st, to))
}

fn choose_item(n: &Value, st: &mut ElState, to: usize) -> Vec<Fired> {
    if st.list.selected() == Some(to) {
        return vec![];
    }
    st.list.select(Some(to));
    fired(n, "change", json!({ "index": to })).into_iter().collect()
}

impl ElState {
    // a page of a list or table: the rows that showed last
    fn list_page(&self) -> usize {
        self.shown.max(1)
    }
}

// ---------- tables ----------

// A Cell: a Text, or `{ "content": Text, "style": Style, "span": n }`.
fn cell(ctx: &Ctx, v: &Value) -> (Cell<'static>, u16, usize) {
    match v.get("content") {
        Some(content) => {
            let t = ctx.text(content);
            let h = t.height();
            let span = v["span"].as_u64().unwrap_or(1).clamp(1, u16::MAX as u64) as u16;
            (Cell::new(t).style(ctx.style(&v["style"])).column_span(span), span, h)
        }
        None => {
            let t = ctx.text(v);
            let h = t.height();
            (Cell::new(t), 1, h)
        }
    }
}

// A Row (`{ "cells": […], "style", "height", "top_margin", "bottom_margin" }`, or just its cells; `base` its style when
// it has none), the rows it takes with its margins, and the columns it spans.
fn row(ctx: &Ctx, v: &Value, base: Style) -> (Row<'static>, u16, usize) {
    let (cells, o) = if v.is_array() { (v, &Value::Null) } else { (&v["cells"], v) };
    let mut cols = 0;
    let mut tallest = 1;
    let cells: Vec<Cell> = cells
        .as_array()
        .into_iter()
        .flatten()
        .map(|x| {
            let (cell, span, h) = cell(ctx, x);
            cols += span as usize;
            tallest = tallest.max(h);
            cell
        })
        .collect();
    let small = |k: &str| o[k].as_u64().map(|n| n.min(u16::MAX as u64) as u16);
    let height = small("height").unwrap_or(tallest.min(u16::MAX as usize) as u16);
    let (top, bottom) = (small("top_margin").unwrap_or(0), small("bottom_margin").unwrap_or(0));
    let r = Row::new(cells).style(ctx.style_or(o, "style", base)).height(height).top_margin(top).bottom_margin(bottom);
    (r, top + height + bottom, cols)
}

#[allow(clippy::too_many_arguments)]
fn table(ctx: &Ctx, c: &mut Canvas, n: &Value, key: &str, area: Rect, elems: &mut Elems, d: &mut Draw, on: bool) {
    let rows: Vec<(Row, u16, usize)> = n["rows"].as_array().into_iter().flatten().map(|r| row(ctx, r, Style::new())).collect();
    let header = n.get("header").filter(|h| !h.is_null()).map(|h| row(ctx, h, Style::new().add_modifier(Modifier::BOLD)));
    let footer = n.get("footer").filter(|h| !h.is_null()).map(|h| row(ctx, h, Style::new().fg(ctx.tone("dim"))));
    let cols = rows.iter().chain(&header).chain(&footer).map(|r| r.2).max().unwrap_or(0);
    let widths: Vec<Constraint> = match n["widths"].as_array() {
        Some(w) => w.iter().map(|v| ui::constraint(v).unwrap_or(Constraint::Fill(1))).collect(),
        None => vec![Constraint::Fill(1); cols],
    };
    let mode = n["select"].as_str().unwrap_or("row");
    let gap = n["column_spacing"].as_u64().unwrap_or(1).min(u16::MAX as u64) as u16;
    let flex = ui::flex(&n["flex"]).unwrap_or(Flex::Start);
    let sel = ctx.selection(on);
    let (row_hl, col_hl, cell_hl) = match mode {
        "cell" => (Style::new().bg(ctx.tint("focus", 0.07)), Style::new(), sel.add_modifier(Modifier::BOLD)),
        "column" => (Style::new(), sel, sel),
        _ => (sel, Style::new(), sel),
    };
    let heights: Vec<u16> = rows.iter().map(|r| r.1).collect();
    let total: usize = heights.iter().map(|h| *h as usize).sum();
    let (above, below) = (header.as_ref().map_or(0, |h| h.1), footer.as_ref().map_or(0, |f| f.1));
    let head = above + below;
    let mut t = Table::new(rows.into_iter().map(|r| r.0), widths.clone())
        .column_spacing(gap)
        .flex(flex)
        .row_highlight_style(ctx.style_or(n, "row_highlight_style", row_hl))
        .column_highlight_style(ctx.style_or(n, "column_highlight_style", col_hl))
        .cell_highlight_style(ctx.style_or(n, "cell_highlight_style", cell_hl))
        .highlight_spacing(spacing(&n["highlight_spacing"]));
    if let Some(h) = header {
        t = t.header(h.0);
    }
    if let Some(f) = footer {
        t = t.footer(f.0);
    }
    let symbol = n["highlight_symbol"].as_str().unwrap_or("");
    if !symbol.is_empty() {
        t = t.highlight_symbol(symbol.to_string());
    }
    let shown = area.height.saturating_sub(head) as usize;
    let (body, bar) = bar_room(n, area, total + head as usize, true);
    let st = elems.entry(key.to_string()).or_default();
    StatefulWidget::render(t, body, c.buf, &mut st.table);
    st.shown = shown;
    // a hit for each cell that shows (row << 8 | column), where ratatui put the columns
    let room = match spacing(&n["highlight_spacing"]) {
        HighlightSpacing::Always => true,
        HighlightSpacing::WhenSelected => st.table.selected().is_some(),
        HighlightSpacing::Never => false,
    };
    let sel_w = if room { width(symbol) as u16 } else { 0 };
    let columns = Layout::horizontal(widths).flex(flex).spacing(gap).split(Rect::new(body.x + sel_w.min(body.width), body.y, body.width.saturating_sub(sel_w), 1));
    let first = st.table.offset();
    let mut y = body.y + above;
    let bottom = body.bottom().saturating_sub(below);
    for (i, h) in heights.iter().enumerate().skip(first) {
        if y >= bottom {
            break;
        }
        let h = (*h).min(bottom - y);
        for (k, col) in columns.iter().enumerate() {
            hit(c, d, key, Rect { y, height: h, ..*col }, (i as i32) << 8 | k.min(255) as i32);
        }
        y += h;
    }
    if bar {
        scrollbar(ctx, c.buf, area, total, shown, heights[..first.min(heights.len())].iter().map(|h| *h as usize).sum());
    }
}

// the cursor's row and column, each when there is one (selecting rows, there's no column; columns, no row)
fn table_holds(st: &ElState) -> Value {
    let mut v = json!({});
    for (k, at) in [("row", st.table.selected()), ("column", st.table.selected_column())] {
        if let Some(at) = at {
            v[k] = json!(at);
        }
    }
    v
}

fn table_key(n: &Value, st: &mut ElState, name: &str) -> Took {
    let mode = n["select"].as_str().unwrap_or("row");
    let rows = len(&n["rows"]);
    let cols = n["rows"].as_array().into_iter().flatten().chain(n.get("header")).map(|r| r.as_array().or(r["cells"].as_array()).map_or(0, |c| c.iter().map(|x| x["span"].as_u64().unwrap_or(1) as usize).sum())).max().unwrap_or(0);
    if name == "enter" {
        if mode == "none" || (st.table.selected().is_none() && st.table.selected_column().is_none()) {
            return None;
        }
        return Some(fired(n, "action", table_holds(st)).into_iter().collect());
    }
    let before = (st.table.selected(), st.table.selected_column());
    let page = st.list_page();
    match (mode, name) {
        ("cell" | "column", "left" | "right") => {
            let to = if name == "left" { "up" } else { "down" };
            let c = moved(to, st.table.selected_column(), cols, 1)?;
            st.table.select_column(Some(c));
        }
        ("column" | "none", _) => {
            // nothing to choose down it: the keys scroll it
            let to = moved(name, Some(st.table.offset()), rows, page)?;
            *st.table.offset_mut() = to;
            return Some(vec![]);
        }
        _ => {
            let r = moved(name, st.table.selected(), rows, page)?;
            st.table.select(Some(r));
        }
    }
    if (st.table.selected(), st.table.selected_column()) == before {
        return Some(vec![]);
    }
    Some(fired(n, "change", table_holds(st)).into_iter().collect())
}

// ---------- tabs ----------

#[allow(clippy::too_many_arguments)]
fn tabs(ctx: &Ctx, c: &mut Canvas, n: &Value, key: &str, area: Rect, elems: &mut Elems, d: &mut Draw, on: bool) {
    let titles: Vec<Line> = n["titles"].as_array().into_iter().flatten().map(|t| ctx.line(t)).collect();
    let count = titles.len();
    let divider = if n["divider"].is_null() { Span::raw("│") } else { ctx.span(&n["divider"]) };
    let pad = |i: usize| n["padding"].get(i).map_or(Line::raw(" "), |p| Line::from(ctx.span(p)));
    let (left, right) = (pad(0), pad(1));
    let st = elems.entry(key.to_string()).or_default();
    st.tab = st.tab.min(count.saturating_sub(1));
    let hl = Style::new().fg(ctx.tone(if on { "focus" } else { "fg" })).add_modifier(Modifier::BOLD);
    let widths: Vec<u16> = titles.iter().map(|t| (left.width() + t.width() + right.width()) as u16).collect();
    Tabs::new(titles)
        .select(st.tab)
        .style(Style::new().fg(ctx.tone("dim")))
        .highlight_style(ctx.style_or(n, "highlight_style", hl))
        .divider(divider.clone())
        .padding(left, right)
        .render(area, c.buf);
    // each title (its padding with it) takes clicks
    let mut x = area.x;
    for (i, w) in widths.into_iter().enumerate() {
        let w = w.min(area.right().saturating_sub(x));
        hit(c, d, key, Rect { x, width: w, height: 1, ..area }, i as i32);
        x = x.saturating_add(w + divider.width() as u16);
        if x >= area.right() {
            break;
        }
    }
}

// The tab the user picked: its `change` and `action`.
fn pick_tab(n: &Value, st: &mut ElState, to: usize) -> Vec<Fired> {
    if to >= len(&n["titles"]) || to == st.tab {
        return vec![];
    }
    st.tab = to;
    ["change", "action"].iter().filter_map(|e| fired(n, e, json!({ "index": to }))).collect()
}

fn tabs_key(n: &Value, st: &mut ElState, name: &str) -> Took {
    let to = match name {
        "left" => st.tab.saturating_sub(1),
        "right" => st.tab + 1,
        "enter" => return Some(fired(n, "action", json!({ "index": st.tab })).into_iter().collect()),
        _ => return None,
    };
    Some(pick_tab(n, st, to))
}

// ---------- what elements do with keys and the pointer ----------

// An element was used: its plugin's action, with the element's params and what it holds.
pub struct Fired {
    pub action: String,
    pub params: Value,
    pub ui: Map<String, Value>,
}

// An element's `event` (action, change or toggle) with what it holds; nothing when the plugin gave it no action for it.
pub fn fired(n: &Value, event: &str, holds: Value) -> Option<Fired> {
    let action = n[event].as_str()?.to_string();
    let mut ui = Map::new();
    if let Some(id) = n["id"].as_str() {
        ui.insert("id".into(), json!(id));
    }
    ui.insert("event".into(), json!(event));
    if let Value::Object(o) = holds {
        ui.extend(o);
    }
    Some(Fired { action, params: n.get("params").filter(|p| p.is_object()).cloned().unwrap_or_else(|| json!({})), ui })
}

// None: the element doesn't use this key; Some: it took it, and these actions run.
pub type Took = Option<Vec<Fired>>;

// A key for the element with the keyboard (fields have their own, in fields.rs).
pub fn element_key(n: &Value, st: &mut ElState, name: &str) -> Took {
    match ty(n) {
        "list" => list_key(n, st, name),
        "table" => table_key(n, st, name),
        "tabs" => tabs_key(n, st, name),
        "tree" => tree::key(n, st, name),
        "diff" => code::diff_key(n, st, name),
        "text" | "code" | "markdown" => scroll_key(n, st, name).then(Vec::new),
        "button" if matches!(name, "enter" | "space") => Some(fired(n, "action", json!({})).into_iter().collect()),
        _ => None,
    }
}

// Scrolling a text, code, a diff without a cursor, or Markdown; ← → move code and diffs that don't wrap sideways.
pub fn scroll_key(n: &Value, st: &mut ElState, name: &str) -> bool {
    let most = st.rows.saturating_sub(st.shown);
    let page = st.shown.max(1);
    let sideways = matches!(ty(n), "code" | "diff") && n["wrap"] != true;
    st.scroll = match name {
        "up" | "k" => st.scroll.saturating_sub(1),
        "down" | "j" => (st.scroll + 1).min(most),
        "pageup" => st.scroll.saturating_sub(page),
        "pagedown" => (st.scroll + page).min(most),
        "home" => 0,
        "end" => most,
        "left" if sideways => {
            st.hscroll = st.hscroll.saturating_sub(8);
            return true;
        }
        "right" if sideways => {
            st.hscroll += 8;
            return true;
        }
        _ => return false,
    };
    st.follow = n["scroll"] == "bottom" && st.scroll == most;
    true
}

// The wheel over an element: what scrolls scrolls, and what's chosen in moves (a notch an item). None: it doesn't.
pub fn wheel(n: &Value, st: &mut ElState, rows: i32) -> Took {
    let name = if rows < 0 { "up" } else { "down" };
    match ty(n) {
        "list" | "table" | "tree" => element_key(n, st, name),
        "text" if !scrolls(n) => None,
        "text" | "code" | "markdown" | "diff" => {
            for _ in 0..rows.unsigned_abs() {
                scroll_key(n, st, name);
            }
            Some(vec![])
        }
        _ => None,
    }
}

// A click on part of an element: a list's item, a table's cell (row << 8 | column), a tab, a tree's row, a diff's line;
// `double` when it's the second on the same part in a moment.
pub fn click(n: &Value, st: &mut ElState, part: i32, double: bool) -> Vec<Fired> {
    if part < 0 {
        return match ty(n) {
            "button" => fired(n, "action", json!({})).into_iter().collect(),
            _ => vec![],
        };
    }
    let part = part as usize;
    match ty(n) {
        "list" => {
            let mut out = choose_item(n, st, part);
            if double {
                out.extend(fired(n, "action", json!({ "index": part })));
            }
            out
        }
        "table" => {
            let (r, col) = (part >> 8, part & 255);
            let before = (st.table.selected(), st.table.selected_column());
            match n["select"].as_str().unwrap_or("row") {
                "none" => return vec![],
                "cell" => st.table.select_cell(Some((r, col))),
                "column" => st.table.select_column(Some(col)),
                _ => st.table.select(Some(r)),
            }
            let mut out = vec![];
            if (st.table.selected(), st.table.selected_column()) != before {
                out.extend(fired(n, "change", table_holds(st)));
            }
            if double {
                out.extend(fired(n, "action", table_holds(st)));
            }
            out
        }
        "tabs" => pick_tab(n, st, part),
        "tree" => tree::click(n, st, part, double),
        "diff" => code::diff_click(n, st, part),
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_keep_what_the_user_did_until_the_plugin_changes_it() {
        let v1 = json!({ "type": "layout", "children": [{ "type": "input", "id": "note", "value": "a" }, { "type": "list", "id": "pick", "items": ["x", "y"], "change": "moved" }] });
        let mut states = reconcile(&mut HashMap::new(), &v1);
        assert_eq!(focusables(&v1), [("#note".to_string(), Focus::Field), ("#pick".to_string(), Focus::Pick)]);
        states.get_mut("#note").unwrap().input = Input::new("ab".into());
        let f = element_key(&v1["children"][1], states.get_mut("#pick").unwrap(), "down").unwrap();
        assert_eq!(json!(f[0].ui), json!({ "id": "pick", "event": "change", "index": 1 }));
        let mut kept = reconcile(&mut states, &v1);
        assert_eq!(kept["#note"].input.value(), "ab");
        assert_eq!(kept["#pick"].list.selected(), Some(1));
        let v2 = json!({ "type": "layout", "children": [{ "type": "input", "id": "note", "value": "new" }, { "type": "list", "id": "pick", "selected": 0, "items": ["x"] }] });
        let next = reconcile(&mut kept, &v2);
        assert_eq!(next["#note"].input.value(), "new");
        assert_eq!(next["#pick"].list.selected(), Some(0));
    }

    #[test]
    fn moves_over_things_and_stops_at_the_ends() {
        assert_eq!(moved("down", None, 3, 10), Some(0));
        assert_eq!(moved("down", Some(2), 3, 10), Some(2));
        assert_eq!(moved("k", Some(0), 3, 10), Some(0));
        assert_eq!(moved("pagedown", Some(0), 30, 10), Some(10));
        assert_eq!(moved("end", Some(0), 30, 10), Some(29));
        assert_eq!(moved("x", Some(0), 30, 10), None);
        assert_eq!(moved("down", None, 0, 10), None);
    }

    #[test]
    fn ansi_keeps_only_colours() {
        let t = ansi("\x1b[31mred\x1b[0m \x1b[2Jplain\x1b]0;title\x07\r\n\x1b[1mbold");
        assert_eq!(t.lines.len(), 2);
        assert_eq!(t.lines[0].spans.iter().map(|s| s.content.as_ref()).collect::<String>(), "red plain");
        assert_eq!(t.lines[0].spans[0].style.fg, Some(Color::Red));
        assert!(t.lines[1].spans[0].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn a_table_action_says_where() {
        let n = json!({ "type": "table", "id": "t", "select": "cell", "rows": [["a", "b"], ["c", "d"]], "action": "go", "change": "moved" });
        let mut st = reconcile(&mut HashMap::new(), &n).remove("#t").unwrap();
        let f = element_key(&n, &mut st, "right").unwrap();
        assert_eq!(json!(f[0].ui), json!({ "id": "t", "event": "change", "row": 0, "column": 1 }));
        element_key(&n, &mut st, "down");
        let f = element_key(&n, &mut st, "enter").unwrap();
        assert_eq!(json!(f[0].ui), json!({ "id": "t", "event": "action", "row": 1, "column": 1 }));
    }
}
