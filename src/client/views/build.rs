// A plugin view's elements, laid out and drawn in the user's theme. The TypeScript client built OpenTUI renderables
// and kept them across updates; this one draws the element tree whole every frame, and keeps what the user did in it
// (what's typed, chosen and scrolled to) in `ElState`s by element key. An update keeps the user's text, choice and scroll
// when the plugin didn't change those itself.
//
// Layout is flexbox, as OpenTUI's (Yoga) was: row/column, gap, padding, grow/shrink, sizes in cells or %, min/max,
// align/justify and wrap. ponytail: a compact subset — one pass of grow/shrink (no re-distribution once a child hits
// its min or max), wrap only for rows, no align-self, and a percentage height measures as content until its parent's
// size is known.
use std::collections::HashMap;

use ratatui::layout::Position;
use ratatui::style::{Modifier, Style as CellStyle};
use serde_json::{json, Map, Value};
use unicode_segmentation::UnicodeSegmentation;

use super::charts::{self, tone_name, Grid, Ink};
use crate::client::design::{agent_mark, color, fit, mix};
use crate::client::draw::{fit_from, Canvas, Hit};
use crate::config::themes::Theme;
use crate::config::BorderStyle;
use crate::core::layout::Rect;
use crate::core::text::{grapheme_width, width};
use crate::platform::logos::Loaded;

// What drawing needs from the client: the theme, how agents' marks are drawn, and the animation clock.
pub struct Ctx<'a> {
    pub th: &'a Theme,
    pub logos: Option<Loaded>,
    pub cell: f64,
    pub tick: u64, // spinner frames since the client started
}

pub fn tone(th: &Theme, t: Option<&str>, fallback: &'static str) -> &'static str {
    match t {
        Some("fg") => th.fg,
        Some("dim") => th.dim,
        Some("accent") => th.accent,
        Some("warn") => th.warn,
        Some("working") => th.working,
        Some("blocked") => th.blocked,
        Some("done") => th.done,
        Some("idle") => th.idle,
        _ => fallback,
    }
}
fn track(th: &Theme) -> String {
    mix(th.bg, th.dim, 0.3)
}

// ---------- what the user did in an element, kept across updates ----------

#[derive(Clone, Debug, Default)]
pub struct ElState {
    pub ty: String,    // the element type it's for: another type at the same key starts afresh
    pub prop: Value,   // the plugin's value / selected / diff as last seen: when it changes, the user's goes
    pub text: Vec<char>, // an input's or textarea's text
    pub cursor: usize, // the caret in it
    pub index: usize,  // a select's or tabs' choice; a diff's line cursor
    pub scroll: i32,   // rows scrolled
    pub stuck: bool,   // a sticky scroll area at its edge: it stays there as content grows
    pub viewport: i32, // the last frame's rows shown…
    pub content: i32,  // …and rows there are
}

// An element's key: its own (`#key`), else its place under its parent.
pub fn key_of(n: &Value, path: &str) -> String {
    n["key"].as_str().map(|k| format!("#{k}")).unwrap_or_else(|| path.to_string())
}
fn ty(n: &Value) -> &str {
    n["type"].as_str().unwrap_or("")
}
pub fn children(n: &Value) -> &[Value] {
    match ty(n) {
        "box" | "scroll" => n["children"].as_array().map(Vec::as_slice).unwrap_or(&[]),
        _ => &[],
    }
}
// every element, depth first as drawn, with its key
pub fn walk<'a>(n: &'a Value, key: String, f: &mut impl FnMut(&'a Value, &str)) {
    f(n, &key);
    for (i, c) in children(n).iter().enumerate() {
        walk(c, key_of(c, &format!("{key}.{i}")), f);
    }
}

// What Tab moves between: a field, a list, a button, or something to scroll.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Focus {
    Field,
    List,
    Button,
    Scroll,
}

pub fn focus_kind(t: &str) -> Option<Focus> {
    match t {
        "input" | "textarea" => Some(Focus::Field),
        "select" | "tabs" => Some(Focus::List),
        "button" => Some(Focus::Button),
        "scroll" | "code" | "diff" | "markdown" => Some(Focus::Scroll),
        _ => None,
    }
}

pub fn focusables(root: &Value) -> Vec<(String, Focus)> {
    let mut out = vec![];
    walk(root, key_of(root, "0"), &mut |n, k| {
        if let Some(f) = focus_kind(ty(n)) {
            out.push((k.to_string(), f));
        }
    });
    out
}

fn options(n: &Value) -> &[Value] {
    n["options"].as_array().map(Vec::as_slice).unwrap_or(&[])
}

// The states for a new tree: an element keeps the user's, unless the plugin changed what that came from.
pub fn reconcile(old: &mut HashMap<String, ElState>, root: &Value) -> HashMap<String, ElState> {
    let mut next = HashMap::new();
    walk(root, key_of(root, "0"), &mut |n, k| {
        let t = ty(n);
        if !matches!(t, "input" | "textarea" | "select" | "tabs" | "scroll" | "code" | "diff" | "markdown") {
            return;
        }
        let kept = old.remove(k).filter(|s| s.ty == t);
        let fresh = kept.is_none();
        let mut st = kept.unwrap_or_else(|| ElState { ty: t.to_string(), ..Default::default() });
        match t {
            "input" | "textarea" => {
                let prop = n["value"].clone();
                if fresh || st.prop != prop {
                    st.text = prop.as_str().unwrap_or("").chars().collect();
                    st.cursor = st.text.len();
                }
                st.prop = prop;
            }
            "select" | "tabs" => {
                let prop = n["selected"].clone();
                if fresh || st.prop != prop {
                    st.index = prop.as_u64().unwrap_or(0) as usize;
                }
                st.index = st.index.min(options(n).len().saturating_sub(1));
                st.prop = prop;
            }
            "diff" => {
                let prop = n["diff"].clone();
                if fresh || st.prop != prop {
                    st.index = 0;
                    st.scroll = 0;
                }
                st.index = st.index.min(diff_body(prop.as_str().unwrap_or("")).len().saturating_sub(1));
                st.prop = prop;
            }
            "scroll" if fresh => st.stuck = n["sticky"].is_string(),
            _ => {}
        }
        next.insert(k.to_string(), st);
    });
    next
}

// ---------- layout ----------

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Dim {
    Cells(i32),
    Pct(f64),
}

fn dim_of(v: &Value) -> Option<Dim> {
    if let Some(n) = v.as_f64() {
        return Some(Dim::Cells(n.max(0.0) as i32));
    }
    v.as_str()?.strip_suffix('%')?.trim().parse::<f64>().ok().map(Dim::Pct)
}

