// `modisa view render <tree.json|-> [--size 80x24] [--theme name] [--ansi]`: a plugin view's tree drawn without a
// session, as the TUI draws it, for plugin authors' snapshot tests.
use std::io::{Read, Write};

use serde_json::Value;

use super::args::Args;
use crate::client::views::headless;
use crate::config::themes::{find_theme, THEMES};

const USAGE: &str = "usage: modisa view render <tree.json|-> [--size 80x24] [--theme name] [--ansi]";

pub fn run_view(verb: Option<&str>, rest: &[&str], a: &Args) -> i32 {
    let (Some("render"), Some(src)) = (verb, rest.first()) else {
        errln!("{USAGE}");
        return 2;
    };
    let size = a.str("size").unwrap_or("80x24");
    let Some((w, h)) = size.split_once('x').and_then(|(w, h)| Some((w.parse::<u16>().ok()?, h.parse::<u16>().ok()?))).filter(|(w, h)| (1..=1000).contains(w) && (1..=1000).contains(h)) else {
        errln!("--size is columns x rows, 1 to 1000 each (80x24)");
        return 2;
    };
    let name = a.str("theme").unwrap_or("tokyonight");
    let Some(th) = find_theme(name) else {
        errln!("no theme {name:?}: {}", THEMES.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", "));
        return 2;
    };
    let mut text = String::new();
    let read = if *src == "-" { std::io::stdin().read_to_string(&mut text).map(drop) } else { std::fs::read_to_string(src).map(|t| text = t) };
    if let Err(e) = read {
        errln!("{src}: {e}");
        return 1;
    }
    let tree: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            errln!("{src}: not JSON: {e}");
            return 1;
        }
    };
    let buf = headless::render(&tree, w, h, th);
    let out = if a.switch("ansi") { headless::ansi(&buf) } else { headless::text(&buf) };
    let _ = std::io::stdout().write_all(out.as_bytes());
    0
}
