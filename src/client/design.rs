// The client's measures and looks: text fitted to cells, colours blended from the theme, where the chrome and panels
// go, and how the sidebar spends its rows.
use ratatui::style::Color;
use unicode_segmentation::UnicodeSegmentation;

use crate::config::agents::brands::{brand, half_variant, logo, logo_halves};
use crate::platform::logos::Loaded;
use crate::config::themes::Theme;
use crate::core::layout::Rect;
use crate::core::text::{grapheme_width, width};

// Measure terminal cells, not bytes (paths and titles can contain emoji/CJK).
pub fn fit(text: &str, w: usize) -> String {
    let clean: String = text.chars().map(|c| if (c as u32) < 0x20 || (0x7f..0xa0).contains(&(c as u32)) { ' ' } else { c }).collect();
    if w == 0 {
        return String::new();
    }
    if width(&clean) <= w {
        return clean;
    }
    let mut out = String::new();
    let mut used = 0;
    for g in clean.graphemes(true) {
        let gw = grapheme_width(g);
        if used + gw > w - 1 {
            break;
        }
        out.push_str(g);
        used += gw;
    }
    out + "…"
}

fn channels(c: &str) -> [u8; 3] {
    let h = |i: usize| u8::from_str_radix(c.get(1 + 2 * i..3 + 2 * i).unwrap_or("00"), 16).unwrap_or(0);
    [h(0), h(1), h(2)]
}

// Blend two #rrggbb colors (t = 0 is a, 1 is b): hover and selection tints derived from any theme.
pub fn mix(a: &str, b: &str, t: f64) -> String {
    let (x, y) = (channels(a), channels(b));
    let ch = |i: usize| ((x[i] as f64 + (y[i] as f64 - x[i] as f64) * t) + 0.5).floor() as u8;
    format!("#{:02x}{:02x}{:02x}", ch(0), ch(1), ch(2))
}

// WCAG contrast ratio of two #rrggbb colours (1 to 21).
pub fn contrast(a: &str, b: &str) -> f64 {
    let lum = |c: &str| {
        let v = channels(c).map(|v| {
            let v = v as f64 / 255.0;
            if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        });
        0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2]
    };
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

pub fn color(hex: &str) -> Color {
    let [r, g, b] = channels(hex);
    Color::Rgb(r, g, b)
}

// An agent's mark: its logo where the terminal shows modisa's logo font (else its glyph), in its brand colour, or in
// the theme's text colour when the brand has none or it would be faint on this theme's sidebar. `cells` is the room it
// takes, its gap after it included: a logo is drawn two cells wide over a one-cell character, so it's given the cell
// after it too, then the gap. `halves` are the logo to centre between two lines, for cells `cell` ems tall.
pub struct Mark {
    pub glyph: String,
    pub color: String,
    pub cells: usize,
    pub halves: Option<(char, char)>,
}

pub fn agent_mark(th: &Theme, agent: &str, logos: Option<Loaded>, cell: f64) -> Mark {
    let b = brand(agent);
    let drawn = logos.and(logo(agent));
    let halves = drawn.and(logos.filter(|l| *l == Loaded::Halves)).and_then(|_| logo_halves(agent, half_variant(cell)));
    let color = b.color.filter(|c| contrast(c, th.bar) >= 3.0).unwrap_or(th.fg).to_string();
    Mark { glyph: drawn.map(String::from).unwrap_or_else(|| b.glyph.to_string()), color, cells: if drawn.is_some() { 3 } else { 2 }, halves }
}

// How tall a terminal's cells are in ems of its font, from its size in pixels: a monospace cell is about 0.6em wide.
pub fn cell_ems(px_w: f64, px_h: f64, cols: f64, rows: f64) -> f64 {
    (0.6 * (px_h / rows)) / (px_w / cols)
}

// The line height of common monospace fonts, in ems (ascent + descent + line gap): for a terminal that doesn't say how
// big its cells are. Nerd Font builds of them are the same.
const LINE_EMS: &[(&str, f64)] = &[
    ("menlo", 1.164), ("sfmono", 1.193), ("jetbrainsmono", 1.32), ("firacode", 1.311), ("firamono", 1.2), ("cascadiacode", 1.172), ("cascadiamono", 1.172),
    ("hack", 1.164), ("sourcecodepro", 1.257), ("droidsansmono", 1.164), ("dejavusansmono", 1.164), ("ibmplexmono", 1.3), ("robotomono", 1.319),
    ("iosevka", 1.25), ("ubuntumono", 1.0), ("consolas", 1.172), ("monaco", 1.334), ("inconsolata", 1.1), ("victormono", 1.37), ("commitmono", 1.3), ("geistmono", 1.3),
];

