// Markdown (CommonMark and GitHub's extensions: tables, task lists, strikethrough, footnotes, alerts), by tui-markdown in
// the theme's colours; fenced code at the top level is highlighted as `code` is. What a document renders to is kept by
// content, with each line's height at the widths it's been drawn at, so a frame draws only the rows that show.
use std::cell::RefCell;
use std::collections::HashMap;

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Widget, Wrap};
use serde_json::Value;
use tui_markdown::{AlertKind, StyleSheet};

use super::build::{bar_room, hash_of, scrolled, scrollbar, Ctx, Elems};
use super::highlight::{self, Look};
use crate::client::design::color;
use crate::config::themes::Theme;
use crate::protocol::ui::token;

// The theme as tui-markdown's style sheet.
#[derive(Clone)]
struct Sheet(Theme);

impl Sheet {
    fn fg(&self, name: &str) -> Style {
        Style::new().fg(color(token(&self.0, name).unwrap_or(self.0.fg)))
    }
}

impl StyleSheet for Sheet {
    fn heading(&self, level: u8) -> Style {
        match level {
            1 => self.fg("accent").add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            2 => self.fg("accent").add_modifier(Modifier::BOLD),
            3 => self.fg("focus").add_modifier(Modifier::BOLD),
            _ => self.fg("focus").add_modifier(Modifier::ITALIC),
        }
    }
    fn code(&self) -> Style {
        self.fg("done")
    }
    fn link(&self) -> Style {
        self.fg("focus").add_modifier(Modifier::UNDERLINED)
    }
    fn blockquote(&self) -> Style {
        self.fg("dim").add_modifier(Modifier::ITALIC)
    }
    fn heading_meta(&self) -> Style {
        self.fg("dim")
    }
    fn metadata_block(&self) -> Style {
        self.fg("dim")
    }
    fn code_block_fence(&self) -> &str {
        ""
    }
    fn html(&self) -> Style {
        self.fg("dim")
    }
    fn footnote_ref(&self) -> Style {
        self.fg("dim").add_modifier(Modifier::ITALIC)
    }
    fn footnote_def(&self) -> Style {
        self.fg("dim")
    }
    fn alert(&self, kind: AlertKind) -> Style {
        self.fg(match kind {
            AlertKind::Note => "focus",
            AlertKind::Tip => "done",
            AlertKind::Important => "accent",
            AlertKind::Warning => "warn",
            AlertKind::Caution => "blocked",
        })
    }
    fn alert_icon(&self, _: AlertKind) -> &str {
        "▍"
    }
    fn table_header(&self) -> Style {
        self.fg("accent").add_modifier(Modifier::BOLD)
    }
    fn table_border(&self) -> Style {
        self.fg("border")
    }
    fn list_marker(&self) -> Style {
        self.fg("accent")
    }
}

