// A terminal screen: alacritty_terminal's emulator fed with a pane's output. The server keeps one per pane (what
// `pane read`, agent detection and replays read); a client keeps one per pane it draws.
use std::fmt::Write;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Processor};

pub use alacritty_terminal::event::Event as VtEvent;

struct Size {
    cols: usize,
    rows: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

// What the emulator reports while it parses (query replies for the program, title changes), collected for the caller.
#[derive(Clone, Default)]
pub struct Events(Arc<Mutex<Vec<Event>>>);

impl EventListener for Events {
    fn send_event(&self, event: Event) {
        self.0.lock().unwrap().push(event);
    }
}

pub struct Screen {
    pub term: Term<Events>,
    parser: Processor,
    events: Events,
}

impl Screen {
    pub fn new(cols: u16, rows: u16, scrollback: usize) -> Screen {
        let events = Events::default();
        let config = Config { scrolling_history: scrollback, kitty_keyboard: true, ..Config::default() };
        let size = Size { cols: cols.max(2) as usize, rows: rows.max(1) as usize };
        Screen { term: Term::new(config, &size, events.clone()), parser: Processor::new(), events }
    }

    // Feed output; returns what the emulator reported while parsing it.
    pub fn write(&mut self, bytes: &[u8]) -> Vec<Event> {
        self.parser.advance(&mut self.term, bytes);
        std::mem::take(&mut *self.events.0.lock().unwrap())
    }

    // A synchronized update (CSI ? 2026 h) holds output back until it ends; when it never does, this deadline passes
    // and `flush_sync` shows what was held.
    pub fn sync_deadline(&self) -> Option<Instant> {
        self.parser.sync_timeout().sync_timeout()
    }

    pub fn flush_sync(&mut self) -> Vec<Event> {
        self.parser.stop_sync(&mut self.term);
        std::mem::take(&mut *self.events.0.lock().unwrap())
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.term.resize(Size { cols: cols.max(2) as usize, rows: rows.max(1) as usize });
    }

    pub fn mode(&self) -> TermMode {
        *self.term.mode()
    }

    pub fn rows(&self) -> usize {
        self.term.grid().screen_lines()
    }

    pub fn history(&self) -> usize {
        self.term.grid().history_size()
    }

    // One row as text, plain or with SGR styles, and whether it soft-wraps into the next. `trim`: without trailing
    // blanks (a row that wraps is kept whole).
    fn row(&self, line: i32, styled: bool, trim: bool) -> (String, bool) {
        let grid = self.term.grid();
        let row = &grid[Line(line)];
        let cols = grid.columns();
        let wrapped = row[Column(cols - 1)].flags.contains(Flags::WRAPLINE);
        let mut end = cols;
        if trim && !wrapped {
            while end > 0 && row[Column(end - 1)].c == ' ' && row[Column(end - 1)].zerowidth().is_none() {
                end -= 1;
            }
        }
        let mut out = String::new();
        let mut pen = String::new(); // the SGR in effect, "" for the default style
        for x in 0..end {
            let cell = &row[Column(x)];
            if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                continue;
            }
            if styled {
                let want = sgr(cell);
                if want != pen {
                    if !pen.is_empty() {
                        out.push_str("\x1b[0m");
                    }
                    if !want.is_empty() {
                        let _ = write!(out, "\x1b[{want}m");
                    }
                    pen = want;
                }
            }
            out.push(cell.c);
            if let Some(extra) = cell.zerowidth() {
                out.extend(extra);
            }
        }
        if !pen.is_empty() {
            out.push_str("\x1b[0m");
        }
        (out, wrapped)
    }

    // Lines from `first` (negative: scrollback) to the screen's last, as the pane wraps them or with soft wraps joined;
    // blank lines at the end left out.
    fn lines(&self, first: i32, styled: bool, unwrap: bool) -> Vec<String> {
        let mut out: Vec<String> = vec![];
        let mut joining = false;
        for line in first..self.rows() as i32 {
            let (text, wrapped) = self.row(line, styled, true);
            if joining {
                out.last_mut().unwrap().push_str(&text);
            } else {
                out.push(text);
            }
            joining = unwrap && wrapped;
        }
        while out.last().is_some_and(|l| l.is_empty()) {
            out.pop();
        }
        out
    }

    // Visible screen only (no scrollback) — what detection rules look at. Soft-wrapped rows are joined so a rule still
    // matches when a narrow pane wraps it.
    pub fn screen_text(&self) -> String {
        self.screen(false)
    }

    fn screen(&self, styled: bool) -> String {
        let mut out = String::new();
        for line in 0..self.rows() as i32 {
            let (text, wrapped) = self.row(line, styled, true);
            out.push_str(&text);
            if !wrapped {
                out.push('\n');
            }
        }
        out.trim_end().to_string()
    }

    // Scrollback + screen as plain text.
    pub fn text(&self) -> String {
        self.lines(-(self.history() as i32), false, false).join("\n")
    }

    // `pane read`: the visible screen, or the last `lines` lines of scrollback + screen as the pane wraps them or with
    // soft wraps joined; as plain text, or with colours and styles. On the alternate screen (vim, less, an agent's
    // full-screen UI) that's all there is: it has no scrollback.
    pub fn read(&self, source: &str, format: &str, lines: usize) -> String {
        let styled = format == "ansi";
        if source == "visible" {
            return self.screen(styled);
        }
        let all = self.lines(-(self.history() as i32), styled, source == "recent-unwrapped");
        all[all.len().saturating_sub(lines)..].join("\n")
    }

