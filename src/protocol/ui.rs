// The small languages of plugin views (UI 3, examples/plugins/VIEWS.md), read into ratatui's own types: colours,
// styles, constraints, text and blocks. The server reads everything a plugin sends through these to check it (an error
// says what's wrong, the caller adds where); the client reads them again to draw, with the user's theme for `$tokens`.
use std::str::FromStr;

use ratatui::layout::{Constraint, Direction, Flex, HorizontalAlignment, Offset};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::merge::MergeStrategy;
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, Padding, Shadow};
use serde_json::Value;

use crate::config::themes::Theme;

pub type Res<T> = Result<T, String>;

// The theme's colours by the names plugins use for them.
pub const TOKENS: &[&str] = &["fg", "bg", "bar", "dim", "border", "focus", "accent", "warn", "working", "blocked", "done", "idle"];

pub fn token(th: &Theme, name: &str) -> Option<&'static str> {
    Some(match name {
        "fg" => th.fg,
        "bg" => th.bg,
        "bar" => th.bar,
        "dim" => th.dim,
        "border" => th.border,
        "focus" => th.focus,
        "accent" => th.accent,
        "warn" => th.warn,
        "working" => th.working,
        "blocked" => th.blocked,
        "done" => th.done,
        "idle" => th.idle,
        _ => return None,
    })
}

// What drawing text needs besides the theme: how an agent's mark (`{ "icon": id }`) is drawn. The server checks with
// one that draws nothing.
pub struct Ink<'a> {
    pub th: &'a Theme,
    pub icon: &'a dyn Fn(&str) -> Vec<Span<'static>>,
}

impl<'a> Ink<'a> {
    pub fn plain(th: &'a Theme) -> Self {
        Ink { th, icon: &|_| vec![] }
    }
}

// ---------- colours and styles ----------

// `$token`, or what ratatui reads: a name (red, light-blue, gray…), #rrggbb, 0–255, reset.
pub fn color(s: &str, th: &Theme) -> Res<Color> {
    if let Some(t) = s.strip_prefix('$') {
        let hex = token(th, t).ok_or_else(|| format!("{s} isn't a theme colour ({})", TOKENS.iter().map(|t| format!("${t}")).collect::<Vec<_>>().join(", ")))?;
        return Ok(Color::from_str(hex).unwrap_or(Color::Reset));
    }
    Color::from_str(s).map_err(|_| format!("{s:?} isn't a colour"))
}

const MODIFIERS: &[(&str, Modifier)] = &[
    ("bold", Modifier::BOLD),
    ("dim", Modifier::DIM),
    ("italic", Modifier::ITALIC),
    ("underlined", Modifier::UNDERLINED),
    ("slow_blink", Modifier::SLOW_BLINK),
    ("rapid_blink", Modifier::RAPID_BLINK),
    ("reversed", Modifier::REVERSED),
    ("hidden", Modifier::HIDDEN),
    ("crossed_out", Modifier::CROSSED_OUT),
];

fn modifier(name: &str) -> Option<Modifier> {
    MODIFIERS.iter().find(|(n, _)| *n == name).map(|(_, m)| *m)
}

// `{ "fg": c, "bg": c, "underline_color": c, "bold": true, … }`, or "bold italic $accent on $bar".
pub fn style(v: &Value, th: &Theme) -> Res<Style> {
    let mut s = Style::default();
    match v {
        Value::Null => {}
        Value::String(text) => {
            let mut words = text.split_whitespace();
            let mut fg = false;
            while let Some(w) = words.next() {
                if w == "on" {
                    let c = words.next().ok_or("\"on\" needs a colour after it")?;
                    s = s.bg(color(c, th)?);
                } else if let Some(m) = modifier(w) {
                    s = s.add_modifier(m);
                } else if !fg {
                    s = s.fg(color(w, th).map_err(|e| if w.starts_with('$') { e } else { format!("{w:?} isn't a colour or a modifier") })?);
                    fg = true;
                } else {
                    return Err(format!("{w:?}: a style has one foreground colour (the background follows \"on\")"));
                }
            }
        }
        Value::Object(o) => {
            for (k, x) in o {
                match k.as_str() {
                    "fg" | "bg" | "underline_color" => {
                        let c = color(x.as_str().ok_or_else(|| format!("{k} must be a colour"))?, th)?;
                        s = match k.as_str() {
                            "fg" => s.fg(c),
                            "bg" => s.bg(c),
                            _ => s.underline_color(c),
                        };
                    }
                    _ => {
                        let m = modifier(k).ok_or_else(|| format!("{k} isn't a style field"))?;
                        s = match x.as_bool().ok_or_else(|| format!("{k} must be true or false"))? {
                            true => s.add_modifier(m),
                            false => s.remove_modifier(m),
                        };
                    }
                }
            }
        }
        _ => return Err("a style is an object or a string".into()),
    }
    Ok(s)
}

