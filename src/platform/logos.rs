// Agents' logos in the terminal. Modisa's logo font (src/config/agents/marks.ttf) goes into the user's font folder, and
// each terminal that has to be told where those characters are learns it: Ghostty and kitty get a codepoint map, in a
// block modisa owns; VS Code and its forks get the font at the end of their terminal font list, after whatever they
// already use, so text looks the same. WezTerm finds the font by itself. Nothing needs admin rights, and uninstalling
// takes back exactly what was added. What was done is kept in DIR/logos.json, and a user who took the logos out isn't
// given them again.
use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::LazyLock;

use regex::{Captures, Regex};
use serde::Deserialize;
use serde_json::json;

use crate::config::agents::brands::LOGO_RANGE;
use crate::core::paths::{which, DIR, HOME};
use crate::integrations::edit::{with_block, write};

// The font, carried in the binary.
static MARKS: &[u8] = include_bytes!("../config/agents/marks.ttf");

pub const FAMILY: &str = "Modisa Marks";
// (the original asks `uname -s` when it starts; a native binary knows what it was built for)
const MAC: bool = cfg!(target_os = "macos");
static FONT: LazyLock<String> = LazyLock::new(|| {
    if MAC {
        format!("{}/Library/Fonts/ModisaMarks.ttf", *HOME)
    } else {
        format!("{}/fonts/ModisaMarks.ttf", std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| format!("{}/.local/share", *HOME)))
    }
});
static STATE: LazyLock<String> = LazyLock::new(|| format!("{}/logos.json", *DIR));
static XDG: LazyLock<String> = LazyLock::new(|| std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| format!("{}/.config", *HOME)));
static SUPPORT: LazyLock<String> = LazyLock::new(|| format!("{}/Library/Application Support", *HOME));
const BEGIN: &str = "# >>> modisa agent logos (managed; `modisa logos uninstall` removes it)";
const END: &str = "# <<< modisa agent logos";

// inserted: VS Code settings whose font list modisa created; installed/updated: when the font went in, and when this
// version of it did (epoch ms)
#[derive(Default, Deserialize)]
#[serde(default)]
struct State {
    removed: Option<bool>,
    inserted: Option<Vec<String>>,
    installed: Option<i64>,
    updated: Option<i64>,
}
fn read_state() -> State {
    std::fs::read(&*STATE).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}
// Bun.file(path).exists(): a file (a directory isn't one)
fn exists(path: &str) -> bool {
    Path::new(path).is_file()
}
fn dir_exists(path: &str) -> bool {
    Path::new(path).is_dir()
}
fn first_existing(paths: &[String]) -> Option<String> {
    paths.iter().find(|p| exists(p)).cloned()
}
// A file's text (Bun.file(path).text()): any failure, a missing file too, is an error.
fn read(path: &str) -> Result<String, String> {
    std::fs::read(path).map(|b| String::from_utf8_lossy(&b).into_owned()).map_err(|e| format!("{path}: {e}"))
}
fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}
// An environment variable as Bun.env has it: "" when unset (`?? ""`), and set only when it's not empty (truthy).
fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_default()
}
fn env_set(name: &str) -> bool {
    !env(name).is_empty()
}

// Ghostty and kitty: a codepoint map, in modisa's block
struct Mapped {
    name: &'static str,
    present: fn() -> bool,
    path: fn() -> String,
    line: fn() -> String,
}
const MAPPED: [Mapped; 2] = [
    Mapped {
        name: "Ghostty",
        present: || which("ghostty").is_some() || (MAC && dir_exists("/Applications/Ghostty.app")) || dir_exists(&format!("{}/ghostty", *XDG)),
        path: || {
            let (x, s) = (&*XDG, &*SUPPORT);
            first_existing(&[format!("{x}/ghostty/config"), format!("{x}/ghostty/config.ghostty"), format!("{s}/com.mitchellh.ghostty/config"), format!("{s}/com.mitchellh.ghostty/config.ghostty")]).unwrap_or_else(|| format!("{x}/ghostty/config"))
        },
        line: || format!("font-codepoint-map = {LOGO_RANGE}={FAMILY}"),
    },
    Mapped {
        name: "kitty",
        present: || which("kitty").is_some() || (MAC && dir_exists("/Applications/kitty.app")) || dir_exists(&format!("{}/kitty", *XDG)),
        path: || format!("{}/kitty/kitty.conf", *XDG),
        line: || format!("symbol_map {LOGO_RANGE} {FAMILY}"),
    },
];

