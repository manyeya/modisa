// Code and diffs: highlighted (highlight.rs), numbered, scrolled; a diff unified or side by side, with a line cursor
// and marked lines when the plugin asks for them.
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget, Wrap};
use serde_json::{json, Value};

use super::build::{bar_room, fired, hash_of, hit, moved, scroll_key, scrolled, scrollbar, Ctx, Draw, ElState, Elems, Fired, Took};
use super::highlight;
use crate::client::draw::Canvas;

fn digits(n: usize) -> usize {
    n.max(1).to_string().len()
}

// the line numbers a view marks
fn numbers(v: &Value) -> Vec<usize> {
    v.as_array().into_iter().flatten().filter_map(|x| x.as_u64().map(|n| n as usize)).collect()
}

// rows a line takes wrapped at `w`
fn wrapped(line: Line<'static>, w: u16) -> usize {
    Paragraph::new(line).wrap(Wrap { trim: false }).line_count(w).max(1)
}

pub fn draw_code(ctx: &Ctx, c: &mut Canvas, n: &Value, key: &str, area: Rect, elems: &mut Elems) {
    let content = n["content"].as_str().unwrap_or("");
    let id = hash_of(content);
    let count = content.lines().count();
    let syntax = highlight::syntax(n["language"].as_str(), content.lines().next().unwrap_or(""));
    let look = highlight::look(ctx.th, n["syntax_theme"].as_str());
    if let Some(bg) = look.bg {
        c.buf.set_style(area, Style::new().bg(bg));
    }
    let first = match &n["line_numbers"] {
        Value::Bool(true) => Some(1),
        v => v.as_u64().map(|n| n as usize),
    };
    let gutter = first.map_or(0, |f| digits(f + count.saturating_sub(1)) + 1) as u16;
    let marked = numbers(&n["highlight"]);
    let wrap = n["wrap"] == true;
    let (body, bar) = bar_room(n, area, count, true);
    let (rows, w) = (body.height as usize, body.width.saturating_sub(gutter));
    let st = elems.entry(key.to_string()).or_default();
    // wrapped, it scrolls a line at a time, as far as the line from which the last ones fill it
    let total = if wrap {
        let mut used = 0;
        let fit = content.lines().rev().take_while(|l| {
            used += wrapped(Line::raw(l.replace('\t', "    ")), w);
            used <= rows
        });
        count - fit.count() + rows
    } else {
        count
    };
    let top = scrolled(st, total, rows);
    let lines = highlight::lines(id, content, syntax, look, top, rows);
    let widest = lines.iter().map(Line::width).max().unwrap_or(0);
    st.hscroll = st.hscroll.min(widest.saturating_sub(w as usize));
    let mut y = body.y;
    for (i, line) in lines.into_iter().enumerate() {
        if y >= body.bottom() {
            break;
        }
        let number = first.unwrap_or(1) + top + i;
        let mark = marked.contains(&number);
        let mut p = Paragraph::new(line);
        let h = if wrap {
            p = p.wrap(Wrap { trim: false });
            (p.line_count(w) as u16).clamp(1, body.bottom() - y)
        } else {
            p = p.scroll((0, st.hscroll.min(u16::MAX as usize) as u16));
            1
        };
        if mark {
            c.buf.set_style(Rect { y, height: h, ..body }, Style::new().bg(ctx.tint("warn", 0.18)));
        }
        if first.is_some() {
            let look = if mark { Style::new().fg(ctx.tone("warn")).add_modifier(Modifier::BOLD) } else { Style::new().fg(ctx.tone("dim")) };
            Span::styled(format!("{number:>0$} ", gutter as usize - 1), look).render(Rect { y, height: 1, width: gutter, ..body }, c.buf);
        }
        p.render(Rect { x: body.x + gutter, y, width: w, height: h }, c.buf);
        y += h;
    }
    if bar {
        scrollbar(ctx, c.buf, area, total, rows, top);
    }
}

// ---------- diffs ----------

// A unified diff, read: its files, its body lines (+, -, context) in order, and the rows each view draws.
#[derive(Default)]
pub struct Diff {
    files: Vec<File>,
    hunks: Vec<String>,
    pub body: Vec<Body>,
    unified: Vec<Row>,
    split: Vec<Row>,
    widest: (usize, usize), // the largest old and new line numbers
}