fn resolve(d: Option<Dim>, total: Option<i32>) -> Option<i32> {
    match d? {
        Dim::Cells(n) => Some(n),
        Dim::Pct(p) => total.map(|t| ((t as f64 * p) / 100.0).floor() as i32),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Align {
    Start,
    Center,
    End,
    Stretch,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Justify {
    Start,
    Center,
    End,
    Between,
    Around,
    Evenly,
}

#[derive(Clone, Debug)]
pub struct Style {
    pub row: bool,
    pub gap: i32,
    pub pad: [i32; 4], // top, right, bottom, left
    pub border: bool,
    pub align: Align,
    pub justify: Justify,
    pub wrap: bool,
    pub width: Option<Dim>,
    pub height: Option<Dim>,
    pub min_w: Option<i32>,
    pub max_w: Option<i32>,
    pub min_h: Option<i32>,
    pub max_h: Option<i32>,
    pub grow: f64,
    pub shrink: f64,
}

// An element's kind's defaults, under its own layout (`Size` in the original).
#[derive(Default, Clone)]
struct Size {
    width: Option<Dim>,
    height: Option<Dim>,
    grow: Option<f64>,
    shrink: Option<f64>,
    min_w: Option<i32>,
    min_h: Option<i32>,
}

// Something a line tall that fills its parent: across a row it takes the room left; down a column it's stretched wide.
fn line(row: bool) -> Size {
    if row { Size { grow: Some(1.0), shrink: Some(1.0), min_w: Some(4), ..Default::default() } } else { Size::default() }
}
// Something that takes the room left in its parent, whichever way that runs.
fn area() -> Size {
    Size { grow: Some(1.0), shrink: Some(1.0), min_h: Some(3), min_w: Some(4), ..Default::default() }
}

fn int(v: &Value) -> Option<i32> {
    v.as_f64().map(|n| n as i32)
}

// An element's own layout over its kind's defaults; `fixed` overrides both (what the original forced: a button is a
// line tall, a raster its cells).
fn style_of(n: &Value, size: Size, fixed: Size) -> Style {
    let own_w = dim_of(&n["width"]);
    let own_h = dim_of(&n["height"]);
    let mut grow = size.grow;
    if (own_w.is_some() || own_h.is_some()) && grow.is_some() {
        grow = None; // a size it was given is the size it gets
    }
    let width = fixed.width.or(own_w).or(size.width);
    let height = fixed.height.or(own_h).or(size.height);
    let grow = n["grow"].as_f64().or(grow).unwrap_or(0.0);
    let explicit = matches!(width, Some(Dim::Cells(_))) || matches!(height, Some(Dim::Cells(_)));
    let shrink = fixed.shrink.or(n["shrink"].as_f64()).or(size.shrink).unwrap_or(if explicit { 0.0 } else { 1.0 });
    let mut st = Style {
        row: false,
        gap: 0,
        pad: [0; 4],
        border: false,
        align: Align::Stretch,
        justify: Justify::Start,
        wrap: false,
        width,
        height,
        min_w: int(&n["minWidth"]).or(size.min_w),
        max_w: int(&n["maxWidth"]),
        min_h: int(&n["minHeight"]).or(size.min_h),
        max_h: int(&n["maxHeight"]),
        grow,
        shrink,
    };
    if ty(n) == "box" {
        st.row = n["direction"] == "row";
        st.gap = int(&n["gap"]).unwrap_or(0).max(0);
        let p = int(&n["padding"]).unwrap_or(0);
        let (px, py) = (int(&n["paddingX"]).unwrap_or(p), int(&n["paddingY"]).unwrap_or(p));
        st.pad = [py, px, py, px];
        st.border = n["border"].as_bool().unwrap_or(false) || n["border"].is_string();
        st.align = match n["align"].as_str() {
            Some("start") => Align::Start,
            Some("center") => Align::Center,
            Some("end") => Align::End,
            _ => Align::Stretch,
        };
        st.justify = match n["justify"].as_str() {
            Some("center") => Justify::Center,
            Some("end") => Justify::End,
            Some("between") => Justify::Between,
            Some("around") => Justify::Around,
            Some("evenly") => Justify::Evenly,
            _ => Justify::Start,
        };
        st.wrap = n["wrap"].as_bool().unwrap_or(false);
    }
    st
}

fn cells(n: i32) -> Option<Dim> {
    Some(Dim::Cells(n))
}

// The element tree for one frame: each element's style, and (after `place`) where it is.
pub struct El<'a> {
    pub n: &'a Value,
    pub ty: &'a str,
    pub key: String,
    pub st: Style,
    pub kids: Vec<El<'a>>,
    pub rect: Rect,
}

pub fn tree(n: &Value, key: String, row_parent: bool) -> El<'_> {
    let t = ty(n);
    let none = Size::default;
    let (size, fixed) = match t {
        "scroll" | "diff" => (area(), none()),
        "progress" | "sparkline" => (Size { height: cells(1), ..line(row_parent) }, none()),
        "chart" => (Size { height: cells(6), ..line(row_parent) }, none()),
        "gauge" => (Size { width: cells(14), height: cells(5), shrink: Some(0.0), ..none() }, none()),
        "heatmap" => {
            let rows = n["values"].as_array().map(Vec::len).unwrap_or(0);
            let w = n["values"].as_array().into_iter().flatten().filter_map(Value::as_array).map(Vec::len).max().unwrap_or(0);
            (Size { width: cells(w as i32), height: cells(rows.div_ceil(2) as i32), shrink: Some(0.0), ..none() }, none())
        }
        "raster" => (none(), Size { width: int(&n["columns"]).and_then(cells), height: int(&n["rows"]).and_then(cells), shrink: Some(0.0), ..none() }),
        "image" => (Size { width: cells(40), height: cells(12), shrink: Some(0.0), ..none() }, none()),
        "button" => (none(), Size { height: cells(1), shrink: Some(0.0), ..none() }),
        "input" | "tabs" => (line(row_parent), Size { height: cells(1), ..none() }),
        "textarea" => (Size { height: cells(5), ..line(row_parent) }, none()),
        "select" => {
            let described = options(n).iter().any(|o| o["description"].as_str().is_some_and(|d| !d.is_empty()));
            let rows = options(n).len().clamp(1, 12) as i32 * if described { 2 } else { 1 };
            (Size { height: cells(rows), ..line(row_parent) }, none())
        }
        _ => (none(), none()),
    };
    let st = style_of(n, size, fixed);
    let row = t == "box" && st.row;
    let kids = children(n).iter().enumerate().map(|(i, c)| tree(c, key_of(c, &format!("{key}.{i}")), row)).collect();
    El { n, ty: t, key, st, kids, rect: Rect::default() }
}

fn clamp_w(st: &Style, w: i32) -> i32 {
    let w = st.max_w.map_or(w, |m| w.min(m));
    st.min_w.map_or(w, |m| w.max(m)).max(0)
}
fn clamp_h(st: &Style, h: i32) -> i32 {
    let h = st.max_h.map_or(h, |m| h.min(m));
    st.min_h.map_or(h, |m| h.max(m)).max(0)
}
// what a box's border and padding take: across, and down
fn frame_of(st: &Style) -> (i32, i32) {
    let b = st.border as i32 * 2;
    (st.pad[1] + st.pad[3] + b, st.pad[0] + st.pad[2] + b)
}
fn inner_of(el: &El) -> Rect {
    let b = el.st.border as i32;
    let r = el.rect;
    let x = r.x + b + el.st.pad[3];
    let y = r.y + b + el.st.pad[0];
    Rect { x, y, w: (r.w - frame_of(&el.st).0).max(0), h: (r.h - frame_of(&el.st).1).max(0) }
}

// The width an element would like, with no limit.
pub fn content_w(ctx: &Ctx, el: &El) -> i32 {
    if let Some(Dim::Cells(w)) = el.st.width {
        return clamp_w(&el.st, w);
    }
    let inner = match el.ty {
        "box" if el.st.row => el.kids.iter().map(|k| content_w(ctx, k)).sum::<i32>() + el.st.gap * (el.kids.len() as i32 - 1).max(0),
        "box" | "scroll" => el.kids.iter().map(|k| content_w(ctx, k)).max().unwrap_or(0),
        _ => leaf_w(ctx, el),
    };
    clamp_w(&el.st, inner + frame_of(&el.st).0)
}

// The height an element would like at width `w`.
pub fn content_h(ctx: &Ctx, el: &El, w: i32) -> i32 {
    if let Some(Dim::Cells(h)) = el.st.height {
        return clamp_h(&el.st, h);
    }
    let (fx, fy) = frame_of(&el.st);
    let iw = (w - fx).max(0);
    let inner = match el.ty {
        "box" if el.st.row => row_lines(ctx, &el.kids, iw, el.st.gap, el.st.wrap).iter().map(|l| l.iter().map(|(i, w)| content_h(ctx, &el.kids[*i], *w)).max().unwrap_or(0)).sum::<i32>() + el.st.gap * (row_lines(ctx, &el.kids, iw, el.st.gap, el.st.wrap).len() as i32 - 1).max(0),
        "box" => column_h(ctx, &el.kids, iw, el.st.gap, el.st.align),
        "scroll" => column_h(ctx, &el.kids, iw, 0, Align::Stretch),
        _ => leaf_h(ctx, el, iw),
    };
    clamp_h(&el.st, inner + fy)
}

fn cross_w(ctx: &Ctx, k: &El, iw: i32, align: Align) -> i32 {
    match resolve(k.st.width, Some(iw)) {
        Some(w) => clamp_w(&k.st, w),
        None if align == Align::Stretch => clamp_w(&k.st, iw),
        None => clamp_w(&k.st, content_w(ctx, k).min(iw)),
    }
}

fn column_h(ctx: &Ctx, kids: &[El], iw: i32, gap: i32, align: Align) -> i32 {
    kids.iter().map(|k| content_h(ctx, k, cross_w(ctx, k, iw, align))).sum::<i32>() + gap * (kids.len() as i32 - 1).max(0)
}

struct Item {
    basis: f64,
    grow: f64,
    shrink: f64,
    min: i32,
    max: Option<i32>,
}

// The sizes a line gives its items along it: their bases, then the room left shared by grow, or the room missing
// taken by shrink (weighted by basis, as Yoga does).
fn distribute(items: &[&Item], room: i32, gap: i32) -> Vec<i32> {
    if items.is_empty() {
        return vec![];
    }
    let used: f64 = items.iter().map(|i| i.basis).sum::<f64>() + (gap * (items.len() as i32 - 1)) as f64;
    let free = room as f64 - used;
    let mut sizes: Vec<f64> = items.iter().map(|i| i.basis).collect();
    if free > 0.0 {
        let g: f64 = items.iter().map(|i| i.grow).sum();
        if g > 0.0 {
            for (s, i) in sizes.iter_mut().zip(items) {
                *s += free * i.grow / g;
            }
        }
    } else if free < 0.0 {
        let total: f64 = items.iter().map(|i| i.shrink * i.basis).sum();
        if total > 0.0 {
            for (s, i) in sizes.iter_mut().zip(items) {
                *s += free * i.shrink * i.basis / total;
            }
        }
    }
    // whole cells, rounded where they end so the line's total doesn't drift
    let (mut acc, mut prev, mut out) = (0.0, 0, vec![]);
    for (s, i) in sizes.iter().zip(items) {
        let s = i.max.map_or(*s, |m| s.min(m as f64)).max(i.min as f64);
        acc += s;
        let end = acc.round() as i32;
        out.push((end - prev).max(0));
        prev = end;
    }
    out
}

fn main_item(ctx: &Ctx, k: &El, row: bool, inner: Rect, cross: i32) -> Item {
    let (basis, min, max) = if row {
        (resolve(k.st.width, Some(inner.w)).map(|w| clamp_w(&k.st, w)).unwrap_or_else(|| content_w(ctx, k)), k.st.min_w, k.st.max_w)
    } else {
        (resolve(k.st.height, Some(inner.h)).map(|h| clamp_h(&k.st, h)).unwrap_or_else(|| content_h(ctx, k, cross)), k.st.min_h, k.st.max_h)
    };
    Item { basis: basis as f64, grow: k.st.grow, shrink: k.st.shrink, min: min.unwrap_or(0), max }
}

// A row's children in lines (one, unless it wraps), each with the width it gets.
fn row_lines(ctx: &Ctx, kids: &[El], iw: i32, gap: i32, wrap: bool) -> Vec<Vec<(usize, i32)>> {
    let inner = Rect { x: 0, y: 0, w: iw, h: 0 };
    let items: Vec<Item> = kids.iter().map(|k| main_item(ctx, k, true, inner, 0)).collect();
    split_lines(&items, iw, gap, wrap).into_iter().map(|l| { let its: Vec<&Item> = l.iter().map(|i| &items[*i]).collect(); l.iter().copied().zip(distribute(&its, iw, gap)).collect() }).collect()
}

fn split_lines(items: &[Item], room: i32, gap: i32, wrap: bool) -> Vec<Vec<usize>> {
    if !wrap {
        return vec![(0..items.len()).collect()];
    }
    let mut lines: Vec<Vec<usize>> = vec![vec![]];
    let mut used = 0.0;
    for (i, it) in items.iter().enumerate() {
        let cur = lines.last_mut().unwrap();
        let need = it.basis + if cur.is_empty() { 0.0 } else { gap as f64 };
        if !cur.is_empty() && used + need > room as f64 {
            lines.push(vec![i]);
            used = it.basis;
        } else {
            cur.push(i);
            used += need;
        }
    }
    lines
}

// Lay `kids` out in `inner` along a row or a column.
#[allow(clippy::too_many_arguments)]
fn flex(ctx: &Ctx, kids: &mut [El], inner: Rect, row: bool, gap: i32, align: Align, justify: Justify, wrap: bool, elems: &mut HashMap<String, ElState>) {
    if kids.is_empty() {
        return;
    }
    let main = if row { inner.w } else { inner.h };
    // a column's children's widths first: their heights depend on them
    let widths: Vec<i32> = kids.iter().map(|k| if row { 0 } else { cross_w(ctx, k, inner.w, align) }).collect();
    let items: Vec<Item> = kids.iter().zip(&widths).map(|(k, w)| main_item(ctx, k, row, inner, *w)).collect();
    let lines = split_lines(&items, main, gap, wrap && row);
    let single = lines.len() == 1;
    let mut cross_at = if row { inner.y } else { inner.x };
    for l in lines {
        let its: Vec<&Item> = l.iter().map(|i| &items[*i]).collect();
        let sizes = distribute(&its, main, gap);
        // each one's size across the line, and the line's
        let natural: Vec<i32> = l
            .iter()
            .zip(&sizes)
            .map(|(&i, &s)| {
                let k = &kids[i];
                if row {
                    resolve(k.st.height, Some(inner.h)).map(|h| clamp_h(&k.st, h)).unwrap_or_else(|| content_h(ctx, k, s))
                } else {
                    widths[i]
                }
            })
            .collect();
        let line_cross = if single { if row { inner.h } else { inner.w } } else { natural.iter().copied().max().unwrap_or(0) };
        let crosses: Vec<i32> = l
            .iter()
            .zip(&natural)
            .map(|(&i, &n)| {
                let k = &kids[i];
                let fixed = if row { k.st.height.is_some() } else { true };
                if row && align == Align::Stretch && !fixed { clamp_h(&k.st, line_cross) } else { n }
            })
            .collect();
        let used: i32 = sizes.iter().sum::<i32>() + gap * (l.len() as i32 - 1).max(0);
        let left = (main - used).max(0) as f64;
        let n = l.len() as f64;
        let (mut at, between) = match justify {
            Justify::Start => (0.0, 0.0),
            Justify::Center => (left / 2.0, 0.0),
            Justify::End => (left, 0.0),
            Justify::Between => (0.0, if n > 1.0 { left / (n - 1.0) } else { 0.0 }),
            Justify::Around => (left / n / 2.0, left / n),
            Justify::Evenly => (left / (n + 1.0), left / (n + 1.0)),
        };
        for ((&i, &size), &cross) in l.iter().zip(&sizes).zip(&crosses) {
            let off = match align {
                Align::Start | Align::Stretch => 0,
                Align::Center => (line_cross - cross) / 2,
                Align::End => line_cross - cross,
            };
            let m = at.round() as i32;
            let rect = if row { Rect { x: inner.x + m, y: cross_at + off, w: size, h: cross } } else { Rect { x: cross_at + off, y: inner.y + m, w: cross, h: size } };
            place(ctx, &mut kids[i], rect, elems);
            at += size as f64 + gap as f64 + between;
        }
        cross_at += line_cross + gap;
    }
}

// Put an element at `rect`, and its children inside it.
pub fn place(ctx: &Ctx, el: &mut El, rect: Rect, elems: &mut HashMap<String, ElState>) {
    el.rect = rect;
    let inner = inner_of(el);
    match el.ty {
        "box" => {
            let st = el.st.clone();
            flex(ctx, &mut el.kids, inner, st.row, st.gap, st.align, st.justify, st.wrap, elems);
        }
        "scroll" => {
            // its content: a column as tall as it needs, scrolled; a bar on the right when it doesn't all show
            let mut content = column_h(ctx, &el.kids, inner.w, 0, Align::Stretch);
            let over = content > inner.h;
            let w = if over { (inner.w - 1).max(0) } else { inner.w };
            if over {
                content = column_h(ctx, &el.kids, w, 0, Align::Stretch);
            }
            let sticky = el.n["sticky"].as_str().unwrap_or("");
            let st = elems.entry(el.key.clone()).or_default();
            let most = (content - inner.h).max(0);
            if st.stuck {
                st.scroll = if sticky == "bottom" { most } else { 0 };
            }
            st.scroll = st.scroll.clamp(0, most);
            st.viewport = inner.h;
            st.content = content;
            let top = inner.y - st.scroll;
            flex(ctx, &mut el.kids, Rect { x: inner.x, y: top, w, h: content }, false, 0, Align::Stretch, Justify::Start, false, elems);
        }
        _ => {}
    }
}

// Lay a view's root out in its frame: the frame is a column, the root its one child.
pub fn layout<'a>(ctx: &Ctx, root: &'a Value, inner: Rect, elems: &mut HashMap<String, ElState>) -> El<'a> {
    let mut el = tree(root, key_of(root, "0"), false);
    flex(ctx, std::slice::from_mut(&mut el), inner, false, 0, Align::Stretch, Justify::Start, false, elems);
    el
}

