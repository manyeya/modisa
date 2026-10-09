// The format language of config.toml (examples/config/CUSTOMIZE.md, Formats): text with {variables}, conditionals and
// #[styles] that the status row, tab labels, pane titles, the window title and the sidebar's agent rows can be drawn
// from. A format becomes runs of styled text, each with what a click on it runs, on the left and (after #[align=right])
// the right.
//
//   {name} {name:arg}               a variable; unknown ones are empty
//   {?name|then|else}                then when name is set and not 0/false/empty (both may hold variables)
//   {name=value|then|else}           then when name is value
//   {name:=N} {name:<N}              at most N cells (cut with …), padded to N cells
//   #[style]  #[]                    style from here (a UI 3 style string: "bold $warn on $bar"); back to the base
//   #[click=action] … #[/click]      a part a click runs (a modisa action, plugin:<p>.<a>, a [[command]], sh:<cmd>)
//   #[align=right]                   what follows goes to the right edge
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use ratatui::style::Style;
use serde_json::Value;

use crate::config::themes::Theme;
use crate::core::text::width;
use crate::protocol::ui;

#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub text: String,
    pub style: Style,
    pub click: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Formatted {
    pub left: Vec<Run>,
    pub right: Vec<Run>,
}

impl Formatted {
    pub fn text(&self) -> String {
        self.left.iter().chain(&self.right).map(|r| r.text.as_str()).collect()
    }
}

// What a variable is: text as it is, or (modisa's own buttons) format text read again.
pub enum Var {
    Text(String),
    Format(String),
}

pub type Lookup<'a> = &'a dyn Fn(&str, Option<&str>) -> Option<Var>;

struct Renderer<'a> {
    out: Formatted,
    right: bool,
    style: Style,
    base: Style,
    click: Option<String>,
    th: &'a Theme,
    vars: Lookup<'a>,
    depth: usize,
}

impl Renderer<'_> {
    fn push(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let side = if self.right { &mut self.out.right } else { &mut self.out.left };
        match side.last_mut() {
            Some(r) if r.style == self.style && r.click == self.click => r.text.push_str(text),
            _ => side.push(Run { text: text.to_string(), style: self.style, click: self.click.clone() }),
        }
    }

    fn directive(&mut self, d: &str) {
        let d = d.trim();
        if d.is_empty() {
            self.style = self.base;
        } else if let Some(a) = d.strip_prefix("click=") {
            self.click = Some(a.trim().to_string());
        } else if d == "/click" {
            self.click = None;
        } else if d == "align=right" {
            self.right = true;
        } else if let Ok(s) = ui::style(&Value::String(d.to_string()), self.th) {
            self.style = self.style.patch(s);
        }
    }

    fn value(&self, name: &str, arg: Option<&str>) -> Option<Var> {
        (self.vars)(name, arg)
    }

    fn text_of(&self, name: &str) -> String {
        match self.value(name, None) {
            Some(Var::Text(t)) | Some(Var::Format(t)) => t,
            None => String::new(),
        }
    }

    fn expr(&mut self, e: &str) {
        if self.depth > 8 {
            return;
        }
        if let Some(rest) = e.strip_prefix('?') {
            let parts = split_top(rest, '|');
            let on = truthy(&self.text_of(parts[0].trim()));
            self.branch(&parts, on);
            return;
        }
        let parts = split_top(e, '|');
        if parts.len() > 1 {
            if let Some((name, want)) = parts[0].split_once('=') {
                let on = self.text_of(name.trim()) == want.trim();
                self.branch(&parts, on);
                return;
            }
        }
        let (name, arg) = match e.split_once(':') {
            Some((n, a)) => (n.trim(), Some(a)),
            None => (e.trim(), None),
        };
        // {name:=N} and {name:<N}: cut or pad what the variable is
        let size = arg.and_then(|a| a.strip_prefix('=').map(|n| (true, n)).or_else(|| a.strip_prefix('<').map(|n| (false, n)))).and_then(|(cut, n)| Some((cut, n.trim().parse::<usize>().ok()?)));
        let value = self.value(name, if size.is_some() { None } else { arg });
        match (value, size) {
            (Some(Var::Format(f)), None) => {
                self.depth += 1;
                self.feed(&f);
                self.depth -= 1;
            }
            (Some(Var::Text(t)) | Some(Var::Format(t)), Some((true, n))) => self.push(&crate::client::design::fit(&t, n)),
            (Some(Var::Text(t)) | Some(Var::Format(t)), Some((false, n))) => {
                let pad = n.saturating_sub(width(&t));
                self.push(&format!("{t}{}", " ".repeat(pad)));
            }
            (Some(Var::Text(t)), None) => self.push(&t),
            (None, Some((false, n))) => self.push(&" ".repeat(n)),
            (None, _) => {}
        }
    }

    fn branch(&mut self, parts: &[&str], on: bool) {
        let chosen = if on { parts.get(1) } else { parts.get(2) };
        if let Some(f) = chosen {
            self.depth += 1;
            self.feed(f);
            self.depth -= 1;
        }
    }

    fn feed(&mut self, f: &str) {
        let mut text = String::new();
        let mut rest = f;
        while let Some(c) = rest.chars().next() {
            if c == '#' && rest[1..].starts_with('[') {
                if let Some(end) = rest.find(']') {
                    self.push(&std::mem::take(&mut text));
                    self.directive(&rest[2..end]);
                    rest = &rest[end + 1..];
                    continue;
                }
            }
            if c == '{' {
                if let Some(end) = matching(rest) {
                    self.push(&std::mem::take(&mut text));
                    self.expr(&rest[1..end]);
                    rest = &rest[end + 1..];
                    continue;
                }
            }
            text.push(c);
            rest = &rest[c.len_utf8()..];
        }
        self.push(&text);
    }
}

