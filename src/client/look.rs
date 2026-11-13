// How the chrome looks beyond a theme's twelve tokens: the shades it mixes from them (raised rows, panels, hairlines,
// shadows), so every theme, built-in or the user's, gets them; its motion (a spinner for what's working, a slow pulse on
// what needs you, toasts sliding in, the focus ring's sweep), which [ui] motion turns off; and its pills' rounded ends,
// where the terminal draws them itself ([ui] caps).
use std::collections::HashMap;
use std::f64::consts::TAU;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};

use super::design::{contrast, mix};
use super::draw::Canvas;
use super::App;
use crate::config::themes::Theme;
use crate::config::IndicatorStyle;
use crate::core::layout::Rect;
use crate::protocol::types::AgentState;

// ---------- shades ----------

pub fn light(th: &Theme) -> bool {
    contrast(th.bg, "#000000") > 11.0 // a luminance over one half
}
// a row lifted off the bar: the selection, a pill
pub fn raised(th: &Theme) -> String {
    mix(th.bar, th.fg, if light(th) { 0.05 } else { 0.065 })
}
// lifted further: a keycap, the active tab
pub fn raised2(th: &Theme) -> String {
    mix(th.bar, th.fg, if light(th) { 0.1 } else { 0.13 })
}
// a pane's border when nothing's happening in it
pub fn hair(th: &Theme) -> String {
    if light(th) { mix(th.border, th.bg, 0.15) } else { mix(th.border, th.fg, 0.07) }
}
// quieter than dim: counts, ids, times
pub fn faint(th: &Theme) -> String {
    mix(th.dim, th.bar, if light(th) { 0.3 } else { 0.35 })
}
// dialogs and toasts float on this
pub fn panel(th: &Theme) -> String {
    if light(th) { mix(th.bg, "#ffffff", 0.7) } else { mix(th.bar, th.fg, 0.035) }
}
pub fn panel_line(th: &Theme) -> String {
    mix(th.border, th.fg, if light(th) { 0.05 } else { 0.14 })
}
// the focus ring runs from the focus colour to this and back
pub fn ring2(th: &Theme) -> String {
    if th.accent == th.focus { mix(th.focus, th.working, 0.55) } else { th.accent.to_string() }
}

// ---------- motion ----------

static START: OnceLock<Instant> = OnceLock::new();

fn secs() -> f64 {
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

const SPIN: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
pub const SWEEP: Duration = Duration::from_millis(1100); // the focus ring's turn when focus moves
pub const SLIDE: Duration = Duration::from_millis(320); // a toast coming in
pub const FADE: Duration = Duration::from_millis(160); // a dialog's veil
pub const WHICH_KEY: Duration = Duration::from_millis(450); // the prefix held this long shows the keys after it

pub fn on(app: &App) -> bool {
    app.cfg.ui.motion
}

// 0 to 1 and back every 1.7s: the pulse on what needs you (still: most of the way up)
pub fn breath(app: &App) -> f64 {
    if on(app) { 0.5 + 0.5 * (secs() * TAU / 1.7).sin() } else { 0.75 }
}

// a working agent's mark: a spinner with symbols in motion, else its indicator
pub fn working_icon(app: &App) -> &'static str {
    if on(app) && app.cfg.indicators.style == IndicatorStyle::Symbols { SPIN[(secs() * 10.0) as usize % SPIN.len()] } else { app.icon(AgentState::Working) }
}

// how far along `d` it is since `at`, eased out (1 when still)
pub fn eased(app: &App, at: Instant, d: Duration) -> f64 {
    if !on(app) {
        return 1.0;
    }
    let t = (at.elapsed().as_secs_f64() / d.as_secs_f64()).min(1.0);
    1.0 - (1.0 - t).powi(3)
}