// the first font of a CSS-style list, as a key: no case, spaces or quotes, and no Nerd Font suffix
pub fn font_ems(family: &str, line_height: f64) -> Option<f64> {
    let first = family.split(',').next().unwrap_or("").to_lowercase().replace(['\'', '"', ' ', '\t'], "");
    let key = ["nerdfontmono", "nerdfontpropo", "nerdfont", "nfm", "nfp", "nf", "nl"].iter().find_map(|s| first.strip_suffix(s)).unwrap_or(&first);
    LINE_EMS.iter().find(|(k, _)| *k == key).map(|(_, e)| e * line_height)
}

// The task an agent's terminal title names, without the spinner or mark it puts in front; "" when the title only names
// the agent (or isn't there).
pub fn agent_task(title: Option<&str>, not: &[Option<&str>]) -> String {
    let t = title.unwrap_or("");
    let t = t.trim_start_matches(|c: char| !c.is_alphanumeric()).trim();
    if t.is_empty() || not.iter().any(|n| *n == Some(t)) { String::new() } else { t.to_string() }
}

pub struct Chrome {
    pub top: i32,
    pub side: i32,
    pub side_x: i32,           // where the sidebar starts: 0 on the left, w - side on the right
    pub tabs_y: Option<i32>,   // the tab bar's row ([tabs] position), none when it's hidden
    pub area: Rect,
}

// Where everything goes: the tab bar on top (or at the bottom, over the status row, or hidden), the sidebar on the left
// (or the right), the panes in what's left.
pub fn chrome(w: i32, h: i32, sidebar: bool, preferred: i64) -> Chrome {
    chrome_at(w, h, sidebar, preferred, "top", false)
}

pub fn chrome_at(w: i32, h: i32, sidebar: bool, preferred: i64, tabs: &str, right: bool) -> Chrome {
    let (top, bottom, tabs_y) = match tabs {
        "bottom" => (0, 2, Some(h - 2)),
        "hidden" => (0, 1, None),
        _ => (1, 1, Some(0)),
    };
    let side = if sidebar && w >= 100 && h >= 22 { (preferred.max(20) as i32).min(48).min(w / 3) } else { 0 };
    let (side_x, area_x) = if right { (w - side, 0) } else { (0, side) };
    Chrome { top, side, side_x, tabs_y, area: Rect { x: area_x, y: top, w: (w - side).max(1), h: (h - top - bottom).max(1) } }
}

pub fn floating(w: i32, h: i32, want_w: i32, want_h: i32, x: Option<i32>, y: Option<i32>) -> Rect {
    let rw = want_w.min(w).max(1);
    let rh = want_h.min(h).max(1);
    let x = x.unwrap_or((w - rw) / 2).min(w - rw).max(0);
    let y = y.unwrap_or((h - rh) / 3).min(h - rh).max(0);
    Rect { x, y, w: rw, h: rh }
}

// Keep the selected tab in view, with an equal cell budget for each visible tab.
pub struct TabWindow {
    pub start: usize,
    pub end: usize,
    pub width: usize,
}

pub fn tab_window(count: usize, active: usize, w: usize) -> TabWindow {
    let visible = (w / 18).min(count).max(1);
    let start = (active as i64 - (visible / 2) as i64).min(count as i64 - visible as i64).max(0) as usize;
    TabWindow { start, end: start + visible, width: (w / visible).max(1) }
}

// Reserve the footer before assigning list rows. Agent rows always occupy exactly two lines; overflow controls are part
// of the budget rather than drawn over it.
#[cfg_attr(not(test), allow(dead_code))] // agent_rows and more_agents are what the tests check; the sidebar spends `lines`
pub struct Budget {
    pub agent_rows: usize,
    pub more_agents: bool,
    pub lines: usize, // what the agent list may use
}

pub fn sidebar_budget(height: i32, agents: usize) -> Budget {
    let remaining = (height - 5 - 3).max(0) as usize; // the footer, then a blank line, the AGENTS heading and a blank line
    let more = agents * 2 > remaining;
    let rows = agents.min((remaining.saturating_sub(more as usize)) / 2);
    Budget { agent_rows: rows, more_agents: agents > rows, lines: remaining }
}

