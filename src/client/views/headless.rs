// A view tree drawn without a session or a terminal (`modisa view render`): into a buffer as the TUI would draw it,
// then out as plain text or with its colours. For plugins' snapshot tests, and this module's.
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use serde_json::{json, Value};

use super::build::{self, Ctx, Draw};
use super::OpenView;
use crate::client::design::color;
use crate::client::draw::Canvas;
use crate::config::themes::Theme;
use crate::core::text::width;

// `tree` drawn in `w`×`h` cells: a whole view (`{ root, title, keys, focus }`) framed as the TUI frames it, or an
// element on its own. What has the keyboard is what would have it as the view opens.
pub fn render(tree: &Value, w: u16, h: u16, th: &Theme) -> Buffer {
    let mut buf = Buffer::empty(Rect::new(0, 0, w, h));
    let mut hits = vec![];
    let area = buf.area;
    let mut c = Canvas { buf: &mut buf, hits: &mut hits, hover: None, w: w as i32, h: h as i32 };
    let ctx = Ctx { th, logos: None, cell: 1.2, tick: 0 };
    let full = crate::core::layout::Rect { x: 0, y: 0, w: w as i32, h: h as i32 };
    if tree.get("root").is_some() {
        super::draw_view(&ctx, &mut c, &mut OpenView::new(tree.clone()), full, true, true);
    } else {
        c.fill(full, th.bg);
        c.buf.set_style(area, Style::new().fg(color(th.fg)));
        let OpenView { state, elems, focus, .. } = &mut OpenView::new(json!({ "root": tree }));
        let mut d = Draw { view: "", focus: focus.as_deref(), active: true, cursor: None };
        build::draw(&ctx, &mut c, &state["root"], &build::root_key(&state["root"]), area, elems, &mut d);
    }
    buf
}

// The buffer's rows as text, without the spaces they end in.
pub fn text(buf: &Buffer) -> String {
    rows(buf, |line, cell, _| line.push_str(cell.symbol()))
}

// The buffer's rows with their colours and attributes (SGR).
pub fn ansi(buf: &Buffer) -> String {
    rows(buf, |line, cell, last: &mut Option<Style>| {
        let st = cell.style();
        if *last != Some(st) {
            line.push_str(&sgr(st));
            *last = Some(st);
        }
        line.push_str(cell.symbol());
    })
}

fn rows(buf: &Buffer, mut put: impl FnMut(&mut String, &ratatui::buffer::Cell, &mut Option<Style>)) -> String {
    let mut out = String::new();
    for y in 0..buf.area.height {
        let (mut line, mut skip, mut last) = (String::new(), 0, None);
        for x in 0..buf.area.width {
            if skip > 0 {
                skip -= 1; // the cells a wide character covers
                continue;
            }
            let cell = &buf[(x, y)];
            put(&mut line, cell, &mut last);
            skip = width(cell.symbol()).saturating_sub(1);
        }
        if last.is_some() {
            line.push_str("\x1b[0m");
        } else {
            line.truncate(line.trim_end().len());
        }
        out.push_str(&line);
        out.push('\n');
    }
    out
}

fn sgr(st: Style) -> String {
    let mut codes = vec!["0".to_string()];
    let m = st.add_modifier;
    for (flag, code) in [(Modifier::BOLD, 1), (Modifier::DIM, 2), (Modifier::ITALIC, 3), (Modifier::UNDERLINED, 4), (Modifier::SLOW_BLINK, 5), (Modifier::RAPID_BLINK, 6), (Modifier::REVERSED, 7), (Modifier::HIDDEN, 8), (Modifier::CROSSED_OUT, 9)] {
        if m.contains(flag) {
            codes.push(code.to_string());
        }
    }
    codes.extend(st.fg.and_then(|c| colour(c, 30)));
    codes.extend(st.bg.and_then(|c| colour(c, 40)));
    format!("\x1b[{}m", codes.join(";"))
}