// Something on screen moves, or is about to: frames come on their own while it does (views::animate).
pub fn moving(app: &App) -> bool {
    if app.prefix_armed && app.prefix_at.elapsed() < WHICH_KEY + Duration::from_millis(100) {
        return true; // the keys after the prefix are about to show, motion or not
    }
    if !on(app) || app.view.is_none() {
        return false;
    }
    let young = |at: Instant, d: Duration| at.elapsed() < d;
    app.view.as_ref().is_some_and(|v| v.panes.iter().any(|p| p.agent.as_ref().is_some_and(|a| matches!(a.state, AgentState::Working | AgentState::Blocked))))
        || !app.toasts.is_empty()
        || young(app.focus_at, SWEEP)
        || (app.modal.is_some() && young(app.modal_at, FADE))
}

// A colour's text shimmering: a highlight moving along `s` every 2.4s; each character with its colour.
pub fn shimmer(app: &App, s: &str, base: &str, hi: &str) -> Vec<(String, String)> {
    let n = s.chars().count() as f64;
    let pos = if on(app) { (secs() % 2.4) / 2.4 * (n + 8.0) - 4.0 } else { -99.0 };
    s.chars().enumerate().map(|(i, ch)| (ch.to_string(), mix(base, hi, 0.7 * (-((i as f64 - pos).powi(2)) / 3.0).exp()))).collect()
}

// What changed since the last frame that starts something moving: focus, a dialog opening, the prefix.
pub fn notice(app: &mut App) {
    let mut start = false;
    if app.ready() {
        let f = app.tab().focused.clone();
        if f != app.focus_last {
            if !app.focus_last.is_empty() {
                app.focus_at = Instant::now();
                start = true;
            }
            app.focus_last = f;
        }
    }
    let (modal, prefix) = (app.modal.is_some(), app.prefix_armed);
    if modal && !app.was.0 {
        app.modal_at = Instant::now();
        start = true;
    }
    if prefix && !app.was.1 {
        app.prefix_at = Instant::now();
        start = true;
    }
    app.was = (modal, prefix);
    if start {
        super::views::animate(app);
    }
}

// ---------- what agents are doing ----------

// How much a pane printed each of its last seconds: an agent's activity, from the output the client already gets.
#[derive(Default)]
pub struct Activity {
    buckets: [u32; 12],
    last: u64, // the second the newest bucket is for
}

impl Activity {
    pub fn add(&mut self, bytes: usize) {
        self.roll();
        let i = (self.last % 12) as usize;
        self.buckets[i] = self.buckets[i].saturating_add(bytes as u32);
    }
    fn roll(&mut self) {
        let now = secs() as u64;
        for s in (self.last + 1)..=now.min(self.last + 12) {
            self.buckets[(s % 12) as usize] = 0;
        }
        self.last = self.last.max(now);
    }
    // the last `n` whole seconds as block heights, oldest first; a quiet second is the baseline
    pub fn spark(&self, n: usize) -> String {
        self.spark_at(secs() as u64, n)
    }
    fn spark_at(&self, now: u64, n: usize) -> String {
        let n = n.min(11);
        let at = |sec: u64| if sec <= self.last && self.last - sec < 12 { self.buckets[(sec % 12) as usize] } else { 0 };
        let vals: Vec<u32> = (1..=n as u64).rev().map(|k| now.checked_sub(k).map_or(0, at)).collect();
        let top = vals.iter().copied().max().unwrap_or(0).max(600) as f64;
        vals.iter().map(|&v| ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"][((v as f64 / top) * 7.0).ceil().min(7.0) as usize]).collect()
    }
}

// what a pane printed in its last `n` seconds, as a sparkline
pub fn spark(app: &App, pane: &str, n: usize) -> String {
    app.activity.get(pane).map(|a| a.spark(n)).unwrap_or_else(|| "▁".repeat(n))
}

// whether a pane printed anything in its last `n` seconds
pub fn active(app: &App, pane: &str, n: usize) -> bool {
    spark(app, pane, n).chars().any(|ch| ch != '▁')
}

// How long it's been in this state, once we've seen it change (an agent found already working has no start we know).
pub fn since(app: &App, pane: &str) -> Option<String> {
    let (_, at, seen) = app.since.get(pane)?;
    if !seen {
        return None;
    }
    let s = at.elapsed().as_secs();
    Some(if s >= 3600 { format!("{}h", s / 3600) } else if s >= 600 { format!("{}m", s / 60) } else { format!("{}:{:02}", s / 60, s % 60) })
}

// the view's agents' states, each with when it began
pub fn track(app: &mut App) {
    let Some(v) = &app.view else { return };
    let mut next: HashMap<String, (AgentState, Instant, bool)> = HashMap::new();
    for p in &v.panes {
        let Some(a) = &p.agent else { continue };
        let entry = match app.since.get(&p.id) {
            Some((s, at, seen)) if *s == a.state => (*s, *at, *seen),
            Some(_) => (a.state, Instant::now(), true),
            None => (a.state, Instant::now(), false),
        };
        next.insert(p.id.clone(), entry);
    }
    app.since = next;
    let live: Vec<String> = v.panes.iter().map(|p| p.id.clone()).collect();
    app.activity.retain(|id, _| live.contains(id));
}

// A derived shade that lasts, for a Theme (whose tokens are &'static str): kept once per colour, so a handful per theme.
pub fn intern(c: String) -> &'static str {
    static POOL: OnceLock<std::sync::Mutex<std::collections::HashSet<&'static str>>> = OnceLock::new();
    let mut pool = POOL.get_or_init(Default::default).lock().unwrap();
    if let Some(s) = pool.get(c.as_str()) {
        return s;
    }
    let s: &'static str = Box::leak(c.into_boxed_str());
    pool.insert(s);
    s
}