pub fn style_at(o: &Value, field: &str, th: &Theme) -> Res<Style> {
    style(&o[field], th).map_err(|e| format!("{field}: {e}"))
}

// ---------- layout ----------

// 12 or "12" Length, "30%" Percentage, "1/3" Ratio, ">=5" Min, "<=20" Max, "*" or "2*" Fill.
pub fn constraint(v: &Value) -> Res<Constraint> {
    let bad = || format!("{v} isn't a constraint (12, \"30%\", \"1/3\", \">=5\", \"<=20\", \"*\", \"2*\")");
    if let Some(n) = v.as_u64() {
        return u16::try_from(n).map(Constraint::Length).map_err(|_| bad());
    }
    let s = v.as_str().ok_or_else(bad)?.trim();
    let n = |t: &str| t.trim().parse::<u16>().map_err(|_| bad());
    Ok(if let Some(p) = s.strip_suffix('%') {
        let p = n(p)?;
        if p > 100 {
            return Err(bad());
        }
        Constraint::Percentage(p)
    } else if let Some(m) = s.strip_prefix(">=") {
        Constraint::Min(n(m)?)
    } else if let Some(m) = s.strip_prefix("<=") {
        Constraint::Max(n(m)?)
    } else if let Some(w) = s.strip_suffix('*') {
        Constraint::Fill(if w.is_empty() { 1 } else { n(w)? })
    } else if let Some((a, b)) = s.split_once('/') {
        let (a, b) = (a.trim().parse::<u32>().map_err(|_| bad())?, b.trim().parse::<u32>().map_err(|_| bad())?);
        if b == 0 {
            return Err(bad());
        }
        Constraint::Ratio(a, b)
    } else {
        Constraint::Length(n(s)?)
    })
}

pub fn direction(v: &Value) -> Res<Direction> {
    match v.as_str() {
        None | Some("vertical") => Ok(Direction::Vertical),
        Some("horizontal") => Ok(Direction::Horizontal),
        Some(o) => Err(format!("direction {o:?} isn't vertical or horizontal")),
    }
}

pub fn flex(v: &Value) -> Res<Flex> {
    Ok(match v.as_str() {
        None | Some("start") => Flex::Start,
        Some("legacy") => Flex::Legacy,
        Some("end") => Flex::End,
        Some("center") => Flex::Center,
        Some("space_between") => Flex::SpaceBetween,
        Some("space_around") => Flex::SpaceAround,
        Some("space_evenly") => Flex::SpaceEvenly,
        Some(o) => return Err(format!("flex {o:?} isn't legacy, start, end, center, space_between, space_around or space_evenly")),
    })
}

pub fn align(v: &Value) -> Res<Option<HorizontalAlignment>> {
    Ok(match v.as_str() {
        None => None,
        Some("left") => Some(HorizontalAlignment::Left),
        Some("center") => Some(HorizontalAlignment::Center),
        Some("right") => Some(HorizontalAlignment::Right),
        Some(o) => return Err(format!("align {o:?} isn't left, center or right")),
    })
}

fn u16_of(v: &Value) -> Res<u16> {
    v.as_u64().and_then(|n| u16::try_from(n).ok()).ok_or_else(|| format!("{v} isn't a size in cells"))
}

