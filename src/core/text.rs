// Text measured and cleaned for a terminal.
use std::sync::LazyLock;

use regex::Regex;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

// Terminal cells a string takes, grapheme by grapheme (a wide character counts 2, an emoji sequence once).
pub fn width(s: &str) -> usize {
    s.graphemes(true).map(grapheme_width).sum()
}

pub fn grapheme_width(g: &str) -> usize {
    match g.chars().next() {
        None => 0,
        Some(c) if (c as u32) < 0x20 || (0x7f..0xa0).contains(&(c as u32)) => 0,
        // ponytail: an emoji presentation sequence counts its widest part; Bun's table differs on rare clusters
        Some(_) => g.chars().map(|c| UnicodeWidthStr::width(c.encode_utf8(&mut [0; 4]) as &str)).max().unwrap_or(0).min(2),
    }
}

static OSC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)?").unwrap());
static CSI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]").unwrap());
static CONTROL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\x00-\x1f\x7f-\x9f]|\p{Cf}").unwrap());

// Text from a stranger (a plugin, a plugin index) shown in a terminal: no escape sequences, control characters, or
// invisible formatting characters (bidi overrides and isolates, zero-width marks) that could reorder or hide part of
// it; and at most `cells` terminal cells, cut between whole characters (a wide character counts 2).
pub fn clean_text(text: &str, cells: usize) -> String {
    let plain = OSC.replace_all(text, "");
    let plain = CSI.replace_all(&plain, "");
    let plain = CONTROL.replace_all(&plain, "");
    let mut out = String::new();
    let mut used = 0;
    for g in plain.graphemes(true) {
        let w = grapheme_width(g);
        if used + w > cells {
            break;
        }
        out.push_str(g);
        used += w;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_and_cuts() {
        assert_eq!(clean_text("\x1b[31mred\x1b[0m\x1b]0;title\x07 ok\u{202e}", 100), "red ok");
        assert_eq!(clean_text("日本語", 5), "日本");
        assert_eq!(width("a日b"), 4);
    }
}