// ---------- pills ----------

pub const CAP_L: &str = "\u{e0b6}";
pub const CAP_R: &str = "\u{e0b4}";

// Rounded ends on pills: [ui] caps, and with auto, terminals that draw Powerline's shapes themselves (not from a font)
pub fn caps(app: &App) -> bool {
    match app.cfg.ui.caps.as_str() {
        "rounded" => true,
        "square" => false,
        _ => {
            static AUTO: OnceLock<bool> = OnceLock::new();
            *AUTO.get_or_init(|| {
                let var = |k: &str| std::env::var(k).unwrap_or_default();
                ["ghostty", "WezTerm", "vscode"].contains(&var("TERM_PROGRAM").as_str()) || var("TERM").contains("kitty") || std::env::var_os("KITTY_WINDOW_ID").is_some() || std::env::var_os("GHOSTTY_RESOURCES_DIR").is_some()
            })
        }
    }
}

// a pill's end at (x, y): its fill against what's around it, rounded or a square cell of the fill; one cell either way
pub fn cap(app: &App, c: &mut Canvas, x: i32, y: i32, left: bool, fill: &str, around: &str) {
    if caps(app) {
        c.text(x, y, if left { CAP_L } else { CAP_R }, fill, Some(around), Modifier::empty(), 1);
    } else {
        c.text(x, y, " ", fill, Some(fill), Modifier::empty(), 1);
    }
}

// ---------- depth ----------

fn shade(col: Color, th: &Theme, by: f64) -> Color {
    let to = if light(th) { "#3a3f5a" } else { "#000000" };
    match col {
        Color::Rgb(r, g, b) => {
            let m = mix(&format!("#{r:02x}{g:02x}{b:02x}"), to, by);
            super::design::color(&m)
        }
        other => other,
    }
}

// A soft shadow to the right of `r` and under it: a darker column, and a half row (▀) under it.
pub fn shadow(c: &mut Canvas, r: Rect, th: &Theme) {
    let by = if light(th) { 0.16 } else { 0.55 };
    let area = c.buf.area;
    let inside = |b: &Buffer, x: i32, y: i32| x >= 0 && y >= 0 && (x as u16) < area.width && (y as u16) < area.height && b.cell((x as u16, y as u16)).is_some();
    for y in r.y + 1..r.y + r.h {
        let x = r.x + r.w;
        if inside(c.buf, x, y) {
            let cell = c.buf.cell_mut((x as u16, y as u16)).unwrap();
            cell.fg = shade(cell.fg, th, by);
            cell.bg = shade(cell.bg, th, by);
        }
    }
    let y = r.y + r.h;
    for x in r.x + 1..=r.x + r.w {
        if inside(c.buf, x, y) {
            let cell = c.buf.cell_mut((x as u16, y as u16)).unwrap();
            let under = cell.bg;
            cell.set_symbol("▀");
            cell.fg = shade(under, th, by);
            cell.modifier = Modifier::empty();
        }
    }
}