#[derive(Default)]
struct File {
    name: String,
    old: String, // the lines on each side (context, and what went or came), for highlighting
    new: String,
    lines: (usize, usize),
}

pub struct Body {
    file: usize,
    sign: char,
    pub old: Option<usize>,
    pub new: Option<usize>,
    pub text: String,
    at: usize, // its line on its side (old for -, new for + and context)
}

#[derive(Clone, Copy)]
enum Row {
    File(usize),
    Hunk(usize),
    Line(usize),
    Pair(Option<usize>, Option<usize>),
}

// what `a/x`, `b/x` or `x\t2026-…` names
fn path(s: &str) -> String {
    let s = s.split('\t').next().unwrap_or(s).trim();
    s.strip_prefix("a/").or_else(|| s.strip_prefix("b/")).unwrap_or(s).to_string()
}

pub fn parse(diff: &str) -> Diff {
    let mut d = Diff::default();
    let mut cur: Option<usize> = None;
    let mut hunked = false; // the current file has a hunk already
    let mut gone = String::new(); // the --- name, for a file that's deleted
    let (mut old, mut new, mut old_left, mut new_left) = (0, 0, 0usize, 0usize);
    for l in diff.lines() {
        if old_left > 0 || new_left > 0 {
            let sign = l.chars().next().unwrap_or(' ');
            if matches!(sign, '+' | '-' | ' ') || l.is_empty() {
                let f = cur.unwrap_or(0);
                let file = &mut d.files[f];
                let text = l.get(1..).unwrap_or("").to_string();
                let (o, n, at) = match sign {
                    '+' => (None, Some(new), file.lines.1),
                    '-' => (Some(old), None, file.lines.0),
                    _ => (Some(old), Some(new), file.lines.1),
                };
                if sign != '+' {
                    file.old.push_str(&text);
                    file.old.push('\n');
                    (file.lines.0, old, old_left) = (file.lines.0 + 1, old + 1, old_left.saturating_sub(1));
                }
                if sign != '-' {
                    file.new.push_str(&text);
                    file.new.push('\n');
                    (file.lines.1, new, new_left) = (file.lines.1 + 1, new + 1, new_left.saturating_sub(1));
                }
                d.widest = (d.widest.0.max(o.unwrap_or(0)), d.widest.1.max(n.unwrap_or(0)));
                d.unified.push(Row::Line(d.body.len()));
                d.body.push(Body { file: f, sign: if sign == '+' || sign == '-' { sign } else { ' ' }, old: o, new: n, text, at });
                continue;
            }
            if l.starts_with('\\') {
                continue; // \ No newline at end of file
            }
            (old_left, new_left) = (0, 0);
        }
        if let Some(rest) = l.strip_prefix("diff --git ") {
            let name = rest.rsplit_once(" b/").map_or(rest.to_string(), |(_, b)| b.to_string());
            d.files.push(File { name, ..Default::default() });
            cur = Some(d.files.len() - 1);
            hunked = false;
            d.unified.push(Row::File(d.files.len() - 1));
        } else if let Some(name) = l.strip_prefix("--- ") {
            gone = path(name);
        } else if let Some(name) = l.strip_prefix("+++ ") {
            let name = if name.starts_with("/dev/null") { gone.clone() } else { path(name) };
            match cur.filter(|_| !hunked) {
                Some(f) => d.files[f].name = name,
                None => {
                    d.files.push(File { name, ..Default::default() });
                    cur = Some(d.files.len() - 1);
                    hunked = false;
                    d.unified.push(Row::File(d.files.len() - 1));
                }
            }
        } else if let Some(h) = l.strip_prefix("@@") {
            if cur.is_none() {
                d.files.push(File::default()); // a hunk with no file named before it
                cur = Some(0);
            }
            hunked = true;
            // @@ -old,count +new,count @@
            let range = |sign: char| -> (usize, usize) {
                let r = h.split_whitespace().find_map(|p| p.strip_prefix(sign)).unwrap_or("1");
                let (at, count) = r.split_once(',').unwrap_or((r, "1"));
                (at.parse().unwrap_or(1), count.parse().unwrap_or(1))
            };
            ((old, old_left), (new, new_left)) = (range('-'), range('+'));
            d.unified.push(Row::Hunk(d.hunks.len()));
            d.hunks.push(l.to_string());
        }
    }
    // side by side: context on both sides; a run of removed lines beside the run of added lines after it
    let mut i = 0;
    while i < d.unified.len() {
        match d.unified[i] {
            Row::Line(b) if d.body[b].sign != ' ' => {
                let run = |i: &mut usize, sign: char| {
                    let mut out = vec![];
                    while let Some(Row::Line(b)) = d.unified.get(*i).filter(|r| matches!(r, Row::Line(b) if d.body[*b].sign == sign)) {
                        out.push(*b);
                        *i += 1;
                    }
                    out
                };
                let (gone, came) = (run(&mut i, '-'), run(&mut i, '+'));
                for k in 0..gone.len().max(came.len()) {
                    d.split.push(Row::Pair(gone.get(k).copied(), came.get(k).copied()));
                }
            }
            Row::Line(b) => {
                d.split.push(Row::Pair(Some(b), Some(b)));
                i += 1;
            }
            r => {
                d.split.push(r);
                i += 1;
            }
        }
    }
    d
}

