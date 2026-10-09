// Syntax highlighting for code, diffs and Markdown's fenced code: two-face's syntaxes (syntect's own and bat's), in the
// user's theme by default (each kind of token one of its colours), or in one of two-face's themes (`syntax_theme`).
// Highlighting runs only as far down as has been shown, and what's done is kept by content: a frame that shows the same
// lines again looks them up.
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;
use std::sync::{LazyLock, Mutex};

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use syntect::highlighting::{Color as SynColor, FontStyle, HighlightIterator, HighlightState, Highlighter, ScopeSelectors, Style as SynStyle, StyleModifier, Theme as SynTheme, ThemeItem, ThemeSettings};
use syntect::parsing::{ParseState, ScopeStack, SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;
use two_face::theme::EmbeddedLazyThemeSet;

use super::build::hash_of;
use crate::client::design::color;
use crate::config::themes::Theme;
use crate::protocol::ui;

static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(two_face::syntax::extra_newlines);
static THEMES: LazyLock<EmbeddedLazyThemeSet> = LazyLock::new(two_face::theme::extra);

// A line longer than this isn't highlighted (a minified file would take seconds); it shows plain.
const LONG: usize = 4096;

// How code is coloured: a syntect theme, and the colours it asks for under and around its tokens (none for the
// user's theme: the element's own show).
#[derive(Clone, Copy)]
pub struct Look {
    pub theme: &'static SynTheme,
    pub fg: Option<Color>,
    pub bg: Option<Color>,
}

// What each kind of token is drawn in, from the user's theme; the most specific selector wins.
const TOKENS: &[(&str, &str, u8)] = &[
    ("comment, punctuation.definition.comment", "dim", ITALIC),
    ("string, constant.character, punctuation.definition.string, markup.raw", "done", 0),
    ("string.regexp, constant.character.escape", "working", 0),
    ("constant.numeric, constant.language, constant.other, support.constant, variable.other.constant", "warn", 0),
    ("keyword, storage, keyword.control, keyword.declaration", "accent", BOLD),
    ("keyword.operator", "fg", 0),
    ("entity.name.type, entity.name.class, entity.name.struct, entity.name.enum, entity.name.trait, entity.name.interface, support.type, support.class, storage.type.primitive", "working", 0),
    ("entity.name.function, support.function, variable.function, meta.function-call.identifier", "focus", 0),
    ("entity.name.tag", "accent", 0),
    ("entity.other.attribute-name, meta.annotation, meta.decorator, entity.name.function.decorator", "working", ITALIC),
    ("variable.language", "warn", ITALIC),
    ("variable.parameter", "fg", ITALIC),
    ("markup.heading, entity.name.section", "accent", BOLD),
    ("markup.bold", "fg", BOLD),
    ("markup.italic", "fg", ITALIC),
    ("markup.underline.link, markup.link", "focus", UNDERLINE),
    ("markup.inserted", "done", 0),
    ("markup.deleted, invalid", "blocked", 0),
    ("markup.changed", "warn", 0),
];
// syntect's FontStyle bits
const BOLD: u8 = 1;
const UNDERLINE: u8 = 2;
const ITALIC: u8 = 4;

fn syn(hex: &str) -> SynColor {
    match color(hex) {
        Color::Rgb(r, g, b) => SynColor { r, g, b, a: 0xff },
        _ => SynColor::WHITE,
    }
}

// The user's theme as a syntect theme.
fn of_tokens(th: &Theme) -> SynTheme {
    let item = |(sel, tone, fs): &(&str, &str, u8)| ThemeItem {
        scope: ScopeSelectors::from_str(sel).unwrap_or_default(),
        style: StyleModifier { foreground: Some(syn(ui::token(th, tone).unwrap_or(th.fg))), background: None, font_style: Some(FontStyle::from_bits_truncate(*fs)) },
    };
    SynTheme { name: None, author: None, settings: ThemeSettings { foreground: Some(syn(th.fg)), ..Default::default() }, scopes: TOKENS.iter().map(item).collect() }
}

// A syntect colour as the terminal's. Like bat, an alpha of 0 means an ANSI colour (its index in red), 1 the default.
fn of_syn(c: SynColor) -> Option<Color> {
    match c.a {
        0 => Some(Color::Indexed(c.r)),
        1 => None,
        _ => Some(Color::Rgb(c.r, c.g, c.b)),
    }
}

// The colours for an element: its `syntax_theme`, else the user's theme.
pub fn look(th: &Theme, name: Option<&str>) -> Look {
    if let Some(t) = name.and_then(|n| ui::syntax_theme(n).ok()).and_then(|n| EmbeddedLazyThemeSet::theme_names().iter().find(|t| t.as_name() == n)) {
        let theme = THEMES.get(*t);
        return Look { theme, fg: theme.settings.foreground.and_then(of_syn), bg: theme.settings.background.and_then(of_syn) };
    }
    // one per theme the user has used (a handful), kept for good
    static MINE: LazyLock<Mutex<HashMap<String, &'static SynTheme>>> = LazyLock::new(Default::default);
    let theme = *MINE.lock().unwrap().entry(format!("{th:?}")).or_insert_with(|| Box::leak(Box::new(of_tokens(th))));
    Look { theme, fg: None, bg: None }
}

// The syntax for a language (a name, a file extension, a file's name or path); without one that's known, the one its
// first line names (#!/bin/sh, a modeline). None: plain text.
pub fn syntax(lang: Option<&str>, first: &str) -> Option<&'static SyntaxReference> {
    let ss: &'static SyntaxSet = &SYNTAXES;
    let named = lang.map(str::trim).filter(|l| !l.is_empty()).and_then(|l| {
        let p = Path::new(l);
        let file = p.file_name().and_then(|f| f.to_str()).unwrap_or(l);
        ss.find_syntax_by_token(l).or_else(|| ss.find_syntax_by_token(file)).or_else(|| p.extension().and_then(|e| e.to_str()).and_then(|e| ss.find_syntax_by_extension(e)))
    });
    named.or_else(|| ss.find_syntax_by_first_line(first)).filter(|s| s.name != "Plain Text")
}