// n, [vertical, horizontal], or [top, right, bottom, left]
pub fn padding(v: &Value) -> Res<Padding> {
    if v.is_null() {
        return Ok(Padding::ZERO);
    }
    if !v.is_array() {
        return Ok(Padding::uniform(u16_of(v)?));
    }
    let n: Vec<u16> = v.as_array().unwrap().iter().map(u16_of).collect::<Res<_>>()?;
    match n[..] {
        [y, x] => Ok(Padding::symmetric(x, y)),
        [t, r, b, l] => Ok(Padding::new(l, r, t, b)),
        _ => Err("padding is n, [vertical, horizontal] or [top, right, bottom, left]".into()),
    }
}

pub fn marker(v: &Value) -> Res<Marker> {
    Ok(match v.as_str() {
        None | Some("braille") => Marker::Braille,
        Some("dot") => Marker::Dot,
        Some("block") => Marker::Block,
        Some("bar") => Marker::Bar,
        Some("half_block") => Marker::HalfBlock,
        Some("quadrant") => Marker::Quadrant,
        Some("sextant") => Marker::Sextant,
        Some("octant") => Marker::Octant,
        Some(s) => {
            let mut cs = s.chars();
            match (cs.next(), cs.next()) {
                (Some(c), None) if crate::core::text::grapheme_width(s) == 1 => Marker::Custom(c),
                _ => return Err(format!("marker {s:?} isn't dot, block, bar, braille, half_block, quadrant, sextant, octant or one character")),
            }
        }
    })
}

// ---------- text ----------

// Text that came from a plugin, safe to draw on one line: tabs as spaces, nothing else that moves the cursor (the
// server has already taken escape sequences and control characters out).
fn one_line(s: &str) -> String {
    s.replace('\t', "    ").replace(['\n', '\r'], " ")
}

// A string, `{ "text": "…", "style": Style }`, or `{ "icon": agent }`.
pub fn spans(v: &Value, ink: &Ink) -> Res<Vec<Span<'static>>> {
    Ok(match v {
        Value::String(s) => vec![Span::raw(one_line(s))],
        Value::Object(o) if o.contains_key("icon") => (ink.icon)(o["icon"].as_str().ok_or("icon must be an agent's id")?),
        Value::Object(o) => {
            let text = o.get("text").and_then(Value::as_str).ok_or("a span is a string or { text, style }")?;
            vec![Span::styled(one_line(text), style_at(v, "style", ink.th)?)]
        }
        _ => return Err("a span is a string, { text, style } or { icon }".into()),
    })
}

// A string, an array of Spans, or `{ "spans": [Span…], "style": Style, "align": … }`.
pub fn line(v: &Value, ink: &Ink) -> Res<Line<'static>> {
    let of = |xs: &Vec<Value>| -> Res<Vec<Span<'static>>> { Ok(xs.iter().map(|x| spans(x, ink)).collect::<Res<Vec<_>>>()?.concat()) };
    Ok(match v {
        Value::Null => Line::default(),
        Value::String(_) => Line::from(spans(v, ink)?),
        Value::Array(xs) => Line::from(of(xs)?),
        Value::Object(o) if o.contains_key("spans") => {
            let xs = o["spans"].as_array().ok_or("spans must be an array")?;
            let mut l = Line::from(of(xs)?).style(style_at(v, "style", ink.th)?);
            l.alignment = align(&v["align"])?;
            l
        }
        Value::Object(_) => Line::from(spans(v, ink)?),
        _ => return Err("a line is a string, an array of spans, or { spans, style, align }".into()),
    })
}

// A string (lines split at \n), an array of Lines, or `{ "lines": [Line…], "style": Style, "align": … }`.
pub fn text(v: &Value, ink: &Ink) -> Res<Text<'static>> {
    Ok(match v {
        Value::Null => Text::default(),
        Value::String(s) => Text::from(s.split('\n').map(|l| Line::raw(one_line(l))).collect::<Vec<_>>()),
        Value::Array(xs) => Text::from(xs.iter().map(|x| line(x, ink)).collect::<Res<Vec<_>>>()?),
        Value::Object(o) if o.contains_key("lines") => {
            let mut t = text(&o["lines"], ink)?.style(style_at(v, "style", ink.th)?);
            t.alignment = align(&v["align"])?;
            t
        }
        Value::Object(_) => Text::from(line(v, ink)?),
        _ => return Err("text is a string, an array of lines, or { lines, style, align }".into()),
    })
}