// ---------- text ----------

// A run of text in one style.
#[derive(Clone, Debug, PartialEq)]
pub struct Seg {
    pub text: String,
    pub fg: String,
    pub bg: Option<String>,
    pub m: Modifier,
}

fn seg(text: impl Into<String>, fg: &str, m: Modifier) -> Seg {
    Seg { text: text.into(), fg: fg.to_string(), bg: None, m }
}

#[derive(Clone, Copy, Default)]
struct Look {
    tone: Option<&'static str>,
    bold: bool,
    italic: bool,
    underline: bool,
    dim: bool,
    strike: bool,
}

impl Look {
    fn of(self, n: &Value) -> Look {
        let mut l = self;
        if let Some(t) = n["tone"].as_str() {
            l.tone = Some(tone_name(t));
        }
        for (k, f) in [("bold", &mut l.bold), ("italic", &mut l.italic), ("underline", &mut l.underline), ("dim", &mut l.dim), ("strike", &mut l.strike)] {
            if let Some(b) = n[k].as_bool() {
                *f = b;
            }
        }
        l
    }
    fn modifier(&self) -> Modifier {
        let mut m = Modifier::empty();
        for (on, x) in [(self.bold, Modifier::BOLD), (self.italic, Modifier::ITALIC), (self.underline, Modifier::UNDERLINED), (self.dim, Modifier::DIM), (self.strike, Modifier::CROSSED_OUT)] {
            if on {
                m |= x;
            }
        }
        m
    }
}

fn tabs(s: &str) -> String {
    s.replace('\t', "    ")
}

// Inline content (strings, spans, agents' marks) as runs of text, styles inherited down.
fn inline(ctx: &Ctx, xs: &Value, look: Look, out: &mut Vec<Seg>) {
    for x in xs.as_array().into_iter().flatten() {
        if let Some(s) = x.as_str() {
            out.push(seg(tabs(s), tone(ctx.th, look.tone, ctx.th.fg), look.modifier()));
        } else if x["type"] == "icon" {
            let mark = agent_mark(ctx.th, x["agent"].as_str().unwrap_or(""), ctx.logos, ctx.cell);
            out.push(seg(format!("{}{}", mark.glyph, " ".repeat(mark.cells.saturating_sub(2))), &mark.color, Modifier::empty()));
        } else {
            inline(ctx, &x["children"], look.of(x), out);
        }
    }
}

fn text_segs(ctx: &Ctx, n: &Value) -> Vec<Seg> {
    let mut out = vec![];
    inline(ctx, &n["children"], Look::default().of(n), &mut out);
    out
}

// Runs of text as lines `width` cells wide: at spaces ("word", a word longer than a line broken where it runs out),
// anywhere ("char"), or only at newlines ("none").
pub fn wrap(segs: &[Seg], width: i32, mode: &str) -> Vec<Vec<Seg>> {
    let mut hard: Vec<Vec<(&str, usize)>> = vec![vec![]];
    for (i, s) in segs.iter().enumerate() {
        for g in s.text.graphemes(true) {
            if g == "\n" || g == "\r\n" {
                hard.push(vec![]);
            } else {
                hard.last_mut().unwrap().push((g, i));
            }
        }
    }
    let mut lines: Vec<Vec<(&str, usize)>> = vec![];
    for l in hard {
        if mode == "none" || width <= 0 {
            lines.push(l);
            continue;
        }
        let room = width as usize;
        let (mut cur, mut used): (Vec<(&str, usize)>, usize) = (vec![], 0);
        for (g, i) in l {
            let w = grapheme_width(g);
            if used + w > room && !cur.is_empty() {
                if mode == "word" && g == " " {
                    lines.push(std::mem::take(&mut cur));
                    used = 0;
                    continue;
                }
                match cur.iter().rposition(|(g, _)| *g == " ").filter(|_| mode == "word") {
                    Some(sp) => {
                        let rest = cur.split_off(sp + 1);
                        cur.pop();
                        lines.push(std::mem::replace(&mut cur, rest));
                        used = cur.iter().map(|(g, _)| grapheme_width(g)).sum();
                    }
                    None => {
                        lines.push(std::mem::take(&mut cur));
                        used = 0;
                    }
                }
            }
            cur.push((g, i));
            used += w;
        }
        lines.push(cur);
    }
    lines
        .into_iter()
        .map(|l| {
            let mut out: Vec<Seg> = vec![];
            let mut last = usize::MAX;
            for (g, i) in l {
                if i == last {
                    out.last_mut().unwrap().text.push_str(g);
                } else {
                    out.push(Seg { text: g.to_string(), ..segs[i].clone() });
                    last = i;
                }
            }
            out
        })
        .collect()
}

fn widest(segs: &[Seg]) -> i32 {
    let all: String = segs.iter().map(|s| s.text.as_str()).collect();
    all.split('\n').map(|l| width(l) as i32).max().unwrap_or(0)
}

// ---------- Markdown ----------

// ponytail: a small Markdown: headings, lists, quotes, rules, fenced code, and **bold**, *italic*, `code` and
// [links](…) inline; no tables or nesting inside emphasis
fn md_inline(ctx: &Ctx, s: &str, fg: &str, m: Modifier, out: &mut Vec<Seg>) {
    let th = ctx.th;
    let chars: Vec<char> = s.chars().collect();
    let find = |from: usize, pat: &[char]| (from..chars.len()).find(|&j| chars[j..].starts_with(pat));
    let mut plain = String::new();
    let mut i = 0;
    let flush = |plain: &mut String, out: &mut Vec<Seg>| {
        if !plain.is_empty() {
            out.push(seg(std::mem::take(plain), fg, m));
        }
    };
    while i < chars.len() {
        let c = chars[i];
        let piece = |a: usize, b: usize| chars[a..b].iter().collect::<String>();
        if c == '`' {
            if let Some(j) = find(i + 1, &['`']) {
                flush(&mut plain, out);
                out.push(seg(piece(i + 1, j), th.done, m));
                i = j + 1;
                continue;
            }
        }
        if chars[i..].starts_with(&['*', '*']) {
            if let Some(j) = find(i + 2, &['*', '*']).filter(|j| *j > i + 2) {
                flush(&mut plain, out);
                out.push(seg(piece(i + 2, j), fg, m | Modifier::BOLD));
                i = j + 2;
                continue;
            }
        }
        if (c == '*' || c == '_') && chars.get(i + 1).is_some_and(|n| !n.is_whitespace()) {
            if let Some(j) = find(i + 1, &[c]).filter(|j| *j > i + 1) {
                flush(&mut plain, out);
                out.push(seg(piece(i + 1, j), fg, m | Modifier::ITALIC));
                i = j + 1;
                continue;
            }
        }
        if c == '[' {
            if let Some(j) = find(i + 1, &[']', '(']) {
                if let Some(e) = find(j + 2, &[')']) {
                    flush(&mut plain, out);
                    out.push(seg(piece(i + 1, j), th.focus, m | Modifier::UNDERLINED));
                    i = e + 1;
                    continue;
                }
            }
        }
        plain.push(c);
        i += 1;
    }
    flush(&mut plain, out);
}

