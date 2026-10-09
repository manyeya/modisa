// A Raster: a grid of cells a plugin paints itself and repaints in place (ui.blit), for animation. Its `cells` are
// base64, per cell three little-endian u32s: the character's code point, then its fg and bg, each an RGB colour, a theme
// tone (TONE | its index in TONES), or DEFAULT (the text colour on the background).
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Style};
use serde_json::Value;

use super::build::Ctx;

const DEFAULT: u32 = 0x0100_0000;
const TONE: u32 = 0x0200_0000;
pub const TONES: [&str; 8] = ["fg", "dim", "accent", "warn", "working", "blocked", "done", "idle"];

pub fn draw(ctx: &Ctx, buf: &mut Buffer, n: &Value, area: Rect) {
    let (columns, rows) = (n["columns"].as_u64().unwrap_or(0) as usize, n["rows"].as_u64().unwrap_or(0) as usize);
    let bytes = crate::protocol::conn::unb64(n["cells"].as_str().unwrap_or(""));
    let word = |i: usize| bytes.get(i * 4..i * 4 + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let ink = |v: u32, default: &str| -> Color {
        if v == DEFAULT {
            ctx.tone(default)
        } else if v & TONE != 0 {
            ctx.tone(TONES.get((v & 0xff) as usize).copied().unwrap_or("fg"))
        } else {
            Color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
        }
    };
    for y in 0..rows.min(area.height as usize) {
        for x in 0..columns.min(area.width as usize) {
            let i = (y * columns + x) * 3;
            let cp = word(i).unwrap_or(0x20);
            let ch = if cp >= 0x20 { char::from_u32(cp).unwrap_or(' ') } else { ' ' };
            let style = Style::new().fg(ink(word(i + 1).unwrap_or(DEFAULT), "fg")).bg(ink(word(i + 2).unwrap_or(DEFAULT), "bg"));
            if let Some(cell) = buf.cell_mut(Position::new(area.x + x as u16, area.y + y as u16)) {
                cell.reset();
                cell.set_char(ch).set_style(style);
            }
        }
    }
}