// The space's agents as a git graph: its tabs are commits on one trunk, each tab's agents branch off under it. A tab row
// is one line, an agent `tall` (two, and plugins' lines; the graph's cells for its first two in `graph`), a rail one;
// rails between tabs only when everything else fits. Too tall, and every tab but the active one folds; still too tall,
// and it's cut, `hidden` agents behind the overflow row.
#[derive(Clone, Debug, PartialEq)]
pub enum GraphRow<T> {
    Tab { tab: usize, node: &'static str, open: bool },
    Agent { tab: usize, agent: T, graph: [&'static str; 2] },
    Rail,
}

pub struct Graph<T> {
    pub rows: Vec<GraphRow<T>>,
    pub hidden: usize,
}

pub fn agent_graph<T: Clone>(tabs: &[(String, Vec<T>)], active: usize, collapsed: &std::collections::HashSet<String>, lines: usize, tall: impl Fn(&T) -> usize) -> Graph<T> {
    let build = |fold_others: bool, rails: bool| {
        let mut rows = vec![];
        for (i, (id, agents)) in tabs.iter().enumerate() {
            if rails && i > 0 {
                rows.push(GraphRow::Rail);
            }
            let open = !agents.is_empty() && !collapsed.contains(id) && !(fold_others && i != active);
            rows.push(GraphRow::Tab { tab: i, node: if i == active { "◉" } else if !agents.is_empty() { "●" } else { "○" }, open });
            if !open {
                continue;
            }
            // the trunk runs on to the next tab; under the last tab's last agent it ends
            for (k, a) in agents.iter().enumerate() {
                let end = i == tabs.len() - 1 && k == agents.len() - 1;
                rows.push(GraphRow::Agent { tab: i, agent: a.clone(), graph: if end { ["╰─", "  "] } else { ["├─", "│ "] } });
            }
        }
        rows
    };
    let row_height = |r: &GraphRow<T>| if let GraphRow::Agent { agent, .. } = r { tall(agent) } else { 1 };
    let height = |rows: &[GraphRow<T>]| rows.iter().map(row_height).sum::<usize>();
    let mut rows = build(false, true);
    if height(&rows) > lines {
        rows = build(false, false);
    }
    if height(&rows) > lines {
        rows = build(true, false);
    }
    if height(&rows) <= lines {
        return Graph { rows, hidden: 0 };
    }
    // cut, leaving a line for the overflow row
    let mut kept = vec![];
    for r in &rows {
        if height(&kept) + row_height(r) > lines.saturating_sub(1) {
            break;
        }
        kept.push(r.clone());
    }
    let agents = |rs: &[GraphRow<T>]| rs.iter().filter(|r| matches!(r, GraphRow::Agent { .. })).count();
    let hidden = agents(&rows) - agents(&kept); // a folded tab's agents aren't hidden: its row counts them
    Graph { rows: kept, hidden }
}

// Right-hand state labels keep their cell budget; Unicode names fit the remainder.
pub fn sidebar_columns(left: &str, right: &str, w: usize) -> (String, String) {
    let tail = fit(right, w);
    let head = fit(left, w.saturating_sub(width(&tail) + (!tail.is_empty()) as usize));
    let pad = w.saturating_sub(width(&head) + width(&tail));
    (head + &" ".repeat(pad), tail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::layout::{display_rects, split, Axis, Node};
    use std::collections::HashSet;

    #[test]
    fn chrome_fits() {
        for (w, h) in [(160, 48), (100, 24), (80, 24), (44, 14), (20, 6)] {
            let c = chrome(w, h, true, 999);
            assert_eq!(c.area.x + c.area.w, w);
            assert!(c.area.y + c.area.h < h);
            assert!(c.side <= 48.min(w / 3));
            if w < 100 {
                assert_eq!(c.side, 0);
            }
        }
        let tree = split(&Node::pane("a"), "a", Axis::Row, "b", 0.5);
        let small = Rect { x: 0, y: 1, w: 44, h: 12 };
        assert_eq!(display_rects(&tree, small, "b", false).into_iter().collect::<Vec<_>>(), vec![("b".to_string(), small)]);
    }

    #[test]
    fn fits_unicode() {
        for text in ["a very long pane title", "日本語のターミナル", "👨‍👩‍👧‍👦 agent", "e\u{301}e\u{301}e\u{301}", "\x1b[31mtitle\nnext"] {
            for w in 0..20 {
                assert!(width(&fit(text, w)) <= w, "{text} at {w}");
            }
        }
        assert_eq!(fit("👨‍👩‍👧‍👦 agent", 3), "👨‍👩‍👧‍👦…");
    }

    #[test]
    fn floats_inside() {
        for (w, h) in [(140, 40), (44, 14), (18, 7)] {
            let r = floating(w, h, 34, 17, Some(w - 1), Some(h - 1));
            assert!(r.x >= 0 && r.y >= 0 && r.x + r.w <= w && r.y + r.h <= h);
        }
        for active in 0..40 {
            let t = tab_window(40, active, 63);
            assert!(t.start <= active && t.end > active && (t.end - t.start) * t.width <= 63);
        }
    }

    #[test]
    fn mixes() {
        assert_eq!(mix("#000000", "#ffffff", 0.0), "#000000");
        assert_eq!(mix("#000000", "#ffffff", 1.0), "#ffffff");
        assert_eq!(mix("#101b2c", "#5ee7ef", 0.5), "#37818e");
        assert_eq!(agent_task(Some("✳ Audit the token cache"), &[]), "Audit the token cache");
        assert_eq!(agent_task(Some("⠋ codex"), &[Some("codex")]), "");
        assert!((cell_ems(1200.0, 700.0, 100.0, 25.0) - 1.4).abs() < 1e-9);
        assert_eq!(font_ems("Menlo, Monaco, 'Courier New', monospace", 1.0), Some(1.164));
        assert_eq!(font_ems("JetBrainsMono Nerd Font", 1.0), Some(1.32));
        assert_eq!(font_ems("Some Font", 1.0), None);
    }

    #[test]
    fn sidebar_budget_and_columns() {
        for h in [20, 22, 30, 38, 58] {
            for agents in [0usize, 1, 3, 30] {
                let b = sidebar_budget(h, agents);
                let rows = 3 + if agents > 0 { b.agent_rows * 2 } else { 2 } + b.more_agents as usize;
                assert!(rows as i32 <= h - 5);
                if agents > 0 {
                    assert!(b.agent_rows >= 1);
                }
                assert_eq!(b.more_agents, agents > b.agent_rows);
            }
        }
        for w in [16, 22, 30] {
            for left in ["@reviewer", "日本語のターミナル", "👨‍👩‍👧‍👦 developer", "line\nbreak"] {
                for right in ["Needs you", "Done", "!", "", "30"] {
                    let (l, r) = sidebar_columns(left, right, w);
                    assert_eq!(r, right);
                    assert_eq!(width(&(l.clone() + &r)), w);
                    assert!(!l.contains('\n'));
                }
            }
        }
    }

    #[test]
    fn graphs() {
        let tabs = vec![("a".to_string(), vec!["x", "y"]), ("b".to_string(), vec![]), ("c".to_string(), vec!["z"])];
        let draw = |g: Graph<&str>| {
            g.rows
                .iter()
                .map(|r| match r {
                    GraphRow::Tab { tab, node, open } => format!("{node}{tab}{}", if *open { "" } else { "+" }),
                    GraphRow::Rail => "|".into(),
                    GraphRow::Agent { agent, graph, .. } => format!("{}{agent}", graph.join("")),
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(draw(agent_graph(&tabs, 0, &HashSet::new(), 20, |_| 2)), ["◉0", "├─│ x", "├─│ y", "|", "○1+", "|", "●2", "╰─  z"]);
        assert_eq!(draw(agent_graph(&tabs, 0, &HashSet::from(["a".to_string()]), 20, |_| 2)), ["◉0+", "|", "○1+", "|", "●2", "╰─  z"]);
        assert_eq!(draw(agent_graph(&tabs, 2, &HashSet::new(), 9, |_| 2)), ["●0", "├─│ x", "├─│ y", "○1+", "◉2", "╰─  z"]);
        assert_eq!(draw(agent_graph(&tabs, 2, &HashSet::new(), 5, |_| 2)), ["●0+", "○1+", "◉2", "╰─  z"]);
        let cut = agent_graph(&[("a".to_string(), vec!["x", "y", "w"])], 0, &HashSet::new(), 4, |_| 2);
        assert_eq!(cut.hidden, 2);
        assert_eq!(draw(cut), ["◉0", "├─│ x"]);
    }
}
