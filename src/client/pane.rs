// Drawing a pane's screen into the frame: its cells at their colours and styles, a selection over them, and where its
// cursor is.
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Rgb};
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color as C, Modifier};

use crate::vt::Screen;

fn rgb(c: Rgb) -> C {
    C::Rgb(c.r, c.g, c.b)
}

// A colour as the outer terminal should show it: the program's own palette entry if it set one (OSC 4/10/11), else the
// outer terminal's ANSI colour, so a pane follows the user's palette; its default text and background are the theme's.
fn color(c: Color, colors: &Colors, fg: C, bg: C) -> C {
    match c {
        Color::Spec(c) => rgb(c),
        Color::Indexed(i) => colors[i as usize].map(rgb).unwrap_or(C::Indexed(i)),
        Color::Named(n) => match colors[n].map(rgb) {
            Some(c) => c,
            None if (n as usize) < 16 => C::Indexed(n as u8),
            None if (NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize).contains(&(n as usize)) => C::Indexed((n as usize - NamedColor::DimBlack as usize) as u8),
            None if matches!(n, NamedColor::Background) => bg,
            None => fg,
        },
    }
}

// Draws the screen into `area`; returns where its cursor is, when it's showing.
pub fn draw(screen: &Screen, area: Rect, buf: &mut Buffer, fg: C, bg: C) -> Option<Position> {
    let content = screen.term.renderable_content();
    let offset = content.display_offset as i32;
    let selection = content.selection;
    for item in content.display_iter {
        let row = item.point.line.0 + offset;
        let col = item.point.column.0 as u16;
        if row < 0 || row as u16 >= area.height || col >= area.width {
            continue;
        }
        let cell = item.cell;
        if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
            continue; // the wide character before it covers it
        }
        let Some(out) = buf.cell_mut((area.x + col, area.y + row as u16)) else { continue };
        match cell.zerowidth() {
            Some(extra) => out.set_symbol(&std::iter::once(cell.c).chain(extra.iter().copied()).collect::<String>()),
            None => out.set_char(cell.c),
        };
        out.fg = color(cell.fg, content.colors, fg, bg);
        out.bg = color(cell.bg, content.colors, fg, bg);
        let f = cell.flags;
        let mut m = Modifier::empty();
        for (flag, modifier) in [
            (Flags::BOLD, Modifier::BOLD),
            (Flags::ITALIC, Modifier::ITALIC),
            (Flags::ALL_UNDERLINES, Modifier::UNDERLINED),
            (Flags::DIM, Modifier::DIM),
            (Flags::INVERSE, Modifier::REVERSED),
            (Flags::HIDDEN, Modifier::HIDDEN),
            (Flags::STRIKEOUT, Modifier::CROSSED_OUT),
        ] {
            if f.intersects(flag) {
                m |= modifier;
            }
        }
        if selection.is_some_and(|s| s.contains(item.point)) {
            m ^= Modifier::REVERSED;
        }
        out.modifier = m;
    }
    let cursor = content.cursor;
    let row = cursor.point.line.0 + offset;
    let shown = cursor.shape != CursorShape::Hidden && content.mode.contains(TermMode::SHOW_CURSOR);
    (shown && row >= 0 && (row as u16) < area.height && (cursor.point.column.0 as u16) < area.width).then(|| Position::new(area.x + cursor.point.column.0 as u16, area.y + row as u16))
}