fn markdown(ctx: &Ctx, content: &str) -> Vec<Seg> {
    let th = ctx.th;
    let mut lines: Vec<Vec<Seg>> = vec![];
    let mut fenced = false;
    for raw in content.lines() {
        let raw = tabs(raw);
        let t = raw.trim_start();
        let mut l = vec![];
        if t.starts_with("```") {
            fenced = !fenced; // the fences themselves are concealed
            continue;
        }
        if fenced {
            l.push(seg(raw.clone(), th.done, Modifier::empty()));
        } else if let Some(h) = t.strip_prefix('#').map(|h| h.trim_start_matches('#')).filter(|h| h.is_empty() || h.starts_with(' ')) {
            md_inline(ctx, h.trim(), th.accent, Modifier::BOLD, &mut l);
        } else if let Some(q) = t.strip_prefix('>') {
            l.push(seg("│ ", th.dim, Modifier::empty()));
            md_inline(ctx, q.trim_start(), th.dim, Modifier::ITALIC, &mut l);
        } else if matches!(t, "---" | "***" | "___") {
            l.push(seg("───", th.dim, Modifier::empty()));
        } else if let Some((marker, rest)) = list_item(t) {
            let indent = raw.len() - t.len();
            l.push(seg(format!("{}{marker} ", " ".repeat(indent)), th.accent, Modifier::empty()));
            md_inline(ctx, rest, th.fg, Modifier::empty(), &mut l);
        } else {
            md_inline(ctx, &raw, th.fg, Modifier::empty(), &mut l);
        }
        lines.push(l);
    }
    let mut out = vec![];
    for (i, l) in lines.into_iter().enumerate() {
        if i > 0 {
            out.push(seg("\n", th.fg, Modifier::empty()));
        }
        out.extend(l);
    }
    out
}

fn list_item(t: &str) -> Option<(&str, &str)> {
    let (marker, rest) = t.split_once(' ')?;
    let ordered = marker.len() > 1 && marker.ends_with('.') && marker[..marker.len() - 1].bytes().all(|b| b.is_ascii_digit());
    (matches!(marker, "-" | "*" | "+") || ordered).then_some((marker, rest))
}

// ---------- code and diffs ----------

// A diff's body lines (+, -, context) in order, as the unified view draws them: row i is body line i.
pub fn diff_body(diff: &str) -> Vec<&str> {
    let mut out = vec![];
    let mut in_hunk = false;
    for l in diff.split('\n') {
        if l.starts_with("@@") {
            in_hunk = true;
        } else if l.starts_with("diff --git ") {
            in_hunk = false;
        } else if in_hunk && (l.starts_with('+') || l.starts_with('-') || l.starts_with(' ')) {
            out.push(l);
        }
    }
    out
}

// The body with each line's numbers: (old, new, line); a hunk header says where its numbers start.
fn diff_numbered(diff: &str) -> Vec<(Option<usize>, Option<usize>, &str)> {
    let mut out = vec![];
    let (mut old, mut new, mut in_hunk) = (1, 1, false);
    for l in diff.split('\n') {
        if let Some(h) = l.strip_prefix("@@") {
            in_hunk = true;
            let num = |sign: char| h.split_whitespace().find_map(|p| p.strip_prefix(sign)).and_then(|p| p.split(',').next()).and_then(|n| n.parse().ok()).unwrap_or(1);
            (old, new) = (num('-'), num('+'));
        } else if l.starts_with("diff --git ") {
            in_hunk = false;
        } else if in_hunk && l.starts_with('+') {
            out.push((None, Some(new), l));
            new += 1;
        } else if in_hunk && l.starts_with('-') {
            out.push((Some(old), None, l));
            old += 1;
        } else if in_hunk && l.starts_with(' ') {
            out.push((Some(old), Some(new), l));
            (old, new) = (old + 1, new + 1);
        }
    }
    out
}

// The split view's rows: context on both sides, a run of removed lines beside the run of added lines after it.
fn diff_split(diff: &str) -> Vec<(Option<(usize, &str)>, Option<(usize, &str)>)> {
    let body = diff_numbered(diff);
    let mut out = vec![];
    let mut i = 0;
    while i < body.len() {
        let (o, n, l) = body[i];
        if l.starts_with(' ') {
            out.push((o.map(|o| (o, l)), n.map(|n| (n, l))));
            i += 1;
            continue;
        }
        let mut gone = vec![];
        while i < body.len() && body[i].2.starts_with('-') {
            gone.push((body[i].0.unwrap_or(0), body[i].2));
            i += 1;
        }
        let mut came = vec![];
        while i < body.len() && body[i].2.starts_with('+') {
            came.push((body[i].1.unwrap_or(0), body[i].2));
            i += 1;
        }
        for k in 0..gone.len().max(came.len()) {
            out.push((gone.get(k).copied(), came.get(k).copied()));
        }
    }
    out
}

fn digits(n: usize) -> usize {
    n.max(1).to_string().len()
}

fn code_gutter(n: &Value) -> i32 {
    if n["lineNumbers"].as_bool() == Some(true) { digits(n["content"].as_str().unwrap_or("").lines().count()) as i32 + 2 } else { 0 }
}

fn diff_gutter(diff: &str) -> i32 {
    let most = diff_numbered(diff).iter().map(|(o, n, _)| o.unwrap_or(0).max(n.unwrap_or(0))).max().unwrap_or(1);
    digits(most) as i32 + 1
}

// ---------- tables and big text ----------

fn table_cells(ctx: &Ctx, n: &Value) -> Vec<Vec<Vec<Seg>>> {
    let header = n["header"].as_bool().unwrap_or(false);
    n["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(i, row)| {
            let look = Look { bold: header && i == 0, ..Default::default() };
            row.as_array()
                .into_iter()
                .flatten()
                .map(|c| {
                    let mut out = vec![];
                    match c.as_str() {
                        Some(s) => inline(ctx, &json!([s]), look, &mut out),
                        None => inline(ctx, c, look, &mut out),
                    }
                    out
                })
                .collect()
        })
        .collect()
}

// Each column's width, padding included: what its widest cell needs, all of them narrowed to fit `room`.
// ponytail: cells are cut to their column, not wrapped
fn table_widths(cells: &[Vec<Vec<Seg>>], border: bool, room: Option<i32>) -> Vec<i32> {
    let cols = cells.iter().map(Vec::len).max().unwrap_or(0);
    let mut ws: Vec<i32> = (0..cols).map(|c| cells.iter().filter_map(|r| r.get(c)).map(|s| widest(s)).max().unwrap_or(0) + 2).collect();
    if let Some(room) = room {
        let lines = if border { cols as i32 + 1 } else { 0 };
        let total: i32 = ws.iter().sum::<i32>() + lines;
        if total > room && total > lines {
            let have = (room - lines).max(cols as i32 * 3) as f64;
            let sum = (total - lines) as f64;
            for w in ws.iter_mut() {
                *w = ((*w as f64 * have / sum).floor() as i32).max(3);
            }
        }
    }
    ws
}

// The `tiny` font, two rows a letter (cfonts', as OpenTUI drew it).
// ponytail: every font is drawn as tiny; the others (block, shade, slick, huge, grid, pallet) are bigger versions of it
fn tiny(c: char) -> [&'static str; 2] {
    match c.to_ascii_uppercase() {
        'A' => ["▄▀█", "█▀█"],
        'B' => ["█▄▄", "█▄█"],
        'C' => ["█▀▀", "█▄▄"],
        'D' => ["█▀▄", "█▄▀"],
        'E' => ["█▀▀", "██▄"],
        'F' => ["█▀▀", "█▀ "],
        'G' => ["█▀▀", "█▄█"],
        'H' => ["█ █", "█▀█"],
        'I' => ["█", "█"],
        'J' => ["  █", "█▄█"],
        'K' => ["█▄▀", "█ █"],
        'L' => ["█  ", "█▄▄"],
        'M' => ["█▀▄▀█", "█ ▀ █"],
        'N' => ["█▄ █", "█ ▀█"],
        'O' => ["█▀█", "█▄█"],
        'P' => ["█▀█", "█▀▀"],
        'Q' => ["█▀█", "▀▀█"],
        'R' => ["█▀█", "█▀▄"],
        'S' => ["█▀▀", "▄▄█"],
        'T' => ["▀█▀", " █ "],
        'U' => ["█ █", "█▄█"],
        'V' => ["█ █", "▀▄▀"],
        'W' => ["█ █ █", "▀▄▀▄▀"],
        'X' => ["▀▄▀", "█ █"],
        'Y' => ["█▄█", " █ "],
        'Z' => ["▀█", "█▄"],
        '0' => ["▞█▚", "▚█▞"],
        '1' => ["▄█", " █"],
        '2' => ["▀█", "█▄"],
        '3' => ["▀▀█", "▄██"],
        '4' => ["█ █", "▀▀█"],
        '5' => ["█▀", "▄█"],
        '6' => ["█▄▄", "█▄█"],
        '7' => ["▀▀█", "  █"],
        '8' => ["███", "█▄█"],
        '9' => ["█▀█", "▀▀█"],
        '!' => ["█", "▄"],
        '?' => ["▀█", " ▄"],
        '.' => [" ", "▄"],
        '+' => ["▄█▄", " ▀ "],
        '-' => ["▄▄", "  "],
        '_' => ["  ", "▄▄"],
        '=' => ["▀▀", "▀▀"],
        '@' => ["▛█▜", "▙▟▃"],
        '#' => ["▟▄▙", "▜▀▛"],
        '$' => ["▖█▗", "▘█▝"],
        '%' => ["▀ ▄▀", "▄▀ ▄"],
        '&' => ["▄▄█", "█▄█"],
        '(' => ["▄▀", "▀▄"],
        ')' => ["▀▄", "▄▀"],
        '/' => ["  ▄▀", "▄▀  "],
        ':' => ["▀", "▄"],
        ';' => ["  ", "▄▀"],
        ',' => [" ", "█"],
        '\'' => ["▀", " "],
        '"' => ["▛ ▜", "   "],
        _ => [" ", " "],
    }
}

fn bigtext(text: &str) -> [String; 2] {
    let mut rows = [String::new(), String::new()];
    for (i, c) in text.chars().enumerate() {
        let g = tiny(c);
        for (r, part) in rows.iter_mut().zip(g) {
            if i > 0 {
                r.push(' ');
            }
            r.push_str(part);
        }
    }
    rows
}

// ---------- measuring leaves ----------