// where the { at the start of `s` is closed, counting the ones inside it
fn matching(s: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

// `s` split at `sep` where it isn't inside { }
fn split_top(s: &str, sep: char) -> Vec<&str> {
    let (mut out, mut depth, mut from) = (vec![], 0i32, 0);
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            c if c == sep && depth == 0 => {
                out.push(&s[from..i]);
                from = i + c.len_utf8();
            }
            _ => {}
        }
    }
    out.push(&s[from..]);
    out
}

pub fn truthy(s: &str) -> bool {
    !s.is_empty() && s != "0" && s != "false"
}

pub fn render(format: &str, vars: Lookup, th: &Theme, base: Style) -> Formatted {
    let mut r = Renderer { out: Formatted::default(), right: false, style: base, base, click: None, th, vars, depth: 0 };
    r.feed(format);
    r.out
}

// ---------- variables every surface has ----------

// {clock:%H:%M} (and {date}): local time in strftime's terms
pub fn clock(fmt: &str) -> String {
    let fmt = std::ffi::CString::new(if fmt.is_empty() { "%H:%M" } else { fmt }).unwrap_or_default();
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&now, &mut tm) };
    let mut buf = [0u8; 128];
    let n = unsafe { libc::strftime(buf.as_mut_ptr().cast(), buf.len(), fmt.as_ptr(), &tm) };
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

thread_local! {
    // {sh:command|interval}: the last line each command printed, when it ran, and whether it's running now
    static SH: RefCell<HashMap<String, (String, Instant, bool)>> = RefCell::new(HashMap::new());
    // asks for a frame when a command's answer comes
    static REDRAW: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
}

pub fn on_answer(redraw: Rc<dyn Fn()>) {
    REDRAW.with_borrow_mut(|r| *r = Some(redraw));
}

