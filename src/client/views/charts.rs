// Charts a plugin view asks for by their data, drawn here at whatever size the layout gives them, as cells of text:
// eighth blocks for bars (8 steps a cell), half blocks for heatmaps (2 rows a cell), braille for lines and arcs (2×4
// dots a cell). Pure: the view renderer paints the cells in the theme's colours.

// What a cell is painted in: a tone of the theme, "track" (the dim groove under a bar or arc), or `mix` of track and a
// tone (0: track, 1: the tone) for heatmaps; `rgb` (#rrggbb) for a colour a plugin's Raster gives itself.
#[derive(Clone, Debug, PartialEq)]
pub struct Ink {
    pub tone: &'static str,
    pub mix: Option<f64>,
    pub rgb: Option<String>,
}

impl Ink {
    pub fn tone(tone: &'static str) -> Ink {
        Ink { tone, mix: None, rgb: None }
    }
    fn mixed(tone: &'static str, mix: f64) -> Ink {
        Ink { tone, mix: Some(mix), rgb: None }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub ch: char,
    pub fg: Option<Ink>,
    pub bg: Option<Ink>,
}

pub type Grid = Vec<Vec<Cell>>; // rows of cells

const EIGHTHS: [char; 8] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉']; // left-aligned, for horizontal bars
const LEVELS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█']; // bottom-aligned, for columns

fn clamp(n: f64, lo: f64, hi: f64) -> f64 {
    if n.is_finite() { n.max(lo).min(hi) } else { lo }
}
fn unit(n: f64) -> f64 {
    clamp(n, 0.0, 1.0)
}
// JavaScript's Math.round: halves go up
fn round(n: f64) -> f64 {
    (n + 0.5).floor()
}
fn blank(w: usize, h: usize) -> Grid {
    (0..h).map(|_| (0..w).map(|_| Cell { ch: ' ', fg: None, bg: None }).collect()).collect()
}
// the largest of `from` and `values`, as Math.max(from, ...values)
fn largest(from: f64, values: impl IntoIterator<Item = f64>) -> f64 {
    values.into_iter().fold(from, f64::max)
}

// The tone names a plugin may use, as the theme knows them (anything else is the text colour).
pub const TONES: [&str; 8] = ["fg", "dim", "accent", "warn", "working", "blocked", "done", "idle"];
pub fn tone_name(s: &str) -> &'static str {
    TONES.iter().find(|t| **t == s).copied().unwrap_or("fg")
}

// A horizontal bar `width` cells long, filled to `value` (0-1) at an eighth of a cell, over a track.
pub fn progress(value: f64, width: usize, tone: &'static str) -> Vec<Cell> {
    let eighths = round(unit(value) * width as f64 * 8.0) as usize;
    let (full, part) = (eighths / 8, eighths % 8);
    (0..width)
        .map(|i| {
            if i < full {
                Cell { ch: '█', fg: Some(Ink::tone(tone)), bg: None }
            } else if i == full && part > 0 {
                Cell { ch: EIGHTHS[part], fg: Some(Ink::tone(tone)), bg: Some(Ink::tone("track")) }
            } else {
                Cell { ch: ' ', fg: None, bg: Some(Ink::tone("track")) }
            }
        })
        .collect()
}

// Columns rising from the bottom of a `width`×`height` box, one a value, the last `width` values; `min`/`max` default
// to 0 and the largest value. One row high, it's a sparkline.
pub fn columns(values: &[f64], width: usize, height: usize, tone: &'static str, min: f64, max: Option<f64>) -> Grid {
    let shown = &values[values.len().saturating_sub(width)..];
    let top = max.unwrap_or_else(|| largest(min, shown.iter().copied()));
    let mut grid = blank(width, height);
    let start = width - shown.len(); // right-aligned: the newest value is at the right edge
    for (i, v) in shown.iter().enumerate() {
        let mut level = round(unit(if top > min { (v - min) / (top - min) } else { 0.0 }) * height as f64 * 8.0) as i64;
        let mut row = height as i64 - 1;
        while row >= 0 && level > 0 {
            grid[row as usize][start + i] = Cell { ch: LEVELS[level.min(8) as usize], fg: Some(Ink::tone(tone)), bg: None };
            row -= 1;
            level -= 8;
        }
    }
    grid
}