fn leaf_w(ctx: &Ctx, el: &El) -> i32 {
    let n = el.n;
    match el.ty {
        "text" => widest(&text_segs(ctx, n)),
        "markdown" => widest(&markdown(ctx, n["content"].as_str().unwrap_or(""))),
        "code" => code_gutter(n) + n["content"].as_str().unwrap_or("").lines().map(|l| width(&tabs(l)) as i32).max().unwrap_or(0),
        "diff" => {
            let d = n["diff"].as_str().unwrap_or("");
            diff_gutter(d) + 2 + diff_body(d).iter().map(|l| width(&tabs(&l[1..])) as i32).max().unwrap_or(0)
        }
        "table" => {
            let cells = table_cells(ctx, n);
            let border = n["border"].as_bool().unwrap_or(true);
            let ws = table_widths(&cells, border, None);
            ws.iter().sum::<i32>() + if border { ws.len() as i32 + 1 } else { 0 }
        }
        "bigtext" => width(&bigtext(n["text"].as_str().unwrap_or(""))[0]) as i32,
        "spinner" => 1 + n["label"].as_str().map_or(0, |l| 1 + width(l) as i32),
        "button" => width(n["label"].as_str().unwrap_or("")) as i32 + 2,
        "select" => options(n).iter().map(|o| 3 + width(o["name"].as_str().unwrap_or("")).max(width(o["description"].as_str().unwrap_or(""))) as i32).max().unwrap_or(0) + 1,
        "tabs" => options(n).len() as i32 * tab_width(n),
        _ => 0,
    }
}

fn leaf_h(ctx: &Ctx, el: &El, w: i32) -> i32 {
    let n = el.n;
    match el.ty {
        "text" => wrap(&text_segs(ctx, n), w, n["wrap"].as_str().unwrap_or("word")).len() as i32,
        "markdown" => wrap(&markdown(ctx, n["content"].as_str().unwrap_or("")), w, "word").len() as i32,
        "code" => n["content"].as_str().unwrap_or("").lines().count().max(1) as i32,
        "diff" => {
            let d = n["diff"].as_str().unwrap_or("");
            (if split_view(n) { diff_split(d).len() } else { diff_body(d).len() }) as i32
        }
        "table" => {
            let rows = n["rows"].as_array().map(Vec::len).unwrap_or(0) as i32;
            if n["border"].as_bool().unwrap_or(true) { rows * 2 + 1 } else { rows }
        }
        "bigtext" => 2,
        "spinner" => 1,
        _ => 0,
    }
}

fn split_view(n: &Value) -> bool {
    n["view"] == "split" && n["cursor"].as_bool() != Some(true)
}

fn tab_width(n: &Value) -> i32 {
    options(n).iter().map(|o| width(o["name"].as_str().unwrap_or("")) as i32 + 4).max().unwrap_or(0).clamp(8, 30)
}

// ---------- drawing ----------

pub fn intersect(a: Rect, b: Rect) -> Rect {
    let (x, y) = (a.x.max(b.x), a.y.max(b.y));
    let (r, btm) = ((a.x + a.w).min(b.x + b.w), (a.y + a.h).min(b.y + b.h));
    Rect { x, y, w: (r - x).max(0), h: (btm - y).max(0) }
}

// `s` at (x, y), only what's inside `clip`; returns the cells it takes, drawn or not.
pub fn put(c: &mut Canvas, clip: Rect, x: i32, y: i32, s: &str, fg: &str, bg: Option<&str>, m: Modifier) -> i32 {
    let w = width(s) as i32;
    if y < clip.y || y >= clip.y + clip.h || x >= clip.x + clip.w || x + w <= clip.x {
        return w;
    }
    let x0 = x.max(clip.x);
    let skip = (x0 - x) as usize;
    let shown = if skip > 0 { fit_from(s, skip) } else { s.to_string() };
    c.text(x0, y, &shown, fg, bg, m, (clip.x + clip.w - x0).max(0) as usize);
    w
}

fn put_segs(c: &mut Canvas, clip: Rect, x: i32, y: i32, segs: &[Seg], bg: Option<&str>) -> i32 {
    let mut at = x;
    for s in segs {
        at += put(c, clip, at, y, &s.text, &s.fg, s.bg.as_deref().or(bg), s.m);
    }
    at - x
}

fn fill(c: &mut Canvas, clip: Rect, r: Rect, bg: &str) {
    let r = intersect(r, clip);
    if r.w > 0 && r.h > 0 {
        c.fill(r, bg);
    }
}

fn put_cell(c: &mut Canvas, clip: Rect, x: i32, y: i32, ch: char, fg: &str, bg: &str) {
    if x < clip.x || y < clip.y || x >= clip.x + clip.w || y >= clip.y + clip.h || x < 0 || y < 0 || x >= c.w || y >= c.h {
        return;
    }
    if let Some(cell) = c.buf.cell_mut(Position::new(x as u16, y as u16)) {
        cell.reset();
        cell.set_char(ch);
        cell.set_style(CellStyle::new().fg(color(fg)).bg(color(bg)));
    }
}

// A box's border, clipped, with its title one cell in from the corner.
pub fn border(c: &mut Canvas, clip: Rect, r: Rect, style: BorderStyle, fg: &str, bg: Option<&str>, title: Option<(&str, &str)>) {
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
    put(c, clip, r.x, r.y, &format!("{tl}{}{tr}", hz.repeat(n)), fg, bg, Modifier::empty());
    put(c, clip, r.x, r.y + r.h - 1, &format!("{bl}{}{br}", hz.repeat(n)), fg, bg, Modifier::empty());
    for y in r.y + 1..r.y + r.h - 1 {
        put(c, clip, r.x, y, vt, fg, bg, Modifier::empty());
        put(c, clip, r.x + r.w - 1, y, vt, fg, bg, Modifier::empty());
    }
    if let Some((t, col)) = title.filter(|(t, _)| !t.is_empty() && r.w > 4) {
        put(c, intersect(clip, Rect { x: r.x + 2, y: r.y, w: r.w - 4, h: 1 }), r.x + 2, r.y, &fit(t, (r.w - 4) as usize), col, bg, Modifier::empty());
    }
}

fn ink(ctx: &Ctx, i: Option<&Ink>, fallback: &str) -> String {
    let th = ctx.th;
    let Some(i) = i else { return fallback.to_string() };
    if let Some(rgb) = &i.rgb {
        return rgb.clone();
    }
    if i.tone == "track" {
        return track(th);
    }
    let c = tone(th, Some(i.tone), th.fg);
    match i.mix {
        Some(m) => mix(&track(th), c, m),
        None => c.to_string(),
    }
}

// A chart's or raster's cells at (x, y). A cell with no ink of its own is in the text colour on the background.
// (The original painted an inkless foreground in the background colour too, which hid a Raster's default text.)
fn paint(ctx: &Ctx, c: &mut Canvas, clip: Rect, r: Rect, grid: &Grid) {
    for (y, row) in grid.iter().enumerate().take(r.h.max(0) as usize) {
        for (x, cell) in row.iter().enumerate().take(r.w.max(0) as usize) {
            let fg = ink(ctx, cell.fg.as_ref(), ctx.th.fg);
            let bg = ink(ctx, cell.bg.as_ref(), ctx.th.bg);
            put_cell(c, clip, r.x + x as i32, r.y + y as i32, cell.ch, &fg, &bg);
        }
    }
}

// A Raster's cells decoded: what plugins paint themselves. See RASTER in protocol/types.ts.
fn raster(cells: &str, columns: usize, rows: usize) -> Grid {
    const DEFAULT: u32 = 0x0100_0000;
    const TONE: u32 = 0x0200_0000;
    let bytes = crate::protocol::conn::unb64(cells);
    let word = |i: usize| bytes.get(i * 4..i * 4 + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let ink = |v: u32| -> Option<Ink> {
        if v == DEFAULT {
            None
        } else if v & TONE != 0 {
            Some(Ink::tone(charts::TONES.get((v & 0xff) as usize).copied().unwrap_or("fg")))
        } else {
            Some(Ink { tone: "fg", mix: None, rgb: Some(format!("#{:06x}", v & 0xff_ffff)) })
        }
    };
    (0..rows)
        .map(|y| {
            (0..columns)
                .map(|x| {
                    let i = (y * columns + x) * 3;
                    let cp = word(i).unwrap_or(0x20);
                    charts::Cell { ch: if cp >= 0x20 { char::from_u32(cp).unwrap_or(' ') } else { ' ' }, fg: ink(word(i + 1).unwrap_or(DEFAULT)), bg: ink(word(i + 2).unwrap_or(DEFAULT)) }
                })
                .collect()
        })
        .collect()
}

const SPIN: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

// What a frame's drawing needs to know and tells back: whose view it is, what has the keyboard, and where the caret is.
pub struct Draw<'a> {
    pub view: &'a str,
    pub focus: Option<&'a str>,
    pub active: bool, // this view has the keyboard
    pub cursor: Option<Position>,
}

fn hit(c: &mut Canvas, clip: Rect, r: Rect, d: &Draw, key: &str, part: i32) {
    let r = intersect(r, clip);
    if r.w > 0 && r.h > 0 {
        c.hit(r, Hit::View { view: d.view.to_string(), key: key.to_string(), part });
    }
}

// a part of a hit that's the element itself, and one that's an area to scroll
pub const WHOLE: i32 = -1;
pub const SCROLLS: i32 = -3;