fn owned(t: Text<'_>) -> Vec<Line<'static>> {
    t.lines
        .into_iter()
        .map(|l| Line { spans: l.spans.into_iter().map(|s| Span::styled(s.content.into_owned(), s.style)).collect(), style: l.style, alignment: l.alignment })
        .collect()
}

// A document as lines: its prose by tui-markdown, its fenced code (outside lists and quotes) highlighted.
// ponytail: fenced code inside a list item or a quote keeps tui-markdown's plain code colour
fn render(th: &Theme, content: &str, look: Look) -> Vec<Line<'static>> {
    let options = tui_markdown::Options::new(Sheet(*th));
    let mut out: Vec<Line<'static>> = vec![];
    let gap = |out: &mut Vec<Line<'static>>| {
        if out.last().is_some_and(|l| l.width() > 0) {
            out.push(Line::default());
        }
    };
    let prose = |out: &mut Vec<Line<'static>>, s: &str| {
        let lines = owned(tui_markdown::from_str_with_options(s, &options));
        if lines.iter().any(|l| l.width() > 0) {
            gap(out);
            out.extend(lines);
        }
    };
    let (mut depth, mut from) = (0, 0);
    let mut code: Option<(String, String)> = None; // (language, text) of the block being read
    // read as tui-markdown reads it, so the blocks are the same
    let read = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS | Options::ENABLE_HEADING_ATTRIBUTES | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS | Options::ENABLE_SUPERSCRIPT | Options::ENABLE_SUBSCRIPT | Options::ENABLE_MATH | Options::ENABLE_FOOTNOTES | Options::ENABLE_DEFINITION_LIST | Options::ENABLE_GFM | Options::ENABLE_TABLES;
    for (ev, at) in Parser::new_ext(content, read).into_offset_iter() {
        match ev {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(lang))) if depth == 0 => {
                prose(&mut out, &content[from..at.start]);
                code = Some((lang.split_whitespace().next().unwrap_or("").to_string(), String::new()));
                depth += 1;
            }
            Event::Start(_) => depth += 1,
            Event::Text(t) => {
                if let Some((_, text)) = code.as_mut() {
                    text.push_str(&t);
                }
            }
            Event::End(TagEnd::CodeBlock) if depth == 1 && code.is_some() => {
                depth -= 1;
                let (lang, text) = code.take().unwrap();
                gap(&mut out);
                out.extend(highlight::snippet(&text, Some(&lang).filter(|l| !l.is_empty()).map(String::as_str), look).into_iter().map(|l| {
                    let mut spans = vec![Span::raw("  ")];
                    spans.extend(l.spans);
                    Line::from(spans).style(l.style)
                }));
                from = at.end;
            }
            Event::End(_) => depth -= 1,
            _ => {}
        }
    }
    prose(&mut out, &content[from..]);
    out
}

// A document rendered, and its lines' heights by width.
struct Doc {
    lines: Vec<Line<'static>>,
    heights: HashMap<u16, (Vec<usize>, usize)>,
}

thread_local! {
    static DOCS: RefCell<HashMap<u64, Doc>> = RefCell::new(HashMap::new());
}

pub fn draw(ctx: &Ctx, c: &mut crate::client::draw::Canvas, n: &Value, key: &str, area: Rect, elems: &mut Elems) {
    let content = n["content"].as_str().unwrap_or("");
    let theme = n["syntax_theme"].as_str();
    let id = hash_of((content, format!("{:?}", ctx.th), theme));
    DOCS.with(|docs| {
        let mut docs = docs.borrow_mut();
        if docs.len() >= 16 && !docs.contains_key(&id) {
            docs.clear();
        }
        let doc = docs.entry(id).or_insert_with(|| Doc { lines: render(ctx.th, content, highlight::look(ctx.th, theme)), heights: HashMap::new() });
        let mut at = |w: u16| -> (Vec<usize>, usize) {
            doc.heights
                .entry(w)
                .or_insert_with(|| {
                    let hs: Vec<usize> = doc.lines.iter().map(|l| Paragraph::new(l.clone()).wrap(Wrap { trim: false }).line_count(w)).collect();
                    let total = hs.iter().sum();
                    (hs, total)
                })
                .clone()
        };
        let (body, bar) = bar_room(n, area, at(area.width).1, true);
        let (heights, total) = at(body.width);
        let st = elems.entry(key.to_string()).or_default();
        let top = scrolled(st, total, body.height as usize);
        // the lines that show: from the one the top row is in, until the rows are filled
        let (mut first, mut above) = (0, 0);
        while first < heights.len() && above + heights[first] <= top {
            above += heights[first];
            first += 1;
        }
        let mut last = first;
        let mut rows = above;
        while last < heights.len() && rows < top + body.height as usize {
            rows += heights[last];
            last += 1;
        }
        Paragraph::new(doc.lines[first..last].to_vec()).wrap(Wrap { trim: false }).scroll(((top - above) as u16, 0)).render(body, c.buf);
        if bar {
            scrollbar(ctx, c.buf, area, total, body.height as usize, top);
        }
    });
}