// Rows of values as half-block cells, two values a cell (top, bottom), each coloured from the track up to `tone` by
// where it sits between 0 (or `min`) and the largest value.
pub fn heatmap(values: &[Vec<f64>], tone: &'static str, min: f64, max: Option<f64>) -> Grid {
    let top = max.unwrap_or_else(|| largest(min, values.iter().flatten().copied()));
    let level = |v: Option<&f64>| v.map(|v| Ink::mixed(tone, unit(if top > min { (v - min) / (top - min) } else { 0.0 })));
    let width = values.iter().map(Vec::len).max().unwrap_or(0);
    let mut grid = vec![];
    for r in (0..values.len()).step_by(2) {
        grid.push(
            (0..width)
                .map(|c| {
                    let up = level(values[r].get(c));
                    let down = level(values.get(r + 1).and_then(|row| row.get(c)));
                    Cell { ch: '▀', fg: Some(up.unwrap_or(Ink::mixed("track", 0.0))), bg: down }
                })
                .collect(),
        );
    }
    grid
}

// ---------- braille ----------

const DOT: [[u8; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]]; // DOT[y % 4][x % 2]

// A `width`×`height` cell box as 2×4 dots a cell. A cell has one colour: the last dot set in it decides it.
pub struct Braille {
    pub width: usize,
    pub height: usize,
    bits: Vec<Vec<u8>>,
    ink: Vec<Vec<Option<Ink>>>,
}

impl Braille {
    pub fn new(width: usize, height: usize) -> Braille {
        Braille { width, height, bits: vec![vec![0; width]; height], ink: vec![vec![None; width]; height] }
    }
    pub fn dots_wide(&self) -> f64 {
        (self.width * 2) as f64
    }
    pub fn dots_high(&self) -> f64 {
        (self.height * 4) as f64
    }
    pub fn set(&mut self, x: f64, y: f64, ink: &Ink) {
        let (x, y) = (round(x), round(y));
        if !(x >= 0.0 && y >= 0.0 && x < self.dots_wide() && y < self.dots_high()) {
            return;
        }
        let (x, y) = (x as usize, y as usize);
        let (cx, cy) = (x >> 1, y >> 2);
        self.bits[cy][cx] |= DOT[y & 3][x & 1];
        self.ink[cy][cx] = Some(ink.clone());
    }
    pub fn line(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, ink: &Ink) {
        let steps = (x1 - x0).abs().max((y1 - y0).abs()).ceil().max(1.0) as usize;
        for i in 0..=steps {
            let t = i as f64 / steps as f64;
            self.set(x0 + (x1 - x0) * t, y0 + (y1 - y0) * t, ink);
        }
    }
    pub fn grid(&self) -> Grid {
        self.bits
            .iter()
            .enumerate()
            .map(|(y, row)| row.iter().enumerate().map(|(x, b)| if *b > 0 { Cell { ch: char::from_u32(0x2800 + *b as u32).unwrap(), fg: self.ink[y][x].clone(), bg: None } } else { Cell { ch: ' ', fg: None, bg: None } }).collect())
            .collect()
    }
}

