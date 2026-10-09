// `modisa theme list | show <name> | import <file> [--name n]`: the themes there are, one as a file to start your own
// from, and another app's colour scheme made a modisa theme (Ghostty, iTerm2, Alacritty, kitty, Windows Terminal,
// base16/base24), written to the themes directory.
use std::collections::HashMap;

use serde_json::Value;

use crate::client::design::mix;
use crate::config::themes::{all, find_theme, load_custom, themes_dir, Theme, THEMES};

pub fn run(verb: Option<&str>, arg: Option<&str>, name: Option<&str>) -> i32 {
    let problems = load_custom();
    match verb {
        Some("list") | None => {
            for (n, _) in all() {
                let yours = !THEMES.iter().any(|(b, _)| *b == n);
                println!("{n}{}", if yours { "  (yours)" } else { "" });
            }
            for (file, p) in problems {
                eprintln!("themes/{file}.toml: {p}");
            }
            0
        }
        Some("show") => {
            let Some(t) = arg.and_then(find_theme) else {
                eprintln!("usage: modisa theme show <name> (modisa theme list names them)");
                return 2;
            };
            print!("{}", file(t, &format!("# {}: copy to {}/<name>.toml and change what you like\n", arg.unwrap_or(""), themes_dir())));
            0
        }
        Some("import") => {
            let Some(path) = arg else {
                eprintln!("usage: modisa theme import <file> [--name n]");
                return 2;
            };
            let text = match std::fs::read_to_string(path) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("modisa: {path}: {e}");
                    return 1;
                }
            };
            let Some(p) = palette(&text) else {
                eprintln!("modisa: {path} isn't a colour scheme modisa reads (Ghostty, iTerm2, Alacritty, kitty, Windows Terminal, base16/base24)");
                return 1;
            };
            let stem = std::path::Path::new(path).file_stem().map(|s| s.to_string_lossy().to_lowercase().replace([' ', '_'], "-")).unwrap_or_else(|| "imported".into());
            let name = name.map(String::from).unwrap_or(stem);
            if THEMES.iter().any(|(b, _)| *b == name) {
                eprintln!("modisa: {name} is a built-in theme's name: pass --name");
                return 1;
            }
            let out = format!("{}/{name}.toml", themes_dir());
            let t = from_palette(&p);
            if let Err(e) = std::fs::create_dir_all(themes_dir()).and_then(|_| std::fs::write(&out, file(&t, &format!("# imported from {path}\n")))) {
                eprintln!("modisa: {out}: {e}");
                return 1;
            }
            println!("{out} written: use it with theme = \"{name}\" in config.toml (or pick it in settings)");
            0
        }
        _ => {
            eprintln!("usage: modisa theme list | show <name> | import <file> [--name n]");
            2
        }
    }
}

fn file(t: &Theme, head: &str) -> String {
    let tokens = [("bg", t.bg), ("bar", t.bar), ("fg", t.fg), ("dim", t.dim), ("border", t.border), ("focus", t.focus), ("accent", t.accent), ("warn", t.warn), ("blocked", t.blocked), ("working", t.working), ("done", t.done), ("idle", t.idle)];
    let mut s = head.to_string();
    for (k, v) in tokens {
        s += &format!("{k} = \"{v}\"\n");
    }
    s + "# [roles]\n# \"tab.active\" = \"bold $fg on $border\"\n"
}

// A scheme's colours: "background", "foreground" and "0"–"15" (the ANSI palette), as #rrggbb.
type Palette = HashMap<String, String>;

fn hex(r: f64, g: f64, b: f64) -> String {
    let c = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", c(r), c(g), c(b))
}

fn norm(v: &str) -> Option<String> {
    let v = v.trim().trim_matches('"').trim_matches('\'');
    let h = v.strip_prefix('#').or_else(|| v.strip_prefix("0x")).unwrap_or(v);
    (h.len() == 6 && h.chars().all(|c| c.is_ascii_hexdigit())).then(|| format!("#{}", h.to_lowercase()))
}

const ANSI: [&str; 8] = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"];