fn tabs(s: &str) -> String {
    s.replace('\t', "    ")
}

fn plain(line: &str, look: Look) -> Line<'static> {
    let l = Line::raw(tabs(line.trim_end_matches(['\n', '\r'])));
    match look.fg {
        Some(fg) => l.style(Style::new().fg(fg)),
        None => l,
    }
}

fn style(s: SynStyle) -> Style {
    let mut st = Style::new();
    if let Some(c) = of_syn(s.foreground) {
        st = st.fg(c);
    }
    for (f, m) in [(FontStyle::BOLD, Modifier::BOLD), (FontStyle::ITALIC, Modifier::ITALIC), (FontStyle::UNDERLINE, Modifier::UNDERLINED)] {
        if s.font_style.contains(f) {
            st = st.add_modifier(m);
        }
    }
    st
}

// Highlighting for one content, as far as it's got.
struct Painted {
    lines: Vec<Line<'static>>,
    parse: ParseState,
    state: HighlightState,
    at: usize, // the byte it's reached
}

thread_local! {
    static DONE: RefCell<HashMap<u64, Painted>> = RefCell::new(HashMap::new());
}

// Lines `from..from + count` of `content` (split as `str::lines` does), highlighted as `syntax`. `id` names the content
// (a hash of it, or of where it came from): for the same id, highlighting carries on where it stopped.
pub fn lines(id: u64, content: &str, syntax: Option<&'static SyntaxReference>, look: Look, from: usize, count: usize) -> Vec<Line<'static>> {
    let Some(syntax) = syntax else {
        return content.lines().skip(from).take(count).map(|l| plain(l, look)).collect();
    };
    let key = hash_of((id, &syntax.name, look.theme as *const SynTheme as usize));
    DONE.with(|done| {
        let mut done = done.borrow_mut();
        if done.len() >= 64 && !done.contains_key(&key) {
            done.clear(); // ponytail: forgets everything at 64 contents, rather than the least used
        }
        let hl = Highlighter::new(look.theme);
        let p = done.entry(key).or_insert_with(|| Painted { lines: vec![], parse: ParseState::new(syntax), state: HighlightState::new(&hl, ScopeStack::new()), at: 0 });
        let want = from.saturating_add(count);
        if p.lines.len() < want {
            for line in LinesWithEndings::from(content.get(p.at..).unwrap_or("")) {
                p.at += line.len();
                let l = paint(line, &mut p.parse, &mut p.state, &hl, look);
                p.lines.push(l);
                if p.lines.len() >= want {
                    break;
                }
            }
        }
        p.lines.iter().skip(from).take(count).cloned().collect()
    })
}

fn paint(line: &str, parse: &mut ParseState, state: &mut HighlightState, hl: &Highlighter, look: Look) -> Line<'static> {
    if line.len() > LONG {
        return plain(line, look);
    }
    let Ok(ops) = parse.parse_line(line, &SYNTAXES) else { return plain(line, look) };
    let spans: Vec<Span> = HighlightIterator::new(state, &ops, line, hl)
        .filter_map(|(s, t)| {
            let t = t.trim_end_matches(['\n', '\r']);
            (!t.is_empty()).then(|| Span::styled(tabs(t), style(s)))
        })
        .collect();
    Line::from(spans)
}