// ---------- blocks ----------

fn border_type(v: &Value) -> Res<BorderType> {
    Ok(match v.as_str() {
        None | Some("plain") => BorderType::Plain,
        Some("rounded") => BorderType::Rounded,
        Some("double") => BorderType::Double,
        Some("thick") => BorderType::Thick,
        Some("light_double_dashed") => BorderType::LightDoubleDashed,
        Some("heavy_double_dashed") => BorderType::HeavyDoubleDashed,
        Some("light_triple_dashed") => BorderType::LightTripleDashed,
        Some("heavy_triple_dashed") => BorderType::HeavyTripleDashed,
        Some("light_quadruple_dashed") => BorderType::LightQuadrupleDashed,
        Some("heavy_quadruple_dashed") => BorderType::HeavyQuadrupleDashed,
        Some("quadrant_inside") => BorderType::QuadrantInside,
        Some("quadrant_outside") => BorderType::QuadrantOutside,
        Some(o) => return Err(format!("border_type {o:?} isn't one ratatui draws")),
    })
}

fn borders(v: &Value) -> Res<Borders> {
    match v {
        Value::Null => Ok(Borders::ALL),
        Value::String(s) if s == "all" => Ok(Borders::ALL),
        Value::String(s) if s == "none" => Ok(Borders::NONE),
        Value::Array(xs) => xs.iter().try_fold(Borders::NONE, |b, x| {
            Ok(b | match x.as_str() {
                Some("top") => Borders::TOP,
                Some("right") => Borders::RIGHT,
                Some("bottom") => Borders::BOTTOM,
                Some("left") => Borders::LEFT,
                _ => return Err(format!("{x} isn't top, right, bottom or left")),
            })
        }),
        _ => Err("borders is \"all\", \"none\" or a list of sides".into()),
    }
}

fn shadow(v: &Value, th: &Theme) -> Res<Option<Shadow>> {
    let base = match v.get("kind").and_then(Value::as_str) {
        None | Some("dark_shade") => Shadow::dark_shade(),
        Some("overlay") => Shadow::overlay(),
        Some("block") => Shadow::block(),
        Some("light_shade") => Shadow::light_shade(),
        Some("medium_shade") => Shadow::medium_shade(),
        Some(o) => return Err(format!("shadow kind {o:?} isn't overlay, block, light_shade, medium_shade or dark_shade")),
    };
    Ok(match v {
        Value::Null | Value::Bool(false) => None,
        Value::Bool(true) => Some(base.style(Style::default().fg(color("$bar", th)?))),
        Value::Object(o) => {
            let mut s = base.style(if o.contains_key("style") { style_at(v, "style", th)? } else { Style::default().fg(color("$bar", th)?) });
            if let Some(off) = o.get("offset") {
                let xy: Vec<i64> = off.as_array().map(|a| a.iter().filter_map(Value::as_i64).collect()).unwrap_or_default();
                let [x, y] = xy[..] else { return Err("shadow offset is [x, y]".into()) };
                s = s.offset(Offset { x: x.clamp(-4, 4) as i32, y: y.clamp(-4, 4) as i32 });
            }
            Some(s)
        }
        _ => return Err("shadow is true or { kind, offset, style }".into()),
    })
}

fn merge(v: &Value) -> Res<MergeStrategy> {
    Ok(match v.as_str() {
        None | Some("replace") => MergeStrategy::Replace,
        Some("exact") => MergeStrategy::Exact,
        Some("fuzzy") => MergeStrategy::Fuzzy,
        Some(o) => return Err(format!("merge {o:?} isn't replace, exact or fuzzy")),
    })
}