    // VT stream that reproduces the current screen on a fresh terminal of the same size: scrollback and screen, the
    // modes a program set, the cursor and its pen.
    // ponytail: on the alternate screen only that screen is replayed (alacritty keeps the primary one out of reach), so
    // a client attaching while vim runs sees an empty shell once it quits
    pub fn replay(&self) -> String {
        let mode = self.mode();
        let mut out = String::from("\x1b[0m");
        let alt = mode.contains(TermMode::ALT_SCREEN);
        if alt {
            out.push_str("\x1b[?1049h\x1b[H\x1b[2J");
        }
        let first = if alt { 0 } else { -(self.history() as i32) };
        for line in first..self.rows() as i32 {
            if alt {
                let _ = write!(out, "\x1b[{};1H", line + 1);
            } else if line > first {
                out.push_str("\r\n");
            }
            out.push_str(&self.row(line, true, true).0);
        }
        for (flag, on) in [
            (TermMode::APP_CURSOR, "\x1b[?1h"),
            (TermMode::APP_KEYPAD, "\x1b="),
            (TermMode::BRACKETED_PASTE, "\x1b[?2004h"),
            (TermMode::MOUSE_REPORT_CLICK, "\x1b[?1000h"),
            (TermMode::MOUSE_DRAG, "\x1b[?1002h"),
            (TermMode::MOUSE_MOTION, "\x1b[?1003h"),
            (TermMode::UTF8_MOUSE, "\x1b[?1005h"),
            (TermMode::SGR_MOUSE, "\x1b[?1006h"),
            (TermMode::FOCUS_IN_OUT, "\x1b[?1004h"),
            (TermMode::INSERT, "\x1b[4h"),
            (TermMode::LINE_FEED_NEW_LINE, "\x1b[20h"),
        ] {
            if mode.contains(flag) {
                out.push_str(on);
            }
        }
        if !mode.contains(TermMode::LINE_WRAP) {
            out.push_str("\x1b[?7l");
        }
        let kitty = (mode & TermMode::KITTY_KEYBOARD_PROTOCOL).bits() >> TermMode::DISAMBIGUATE_ESC_CODES.bits().trailing_zeros();
        if kitty != 0 {
            let _ = write!(out, "\x1b[>{kitty}u");
        }
        let cursor = &self.term.grid().cursor;
        let _ = write!(out, "\x1b[{};{}H", cursor.point.line.0 + 1, cursor.point.column.0 + 1);
        let pen = sgr(&cursor.template);
        if !pen.is_empty() {
            let _ = write!(out, "\x1b[{pen}m");
        }
        let style = self.term.cursor_style();
        let shape = match style.shape {
            CursorShape::Underline => 4,
            CursorShape::Beam => 6,
            _ => 2,
        } - style.blinking as u8;
        let _ = write!(out, "\x1b[{shape} q");
        if !mode.contains(TermMode::SHOW_CURSOR) {
            out.push_str("\x1b[?25l");
        }
        out
    }
}

// A cell's style as SGR parameters, "" for the default.
fn sgr(cell: &Cell) -> String {
    let mut p: Vec<String> = vec![];
    let f = cell.flags;
    for (flag, n) in [(Flags::BOLD, "1"), (Flags::DIM, "2"), (Flags::ITALIC, "3"), (Flags::ALL_UNDERLINES, "4"), (Flags::INVERSE, "7"), (Flags::HIDDEN, "8"), (Flags::STRIKEOUT, "9")] {
        if f.intersects(flag) {
            p.push(n.into());
        }
    }
    if let Some(c) = color(cell.fg, 30) {
        p.push(c);
    }
    if let Some(c) = color(cell.bg, 40) {
        p.push(c);
    }
    p.join(";")
}

// base: 30 for foreground, 40 for background
fn color(c: Color, base: u8) -> Option<String> {
    match c {
        Color::Spec(c) => Some(format!("{};2;{};{};{}", base + 8, c.r, c.g, c.b)),
        Color::Indexed(i) => Some(format!("{};5;{i}", base + 8)),
        Color::Named(n) => {
            let i = n as usize;
            match i {
                0..=7 => Some(format!("{}", base as usize + i)),
                8..=15 => Some(format!("{}", base as usize + 60 + i - 8)),
                _ if (NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize).contains(&i) => Some(format!("{}", base as usize + i - NamedColor::DimBlack as usize)),
                _ => None, // the terminal's default
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_text_and_styles() {
        let mut s = Screen::new(10, 4, 100);
        s.write(b"\x1b[31mred-text\x1b[0m\r\nplain   \r\n");
        assert_eq!(s.read("recent", "text", 50), "red-text\nplain");
        assert_eq!(s.read("recent", "ansi", 50), "\x1b[31mred-text\x1b[0m\nplain");
        assert_eq!(s.screen_text(), "red-text\nplain");
        s.write(b"0123456789abc");
        assert_eq!(s.read("recent", "text", 50), "red-text\nplain\n0123456789\nabc");
        assert_eq!(s.read("recent-unwrapped", "text", 50), "red-text\nplain\n0123456789abc");
        assert_eq!(s.read("recent", "text", 2), "0123456789\nabc");
    }

    #[test]
    fn replays_a_screen() {
        let mut s = Screen::new(20, 5, 100);
        s.write(b"one\r\n\x1b[1;32mtwo\x1b[0m\r\nthree\x1b[?2004h\x1b[?1h");
        let mut fresh = Screen::new(20, 5, 100);
        fresh.write(s.replay().as_bytes());
        assert_eq!(fresh.read("recent", "ansi", 50), s.read("recent", "ansi", 50));
        assert!(fresh.mode().contains(TermMode::BRACKETED_PASTE | TermMode::APP_CURSOR));
        let (a, b) = (&s.term.grid().cursor.point, &fresh.term.grid().cursor.point);
        assert_eq!((a.line, a.column), (b.line, b.column));
    }
}