// {sh:git log -1 --format=%s|30s}: its first line of output, run again after its interval (default 10s); empty until
// it first answers. It runs in the background, where the client was started.
pub fn sh(arg: &str) -> String {
    let (cmd, every) = match arg.rsplit_once('|') {
        Some((c, i)) if i.trim().ends_with('s') && i.trim()[..i.trim().len() - 1].parse::<u64>().is_ok() => (c, Duration::from_secs(i.trim()[..i.trim().len() - 1].parse().unwrap_or(10))),
        _ => (arg, Duration::from_secs(10)),
    };
    let (value, due) = SH.with_borrow(|m| match m.get(cmd) {
        Some((v, at, running)) => (v.clone(), !running && at.elapsed() >= every),
        None => (String::new(), true),
    });
    if due {
        SH.with_borrow_mut(|m| {
            let e = m.entry(cmd.to_string()).or_insert((String::new(), Instant::now(), true));
            e.2 = true;
        });
        let cmd = cmd.to_string();
        tokio::task::spawn_local(async move {
            let out = tokio::process::Command::new("sh").arg("-c").arg(&cmd).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().await;
            let line = out.map(|o| String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or("").to_string()).unwrap_or_default();
            SH.with_borrow_mut(|m| m.insert(cmd, (crate::core::text::clean_text(&line, 200), Instant::now(), false)));
            if let Some(r) = REDRAW.with_borrow(|r| r.clone()) {
                r();
            }
        });
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::themes::THEMES;
    use ratatui::style::Modifier;

    fn vars(name: &str, arg: Option<&str>) -> Option<Var> {
        Some(Var::Text(match (name, arg) {
            ("space", _) => "dev".into(),
            ("blocked", _) => "2".into(),
            ("working", _) => "0".into(),
            ("name", _) => "reviewer-of-everything".into(),
            ("state", _) => "blocked".into(),
            ("echo", Some(a)) => a.to_string(),
            ("button", _) => return Some(Var::Format("#[click=palette]:#[/click]".into())),
            _ => return None,
        }))
    }

    #[test]
    fn variables_conditionals_and_sizes() {
        let th = &THEMES[0].1;
        let f = |s: &str| render(s, &vars, th, Style::new()).text();
        assert_eq!(f("{space} {nope}|"), "dev |");
        assert_eq!(f("{?blocked|{blocked} need you|calm}"), "2 need you");
        assert_eq!(f("{?working|busy|calm}"), "calm");
        assert_eq!(f("{state=blocked|!|·}{state=done|✓|}"), "!");
        assert_eq!(f("[{name:=8}]"), "[reviewe…]");
        assert_eq!(f("[{space:<5}]"), "[dev  ]");
        assert_eq!(f("{echo:a:b}"), "a:b");
        assert_eq!(f("{?blocked|x {?working|y|{space}}|}"), "x dev");
    }

    #[test]
    fn styles_clicks_and_the_right_side() {
        let th = &THEMES[0].1;
        let out = render("#[bold $warn]a#[]b#[click=zoom]c#[/click]{button}#[align=right]d", &vars, th, Style::new());
        assert_eq!(out.left.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), ["a", "b", "c", ":"]);
        assert!(out.left[0].style.add_modifier.contains(Modifier::BOLD) && out.left[0].style.fg.is_some());
        assert_eq!(out.left[1].style, Style::new());
        assert_eq!(out.left[2].click.as_deref(), Some("zoom"));
        assert_eq!(out.left[3].click.as_deref(), Some("palette")); // a button: format, read again
        assert_eq!(out.right[0].text, "d");
    }

    #[test]
    fn a_value_is_never_read_as_a_format() {
        let th = &THEMES[0].1;
        let sneaky = |_: &str, _: Option<&str>| Some(Var::Text("#[click=sh:rm -rf ~]x{space}".into()));
        let out = render("{title}", &sneaky, th, Style::new());
        assert_eq!(out.left.len(), 1);
        assert_eq!(out.left[0].click, None);
        assert_eq!(out.left[0].text, "#[click=sh:rm -rf ~]x{space}");
    }

    #[test]
    fn the_clock_formats_local_time() {
        let t = clock("%H:%M");
        assert!(t.len() == 5 && t.as_bytes()[2] == b':', "{t}");
    }
}