// A Block from its fields (an element's `block`, or a `block` element's own).
pub fn block(v: &Value, ink: &Ink) -> Res<Block<'static>> {
    let th = ink.th;
    let mut b = Block::new()
        .borders(borders(&v["borders"])?)
        .border_type(border_type(&v["border_type"])?)
        .border_style(style_at(v, "border_style", th)?)
        .style(style_at(v, "style", th)?)
        .padding(padding(&v["padding"])?)
        .merge_borders(merge(&v["merge"])?);
    if !v["title"].is_null() {
        b = b.title(line(&v["title"], ink).map_err(|e| format!("title: {e}"))?);
    }
    for t in v["titles"].as_array().map(Vec::as_slice).unwrap_or_default() {
        let mut l = line(&t["content"], ink).map_err(|e| format!("titles: {e}"))?;
        if let Some(a) = align(&t["align"])? {
            l = l.alignment(a);
        }
        b = match t["position"].as_str() {
            None | Some("top") => b.title_top(l),
            Some("bottom") => b.title_bottom(l),
            Some(o) => return Err(format!("title position {o:?} isn't top or bottom")),
        };
    }
    if let Some(s) = shadow(&v["shadow"], th)? {
        b = b.shadow(s);
    }
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::themes::THEMES;
    use serde_json::json;

    #[test]
    fn reads_constraints() {
        assert_eq!(constraint(&json!(12)), Ok(Constraint::Length(12)));
        assert_eq!(constraint(&json!("12")), Ok(Constraint::Length(12)));
        assert_eq!(constraint(&json!("30%")), Ok(Constraint::Percentage(30)));
        assert_eq!(constraint(&json!("1/3")), Ok(Constraint::Ratio(1, 3)));
        assert_eq!(constraint(&json!(">=5")), Ok(Constraint::Min(5)));
        assert_eq!(constraint(&json!("<=20")), Ok(Constraint::Max(20)));
        assert_eq!(constraint(&json!("*")), Ok(Constraint::Fill(1)));
        assert_eq!(constraint(&json!("2*")), Ok(Constraint::Fill(2)));
        for bad in [json!("101%"), json!("1/0"), json!("x"), json!(-1), json!(70000)] {
            assert!(constraint(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn reads_styles() {
        let th = &THEMES[0].1;
        let s = style(&json!("bold italic $accent on #102030"), th).unwrap();
        assert_eq!(s.fg, Some(Color::from_str(th.accent).unwrap()));
        assert_eq!(s.bg, Some(Color::Rgb(0x10, 0x20, 0x30)));
        assert_eq!(s.add_modifier, Modifier::BOLD | Modifier::ITALIC);
        let s = style(&json!({ "fg": "light-red", "bold": false, "underline_color": "42" }), th).unwrap();
        assert_eq!((s.fg, s.sub_modifier, s.underline_color), (Some(Color::LightRed), Modifier::BOLD, Some(Color::Indexed(42))));
        assert!(style(&json!("$nope"), th).unwrap_err().contains("theme colour"));
        assert!(style(&json!("red blue"), th).is_err());
        assert!(style(&json!({ "colour": "red" }), th).is_err());
    }

    #[test]
    fn reads_text() {
        let th = &THEMES[0].1;
        let ink = Ink::plain(th);
        let t = text(&json!(["plain", [{ "text": "a\tb", "style": "bold" }, "c"], { "spans": ["d"], "align": "right" }]), &ink).unwrap();
        assert_eq!(t.lines.len(), 3);
        assert_eq!(t.lines[1].spans[0].content, "a    b");
        assert_eq!(t.lines[1].spans[0].style.add_modifier, Modifier::BOLD);
        assert_eq!(t.lines[2].alignment, Some(HorizontalAlignment::Right));
        assert_eq!(text(&json!("one\ntwo"), &ink).unwrap().lines.len(), 2);
        assert_eq!(text(&json!({ "lines": [{ "spans": ["no align"] }] }), &ink).unwrap().alignment, None);
        assert!(line(&json!(3), &ink).is_err());
    }

    #[test]
    fn reads_blocks() {
        let ink = Ink::plain(&THEMES[0].1);
        assert!(block(&json!({ "borders": ["top", "left"], "border_type": "rounded", "title": "x", "titles": [{ "content": "y", "position": "bottom", "align": "right" }], "padding": [1, 2], "shadow": true }), &ink).is_ok());
        assert!(block(&json!({ "border_type": "wavy" }), &ink).is_err());
        assert!(block(&json!({ "padding": [1, 2, 3] }), &ink).is_err());
        assert_eq!(marker(&json!("x")), Ok(Marker::Custom('x')));
        assert!(marker(&json!("xy")).is_err());
    }
}
