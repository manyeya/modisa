// Fields: `input`, a line (tui-input), and `textarea`, lines (ratatui-textarea). What's typed is kept until the plugin
// changes `value`; `change` runs as it's edited (at most every 150ms: views/mod.rs), `action` on Enter in an input and
// Ctrl+S in a textarea (where Enter is a new line).
use crossterm::event::{Event, KeyEvent};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Paragraph, Widget};
use ratatui_textarea::{CursorMove, TextArea, WrapMode};
use serde_json::{json, Value};
use tui_input::backend::crossterm::EventHandler;
use tui_input::InputRequest;

use super::build::{fired, Ctx, Draw, ElState, Elems, Fired};
use crate::client::design::{color, mix};
use crate::client::draw::Canvas;
use crate::core::text::width;

pub fn seed(n: &Value, st: &mut ElState) {
    let mut a = TextArea::new(n["value"].as_str().unwrap_or("").split('\n').map(String::from).collect());
    a.set_wrap_mode(WrapMode::Word);
    a.move_cursor(CursorMove::Bottom);
    a.move_cursor(CursorMove::End);
    st.area = a;
}

pub fn value(n: &Value, st: &ElState) -> String {
    if n["type"] == "textarea" { st.area.lines().join("\n") } else { st.input.value().to_string() }
}

// a field's ground: the bar's colour, tinted while it has the keyboard
fn ground(ctx: &Ctx, n: &Value, c: &mut Canvas, area: Rect, on: bool) {
    if n["style"].is_null() {
        c.buf.set_style(area, Style::new().fg(ctx.tone("fg")).bg(color(&if on { mix(ctx.th.bar, ctx.th.focus, 0.12) } else { ctx.th.bar.to_string() })));
    }
}

#[allow(clippy::too_many_arguments)]
pub fn draw_input(ctx: &Ctx, c: &mut Canvas, n: &Value, key: &str, area: Rect, elems: &mut Elems, d: &mut Draw, on: bool) {
    ground(ctx, n, c, area, on);
    let st = elems.entry(key.to_string()).or_default();
    let row = Rect { height: 1, ..area };
    let text = st.input.value();
    // what shows: the text, or a mask character for each of its characters; scrolled to keep the caret in it
    let (shown, caret) = match n["mask"].as_str().filter(|m| !m.is_empty()) {
        Some(m) => (m.repeat(text.chars().count()), st.input.cursor() * width(m)),
        None => (text.to_string(), st.input.visual_cursor()),
    };
    let skip = caret.saturating_sub(row.width.saturating_sub(1) as usize);
    if text.is_empty() {
        Span::styled(n["placeholder"].as_str().unwrap_or("").to_string(), Style::new().fg(ctx.tone("dim"))).render(row, c.buf);
    } else {
        Paragraph::new(shown).scroll((0, skip.min(u16::MAX as usize) as u16)).render(row, c.buf);
    }
    if on {
        d.cursor = Some(Position::new(row.x + (caret - skip) as u16, row.y));
    }
}

pub fn draw_textarea(ctx: &Ctx, c: &mut Canvas, n: &Value, key: &str, area: Rect, elems: &mut Elems, on: bool) {
    ground(ctx, n, c, area, on);
    let a = &mut elems.entry(key.to_string()).or_default().area;
    a.set_style(Style::new());
    a.set_cursor_line_style(Style::new());
    a.set_placeholder_text(n["placeholder"].as_str().unwrap_or(""));
    a.set_placeholder_style(Style::new().fg(ctx.tone("dim")));
    // its own caret, drawn only while it has the keyboard
    a.set_cursor_style(if on { Style::new().add_modifier(Modifier::REVERSED) } else { Style::new() });
    if n["line_numbers"] == true {
        a.set_line_number_style(Style::new().fg(ctx.tone("dim")));
    } else {
        a.remove_line_number();
    }
    (&*a).render(area, c.buf);
}

// A key while a field has the keyboard: it takes them all. Returns what runs, and whether the text changed.
pub fn key(n: &Value, st: &mut ElState, k: &KeyEvent, name: &str) -> (Vec<Fired>, bool) {
    let area = n["type"] == "textarea";
    let act = if area { name == "C-s" } else { name == "enter" };
    if act {
        return (fired(n, "action", json!({ "value": value(n, st) })).into_iter().collect(), false);
    }
    if area {
        return (vec![], st.area.input(*k));
    }
    (vec![], st.input.handle_event(&Event::Key(*k)).is_some_and(|c| c.value))
}

// Text pasted into a field.
pub fn paste(n: &Value, st: &mut ElState, text: &str) {
    if n["type"] == "textarea" {
        st.area.insert_str(text.replace("\r\n", "\n"));
    } else {
        for ch in text.chars().filter(|c| !c.is_control()) {
            st.input.handle(InputRequest::InsertChar(ch));
        }
    }
}

// A field's change, with what it holds now.
pub fn change(n: &Value, st: &ElState) -> Option<Fired> {
    fired(n, "change", json!({ "value": value(n, st) }))
}