// All of a snippet, highlighted (Markdown's fenced code).
pub fn snippet(content: &str, lang: Option<&str>, look: Look) -> Vec<Line<'static>> {
    lines(hash_of(content), content, syntax(lang, content.lines().next().unwrap_or("")), look, 0, usize::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::themes::THEMES as MODISA;

    #[test]
    fn finds_languages_by_name_extension_path_or_first_line() {
        let name = |l: Option<&str>, first: &str| syntax(l, first).map(|s| s.name.clone());
        assert_eq!(name(Some("rust"), ""), Some("Rust".into()));
        assert_eq!(name(Some("ts"), ""), Some("TypeScript".into()));
        assert_eq!(name(Some("src/client/x.toml"), ""), Some("TOML".into()));
        assert_eq!(name(Some("Dockerfile"), ""), Some("Dockerfile".into()));
        assert_eq!(name(None, "#!/bin/bash"), Some("Bourne Again Shell (bash)".into()));
        assert_eq!(name(None, "just words"), None);
    }

    #[test]
    fn colours_tokens_from_the_theme() {
        let th = &MODISA[0].1;
        let look = look(th, None);
        let l = &lines(1, "fn main() { let s = \"hi\"; } // done", syntax(Some("rust"), ""), look, 0, 1)[0];
        let of = |text: &str| l.spans.iter().find(|s| s.content == text).map(|s| s.style).unwrap_or_else(|| panic!("{:?}", l.spans));
        assert_eq!(of("fn").fg, Some(color(th.accent)));
        assert!(of("fn").add_modifier.contains(Modifier::BOLD));
        assert_eq!(of("hi").fg, Some(color(th.done)));
        assert_eq!(of("main").fg, Some(color(th.focus)));
        assert_eq!(of(" done").fg, Some(color(th.dim)));
        assert!(of(" done").add_modifier.contains(Modifier::ITALIC));
        assert_eq!(l.spans.iter().map(|s| s.content.as_ref()).collect::<String>(), "fn main() { let s = \"hi\"; } // done");
    }

    #[test]
    fn every_syntax_theme_is_there() {
        for name in ui::SYNTAX_THEMES {
            let l = look(&MODISA[0].1, Some(name));
            assert!(l.theme.name.is_some() || l.fg.is_some() || l.bg.is_some(), "{name}");
        }
        assert!(look(&MODISA[0].1, Some("dracula")).bg.is_some());
    }

    #[test]
    fn big_inputs_highlight_only_as_far_as_shown() {
        let big: String = (0..20_000).map(|i| format!("let x{i} = {i}; // line\n")).collect::<String>() + &"x".repeat(100_000);
        let start = std::time::Instant::now();
        let shown = lines(hash_of(&big), &big, syntax(Some("rs"), ""), look(&MODISA[0].1, None), 0, 40);
        assert_eq!(shown.len(), 40);
        assert!(start.elapsed().as_secs() < 2);
        let last = lines(hash_of(&big), &big, syntax(Some("rs"), ""), look(&MODISA[0].1, None), 20_000, 5);
        assert_eq!(last[0].width(), 100_000); // too long to highlight: plain
    }
}