// Series as lines across a `width`×`height` box, every series on one scale (`min`/`max` default to the data's).
pub fn lines(series: &[(Vec<f64>, &'static str)], width: usize, height: usize, min: Option<f64>, max: Option<f64>) -> Grid {
    let all: Vec<f64> = series.iter().flat_map(|(v, _)| v.iter().copied()).filter(|v| v.is_finite()).collect();
    let lo = min.unwrap_or_else(|| all.iter().copied().fold(0.0, f64::min));
    let hi = max.unwrap_or_else(|| largest(lo + 1e-9, all.iter().copied()));
    let mut b = Braille::new(width, height);
    let high = b.dots_high();
    let y = |v: f64| (high - 1.0) * (1.0 - unit((v - lo) / (hi - lo)));
    for (values, tone) in series {
        let n = values.len();
        if n == 0 {
            continue;
        }
        let wide = b.dots_wide();
        let x = |i: usize| if n == 1 { 0.0 } else { (i as f64 * (wide - 1.0)) / (n - 1) as f64 };
        let ink = Ink::tone(tone);
        if n == 1 {
            b.set(0.0, y(values[0]), &ink);
        }
        for i in 1..n {
            b.line(x(i - 1), y(values[i - 1]), x(i), y(values[i]), &ink);
        }
    }
    b.grid()
}

// A gauge: an arc over 240°, open at the bottom, its track dim and filled clockwise to `value` (0-1), with `label`
// (the percentage when absent) in the middle. Sized to the box: as big a circle as fits.
pub fn gauge(value: f64, width: usize, height: usize, tone: &'static str, label: Option<&str>) -> Grid {
    let mut b = Braille::new(width, height);
    // the arc reaches r above its centre and r·sin 30° = r/2 below: 1.5r tall, centred in the box that way
    let cx = (b.dots_wide() - 1.0) / 2.0;
    let r = cx.min((b.dots_high() - 1.0) / 1.5).max(1.0);
    let thick = round(r / 5.0).max(1.0) as usize;
    let cy = r + (b.dots_high() - 1.0 - 1.5 * r) / 2.0;
    let (from, sweep) = (210f64.to_radians(), 240f64.to_radians()); // from lower left, clockwise over the top
    let filled = unit(value) * sweep;
    let steps = (sweep * r * 2.0).ceil() as usize;
    // the track first, then the fill over it: a cell both reach is the fill's colour
    for (upto, ink) in [(sweep, Ink::tone("track")), (filled, Ink::tone(tone))] {
        for i in 0..=steps {
            let a = (i as f64 / steps as f64) * sweep;
            if a > upto {
                break;
            }
            let t = from - a;
            for k in 0..thick {
                let rr = r - k as f64;
                b.set(cx + rr * t.cos(), cy - rr * t.sin(), &ink);
            }
        }
    }
    let mut grid = b.grid();
    let text = label.map(String::from).unwrap_or_else(|| format!("{}%", round(unit(value) * 100.0)));
    let chars: Vec<char> = text.chars().collect();
    let row = round(cy / 4.0) as usize;
    let col = ((width as i64 - chars.len() as i64) / 2).max(0) as usize;
    if let Some(line) = grid.get_mut(row) {
        for (i, ch) in chars.into_iter().take(width).enumerate() {
            if col + i < width {
                line[col + i] = Cell { ch, fg: Some(Ink::tone(tone)), bg: None };
            }
        }
    }
    grid
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(g: &Grid) -> Vec<String> {
        g.iter().map(|r| r.iter().map(|c| c.ch).collect()).collect()
    }

    #[test]
    fn a_progress_bar_fills_to_an_eighth_of_a_cell_over_a_track() {
        assert_eq!(progress(0.23, 20, "accent").iter().map(|c| c.ch).collect::<String>(), format!("████▋{}", " ".repeat(15)));
        assert!(progress(1.0, 4, "accent").iter().all(|c| c.ch == '█'));
        assert!(progress(-3.0, 3, "accent").iter().all(|c| c.ch == ' ' && c.bg.as_ref().is_some_and(|b| b.tone == "track")));
        assert_eq!(progress(0.5, 3, "warn")[1], Cell { ch: '▌', fg: Some(Ink::tone("warn")), bg: Some(Ink::tone("track")) });
    }

    #[test]
    fn columns_rise_from_the_bottom_newest_at_the_right() {
        assert_eq!(text(&columns(&[0.0, 4.0, 8.0], 5, 1, "accent", 0.0, None)), ["   ▄█"]);
        assert_eq!(text(&columns(&[8.0, 16.0], 2, 2, "accent", 0.0, Some(16.0))), [" █", "██"]);
        assert_eq!(text(&columns(&[1.0, 2.0, 3.0], 2, 1, "accent", 0.0, None)), ["▅█"]); // only the last `width` values
    }

    #[test]
    fn braille_two_by_four_dots_a_cell_the_last_dot_colours_it() {
        let mut b = Braille::new(2, 1);
        b.set(0.0, 0.0, &Ink::tone("accent"));
        b.set(3.0, 3.0, &Ink::tone("warn"));
        b.set(9.0, 9.0, &Ink::tone("warn")); // outside: ignored
        assert_eq!(b.grid(), vec![vec![Cell { ch: '⠁', fg: Some(Ink::tone("accent")), bg: None }, Cell { ch: '⢀', fg: Some(Ink::tone("warn")), bg: None }]]);
    }

    #[test]
    fn lines_gauges_and_heatmaps_fill_the_box_they_are_given() {
        let l = lines(&[(vec![0.0, 10.0], "accent")], 4, 2, None, None);
        assert_eq!(l.len(), 2);
        assert_ne!(text(&l)[1].chars().next(), Some(' ')); // starts bottom left
        assert_ne!(text(&l)[0].chars().nth(3), Some(' ')); // ends top right
        let g = gauge(0.5, 12, 5, "accent", None);
        assert_eq!(g.len(), 5);
        assert!(text(&g).join("\n").contains("50%"));
        assert!(g.iter().flatten().any(|c| c.fg.as_ref().is_some_and(|f| f.tone == "track"))); // the unfilled half
        assert!(text(&gauge(0.9, 20, 8, "blocked", Some("5h 90%"))).join("").contains("5h 90%"));
        assert_eq!(
            heatmap(&[vec![0.0, 10.0], vec![10.0, 0.0]], "accent", 0.0, None),
            vec![vec![Cell { ch: '▀', fg: Some(Ink::mixed("accent", 0.0)), bg: Some(Ink::mixed("accent", 1.0)) }, Cell { ch: '▀', fg: Some(Ink::mixed("accent", 1.0)), bg: Some(Ink::mixed("accent", 0.0)) }]]
        );
    }
}