fn palette(text: &str) -> Option<Palette> {
    let mut p = Palette::new();
    let t = text.trim_start();
    if t.starts_with("<?xml") || t.contains("<plist") {
        // iTerm2: <key>Ansi 1 Color</key><dict>…Red Component <real>…
        let mut rest = text;
        while let Some(i) = rest.find("<key>") {
            rest = &rest[i + 5..];
            let Some(j) = rest.find("</key>") else { break };
            let key = rest[..j].to_string();
            let Some(dict_end) = rest.find("</dict>") else { break };
            let dict = &rest[..dict_end];
            let comp = |name: &str| -> Option<f64> { dict.split(&format!("<key>{name}</key>")).nth(1)?.split("<real>").nth(1)?.split("</real>").next()?.trim().parse().ok() };
            if let (Some(r), Some(g), Some(b)) = (comp("Red Component"), comp("Green Component"), comp("Blue Component")) {
                let slot = match key.as_str() {
                    "Background Color" => Some("background".to_string()),
                    "Foreground Color" => Some("foreground".to_string()),
                    k => k.strip_prefix("Ansi ").and_then(|k| k.strip_suffix(" Color")).map(String::from),
                };
                if let Some(s) = slot {
                    p.insert(s, hex(r, g, b));
                }
            }
        }
    } else if t.starts_with('{') {
        // Windows Terminal: { "background", "foreground", "black", …, "brightWhite" }
        let v: Value = serde_json::from_str(text).ok()?;
        for (k, x) in v.as_object()? {
            let Some(c) = x.as_str().and_then(norm) else { continue };
            let slot = match k.as_str() {
                "background" | "foreground" => Some(k.clone()),
                k => ANSI.iter().position(|a| *a == k).map(|i| i.to_string()).or_else(|| k.strip_prefix("bright").and_then(|b| ANSI.iter().position(|a| a.eq_ignore_ascii_case(b))).map(|i| (i + 8).to_string())),
            };
            if let Some(s) = slot {
                p.insert(s, c);
            }
        }
    } else if text.contains("base00") {
        // base16/base24 YAML: base00 … base0F (in a palette: section or at the top)
        for line in text.lines() {
            let Some((k, v)) = line.split_once(':') else { continue };
            if let (Some(n), Some(c)) = (k.trim().strip_prefix("base"), norm(v.split('#').nth(1).map(|h| format!("#{h}")).as_deref().unwrap_or(v))) {
                p.insert(format!("base{}", n.to_uppercase()), c);
            }
        }
        let b = |k: &str| p.get(k).cloned();
        let mut q = Palette::new();
        for (slot, base) in [("background", "base00"), ("foreground", "base05"), ("1", "base08"), ("2", "base0B"), ("3", "base0A"), ("4", "base0D"), ("5", "base0E"), ("6", "base0C"), ("8", "base03"), ("bar", "base01"), ("border", "base02"), ("orange", "base09")] {
            if let Some(c) = b(base) {
                q.insert(slot.to_string(), c);
            }
        }
        return (q.len() >= 6).then_some(q);
    } else if text.contains("[colors") {
        // Alacritty: [colors.primary] background/foreground, [colors.normal]/[colors.bright] black … white
        let v: toml::Table = toml::from_str(text).ok()?;
        let colors = v.get("colors")?.as_table()?;
        for (k, x) in colors.get("primary").and_then(|t| t.as_table()).into_iter().flatten() {
            if let (true, Some(c)) = (k == "background" || k == "foreground", x.as_str().and_then(norm)) {
                p.insert(k.clone(), c);
            }
        }
        for (group, off) in [("normal", 0), ("bright", 8)] {
            for (k, x) in colors.get(group).and_then(|t| t.as_table()).into_iter().flatten() {
                if let (Some(i), Some(c)) = (ANSI.iter().position(|a| a == k), x.as_str().and_then(norm)) {
                    p.insert((i + off).to_string(), c);
                }
            }
        }
    } else {
        // Ghostty (background = #…, palette = 1=#…) and kitty (background #…, color1 #…): a key and a value per line
        for line in text.lines().map(str::trim).filter(|l| !l.starts_with('#') && !l.is_empty()) {
            let (k, v) = line.split_once('=').map(|(k, v)| (k.trim(), v.trim())).or_else(|| line.split_once(char::is_whitespace).map(|(k, v)| (k.trim(), v.trim())))?;
            match k {
                "background" | "foreground" => drop(norm(v).map(|c| p.insert(k.to_string(), c))),
                "palette" => drop(v.split_once('=').and_then(|(n, c)| Some((n.trim().parse::<u8>().ok()?, norm(c)?))).map(|(n, c)| p.insert(n.to_string(), c))),
                k if k.starts_with("color") => drop(k[5..].parse::<u8>().ok().zip(norm(v)).map(|(n, c)| p.insert(n.to_string(), c))),
                _ => {}
            }
        }
    }
    (p.contains_key("background") && p.contains_key("foreground") && p.len() >= 8).then_some(p)
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

// modisa's tokens from a scheme: its background and text, its bright black for what's dim, its blue for focus, magenta
// for accent, yellow for warnings and work, red for what's blocked, cyan for done, green for idle.
fn from_palette(p: &Palette) -> Theme {
    let get = |k: &str, or: &str| p.get(k).cloned().unwrap_or_else(|| or.to_string());
    let bg = get("background", "#1a1b26");
    let fg = get("foreground", "#c0caf5");
    let light = {
        let c = |i: usize| u8::from_str_radix(&bg[i..i + 2], 16).unwrap_or(0) as f64 / 255.0;
        0.2126 * c(1) + 0.7152 * c(3) + 0.0722 * c(5) > 0.5
    };
    let bar = p.get("bar").cloned().unwrap_or_else(|| mix(&bg, "#000000", if light { 0.05 } else { 0.18 }));
    let border = p.get("border").cloned().unwrap_or_else(|| mix(&bg, &fg, 0.2));
    let dim = p.get("8").cloned().unwrap_or_else(|| mix(&bg, &fg, 0.45));
    Theme {
        bg: leak(bg),
        bar: leak(bar),
        fg: leak(fg.clone()),
        dim: leak(dim),
        border: leak(border),
        focus: leak(get("4", &fg)),
        accent: leak(get("5", &fg)),
        warn: leak(p.get("orange").cloned().unwrap_or_else(|| get("3", &fg))),
        blocked: leak(get("1", &fg)),
        working: leak(get("3", &fg)),
        done: leak(get("6", &fg)),
        idle: leak(get("2", &fg)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_each_kind_of_scheme() {
        let ghostty = "background = #101010\nforeground = #e0e0e0\npalette = 1=#ff0000\npalette = 2=#00ff00\npalette = 3=#ffff00\npalette = 4=#0000ff\npalette = 5=#ff00ff\npalette = 6=#00ffff\npalette = 8=#808080\n";
        let kitty = "background #101010\nforeground #e0e0e0\ncolor1 #ff0000\ncolor2 #00ff00\ncolor3 #ffff00\ncolor4 #0000ff\ncolor5 #ff00ff\ncolor6 #00ffff\n";
        let alacritty = "[colors.primary]\nbackground = '#101010'\nforeground = '#e0e0e0'\n[colors.normal]\nred = '#ff0000'\ngreen = '#00ff00'\nyellow = '#ffff00'\nblue = '0x0000ff'\nmagenta = '#ff00ff'\ncyan = '#00ffff'\n";
        let wt = r##"{ "name": "x", "background": "#101010", "foreground": "#E0E0E0", "red": "#ff0000", "green": "#00ff00", "yellow": "#ffff00", "blue": "#0000ff", "purple": "#ff00ff", "magenta": "#ff00ff", "cyan": "#00ffff", "brightBlack": "#808080" }"##;
        let base16 = "scheme: x\npalette:\n  base00: \"101010\"\n  base01: \"181818\"\n  base02: \"282828\"\n  base03: \"808080\"\n  base05: \"e0e0e0\"\n  base08: \"ff0000\"\n  base09: \"ff8800\"\n  base0A: \"ffff00\"\n  base0B: \"00ff00\"\n  base0C: \"00ffff\"\n  base0D: \"0000ff\"\n  base0E: \"ff00ff\"\n";
        let iterm = "<?xml version=\"1.0\"?><plist><dict><key>Background Color</key><dict><key>Blue Component</key><real>0.0627</real><key>Green Component</key><real>0.0627</real><key>Red Component</key><real>0.0627</real></dict><key>Foreground Color</key><dict><key>Blue Component</key><real>0.878</real><key>Green Component</key><real>0.878</real><key>Red Component</key><real>0.878</real></dict>"
            .to_string()
            + &(1..=6).map(|i| format!("<key>Ansi {i} Color</key><dict><key>Blue Component</key><real>{}</real><key>Green Component</key><real>0</real><key>Red Component</key><real>1</real></dict>", (i % 2) as f64)).collect::<String>()
            + "</dict></plist>";
        for (what, text) in [("ghostty", ghostty), ("kitty", kitty), ("alacritty", alacritty), ("windows terminal", wt), ("base16", base16), ("iterm2", iterm.as_str())] {
            let p = palette(text).unwrap_or_else(|| panic!("{what}"));
            let t = from_palette(&p);
            assert_eq!((t.bg, t.fg), ("#101010", "#e0e0e0"), "{what}");
            assert!(t.blocked.starts_with("#ff"), "{what}: {}", t.blocked);
        }
        assert_eq!(from_palette(&palette(base16).unwrap()).warn, "#ff8800"); // base16's orange
        assert!(palette("just words\n").is_none());
    }
}