thread_local! {
    static DIFFS: RefCell<HashMap<u64, Rc<Diff>>> = RefCell::new(HashMap::new());
}

// A diff read once, kept by its content.
pub fn parsed(id: u64, text: &str) -> Rc<Diff> {
    DIFFS.with(|ds| {
        let mut ds = ds.borrow_mut();
        if ds.len() >= 16 && !ds.contains_key(&id) {
            ds.clear();
        }
        ds.entry(id).or_insert_with(|| Rc::new(parse(text))).clone()
    })
}

fn diff_of(n: &Value) -> (u64, Rc<Diff>) {
    let text = n["diff"].as_str().unwrap_or("");
    let id = hash_of(text);
    (id, parsed(id, text))
}

impl Row {
    // the row shows body line `b`
    fn holds(self, b: usize) -> bool {
        match self {
            Row::Line(x) => x == b,
            Row::Pair(l, r) => l == Some(b) || r == Some(b),
            _ => false,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn draw_diff(ctx: &Ctx, c: &mut Canvas, n: &Value, key: &str, area: Rect, elems: &mut Elems, d: &Draw, on: bool) {
    let (id, diff) = diff_of(n);
    let split = n["view"] == "split";
    let rows = if split { &diff.split } else { &diff.unified };
    let look = highlight::look(ctx.th, n["syntax_theme"].as_str());
    if let Some(bg) = look.bg {
        c.buf.set_style(area, Style::new().bg(bg));
    }
    let (body, bar) = bar_room(n, area, rows.len(), true);
    let h = body.height as usize;
    let st = elems.entry(key.to_string()).or_default();
    let cursor = (n["cursor"] == true && !diff.body.is_empty()).then(|| st.line.min(diff.body.len() - 1));
    if let Some(at) = cursor.filter(|_| st.reveal) {
        if let Some(r) = rows.iter().position(|r| r.holds(at)) {
            st.scroll = st.scroll.min(r).max((r + 1).saturating_sub(h));
        }
        st.reveal = false;
    }
    let top = scrolled(st, rows.len(), h);
    let shown = &rows[top.min(rows.len())..(top + h).min(rows.len())];
    // each file's lines, highlighted on each side (a context line shows its new side's) as far as these rows need
    let mut span: HashMap<(usize, bool), (usize, usize)> = HashMap::new();
    let mut need = |b: usize, old: bool| {
        let x = &diff.body[b];
        let e = span.entry((x.file, old)).or_insert((x.at, x.at));
        *e = (e.0.min(x.at), e.1.max(x.at));
    };
    for r in shown {
        match *r {
            Row::Line(b) => need(b, diff.body[b].sign == '-'),
            Row::Pair(l, r) => {
                if let Some(b) = l {
                    need(b, diff.body[b].sign == '-');
                }
                if let Some(b) = r {
                    need(b, false);
                }
            }
            _ => {}
        }
    }
    let lang = n["language"].as_str();
    let code: HashMap<(usize, bool), (usize, Vec<Line>)> = span
        .into_iter()
        .map(|((f, old), (a, z))| {
            let file = &diff.files[f];
            let stream = if old { &file.old } else { &file.new };
            let syntax = highlight::syntax(lang.or(Some(&file.name)).filter(|l| !l.is_empty()), stream.lines().next().unwrap_or(""));
            ((f, old), (a, highlight::lines(hash_of((id, f, old)), stream, syntax, look, a, z - a + 1)))
        })
        .collect();
    let line_of = |b: usize, old: bool| -> Line<'static> {
        let x = &diff.body[b];
        code.get(&(x.file, old)).and_then(|(from, ls)| ls.get(x.at - from)).cloned().unwrap_or_else(|| Line::raw(x.text.replace('\t', "    ")))
    };
    let marks = numbers(&n["marks"]);
    let gutters = if n["line_numbers"] == false { (0, 0) } else { (digits(diff.widest.0), digits(diff.widest.1)) };
    let hs = st.hscroll.min(u16::MAX as usize) as u16;
    let side = |c: &mut Canvas, r: Rect, b: usize, nums: &[(Option<usize>, usize)]| {
        let x = &diff.body[b];
        let bg = if cursor == Some(b) {
            Some(ctx.tint("focus", if on { 0.34 } else { 0.22 }))
        } else if marks.contains(&b) {
            Some(ctx.tint("warn", 0.22))
        } else {
            match x.sign {
                '+' => Some(ctx.tint("done", 0.16)),
                '-' => Some(ctx.tint("blocked", 0.16)),
                _ => None,
            }
        };
        if let Some(bg) = bg {
            c.buf.set_style(r, Style::new().bg(bg));
        }
        let mut gutter: Vec<Span> = nums.iter().filter(|(_, w)| *w > 0).map(|(num, w)| Span::styled(format!("{:>w$} ", num.map(|n| n.to_string()).unwrap_or_default()), Style::new().fg(ctx.tone("dim")))).collect();
        let sign = match x.sign {
            '+' => ctx.tone("done"),
            '-' => ctx.tone("blocked"),
            _ => ctx.tone("dim"),
        };
        gutter.push(Span::styled(format!("{} ", x.sign), Style::new().fg(sign)));
        let gw = gutter.iter().map(Span::width).sum::<usize>() as u16;
        Line::from(gutter).render(r, c.buf);
        let at = Rect { x: r.x + gw.min(r.width), width: r.width.saturating_sub(gw), ..r };
        Paragraph::new(line_of(b, x.sign == '-')).scroll((0, hs)).render(at, c.buf);
        hit(c, d, key, r, b as i32);
    };
    for (i, row) in shown.iter().enumerate() {
        let r = Rect { y: body.y + i as u16, height: 1, ..body };
        match *row {
            Row::File(f) => {
                c.buf.set_style(r, Style::new().bg(ctx.tone("bar")));
                Line::from(vec![Span::styled("▍ ", Style::new().fg(ctx.tone("accent"))), Span::styled(diff.files[f].name.clone(), Style::new().fg(ctx.tone("fg")).add_modifier(Modifier::BOLD))]).render(r, c.buf);
            }
            Row::Hunk(k) => Span::styled(diff.hunks[k].clone(), Style::new().fg(ctx.tone("dim"))).render(r, c.buf),
            Row::Line(b) => side(c, r, b, &[(diff.body[b].old, gutters.0), (diff.body[b].new, gutters.1)]),
            Row::Pair(left, right) => {
                let half = r.width / 2;
                let (lr, rr) = (Rect { width: half, ..r }, Rect { x: r.x + half, width: r.width - half, ..r });
                match left {
                    Some(b) => side(c, lr, b, &[(diff.body[b].old, gutters.0)]),
                    None => c.buf.set_style(lr, Style::new().bg(ctx.tone("bar"))),
                }
                match right {
                    Some(b) => side(c, rr, b, &[(diff.body[b].new, gutters.1)]),
                    None => c.buf.set_style(rr, Style::new().bg(ctx.tone("bar"))),
                }
            }
        }
    }
    if bar {
        scrollbar(ctx, c.buf, area, rows.len(), h, top);
    }
}

// What a diff's line holds, for its action and change: its place among the body lines, its numbers on the sides it's
// on, and the line as the diff has it (its +, - or space first).
fn holds(diff: &Diff, i: usize) -> Value {
    let b = &diff.body[i];
    let mut v = json!({ "line": i, "text": format!("{}{}", b.sign, b.text) });
    for (side, n) in [("old", b.old), ("new", b.new)] {
        if let Some(n) = n {
            v[side] = json!(n);
        }
    }
    v
}

// A diff's keys: with a cursor, ↑ ↓ (j k), pages, home and end move it (its change) and Enter runs its action; without
// one, they scroll it. ← → move it sideways.
pub fn diff_key(n: &Value, st: &mut ElState, name: &str) -> Took {
    let (_, diff) = diff_of(n);
    if n["cursor"] != true || diff.body.is_empty() || matches!(name, "left" | "right") {
        return scroll_key(n, st, name).then(Vec::new);
    }
    if name == "enter" {
        return Some(fired(n, "action", holds(&diff, st.line.min(diff.body.len() - 1))).into_iter().collect());
    }
    let to = moved(name, Some(st.line), diff.body.len(), st.shown)?;
    if to == st.line {
        return Some(vec![]);
    }
    (st.line, st.reveal) = (to, true);
    Some(fired(n, "change", holds(&diff, to)).into_iter().collect())
}

// A click on a diff's line: the cursor goes there (its change), and the line's action runs.
pub fn diff_click(n: &Value, st: &mut ElState, b: usize) -> Vec<Fired> {
    let (_, diff) = diff_of(n);
    if b >= diff.body.len() {
        return vec![];
    }
    let mut out = vec![];
    if n["cursor"] == true && st.line != b {
        st.line = b;
        out.extend(fired(n, "change", holds(&diff, b)));
    }
    out.extend(fired(n, "action", holds(&diff, b)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = "diff --git a/a.ts b/a.ts\nindex 1..2 100644\n--- a/a.ts\n+++ b/a.ts\n@@ -1,4 +1,5 @@\n const a = 1;\n-const b = 2;\n+const b = 3;\n+const c = 4;\n const d = 5;\n--- x\n";

    #[test]
    fn reads_a_diff() {
        let d = parse(DIFF);
        let texts: Vec<(char, &str, Option<usize>, Option<usize>)> = d.body.iter().map(|b| (b.sign, b.text.as_str(), b.old, b.new)).collect();
        assert_eq!(texts, [(' ', "const a = 1;", Some(1), Some(1)), ('-', "const b = 2;", Some(2), None), ('+', "const b = 3;", None, Some(2)), ('+', "const c = 4;", None, Some(3)), (' ', "const d = 5;", Some(3), Some(4)), ('-', "-- x", Some(4), None)]);
        assert_eq!(d.files[0].name, "a.ts");
        assert_eq!(d.files[0].new, "const a = 1;\nconst b = 3;\nconst c = 4;\nconst d = 5;\n");
        assert_eq!(d.unified.len(), 2 + 6); // the file, the hunk, its lines
        assert_eq!(d.split.len(), 2 + 5); // a; the new b and c beside the old b; d; x beside nothing
    }

    #[test]
    fn a_diffs_cursor_moves_and_acts() {
        let n = json!({ "type": "diff", "id": "d", "diff": DIFF, "cursor": true, "action": "go", "change": "moved" });
        let mut st = ElState { shown: 5, ..Default::default() };
        diff_key(&n, &mut st, "j");
        let f = diff_key(&n, &mut st, "j").unwrap();
        assert_eq!(json!(f[0].ui), json!({ "id": "d", "event": "change", "line": 2, "new": 2, "text": "+const b = 3;" }));
        let f = diff_key(&n, &mut st, "enter").unwrap();
        assert_eq!(f[0].action, "go");
        assert_eq!(diff_click(&n, &mut st, 0).len(), 2);
    }
}