// a colour's SGR code, from the foreground's base (30) or the background's (40)
fn colour(c: Color, base: u8) -> Option<String> {
    let named = |i: u8| Some((if i < 8 { base + i } else { base + 60 + i - 8 }).to_string());
    match c {
        Color::Reset => None,
        Color::Black => named(0),
        Color::Red => named(1),
        Color::Green => named(2),
        Color::Yellow => named(3),
        Color::Blue => named(4),
        Color::Magenta => named(5),
        Color::Cyan => named(6),
        Color::Gray => named(7),
        Color::DarkGray => named(8),
        Color::LightRed => named(9),
        Color::LightGreen => named(10),
        Color::LightYellow => named(11),
        Color::LightBlue => named(12),
        Color::LightMagenta => named(13),
        Color::LightCyan => named(14),
        Color::White => named(15),
        Color::Indexed(i) => Some(format!("{};5;{i}", base + 8)),
        Color::Rgb(r, g, b) => Some(format!("{};2;{r};{g};{b}", base + 8)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::themes::find_theme;

    // A tree drawn at w×h, as text.
    fn draw(tree: Value, w: u16, h: u16) -> String {
        text(&render(&tree, w, h, find_theme("tokyonight").unwrap()))
    }

    fn snap(tree: Value, w: u16, h: u16, want: &str) {
        let got = draw(tree, w, h);
        let want: String = want.lines().map(|l| format!("{}\n", l.trim_end())).collect();
        assert_eq!(got, want, "\n{got}");
    }

    #[test]
    fn a_layout_shares_room_by_constraints() {
        snap(
            json!({ "type": "layout", "direction": "horizontal", "constraints": ["4", "*", "30%"], "spacing": 1, "children": [{ "type": "text", "text": "abcdef" }, { "type": "text", "text": "middle", "align": "center" }, { "type": "text", "text": "end" }] }),
            30,
            2,
            "abcd     middle      end\nef",
        );
    }

    #[test]
    fn hide_below_gives_a_child_no_room() {
        let tree = |w| draw(json!({ "type": "layout", "direction": "horizontal", "children": [{ "type": "text", "text": "side", "size": 6, "hide_below": { "width": 6 } }, { "type": "text", "text": "main" }] }), w, 1);
        assert_eq!(tree(20), "side  main\n");
        assert_eq!(draw(json!({ "type": "layout", "direction": "horizontal", "children": [{ "type": "text", "text": "side", "size": "20%", "hide_below": { "width": 6 } }, { "type": "text", "text": "main" }] }), 20, 1), "main\n");
    }

    #[test]
    fn a_block_frames_and_titles() {
        snap(
            json!({ "type": "block", "title": "Box", "border_type": "rounded", "titles": [{ "content": "end", "position": "bottom", "align": "right" }], "child": { "type": "text", "text": "inside" } }),
            12,
            3,
            "╭Box───────╮\n│inside    │\n╰───────end╯",
        );
        // any element can have one
        snap(json!({ "type": "text", "text": "hi", "block": { "borders": ["top"], "title": "t" } }), 6, 2, "t─────\nhi");
    }

    #[test]
    fn text_wraps_aligns_and_reads_ansi() {
        snap(json!({ "type": "text", "text": "one two three", "align": "right" }), 8, 2, " one two\n   three");
        snap(json!({ "type": "text", "text": "one two three", "wrap": false }), 8, 1, "one two");
        snap(json!({ "type": "text", "ansi": "\u{1b}[31mred\u{1b}[0m\u{1b}[2J ok" }), 8, 1, "red ok");
    }

    #[test]
    fn scrolling_text_follows_its_end() {
        let lines: Vec<String> = (1..=10).map(|i| format!("line {i}")).collect();
        snap(json!({ "type": "text", "id": "log", "text": lines.join("\n"), "scroll": "bottom", "scrollbar": true }), 10, 3, "line 8\nline 9\nline 10  ▐");
    }

    #[test]
    fn a_list_shows_its_selection() {
        let tree = json!({ "type": "list", "id": "l", "items": ["alpha", { "content": "beta\n  two lines" }, "gamma"], "selected": 1, "highlight_symbol": "▶ " });
        snap(tree.clone(), 14, 4, "  alpha\n▶ beta\n    two lines\n  gamma");
        let buf = render(&tree, 14, 4, find_theme("tokyonight").unwrap());
        assert_ne!(buf[(2, 1)].bg, buf[(2, 0)].bg); // the selection is tinted
    }

    #[test]
    fn a_table_spans_columns_and_selects_cells() {
        snap(
            json!({ "type": "table", "id": "t", "header": ["Name", "Size"], "widths": ["6", "*"], "rows": [["a.rs", "12"], [{ "content": "spanning both", "span": 2 }]], "select": "cell", "selected": [0, 1] }),
            16,
            3,
            "Name   Size\na.rs   12\nspanning both",
        );
    }

    #[test]
    fn tabs_divide_their_titles() {
        snap(json!({ "type": "tabs", "id": "t", "titles": ["One", "Two", "Three"], "selected": 1 }), 20, 1, " One │ Two │ Three");
    }

    #[test]
    fn gauges_fill_to_their_ratio() {
        snap(json!({ "type": "gauge", "ratio": 0.5, "unicode": false }), 10, 1, "███50%");
        snap(json!({ "type": "line_gauge", "percent": 50, "label": "half", "filled_symbol": "=", "unfilled_symbol": "-" }), 14, 1, "half ====-----");
    }

    #[test]
    fn a_sparkline_and_a_bar_chart() {
        snap(json!({ "type": "sparkline", "data": [0, 4, 8, null, 2], "max": 8 }), 5, 1, " ▄█ ▂");
        snap(json!({ "type": "bar_chart", "data": [["a", 2], ["b", 4]], "bar_width": 1, "max": 4 }), 5, 3, "  █\n2 4\na b");
    }

    #[test]
    fn a_chart_and_a_canvas_draw() {
        let out = draw(json!({ "type": "chart", "datasets": [{ "name": "up", "data": [[0, 0], [10, 10]], "marker": "dot" }], "legend": "none" }), 24, 8);
        assert!(out.contains('•') && out.contains("10"), "{out}");
        let out = draw(json!({ "type": "canvas", "marker": "block", "shapes": [{ "rectangle": [10, 10, 80, 80] }, { "layer": true }, { "text": "hi", "at": [45, 50] }] }), 12, 6);
        assert!(out.contains('█') && out.contains("hi"), "{out}");
        let out = draw(json!({ "type": "canvas", "shapes": [{ "map": "low" }] }), 40, 10);
        assert!(out.chars().any(|c| ('\u{2801}'..='\u{28ff}').contains(&c)), "{out}");
    }

    #[test]
    fn a_calendar_shows_its_month() {
        let out = draw(json!({ "type": "calendar", "year": 2026, "month": 10, "events": { "2026-10-09": "bold $accent" } }), 24, 8);
        assert!(out.contains("October 2026") && out.contains(" 9 "), "{out}");
    }

    #[test]
    fn fill_and_clear_paint_their_area() {
        snap(json!({ "type": "fill", "symbol": "·" }), 3, 2, "···\n···");
        snap(json!({ "type": "layout", "spacing": -1, "constraints": ["2", "1"], "children": [{ "type": "fill", "symbol": "x" }, { "type": "clear" }] }), 3, 2, "xxx\n ");
    }

    #[test]
    fn code_is_numbered_and_marked() {
        snap(json!({ "type": "code", "id": "c", "content": "fn main() {\n    go();\n}", "language": "rust", "line_numbers": true, "highlight": [2] }), 16, 3, "1 fn main() {\n2     go();\n3 }");
        let buf = render(&json!({ "type": "code", "content": "let x = 1;", "language": "rust" }), 12, 1, find_theme("tokyonight").unwrap());
        assert_eq!(buf[(0, 0)].fg, color(find_theme("tokyonight").unwrap().accent)); // `let`: a keyword
        snap(json!({ "type": "code", "content": "a\tb", "syntax_theme": "Dracula" }), 8, 1, "a    b");
    }

    #[test]
    fn a_diff_unified_and_split() {
        let diff = "diff --git a/x.rs b/x.rs\n--- a/x.rs\n+++ b/x.rs\n@@ -1,2 +1,2 @@\n keep\n-old\n+new\n";
        snap(json!({ "type": "diff", "id": "d", "diff": diff, "cursor": true }), 24, 5, "▍ x.rs\n@@ -1,2 +1,2 @@\n1 1   keep\n2   - old\n  2 + new");
        snap(json!({ "type": "diff", "diff": diff, "view": "split" }), 24, 4, "▍ x.rs\n@@ -1,2 +1,2 @@\n1   keep    1   keep\n2 - old     2 + new");
    }

    #[test]
    fn markdown_renders_and_highlights_its_code() {
        let out = draw(json!({ "type": "markdown", "id": "m", "content": "# Title\n\nSome **bold** text.\n\n- [x] done\n\n```rust\nfn x() {}\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n" }), 30, 14);
        assert!(out.contains("# Title") || out.contains("Title"), "{out}");
        assert!(out.contains("Some bold text."), "{out}");
        assert!(out.contains("  fn x() {}"), "{out}");
    }

    #[test]
    fn big_text_is_big() {
        let out = draw(json!({ "type": "big_text", "text": "Hi", "pixel_size": "half_height" }), 16, 4);
        assert!(out.lines().filter(|l| !l.is_empty()).count() >= 3 && out.contains('█'), "{out}");
    }

    #[test]
    fn an_image_draws_in_half_blocks_or_shows_its_alt() {
        // a 2×2 PNG: red, green / blue, white
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAEklEQVR4nGP4z8DAAMIM/4EAAB/uBfsL2WiLAAAAAElFTkSuQmCC";
        let buf = render(&json!({ "type": "image", "id": "i", "data": png, "resize": "scale" }), 4, 2, find_theme("tokyonight").unwrap());
        assert!(text(&buf).contains('▀') || text(&buf).contains('▄'), "{}", text(&buf));
        snap(json!({ "type": "image", "data": "bm90IGFuIGltYWdl", "alt": "a cat" }), 8, 1, "a cat");
    }

    #[test]
    fn fields_show_their_text_placeholder_or_mask() {
        snap(json!({ "type": "input", "id": "q", "placeholder": "search…" }), 12, 1, "search…");
        snap(json!({ "type": "input", "id": "p", "value": "hunter2", "mask": "•" }), 12, 1, "•••••••");
        snap(json!({ "type": "textarea", "id": "t", "value": "one\ntwo", "line_numbers": true }), 12, 2, " 1 one\n 2 two");
    }

    #[test]
    fn a_tree_opens_what_it_says() {
        let items = json!([{ "id": "src", "text": "src", "children": [{ "id": "main.rs", "text": "main.rs" }] }, { "id": "README.md", "text": "README.md" }]);
        snap(json!({ "type": "tree", "id": "t", "items": items, "open": [["src"]], "selected": ["src", "main.rs"] }), 20, 3, "▾ src\n    main.rs\n  README.md");
    }

    #[test]
    fn buttons_and_spinners() {
        snap(json!({ "type": "button", "id": "ok", "label": "OK", "block": {} }), 8, 3, "┌──────┐\n│  OK  │\n└──────┘");
        let out = draw(json!({ "type": "spinner", "label": "working" }), 12, 1);
        assert!(out.ends_with("working\n"), "{out}");
    }

    #[test]
    fn a_raster_paints_its_cells() {
        // "a" in the theme's text colour, "b" in red on blue
        let mut cells = vec![];
        for (ch, fg, bg) in [('a', 0x0100_0000u32, 0x0100_0000u32), ('b', 0xff0000, 0x0000ff)] {
            for w in [ch as u32, fg, bg] {
                cells.extend(w.to_le_bytes());
            }
        }
        let tree = json!({ "type": "raster", "id": "r", "columns": 2, "rows": 1, "cells": crate::protocol::conn::b64(&cells) });
        let buf = render(&tree, 2, 1, find_theme("tokyonight").unwrap());
        assert_eq!(text(&buf), "ab\n");
        assert_eq!((buf[(1, 0)].fg, buf[(1, 0)].bg), (Color::Rgb(255, 0, 0), Color::Rgb(0, 0, 255)));
    }

    #[test]
    fn a_view_is_framed_with_its_keys() {
        let view = json!({ "plugin": "demo", "title": "Hello", "keys": [{ "key": "s", "action": "send", "description": "send" }], "root": { "type": "layout", "children": [{ "type": "input", "id": "a" }, { "type": "button", "id": "b", "label": "Go", "size": 1 }] } });
        let out = draw(view, 50, 5);
        assert!(out.starts_with("╭─ demo · Hello ─"), "{out}");
        assert!(out.contains("s send · ? keys · tab moves · esc closes"), "{out}");
    }

    #[test]
    fn what_has_the_keyboard_shows_it() {
        let th = find_theme("tokyonight").unwrap();
        let view = |focus: &str| json!({ "focus": focus, "rev": 1, "root": { "type": "layout", "children": [{ "type": "list", "id": "a", "items": ["x"], "selected": 0, "block": {} }, { "type": "list", "id": "b", "items": ["y"], "selected": 0, "block": {} }] } });
        let buf = render(&view("b"), 20, 10, th);
        // inside the frame (2 across, 1 down): a's border is the theme's border colour, b's the focus colour, and b's
        // selection is the brighter one
        assert_eq!((buf[(2, 1)].fg, buf[(2, 5)].fg), (color(th.border), color(th.focus)));
        assert_ne!(buf[(3, 2)].bg, buf[(3, 6)].bg);
    }

    #[test]
    fn ansi_output_has_colours() {
        let out = ansi(&render(&json!({ "type": "text", "text": { "spans": [{ "text": "x", "style": "bold #ff0000" }] } }), 2, 1, find_theme("tokyonight").unwrap()));
        assert!(out.contains("\x1b[0;1;38;2;255;0;0;48;2;26;27;38mx"), "{out:?}");
    }

    #[test]
    fn big_inputs_draw_in_time() {
        let code: String = (0..30_000).map(|i| format!("const x{i} = \"{i}\"; // {i}\n")).collect();
        let diff: String = format!("--- a/big.ts\n+++ b/big.ts\n@@ -1,30000 +1,30000 @@\n{}", code.lines().map(|l| format!("+{l}\n")).collect::<String>());
        let md: String = (0..5_000).map(|i| format!("## Part {i}\n\nSome *text* here.\n\n```ts\nlet a{i} = {i};\n```\n\n")).collect();
        let start = std::time::Instant::now();
        draw(json!({ "type": "code", "content": code, "language": "ts", "line_numbers": true }), 80, 40);
        draw(json!({ "type": "diff", "diff": diff }), 80, 40);
        draw(json!({ "type": "markdown", "content": md }), 80, 40);
        assert!(start.elapsed().as_secs() < 20, "{:?}", start.elapsed());
    }
}