pub fn draw(ctx: &Ctx, c: &mut Canvas, el: &El, clip: Rect, elems: &mut HashMap<String, ElState>, d: &mut Draw) {
    let th = ctx.th;
    let n = el.n;
    let r = el.rect;
    if intersect(r, clip).w == 0 && el.ty != "box" {
        return;
    }
    let on = d.active && d.focus == Some(el.key.as_str());
    let key = el.key.as_str();
    match el.ty {
        "box" => {
            if let Some(bg) = n["bg"].as_str() {
                fill(c, clip, r, &mix(th.bg, tone(th, Some(bg), th.fg), 0.15));
            }
            if el.st.border {
                let style = match n["border"].as_str() {
                    Some("single") => BorderStyle::Single,
                    Some("double") => BorderStyle::Double,
                    Some("heavy") => BorderStyle::Heavy,
                    _ => BorderStyle::Rounded,
                };
                let title = n["title"].as_str().map(|t| format!(" {t} "));
                let tc = tone(th, n["tone"].as_str(), th.dim);
                border(c, clip, r, style, tone(th, n["tone"].as_str(), th.border), None, title.as_deref().map(|t| (t, tc)));
            }
            for k in &el.kids {
                draw(ctx, c, k, clip, elems, d);
            }
        }
        "scroll" => {
            let inner = inner_of(el);
            hit(c, clip, r, d, key, SCROLLS);
            let inside = intersect(clip, inner);
            for k in &el.kids {
                draw(ctx, c, k, inside, elems, d);
            }
            let st = elems.get(key).cloned().unwrap_or_default();
            if let Some((top, size)) = thumb(st.content, st.viewport, st.scroll, inner.h) {
                for y in 0..inner.h {
                    let on_thumb = y >= top && y < top + size;
                    put(c, clip, inner.x + inner.w - 1, inner.y + y, if on_thumb { "▐" } else { " " }, th.border, None, Modifier::empty());
                }
            }
        }
        "text" => {
            for (i, l) in wrap(&text_segs(ctx, n), r.w, n["wrap"].as_str().unwrap_or("word")).iter().enumerate().take(r.h.max(0) as usize) {
                put_segs(c, intersect(clip, r), r.x, r.y + i as i32, l, None);
            }
        }
        "markdown" => {
            let lines = wrap(&markdown(ctx, n["content"].as_str().unwrap_or("")), r.w, "word");
            let top = scrolled(elems, key, lines.len() as i32, r.h);
            hit(c, clip, r, d, key, SCROLLS);
            for (i, l) in lines.iter().skip(top as usize).enumerate().take(r.h.max(0) as usize) {
                put_segs(c, intersect(clip, r), r.x, r.y + i as i32, l, None);
            }
        }
        // ponytail: code and diffs are plain text in the theme's colours, not syntax-highlighted
        "code" => {
            let content = n["content"].as_str().unwrap_or("");
            let lines: Vec<&str> = content.lines().collect();
            let gutter = code_gutter(n);
            let top = scrolled(elems, key, lines.len() as i32, r.h);
            hit(c, clip, r, d, key, SCROLLS);
            let box_clip = intersect(clip, r);
            for (i, l) in lines.iter().enumerate().skip(top as usize).take(r.h.max(0) as usize) {
                let y = r.y + i as i32 - top;
                if gutter > 0 {
                    put(c, box_clip, r.x, y, &format!("{:>w$} ", i + 1, w = (gutter - 2) as usize), th.dim, None, Modifier::empty());
                }
                put(c, box_clip, r.x + gutter, y, &tabs(l), th.fg, None, Modifier::empty());
            }
        }
        "diff" => draw_diff(ctx, c, el, clip, elems, d),
        "table" => {
            let cells = table_cells(ctx, n);
            let b = n["border"].as_bool().unwrap_or(true);
            let ws = table_widths(&cells, b, Some(r.w));
            let box_clip = intersect(clip, r);
            let line = |l: &str, m: &str, rr: &str| format!("{l}{}{rr}", ws.iter().map(|w| "─".repeat(*w as usize)).collect::<Vec<_>>().join(m));
            let mut y = r.y;
            if b {
                put(c, box_clip, r.x, y, &line("╭", "┬", "╮"), th.border, None, Modifier::empty());
                y += 1;
            }
            for (i, row) in cells.iter().enumerate() {
                let mut x = r.x;
                for (k, w) in ws.iter().enumerate() {
                    if b {
                        x += put(c, box_clip, x, y, "│", th.border, None, Modifier::empty());
                    }
                    if let Some(cell) = row.get(k) {
                        let room = (w - 2).max(0);
                        let cut = wrap(cell, room, "none").into_iter().next().unwrap_or_default();
                        let cl = intersect(box_clip, Rect { x: x + 1, y, w: room, h: 1 });
                        put_segs(c, cl, x + 1, y, &cut, None);
                    }
                    x += w;
                }
                if b {
                    put(c, box_clip, x, y, "│", th.border, None, Modifier::empty());
                    y += 1;
                    let last = i + 1 == cells.len();
                    put(c, box_clip, r.x, y, &if last { line("╰", "┴", "╯") } else { line("├", "┼", "┤") }, th.border, None, Modifier::empty());
                }
                y += 1;
            }
        }
        "bigtext" => {
            let col = tone(th, n["tone"].as_str(), th.accent);
            for (i, l) in bigtext(n["text"].as_str().unwrap_or("")).iter().enumerate() {
                put(c, intersect(clip, r), r.x, r.y + i as i32, l, col, None, Modifier::empty());
            }
        }
        "progress" | "sparkline" | "chart" | "gauge" | "heatmap" => {
            let t = tone_name(n["tone"].as_str().unwrap_or("accent"));
            let nums = |v: &Value| v.as_array().into_iter().flatten().map(|x| x.as_f64().unwrap_or(f64::NAN)).collect::<Vec<f64>>();
            let (w, h) = (r.w.max(0) as usize, r.h.max(0) as usize);
            let grid = match el.ty {
                "progress" => vec![charts::progress(n["value"].as_f64().unwrap_or(0.0), w, t)],
                "sparkline" => charts::columns(&nums(&n["values"]), w, h, t, n["min"].as_f64().unwrap_or(0.0), n["max"].as_f64()),
                "chart" => {
                    let series: Vec<(Vec<f64>, &'static str)> = n["series"].as_array().into_iter().flatten().map(|s| (nums(&s["values"]), tone_name(s["tone"].as_str().unwrap_or("accent")))).collect();
                    charts::lines(&series, w, h, n["min"].as_f64(), n["max"].as_f64())
                }
                "gauge" => charts::gauge(n["value"].as_f64().unwrap_or(0.0), w, h, t, n["label"].as_str()),
                _ => charts::heatmap(&n["values"].as_array().into_iter().flatten().map(nums).collect::<Vec<_>>(), t, n["min"].as_f64().unwrap_or(0.0), n["max"].as_f64()),
            };
            paint(ctx, c, clip, r, &grid);
        }
        "raster" => {
            let grid = raster(n["cells"].as_str().unwrap_or(""), n["columns"].as_u64().unwrap_or(0) as usize, n["rows"].as_u64().unwrap_or(0) as usize);
            paint(ctx, c, clip, r, &grid);
        }
        // ponytail: an image is its alt text; decoding PNG takes a crate this build doesn't have
        "image" => {
            put(c, intersect(clip, r), r.x, r.y, n["alt"].as_str().unwrap_or("[image]"), th.dim, None, Modifier::empty());
        }
        "spinner" => {
            let at = r.x + put(c, intersect(clip, r), r.x, r.y, &SPIN[(ctx.tick % 10) as usize].to_string(), tone(th, n["tone"].as_str(), th.working), None, Modifier::empty());
            if let Some(l) = n["label"].as_str() {
                put(c, intersect(clip, r), at, r.y, &format!(" {l}"), th.dim, None, Modifier::empty());
            }
        }
        "button" => {
            let t = tone(th, n["tone"].as_str(), th.accent);
            let bg = if on { t.to_string() } else { mix(th.bg, t, 0.15) };
            fill(c, clip, r, &bg);
            let label = format!(" {} ", n["label"].as_str().unwrap_or(""));
            put(c, intersect(clip, r), r.x, r.y, &label, if on { th.bg } else { t }, Some(&bg), if on { Modifier::BOLD } else { Modifier::empty() });
            hit(c, clip, r, d, key, WHOLE);
        }
        "input" => {
            let st = elems.entry(key.to_string()).or_default();
            let bg = if on { mix(th.bar, th.focus, 0.12) } else { th.bar.to_string() };
            fill(c, clip, r, &bg);
            let box_clip = intersect(clip, r);
            if st.text.is_empty() {
                put(c, box_clip, r.x, r.y, n["placeholder"].as_str().unwrap_or(""), th.dim, Some(&bg), Modifier::empty());
            }
            // the caret stays in view: what's typed scrolls left as it grows past the field
            let before = width(&st.text[..st.cursor].iter().collect::<String>()) as i32;
            let skip = (before - r.w + 1).max(0);
            let text: String = st.text.iter().collect();
            put(c, box_clip, r.x - skip, r.y, &text, th.fg, Some(&bg), Modifier::empty());
            if on {
                d.cursor = in_screen(c, r.x + before - skip, r.y);
            }
            hit(c, clip, r, d, key, WHOLE);
        }
        "textarea" => {
            let bg = if on { mix(th.bar, th.focus, 0.12) } else { th.bar.to_string() };
            fill(c, clip, r, &bg);
            let box_clip = intersect(clip, r);
            let st = elems.entry(key.to_string()).or_default();
            let (rows, at) = textarea_rows(&st.text, st.cursor, r.w.max(1) as usize);
            if at.0 < st.scroll as usize {
                st.scroll = at.0 as i32;
            } else if at.0 as i32 >= st.scroll + r.h {
                st.scroll = at.0 as i32 - r.h + 1;
            }
            st.viewport = r.h;
            if st.text.is_empty() {
                put(c, box_clip, r.x, r.y, n["placeholder"].as_str().unwrap_or(""), th.dim, Some(&bg), Modifier::empty());
            }
            for (i, l) in rows.iter().enumerate().skip(st.scroll as usize).take(r.h.max(0) as usize) {
                put(c, box_clip, r.x, r.y + i as i32 - st.scroll, l, th.fg, Some(&bg), Modifier::empty());
            }
            if on {
                d.cursor = in_screen(c, r.x + at.1 as i32, r.y + at.0 as i32 - st.scroll);
            }
            hit(c, clip, r, d, key, WHOLE);
        }
        "select" => draw_select(ctx, c, el, clip, elems, d, on),
        "tabs" => {
            let st = elems.entry(key.to_string()).or_default();
            let opts = options(n);
            let tw = tab_width(n);
            let visible = (r.w / tw).max(1) as usize;
            let first = (st.index as i64 - (visible / 2) as i64).min(opts.len() as i64 - visible as i64).max(0) as usize;
            let box_clip = intersect(clip, r);
            fill(c, clip, r, th.bg);
            for (i, o) in opts.iter().enumerate().skip(first).take(visible) {
                let x = r.x + (i - first) as i32 * tw;
                if x >= r.x + r.w {
                    break;
                }
                let w = tw.min(r.x + r.w - x);
                let selected = i == st.index;
                let bg = if selected { mix(th.bg, th.focus, 0.22) } else { th.bg.to_string() };
                fill(c, box_clip, Rect { x, y: r.y, w, h: 1 }, &bg);
                let name = o["name"].as_str().unwrap_or("");
                let fg = if selected || on { th.fg } else { th.dim };
                put(c, box_clip, x + 1, r.y, &fit(name, (w - 2).max(0) as usize), fg, Some(&bg), Modifier::empty());
                hit(c, box_clip, Rect { x, y: r.y, w, h: 1 }, d, key, i as i32);
            }
            if first > 0 {
                put(c, box_clip, r.x, r.y, "‹", "#aaaaaa", None, Modifier::empty());
            }
            if first + visible < opts.len() {
                put(c, box_clip, r.x + r.w - 1, r.y, "›", "#aaaaaa", None, Modifier::empty());
            }
        }
        _ => {}
    }
}

fn in_screen(c: &Canvas, x: i32, y: i32) -> Option<Position> {
    (x >= 0 && y >= 0 && x < c.w && y < c.h).then(|| Position::new(x as u16, y as u16))
}

// An element's scroll, kept within what there is to scroll; returns it.
fn scrolled(elems: &mut HashMap<String, ElState>, key: &str, content: i32, viewport: i32) -> i32 {
    let st = elems.entry(key.to_string()).or_default();
    st.content = content;
    st.viewport = viewport;
    st.scroll = st.scroll.clamp(0, (content - viewport).max(0));
    st.scroll
}

// A scrollbar for `total` rows, `shown` of them from `first`, `height` cells tall: the thumb's cells.
fn thumb(total: i32, shown: i32, first: i32, height: i32) -> Option<(i32, i32)> {
    if total <= shown || shown <= 0 {
        return None;
    }
    let size = ((shown as f64 / total as f64) * height as f64).round().max(1.0) as i32;
    let top = ((first as f64 / (total - shown).max(1) as f64) * (height - size) as f64).round() as i32;
    Some((top, size))
}

// A textarea's text as rows `w` cells wide (its lines, wrapped where they run out), and the caret's (row, column).
// ponytail: wrapped at any character, not at words
pub fn textarea_rows(text: &[char], cursor: usize, w: usize) -> (Vec<String>, (usize, usize)) {
    let mut rows = vec![String::new()];
    let (mut col, mut at) = (0usize, (0usize, 0usize));
    for (i, ch) in text.iter().enumerate() {
        if i == cursor {
            at = (rows.len() - 1, col);
        }
        if *ch == '\n' {
            rows.push(String::new());
            col = 0;
            continue;
        }
        let cw = width(ch.encode_utf8(&mut [0; 4]));
        if col + cw > w {
            rows.push(String::new());
            col = 0;
        }
        rows.last_mut().unwrap().push(*ch);
        col += cw;
    }
    if cursor >= text.len() {
        at = (rows.len() - 1, col);
        if col >= w {
            rows.push(String::new());
            at = (rows.len() - 1, 0);
        }
    }
    (rows, at)
}

#[allow(clippy::too_many_arguments)]
fn draw_select(ctx: &Ctx, c: &mut Canvas, el: &El, clip: Rect, elems: &mut HashMap<String, ElState>, d: &mut Draw, _on: bool) {
    let th = ctx.th;
    let (n, r, key) = (el.n, el.rect, el.key.as_str());
    let opts = options(n);
    let described = opts.iter().any(|o| o["description"].as_str().is_some_and(|x| !x.is_empty()));
    let per = if described { 2 } else { 1 };
    let visible = (r.h / per).max(1) as usize;
    let st = elems.entry(key.to_string()).or_default();
    // the selection stays in the middle of what shows, as OpenTUI's list did
    let first = (st.index as i64 - (visible / 2) as i64).min(opts.len() as i64 - visible as i64).max(0) as usize;
    let box_clip = intersect(clip, r);
    fill(c, clip, r, th.bg);
    for (i, o) in opts.iter().enumerate().skip(first).take(visible) {
        let y = r.y + (i - first) as i32 * per;
        if y + per - 1 >= r.y + r.h {
            break;
        }
        let selected = i == st.index;
        let bg = if selected { mix(th.bg, th.focus, 0.22) } else { th.bg.to_string() };
        if selected {
            fill(c, box_clip, Rect { x: r.x, y, w: r.w, h: per }, &bg);
        }
        let name = format!("{}{}", if selected { "▶ " } else { "  " }, o["name"].as_str().unwrap_or(""));
        put(c, box_clip, r.x + 1, y, &name, th.fg, Some(&bg), Modifier::empty());
        if described {
            put(c, box_clip, r.x + 3, y + 1, o["description"].as_str().unwrap_or(""), th.dim, Some(&bg), Modifier::empty());
        }
        hit(c, box_clip, Rect { x: r.x, y, w: r.w, h: per }, d, key, i as i32);
    }
    if opts.len() > 12 && opts.len() > visible {
        let most = opts.len() - visible;
        let room = (r.h - 2).max(1);
        let y = r.y + 1 + ((first as f64 / most as f64) * room as f64).floor() as i32;
        put(c, box_clip, r.x + r.w - 1, y, "█", "#666666", None, Modifier::empty());
    }
}

fn draw_diff(ctx: &Ctx, c: &mut Canvas, el: &El, clip: Rect, elems: &mut HashMap<String, ElState>, d: &mut Draw) {
    let th = ctx.th;
    let (n, r, key) = (el.n, el.rect, el.key.as_str());
    let diff = n["diff"].as_str().unwrap_or("");
    let (added, removed) = (mix(th.bg, th.done, 0.16), mix(th.bg, th.blocked, 0.16));
    let numbers = n["lineNumbers"].as_bool().unwrap_or(true);
    let gw = if numbers { diff_gutter(diff) } else { 0 };
    let box_clip = intersect(clip, r);
    hit(c, clip, r, d, key, SCROLLS);
    // one side of a row: its number, its sign and its text, on the colour of what happened to it
    let side = |c: &mut Canvas, x: i32, y: i32, w: i32, num: Option<usize>, line: &str, lit: Option<&str>| {
        let sign = line.chars().next().unwrap_or(' ');
        let bg = lit.map(String::from).unwrap_or_else(|| match sign {
            '+' => added.clone(),
            '-' => removed.clone(),
            _ => th.bg.to_string(),
        });
        let cl = intersect(box_clip, Rect { x, y, w, h: 1 });
        fill(c, cl, Rect { x, y, w, h: 1 }, &bg);
        let mut at = x;
        if numbers {
            at += put(c, cl, at, y, &format!("{:>w$} ", num.map(|n| n.to_string()).unwrap_or_default(), w = (gw - 1) as usize), th.dim, Some(&bg), Modifier::empty());
        }
        let sc = match sign {
            '+' => th.done,
            '-' => th.blocked,
            _ => th.dim,
        };
        at += put(c, cl, at, y, &format!("{sign} "), sc, Some(&bg), Modifier::empty());
        put(c, cl, at, y, &tabs(line.get(1..).unwrap_or("")), th.fg, Some(&bg), Modifier::empty());
    };
    if split_view(n) {
        let rows = diff_split(diff);
        let top = scrolled(elems, key, rows.len() as i32, r.h);
        let half = r.w / 2;
        for (i, (l, rr)) in rows.iter().enumerate().skip(top as usize).take(r.h.max(0) as usize) {
            let y = r.y + i as i32 - top;
            match l {
                Some((num, line)) => side(c, r.x, y, half, Some(*num), line, None),
                None => fill(c, box_clip, Rect { x: r.x, y, w: half, h: 1 }, th.bg),
            }
            match rr {
                Some((num, line)) => side(c, r.x + half, y, r.w - half, Some(*num), line, None),
                None => fill(c, box_clip, Rect { x: r.x + half, y, w: r.w - half, h: 1 }, th.bg),
            }
        }
        return;
    }
    let body = diff_numbered(diff);
    let cursor = n["cursor"].as_bool() == Some(true);
    let marks: Vec<usize> = n["marks"].as_array().into_iter().flatten().filter_map(Value::as_u64).map(|m| m as usize).collect();
    let at = elems.get(key).map_or(0, |s| s.index);
    let top = scrolled(elems, key, body.len() as i32, r.h);
    let (cursor_bg, mark_bg) = (mix(th.bg, th.focus, 0.32), mix(th.bg, th.warn, 0.22));
    for (i, (o, nw, line)) in body.iter().enumerate().skip(top as usize).take(r.h.max(0) as usize) {
        let lit = if cursor && i == at { Some(cursor_bg.as_str()) } else if cursor && marks.contains(&i) { Some(mark_bg.as_str()) } else { None };
        side(c, r.x, r.y + i as i32 - top, r.w, if line.starts_with('-') { *o } else { *nw }, line, lit);
    }
}

// ---------- what an element does with a key ----------

// An element was used: its plugin's action, with the element's params and what it holds.
pub struct Fired {
    pub action: Option<String>,
    pub params: Value,
    pub ui: Map<String, Value>,
}

fn fired(n: &Value, action: &Value, extra: Value) -> Fired {
    let mut ui = Map::new();
    if let Some(k) = n["key"].as_str() {
        ui.insert("key".into(), json!(k));
    }
    for (k, v) in extra.as_object().into_iter().flatten() {
        if !v.is_null() {
            ui.insert(k.clone(), v.clone());
        }
    }
    Fired { action: action.as_str().map(String::from), params: n.get("params").cloned().unwrap_or(json!({})), ui }
}

// None: not this element's key; Some(None): taken; Some(Some(f)): taken, and it runs an action.
pub type Took = Option<Option<Fired>>;

// A key while a field has the keyboard: typing and moving the caret; Enter runs the input's action (a textarea's
// newline; Ctrl+Enter or Ctrl+S runs it). What a field doesn't use, nothing else gets.
pub fn field_key(n: &Value, st: &mut ElState, name: &str, ch: Option<char>) -> Took {
    let area = n["type"] == "textarea";
    let value = |st: &ElState| json!({ "value": st.text.iter().collect::<String>() });
    if let Some(ch) = ch {
        let most = n["maxLength"].as_u64().map(|m| m as usize).unwrap_or(usize::MAX);
        if st.text.len() < most {
            st.text.insert(st.cursor, ch);
            st.cursor += 1;
        }
        return Some(None);
    }
    match name {
        "enter" if area => {
            st.text.insert(st.cursor, '\n');
            st.cursor += 1;
        }
        "enter" if !area => return Some(Some(fired(n, &n["action"], value(st)))),
        "C-enter" | "C-s" if area => return Some(Some(fired(n, &n["action"], value(st)))),
        "backspace" if st.cursor > 0 => {
            st.cursor -= 1;
            st.text.remove(st.cursor);
        }
        "delete" if st.cursor < st.text.len() => {
            st.text.remove(st.cursor);
        }
        "left" => st.cursor = st.cursor.saturating_sub(1),
        "right" => st.cursor = (st.cursor + 1).min(st.text.len()),
        "home" | "C-a" => st.cursor = if area { line_start(&st.text, st.cursor) } else { 0 },
        "end" | "C-e" => st.cursor = if area { line_end(&st.text, st.cursor) } else { st.text.len() },
        "C-u" => {
            let from = if area { line_start(&st.text, st.cursor) } else { 0 };
            st.text.drain(from..st.cursor);
            st.cursor = from;
        }
        "C-k" => {
            let to = if area { line_end(&st.text, st.cursor) } else { st.text.len() };
            st.text.drain(st.cursor..to);
        }
        "C-w" => {
            let mut i = st.cursor;
            while i > 0 && st.text[i - 1].is_whitespace() {
                i -= 1;
            }
            while i > 0 && !st.text[i - 1].is_whitespace() {
                i -= 1;
            }
            st.text.drain(i..st.cursor);
            st.cursor = i;
        }
        "up" | "down" if area => {
            // the same column on the line above or below
            let start = line_start(&st.text, st.cursor);
            let col = st.cursor - start;
            if name == "up" && start > 0 {
                let prev = line_start(&st.text, start - 1);
                st.cursor = (prev + col).min(start - 1);
            } else if name == "down" {
                let end = line_end(&st.text, st.cursor);
                if end < st.text.len() {
                    st.cursor = (end + 1 + col).min(line_end(&st.text, end + 1));
                }
            }
        }
        _ => {}
    }
    Some(None)
}

fn line_start(t: &[char], at: usize) -> usize {
    t[..at].iter().rposition(|c| *c == '\n').map_or(0, |i| i + 1)
}
fn line_end(t: &[char], at: usize) -> usize {
    t[at..].iter().position(|c| *c == '\n').map_or(t.len(), |i| at + i)
}

fn choice(n: &Value, i: usize) -> Value {
    let o = &options(n).get(i).cloned().unwrap_or(Value::Null);
    json!({ "index": i, "value": o["value"].as_str().or(o["name"].as_str()) })
}

// A list's keys: a select moves (running `change`) and Enter runs its action; tabs move with ←→ / [ ] and run theirs.
pub fn list_key(n: &Value, st: &mut ElState, name: &str) -> Took {
    let count = options(n).len();
    if n["type"] == "tabs" {
        let next = match name {
            "left" | "[" if st.index > 0 => st.index - 1,
            "right" | "]" if st.index + 1 < count => st.index + 1,
            _ => return Some(None),
        };
        st.index = next;
        return Some(Some(fired(n, &n["action"], choice(n, next))));
    }
    let by: i64 = match name {
        "up" | "k" => -1,
        "down" | "j" => 1,
        "S-up" => -5,
        "S-down" => 5,
        "enter" => return Some(Some(fired(n, &n["action"], choice(n, st.index)))),
        _ => return Some(None),
    };
    st.index = (st.index as i64 + by).clamp(0, count.saturating_sub(1) as i64) as usize;
    Some(Some(fired(n, &n["change"], choice(n, st.index))))
}

// A diff's line cursor: j/k (and the arrows, PageUp/PageDown, g/G) move it over the body lines, Enter runs `action`
// with the line (`index` into the body, `value` the line as the diff has it), moving runs `change`.
pub fn diff_key(n: &Value, st: &mut ElState, name: &str) -> Took {
    if n["cursor"].as_bool() != Some(true) {
        return None;
    }
    let body = diff_body(n["diff"].as_str().unwrap_or(""));
    let last = body.len().saturating_sub(1) as i64;
    let to: i64 = match name {
        "j" | "down" => st.index as i64 + 1,
        "k" | "up" => st.index as i64 - 1,
        "pagedown" | "C-d" => st.index as i64 + 10,
        "pageup" | "C-u" => st.index as i64 - 10,
        "g" | "home" => 0,
        "G" | "end" => last,
        "enter" if !body.is_empty() => return Some(Some(fired(n, &n["action"], json!({ "index": st.index, "value": body[st.index] })))),
        _ => return None,
    };
    let next = to.clamp(0, last) as usize;
    if next == st.index {
        return Some(None);
    }
    st.index = next;
    // keep it in view
    let h = st.viewport.max(1);
    if (next as i32) < st.scroll {
        st.scroll = next as i32;
    } else if next as i32 >= st.scroll + h {
        st.scroll = next as i32 - h + 1;
    }
    Some(Some(fired(n, &n["change"], json!({ "index": next, "value": body[next] }))))
}

pub fn press(n: &Value) -> Fired {
    fired(n, &n["action"], json!({}))
}

// Scroll an area by `rows`; a sticky one stays stuck while it's at its edge.
pub fn scroll_by(n: &Value, st: &mut ElState, rows: i32) {
    let most = (st.content - st.viewport).max(0);
    st.scroll = (st.scroll + rows).clamp(0, most);
    if n["type"] == "scroll" {
        st.stuck = match n["sticky"].as_str() {
            Some("bottom") => st.scroll == most,
            Some("top") => st.scroll == 0,
            _ => false,
        };
    }
}

// What a click on a list's row chooses: a select's option (as moving to it would), a tab (as moving to it would).
pub fn pick(n: &Value, st: &mut ElState, i: usize) -> Option<Fired> {
    if i >= options(n).len() {
        return None;
    }
    let moved = st.index != i;
    st.index = i;
    if n["type"] == "tabs" {
        return moved.then(|| fired(n, &n["action"], choice(n, i)));
    }
    Some(fired(n, &n["change"], choice(n, i)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::themes::THEMES;

    fn ctx() -> Ctx<'static> {
        Ctx { th: &THEMES[0].1, logos: None, cell: 1.2, tick: 0 }
    }

    fn rects(root: &Value, w: i32, h: i32) -> Vec<(String, Rect)> {
        let mut elems = HashMap::new();
        let el = layout(&ctx(), root, Rect { x: 0, y: 0, w, h }, &mut elems);
        let mut out = vec![];
        fn all(el: &El, out: &mut Vec<(String, Rect)>) {
            out.push((el.key.clone(), el.rect));
            for k in &el.kids {
                all(k, out);
            }
        }
        all(&el, &mut out);
        out
    }

    #[test]
    fn a_column_stacks_with_gaps_and_stretches_across() {
        let root = json!({ "type": "box", "gap": 1, "children": [{ "type": "text", "children": ["hello"] }, { "type": "progress", "value": 0.5, "width": 20 }, { "type": "button", "label": "Go" }] });
        let r = rects(&root, 40, 20);
        assert_eq!(r[0].1, Rect { x: 0, y: 0, w: 40, h: 5 });
        assert_eq!(r[1].1, Rect { x: 0, y: 0, w: 40, h: 1 });
        assert_eq!(r[2].1, Rect { x: 0, y: 2, w: 20, h: 1 });
        assert_eq!(r[3].1, Rect { x: 0, y: 4, w: 40, h: 1 });
    }

    #[test]
    fn a_row_grows_shrinks_justifies_and_wraps() {
        let row = |justify: &str, kids: Value| json!({ "type": "box", "direction": "row", "justify": justify, "gap": 1, "children": kids });
        let r = rects(&row("start", json!([{ "type": "button", "label": "ab" }, { "type": "input", "key": "i" }])), 30, 3);
        assert_eq!(r[1].1, Rect { x: 0, y: 0, w: 4, h: 1 });
        assert_eq!(r[2].1, Rect { x: 5, y: 0, w: 25, h: 1 }); // the input takes what's left
        let r = rects(&row("end", json!([{ "type": "button", "label": "ab" }])), 30, 3);
        assert_eq!(r[1].1.x, 26);
        let r = rects(&row("between", json!([{ "type": "button", "label": "ab" }, { "type": "button", "label": "cd" }])), 30, 3);
        assert_eq!((r[1].1.x, r[2].1.x), (0, 26));
        let r = rects(&json!({ "type": "box", "direction": "row", "wrap": true, "children": [{ "type": "box", "width": 8, "height": 2 }, { "type": "box", "width": 8, "height": 2 }, { "type": "box", "width": 8, "height": 2 }] }), 20, 10);
        assert_eq!(r[3].1, Rect { x: 0, y: 2, w: 8, h: 2 }); // the third goes to a second line
        let r = rects(&json!({ "type": "box", "direction": "row", "children": [{ "type": "text", "children": ["aaaa"] }, { "type": "text", "children": ["bbbbbb"] }] }), 5, 3);
        assert_eq!(r[1].1.w + r[2].1.w, 5); // both shrink to fit, by how big they are
        assert!(r[2].1.w > r[1].1.w);
    }

    #[test]
    fn sizes_padding_borders_and_percentages() {
        let root = json!({ "type": "box", "border": true, "padding": 1, "children": [{ "type": "box", "width": "50%", "height": 3, "align": "center", "children": [{ "type": "button", "label": "x" }] }] });
        let r = rects(&root, 40, 20);
        assert_eq!(r[0].1.h, 7); // border, padding, 3 rows, padding, border
        assert_eq!(r[1].1, Rect { x: 2, y: 2, w: 18, h: 3 });
        assert_eq!(r[2].1, Rect { x: 9, y: 2, w: 3, h: 1 }); // centred across
        let scroll = json!({ "type": "scroll", "children": (0..30).map(|i| json!({ "type": "text", "children": [format!("line {i}")] })).collect::<Vec<_>>() });
        let r = rects(&json!({ "type": "box", "children": [scroll] }), 20, 10);
        assert_eq!(r[1].1.h, 10); // a scroll area takes the room there is, not its content's
    }

    #[test]
    fn text_wraps_at_words() {
        let segs = vec![seg("hello there world", "#ffffff", Modifier::empty())];
        let lines: Vec<String> = wrap(&segs, 11, "word").iter().map(|l| l.iter().map(|s| s.text.clone()).collect()).collect();
        assert_eq!(lines, ["hello there", "world"]);
        let lines: Vec<String> = wrap(&segs, 4, "char").iter().map(|l| l.iter().map(|s| s.text.clone()).collect()).collect();
        assert_eq!(lines[0], "hell");
        assert_eq!(wrap(&segs, 4, "none").len(), 1);
    }

    #[test]
    fn keys_keep_what_the_user_did_until_the_plugin_changes_it() {
        let v1 = json!({ "type": "box", "children": [{ "type": "input", "key": "note", "value": "a" }, { "type": "select", "key": "pick", "options": [{ "name": "x" }, { "name": "y" }] }] });
        let mut states = reconcile(&mut HashMap::new(), &v1);
        assert_eq!(focusables(&v1), [("#note".to_string(), Focus::Field), ("#pick".to_string(), Focus::List)]);
        let st = states.get_mut("#note").unwrap();
        field_key(&v1["children"][0], st, "", Some('b'));
        let f = list_key(&v1["children"][1], states.get_mut("#pick").unwrap(), "down").unwrap().unwrap();
        assert_eq!(json!(f.ui), json!({ "key": "pick", "index": 1, "value": "y" }));
        let mut kept = reconcile(&mut states, &v1);
        assert_eq!(kept["#note"].text.iter().collect::<String>(), "ab");
        assert_eq!(kept["#pick"].index, 1);
        let v2 = json!({ "type": "box", "children": [{ "type": "input", "key": "note", "value": "new" }, { "type": "select", "key": "pick", "selected": 0, "options": [{ "name": "x" }] }] });
        let next = reconcile(&mut kept, &v2);
        assert_eq!(next["#note"].text.iter().collect::<String>(), "new");
        assert_eq!(next["#pick"].index, 0);
    }

    #[test]
    fn a_diffs_body_and_cursor() {
        let diff = "diff --git a/a.ts b/a.ts\n--- a/a.ts\n+++ b/a.ts\n@@ -1,3 +1,4 @@\n const a = 1;\n-const b = 2;\n+const b = 3;\n+const c = 4;\n const d = 5;\n";
        assert_eq!(diff_body(diff), [" const a = 1;", "-const b = 2;", "+const b = 3;", "+const c = 4;", " const d = 5;"]);
        let n = json!({ "type": "diff", "diff": diff, "cursor": true, "action": "go" });
        let mut st = ElState { viewport: 5, ..Default::default() };
        diff_key(&n, &mut st, "j");
        diff_key(&n, &mut st, "j");
        let f = diff_key(&n, &mut st, "enter").unwrap().unwrap();
        assert_eq!(json!(f.ui), json!({ "index": 2, "value": "+const b = 3;" }));
        assert_eq!(diff_split(diff).len(), 4);
    }
}