// The screen behind a dialog: drained of colour and sunk toward the background (`by` 0 to 1, as the dialog opens).
pub fn veil(c: &mut Canvas, th: &Theme, by: f64) {
    let to = if light(th) { th.bg.to_string() } else { mix(th.bg, "#000000", 0.35) };
    let [tr, tg, tb] = rgb(&to);
    let sink = |col: Color, k: f64| match col {
        Color::Rgb(r, g, b) => {
            let y = 0.3 * r as f64 + 0.59 * g as f64 + 0.11 * b as f64;
            let grey = |v: u8| v as f64 + (y - v as f64) * 0.45;
            let to = |v: f64, t: u8| (v + (t as f64 - v) * k).round().clamp(0.0, 255.0) as u8;
            Some(Color::Rgb(to(grey(r), tr), to(grey(g), tg), to(grey(b), tb)))
        }
        _ => None,
    };
    for cell in c.buf.content.iter_mut() {
        match sink(cell.fg, 0.6 * by) {
            Some(col) => cell.fg = col,
            None if by > 0.5 => cell.modifier |= Modifier::DIM,
            None => {}
        }
        if let Some(col) = sink(cell.bg, 0.5 * by) {
            cell.bg = col;
        }
    }
}

fn rgb(c: &str) -> [u8; 3] {
    let h = |i: usize| u8::from_str_radix(c.get(1 + 2 * i..3 + 2 * i).unwrap_or("00"), 16).unwrap_or(0);
    [h(0), h(1), h(2)]
}

// A border's cells recoloured along its length, clockwise from its top-left corner: `at(t)` for t in 0..1.
pub fn ring(c: &mut Canvas, r: Rect, at: impl Fn(f64) -> String) {
    if r.w < 2 || r.h < 2 {
        return;
    }
    let mut cells = vec![];
    cells.extend((0..r.w).map(|i| (r.x + i, r.y)));
    cells.extend((1..r.h).map(|j| (r.x + r.w - 1, r.y + j)));
    cells.extend((0..r.w - 1).rev().map(|i| (r.x + i, r.y + r.h - 1)));
    cells.extend((1..r.h - 1).rev().map(|j| (r.x, r.y + j)));
    let n = cells.len() as f64;
    let area = c.buf.area;
    for (k, (x, y)) in cells.into_iter().enumerate() {
        if x < 0 || y < 0 || x as u16 >= area.width || y as u16 >= area.height {
            continue;
        }
        if let Some(cell) = c.buf.cell_mut((x as u16, y as u16)) {
            cell.fg = super::design::color(&at(k as f64 / n));
        }
    }
}

// the focus ring's colour at `t` along it; `turn` is how far it has swept round since focus came (0 to 1)
pub fn ring_color(th: &Theme, t: f64, turn: f64) -> String {
    let second = ring2(th);
    mix(th.focus, &second, 0.5 - 0.5 * (TAU * (t + 0.1 + turn)).cos())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_is_seconds_of_output_as_blocks() {
        let mut a = Activity::default();
        assert_eq!(a.spark_at(100, 6), "▁▁▁▁▁▁"); // quiet
        a.last = 100;
        a.buckets[(99 % 12) as usize] = 6000; // second 99 was busy, 98 a little
        a.buckets[(98 % 12) as usize] = 1000;
        assert_eq!(a.spark_at(100, 4), "▁▁▃█");
        assert_eq!(a.spark_at(120, 4), "▁▁▁▁"); // long ago
    }

    #[test]
    fn shades_hold_in_light_and_dark_themes() {
        let dark = crate::config::themes::find_theme("pasture").unwrap();
        let day = crate::config::themes::find_theme("github-light").unwrap();
        assert!(!light(dark) && light(day));
        for th in [dark, day] {
            assert!(contrast(&raised(th), th.fg) > 7.0, "text on a raised row reads");
            assert!(contrast(&panel(th), th.fg) > 7.0, "text on a panel reads");
            assert_ne!(ring2(th), th.focus);
        }
    }
}