// VS Code and its forks: the settings files that exist
const VSCODES: [&str; 4] = ["VS Code", "Code - Insiders", "Cursor", "VSCodium"];
fn vscode_settings() -> Vec<(&'static str, String)> {
    let base = if MAC { &*SUPPORT } else { &*XDG };
    let mut found = Vec::new();
    for app in ["Code", "Code - Insiders", "Cursor", "VSCodium"] {
        let path = format!("{base}/{app}/User/settings.json");
        if exists(&path) {
            found.push((if app == "Code" { "VS Code" } else { app }, path));
        }
    }
    found
}

// Edited as text, not parsed and rewritten: settings.json may hold comments and trailing commas, and every byte of it
// that isn't the terminal's font list stays as it was.
static KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"("terminal\.integrated\.fontFamily"\s*:\s*")((?:[^"\\]|\\.)*)(")"#).unwrap());
static EDITOR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""editor\.fontFamily"\s*:\s*"((?:[^"\\]|\\.)*)""#).unwrap());
static EMPTY_OBJECT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\{\s*\}$").unwrap());
static KEY_LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\n[ \t]*"terminal\.integrated\.fontFamily"\s*:\s*"(?:[^"\\]|\\.)*",?"#).unwrap());
const DEFAULT_FONTS: &str = if MAC { "Menlo, Monaco, 'Courier New', monospace" } else { "'Droid Sans Mono', 'monospace', monospace" };

// (text, created): `created` going in says modisa made the font list itself (so taking the font out takes the whole
// line), and coming out of an add says it just did.
pub fn with_vscode_font(text: &str, add: bool, created: bool) -> (String, bool) {
    let found = KEY.captures(text);
    if add {
        if let Some(found) = found {
            if found[2].contains(FAMILY) {
                return (text.to_string(), false);
            }
            return (KEY.replace(text, |c: &Captures| format!("{}{}, '{FAMILY}'{}", &c[1], &c[2], &c[3])).into_owned(), false);
        }
        let editor = EDITOR.captures(text).map(|c| c[1].to_string()).filter(|e| !e.is_empty()).unwrap_or_else(|| DEFAULT_FONTS.into());
        let line = format!("\"terminal.integrated.fontFamily\": \"{editor}, '{FAMILY}'\"");
        return match text.find('{') {
            Some(open) if !EMPTY_OBJECT.is_match(text.trim()) => (format!("{}\n    {line},{}", &text[..=open], &text[open + 1..]), true),
            _ => (format!("{{\n    {line}\n}}\n"), true),
        };
    }
    let Some(found) = found else { return (text.to_string(), false) };
    if created {
        return (KEY_LINE.replace(text, "").into_owned(), false);
    }
    let rest = found[2].split(',').map(str::trim).filter(|f| f.replace(['\'', '"'], "") != FAMILY).collect::<Vec<_>>().join(", ");
    (KEY.replace(text, |c: &Captures| format!("{}{rest}{}", &c[1], &c[3])).into_owned(), false)
}

#[derive(Clone, Debug, PartialEq)]
pub struct TerminalStatus {
    pub name: &'static str,
    pub path: String,
    pub configured: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct LogoStatus {
    pub font: Option<String>,
    pub current: bool,
    pub terminals: Vec<TerminalStatus>,
}

pub fn logo_status() -> Result<LogoStatus, String> {
    let mut terminals = Vec::new();
    for t in &MAPPED {
        if !(t.present)() {
            continue;
        }
        let path = (t.path)();
        let configured = read(&path).unwrap_or_default().contains(&format!("{BEGIN}\n{}\n{END}", (t.line)()));
        terminals.push(TerminalStatus { name: t.name, path, configured });
    }
    for (name, path) in vscode_settings() {
        let configured = KEY.captures(&read(&path)?).is_some_and(|c| c[2].contains(FAMILY));
        terminals.push(TerminalStatus { name, path, configured });
    }
    Ok(LogoStatus { font: exists(&FONT).then(|| FONT.clone()), current: current(), terminals })
}

// The font, and every terminal found that needs telling: returns the terminals it set up.
pub async fn install_logos() -> Result<Vec<&'static str>, String> {
    let font = Path::new(&*FONT);
    if let Some(dir) = font.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(font, MARKS).map_err(|e| format!("{}: {e}", *FONT))?;
    if !MAC && which("fc-cache").is_some() {
        let dir = &FONT[..FONT.rfind('/').unwrap_or(0)];
        let _ = tokio::process::Command::new("fc-cache").args(["-f", dir]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().await;
    }
    let state = read_state();
    // a Set: in the order they went in, each once
    let mut inserted: Vec<String> = Vec::new();
    for path in state.inserted.unwrap_or_default() {
        if !inserted.contains(&path) {
            inserted.push(path);
        }
    }
    let mut done = Vec::new();
    for t in &MAPPED {
        if !(t.present)() {
            continue;
        }
        let path = (t.path)();
        write(&path, &with_block(&read(&path).unwrap_or_default(), BEGIN, END, Some(&format!("{}\n", (t.line)()))))?;
        done.push(t.name);
    }
    for (name, path) in vscode_settings() {
        let (text, created) = with_vscode_font(&read(&path)?, true, false);
        write(&path, &text)?;
        if created && !inserted.contains(&path) {
            inserted.push(path);
        }
        done.push(name);
    }
    let now = now();
    write(&STATE, &json!({ "inserted": inserted, "installed": state.installed.unwrap_or(now), "updated": now }).to_string())?;
    Ok(done)
}

pub fn uninstall_logos() -> Result<(), String> {
    let state = read_state();
    let _ = std::fs::remove_file(&*FONT);
    for t in &MAPPED {
        let path = (t.path)();
        if let Ok(text) = read(&path) {
            if text.contains(BEGIN) {
                write(&path, &with_block(&text, BEGIN, END, None))?;
            }
        }
    }
    let inserted = state.inserted.unwrap_or_default();
    for (_, path) in vscode_settings() {
        let text = read(&path)?;
        let (next, _) = with_vscode_font(&text, false, inserted.contains(&path));
        if next != text {
            write(&path, &next)?;
        }
    }
    write(&STATE, &json!({ "removed": true }).to_string())
}

// Whether the installed font is this modisa's: a newer one has more in it (and its terminals' maps cover more).
fn current() -> bool {
    exists(&FONT) && std::fs::read(&*FONT).is_ok_and(|b| b == MARKS)
}

// What install_logos_once did: whether the font was there already (an update, not a first install), and the terminals
// it set up.
#[derive(Clone, Debug, PartialEq)]
pub struct Installed {
    pub updated: bool,
    pub terminals: Vec<&'static str>,
}

// When a TUI starts: install them the first time, and bring them up to date after an update, unless the user took them
// out. Returns what it did and the terminals it set up, or nothing when there was nothing to do.
//
// setupLogos (src/client/logos.ts) calls it only when the logos mode is "auto" (see logos_off). An error goes to the
// debug log ("logos not installed: <error>") and counts as nothing done. When it did something it toasts, in the
// theme's accent, with ` for <terminals, joined by ", ">` as `where` when there are any:
//   updated: "agent logos updated{where}: quit and reopen the terminal to centre them"
//   else:    "agent logos installed{where}: quit and reopen the terminal to see them"
pub async fn install_logos_once() -> Result<Option<Installed>, String> {
    let state = read_state();
    if state.removed.unwrap_or(false) || current() {
        return Ok(None);
    }
    let updated = exists(&FONT);
    Ok(Some(Installed { updated, terminals: install_logos().await? }))
}

// Whether the user turned the logos off for this process: MODISA_LOGOS=off (the test suite sets it, so a real home's
// fonts are never touched). setupLogos takes it over the config: mode = "off" when this says so, else
// [sidebar] logos ("auto", "on" or "off").
pub fn logos_off() -> bool {
    std::env::var("MODISA_LOGOS").is_ok_and(|v| v == "off")
}

#[derive(Clone, Debug, PartialEq)]
pub struct TerminalFont {
    pub family: String,
    pub line_height: f64,
}

static VSCODE_FAMILY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""terminal\.integrated\.fontFamily"\s*:\s*"((?:[^"\\]|\\.)*)""#).unwrap());
static VSCODE_LINE_HEIGHT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""terminal\.integrated\.lineHeight"\s*:\s*([0-9.]+)"#).unwrap());
static GHOSTTY_FAMILY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?m)^\s*font-family\s*=\s*"?([^"\n]+)"?\s*$"#).unwrap());
static KITTY_FAMILY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^\s*font_family\s+(.+)$").unwrap());

// The font this terminal draws text in, from its settings (or its default): for guessing its cell height when it
// doesn't say how big its cells are.
//
// setupLogos sets the cell height it guesses from it: fontEms(family, line_height) (client/design), or 1.2 when that
// doesn't know the font or this fails.
pub fn terminal_font() -> Result<TerminalFont, String> {
    let (program, term) = (env("TERM_PROGRAM"), env("TERM"));
    let setting = |text: &str, re: &Regex| re.captures(text).map(|c| c[1].trim().to_string());
    let font = |family: &str, line_height: f64| TerminalFont { family: family.into(), line_height };
    if program == "vscode" {
        for (_, path) in vscode_settings() {
            let text = read(&path)?;
            let family = setting(&text, &VSCODE_FAMILY).filter(|f| !f.is_empty()).or_else(|| setting(&text, &EDITOR)).filter(|f| !f.is_empty());
            // Number(…) || 1: what isn't a number, or is 0, is 1
            let line_height = setting(&text, &VSCODE_LINE_HEIGHT).and_then(|n| n.parse::<f64>().ok()).filter(|n| *n != 0.0 && !n.is_nan()).unwrap_or(1.0);
            if let Some(family) = family {
                return Ok(TerminalFont { family, line_height });
            }
        }
        return Ok(font(if MAC { "Menlo" } else { "Droid Sans Mono" }, 1.0));
    }
    if program == "ghostty" || term == "xterm-ghostty" || env_set("GHOSTTY_RESOURCES_DIR") {
        let text = read(&(MAPPED[0].path)()).unwrap_or_default();
        return Ok(font(&setting(&text, &GHOSTTY_FAMILY).filter(|f| !f.is_empty()).unwrap_or_else(|| "JetBrains Mono".into()), 1.0));
    }
    if env_set("KITTY_WINDOW_ID") || term == "xterm-kitty" {
        let text = read(&(MAPPED[1].path)()).unwrap_or_default();
        return Ok(font(&setting(&text, &KITTY_FAMILY).filter(|f| !f.is_empty()).unwrap_or_else(|| (if MAC { "Menlo" } else { "DejaVu Sans Mono" }).into()), 1.0));
    }
    if program == "WezTerm" {
        return Ok(font("JetBrains Mono", 1.0));
    }
    Ok(font("", 1.0))
}

static PS_LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*([0-9]+)\s+([0-9]+)\s+(.+?)\s*$").unwrap());
const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];

// ps's lstart ("Thu Oct  9 10:12:34 2026", local time) in epoch ms, as Date.parse reads it.
fn parse_lstart(s: &str) -> Option<i64> {
    let fields: Vec<&str> = s.split_whitespace().collect();
    let [_, month, day, time, year] = fields[..] else { return None };
    let month = MONTHS.iter().position(|m| month.eq_ignore_ascii_case(m))?;
    let hms: Vec<i32> = time.split(':').map(|n| n.parse().ok()).collect::<Option<_>>()?;
    let [hour, min, sec] = hms[..] else { return None };
    // SAFETY: a zeroed tm is a valid one, and mktime only reads and normalises the one it's given.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    tm.tm_year = year.parse::<i32>().ok()? - 1900;
    tm.tm_mon = month as i32;
    tm.tm_mday = day.parse().ok()?;
    (tm.tm_hour, tm.tm_min, tm.tm_sec) = (hour, min, sec);
    tm.tm_isdst = -1; // whether summer time applies then: mktime works it out
    let secs = unsafe { libc::mktime(&mut tm) };
    (secs != -1).then_some(secs as i64 * 1000)
}

// When the terminal this runs in started: the app at the top of this process's ancestry (Ghostty, VS Code, kitty…),
// the one just below launchd or init. A terminal loads its fonts when it starts, so this says which font it has.
pub async fn terminal_started(pid: u32) -> Option<i64> {
    let out = tokio::process::Command::new("ps").args(["-axo", "pid=,ppid=,lstart="]).stdin(Stdio::null()).stderr(Stdio::null()).output().await;
    let text = out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let mut table: HashMap<i64, (i64, Option<i64>)> = HashMap::new(); // pid: (parent, started)
    for line in text.split('\n') {
        if let Some(m) = PS_LINE.captures(line) {
            if let (Ok(pid), Ok(parent)) = (m[1].parse(), m[2].parse()) {
                table.insert(pid, (parent, parse_lstart(&m[3])));
            }
        }
    }
    let mut at = table.get(&(pid as i64));
    let mut hops = 0;
    while let Some(&(parent, _)) = at {
        if parent <= 1 || hops >= 64 {
            break;
        }
        let Some(up) = table.get(&parent) else { break };
        at = Some(up);
        hops += 1;
    }
    at.and_then(|&(_, started)| started)
}

// How much of the logo font a terminal has loaded: the older font's whole logos, or this one's halves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loaded {
    Whole,
    Halves,
}

// Which of the logo font's glyphs the terminal can draw. A terminal finds a newly installed font while it runs
// (Ghostty does), but keeps the version it first loaded: after an update it has the older font's whole logos, which
// are at the same characters, and gets a logo's halves once it's been started since the update. So an update never
// takes the logos away meanwhile.
pub fn font_loaded(started: Option<i64>, updated: i64) -> Loaded {
    let slack = 60_000; // ps reports whole seconds, and a terminal that started just before still hadn't looked
    match started {
        Some(started) if started < updated - slack => Loaded::Whole,
        _ => Loaded::Halves,
    }
}

// setupLogos, in "auto" mode and once logos_visible() says yes, draws logos as this says ("on" mode draws Halves
// without asking); the sidebar's logos are then Some(that), else None (plain marks). When that differs from what it
// had, it sets it, clears the chrome's signature (chromeSig = "", so the chrome is drawn again) and renders.
pub async fn logos_loaded() -> Loaded {
    let state = read_state();
    // a modisa before 0.1.14 didn't note when it updated the font: the font file says when it last changed (and a
    // file that isn't there, as Bun says it, changed at the end of time)
    let modified = || {
        let at = std::fs::metadata(&*FONT).and_then(|m| m.modified()).ok()?;
        Some(at.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as i64)
    };
    let updated = state.updated.or_else(modified).unwrap_or(4_503_599_627_370_495);
    font_loaded(terminal_started(std::process::id()).await, updated)
}

// Whether the terminal this runs in will draw the logos: the font is installed and the terminal is one that finds it
// (told by its codepoint map or settings, or on its own). Inside tmux or screen the terminal outside isn't known.
//
// setupLogos asks it in "auto" mode: yes, and logos_loaded() says how they're drawn; no, and the sidebar shows plain
// marks. (A VS Code settings file it can't read is a no; the original's setupLogos stopped there instead.)
pub fn logos_visible() -> bool {
    if !exists(&FONT) || env_set("TMUX") || env_set("STY") {
        return false;
    }
    let (program, term) = (env("TERM_PROGRAM"), env("TERM"));
    let Ok(status) = logo_status() else { return false };
    let configured = |name: &str| status.terminals.iter().any(|t| t.name == name && t.configured);
    if program == "ghostty" || term == "xterm-ghostty" || env_set("GHOSTTY_RESOURCES_DIR") {
        return configured("Ghostty");
    }
    if env_set("KITTY_WINDOW_ID") || term == "xterm-kitty" {
        return configured("kitty");
    }
    if program == "WezTerm" {
        return true;
    }
    if program == "vscode" {
        return status.terminals.iter().any(|t| VSCODES.contains(&t.name) && t.configured);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const SETTINGS: &str = "{\n    // my settings\n    \"editor.fontFamily\": \"JetBrains Mono, monospace\",\n    \"files.autoSave\": \"afterDelay\",\n}\n";

    #[test]
    fn vscode_the_logo_font_goes_at_the_end_of_the_terminals_font_list_comments_and_commas_untouched() {
        let (added, created) = with_vscode_font(SETTINGS, true, false);
        assert!(created); // there was no terminal font list: one is made from the editor's
        assert!(added.contains(&format!("\"terminal.integrated.fontFamily\": \"JetBrains Mono, monospace, '{FAMILY}'\",")));
        assert!(added.contains("// my settings"));
        assert!(added.ends_with("\"files.autoSave\": \"afterDelay\",\n}\n"));
        assert_eq!(with_vscode_font(&added, true, false).0, added); // twice is once
        assert_eq!(with_vscode_font(&added, false, true).0, SETTINGS); // and out again, byte for byte
    }

    #[test]
    fn vscode_an_existing_terminal_font_list_keeps_its_fonts_gets_the_logo_font_last_and_loses_only_it_again() {
        let own = r#"{ "terminal.integrated.fontFamily": "Fira Code, 'Symbols Nerd Font'" }"#;
        let added = with_vscode_font(own, true, false);
        assert_eq!(added, (format!(r#"{{ "terminal.integrated.fontFamily": "Fira Code, 'Symbols Nerd Font', '{FAMILY}'" }}"#), false));
        assert_eq!(with_vscode_font(&added.0, false, false).0, own);
        assert!(with_vscode_font("{}", true, false).0.contains(&format!(", '{FAMILY}'\"\n}}"))); // an empty object: no stray comma
    }

    #[tokio::test]
    async fn after_an_update_a_terminal_draws_the_older_fonts_whole_logos_and_their_halves_once_its_restarted() {
        let hour = 3_600_000;
        let updated = 20 * hour;
        assert_eq!(font_loaded(Some(15 * hour), updated), Loaded::Whole); // running since before the update: it has the older font
        assert_eq!(font_loaded(Some(21 * hour), updated), Loaded::Halves); // restarted since
        assert_eq!(font_loaded(Some(updated - 30_000), updated), Loaded::Halves); // within ps's second and a bit
        assert_eq!(font_loaded(None, updated), Loaded::Halves); // can't tell
        let started = terminal_started(std::process::id()).await;
        assert!(started.is_none_or(|s| s <= now()));
    }

    #[test]
    fn ps_start_times_are_read_as_local_time() {
        let at = parse_lstart("Thu Oct  9 10:12:34 2026").unwrap();
        // whatever the zone, the same wall-clock second a day later is a day (give or take a summer-time hour) later
        let next = parse_lstart("Fri Oct 10 10:12:34 2026").unwrap();
        assert!((next - at - 86_400_000).abs() <= 3_600_000);
        assert_eq!(at % 1000, 0);
        assert_eq!(parse_lstart("not a date"), None);
    }
}
