// A key as the program in a pane expects it: the bytes a terminal sends for it, in the pane's current modes. A program
// that turned on the kitty keyboard protocol (CSI > flags u: Claude Code, Codex…) gets its unambiguous encoding, so
// Shift+Enter, Ctrl+I and Esc are told apart from Enter, Tab and the start of an escape sequence; any other gets the
// legacy xterm encoding.
use alacritty_terminal::term::TermMode;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

// xterm's modifier parameter: 1 + shift + 2·alt + 4·ctrl (kitty's adds 8·super)
fn modifier_param(m: KeyModifiers) -> u8 {
    1 + m.contains(KeyModifiers::SHIFT) as u8 + 2 * m.contains(KeyModifiers::ALT) as u8 + 4 * m.contains(KeyModifiers::CONTROL) as u8 + 8 * m.contains(KeyModifiers::SUPER) as u8
}

pub fn encode(key: &KeyEvent, mode: TermMode) -> Vec<u8> {
    if key.kind == KeyEventKind::Release {
        return vec![]; // ponytail: the outer terminal isn't asked for releases, so a program asking for them gets presses only
    }
    if mode.intersects(TermMode::KITTY_KEYBOARD_PROTOCOL) {
        return kitty(key, mode);
    }
    legacy(key, mode)
}

// The kitty keyboard protocol (sw.kovidgoyal.net/kitty/keyboard-protocol): CSI code ; modifiers[:event] [; text] u for
// what's ambiguous in the legacy encoding, the legacy forms for keys that aren't.
fn kitty(key: &KeyEvent, mode: TermMode) -> Vec<u8> {
    let m = if key.code == KeyCode::BackTab { key.modifiers | KeyModifiers::SHIFT } else { key.modifiers };
    let mods = modifier_param(m);
    let all = mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC);
    // a repeat says so when the program asked for event types; a press is the default and goes unsaid
    let event = if key.kind == KeyEventKind::Repeat && mode.contains(TermMode::REPORT_EVENT_TYPES) { ":2" } else { "" };
    let plain = mods == 1 && event.is_empty();
    let csi_u = |code: u32, text: Option<char>| {
        let mut s = format!("\x1b[{code}");
        if !plain || text.is_some() {
            s += &format!(";{mods}{event}");
        }
        if let Some(t) = text {
            s += &format!(";{}", t as u32);
        }
        s.push('u');
        s.into_bytes()
    };
    // keys whose legacy encoding is already unambiguous keep it, with kitty's modifiers when there are any
    let letter = |c: char| if plain { legacy(key, mode) } else { format!("\x1b[1;{mods}{event}{c}").into_bytes() };
    let tilde = |n: u8| if plain { format!("\x1b[{n}~").into_bytes() } else { format!("\x1b[{n};{mods}{event}~").into_bytes() };
    match key.code {
        KeyCode::Char(c) => {
            let chorded = m.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER);
            if !chorded && !all {
                return c.to_string().into_bytes(); // text, shifted or not, is just text
            }
            // the key's own code is the unshifted one: shift is in the modifiers
            let base = if c.is_ascii_uppercase() { c.to_ascii_lowercase() } else { c };
            let text = (all && mode.contains(TermMode::REPORT_ASSOCIATED_TEXT) && !chorded).then_some(c);
            csi_u(base as u32, text)
        }
        // Enter, Tab and Backspace keep their legacy bytes unmodified, so a shell a program left in this mode still works
        KeyCode::Enter if plain && !all => b"\r".to_vec(),
        KeyCode::Tab if plain && !all => b"\t".to_vec(),
        KeyCode::Backspace if plain && !all => b"\x7f".to_vec(),
        KeyCode::Enter => csi_u(13, None),
        KeyCode::Tab | KeyCode::BackTab => csi_u(9, None),
        KeyCode::Backspace => csi_u(127, None),
        KeyCode::Esc => csi_u(27, None),
        KeyCode::Up => letter('A'),
        KeyCode::Down => letter('B'),
        KeyCode::Right => letter('C'),
        KeyCode::Left => letter('D'),
        KeyCode::Home => letter('H'),
        KeyCode::End => letter('F'),
        KeyCode::F(1) => letter('P'),
        KeyCode::F(2) => letter('Q'),
        KeyCode::F(3) => tilde(13), // not CSI R: that's a cursor position report
        KeyCode::F(4) => letter('S'),
        KeyCode::Insert => tilde(2),
        KeyCode::Delete => tilde(3),
        KeyCode::PageUp => tilde(5),
        KeyCode::PageDown => tilde(6),
        KeyCode::F(n @ 5..=12) => tilde([15, 17, 18, 19, 20, 21, 23, 24][n as usize - 5]),
        _ => legacy(key, mode),
    }
}

fn legacy(key: &KeyEvent, mode: TermMode) -> Vec<u8> {
    let m = key.modifiers;
    let (ctrl, alt) = (m.contains(KeyModifiers::CONTROL), m.contains(KeyModifiers::ALT));
    let param = modifier_param(m - KeyModifiers::SUPER);
    // CSI 1;<mods> X when modified, else SS3 X in application cursor mode, else CSI X
    let cursor = |c: char| match (param, mode.contains(TermMode::APP_CURSOR)) {
        (1, true) => format!("\x1bO{c}"),
        (1, false) => format!("\x1b[{c}"),
        (p, _) => format!("\x1b[1;{p}{c}"),
    };
    let tilde = |n: u8| if param == 1 { format!("\x1b[{n}~") } else { format!("\x1b[{n};{param}~") };
    let out = match key.code {
        KeyCode::Char(c) if ctrl => {
            let b = match c.to_ascii_lowercase() {
                c @ 'a'..='z' => c as u8 - b'a' + 1,
                ' ' | '@' | '2' => 0,
                '[' | '3' => 27,
                '\\' | '4' => 28,
                ']' | '5' => 29,
                '^' | '6' => 30,
                '_' | '-' | '7' => 31,
                '?' | '8' => 127,
                _ => return prefix_alt(alt, c.to_string()),
            };
            return prefix_alt(alt, (b as char).to_string());
        }
        KeyCode::Char(c) => return prefix_alt(alt, c.to_string()),
        KeyCode::Enter => return prefix_alt(alt, "\r".into()),
        KeyCode::Tab => "\t".into(),
        KeyCode::BackTab => "\x1b[Z".into(),
        KeyCode::Backspace => return prefix_alt(alt, if ctrl { "\x08" } else { "\x7f" }.into()),
        KeyCode::Esc => return prefix_alt(alt, "\x1b".into()),
        KeyCode::Up => cursor('A'),
        KeyCode::Down => cursor('B'),
        KeyCode::Right => cursor('C'),
        KeyCode::Left => cursor('D'),
        KeyCode::Home => cursor('H'),
        KeyCode::End => cursor('F'),
        KeyCode::Insert => tilde(2),
        KeyCode::Delete => tilde(3),
        KeyCode::PageUp => tilde(5),
        KeyCode::PageDown => tilde(6),
        KeyCode::F(n @ 1..=4) => {
            let c = (b'P' + n - 1) as char;
            if param == 1 { format!("\x1bO{c}") } else { format!("\x1b[1;{param}{c}") }
        }
        KeyCode::F(n @ 5..=12) => tilde([15, 17, 18, 19, 20, 21, 23, 24][n as usize - 5]),
        _ => return vec![],
    };
    out.into_bytes()
}

fn prefix_alt(alt: bool, s: String) -> Vec<u8> {
    if alt { format!("\x1b{s}").into_bytes() } else { s.into_bytes() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, m: KeyModifiers) -> Vec<u8> {
        encode(&KeyEvent::new(code, m), TermMode::default())
    }

    fn kitty(code: KeyCode, m: KeyModifiers, flags: TermMode) -> String {
        String::from_utf8(encode(&KeyEvent::new(code, m), flags)).unwrap()
    }

    #[test]
    fn encodes_legacy_keys() {
        assert_eq!(key(KeyCode::Char('c'), KeyModifiers::CONTROL), b"\x03");
        assert_eq!(key(KeyCode::Char('A'), KeyModifiers::SHIFT), b"A");
        assert_eq!(key(KeyCode::Char('x'), KeyModifiers::ALT), b"\x1bx");
        assert_eq!(key(KeyCode::Up, KeyModifiers::NONE), b"\x1b[A");
        assert_eq!(encode(&KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), TermMode::APP_CURSOR), b"\x1bOA");
        assert_eq!(key(KeyCode::Right, KeyModifiers::CONTROL), b"\x1b[1;5C");
        assert_eq!(key(KeyCode::Delete, KeyModifiers::NONE), b"\x1b[3~");
        assert_eq!(key(KeyCode::F(5), KeyModifiers::SHIFT), b"\x1b[15;2~");
        assert_eq!(key(KeyCode::Backspace, KeyModifiers::NONE), b"\x7f");
        assert_eq!(key(KeyCode::Enter, KeyModifiers::SHIFT), b"\r"); // legacy has no Shift+Enter
    }

    #[test]
    fn encodes_kitty_keys() {
        let d = TermMode::DISAMBIGUATE_ESC_CODES;
        assert_eq!(kitty(KeyCode::Enter, KeyModifiers::SHIFT, d), "\x1b[13;2u");
        assert_eq!(kitty(KeyCode::Enter, KeyModifiers::NONE, d), "\r");
        assert_eq!(kitty(KeyCode::Esc, KeyModifiers::NONE, d), "\x1b[27u");
        assert_eq!(kitty(KeyCode::Char('c'), KeyModifiers::CONTROL, d), "\x1b[99;5u");
        assert_eq!(kitty(KeyCode::Char('I'), KeyModifiers::CONTROL | KeyModifiers::SHIFT, d), "\x1b[105;6u");
        assert_eq!(kitty(KeyCode::Char('a'), KeyModifiers::NONE, d), "a");
        assert_eq!(kitty(KeyCode::Char('A'), KeyModifiers::SHIFT, d), "A");
        assert_eq!(kitty(KeyCode::Tab, KeyModifiers::CONTROL, d), "\x1b[9;5u");
        assert_eq!(kitty(KeyCode::BackTab, KeyModifiers::SHIFT, d), "\x1b[9;2u");
        assert_eq!(kitty(KeyCode::Up, KeyModifiers::NONE, d), "\x1b[A");
        assert_eq!(kitty(KeyCode::Up, KeyModifiers::ALT, d), "\x1b[1;3A");
        assert_eq!(kitty(KeyCode::F(3), KeyModifiers::NONE, d), "\x1b[13~");
        let all = d | TermMode::REPORT_ALL_KEYS_AS_ESC;
        assert_eq!(kitty(KeyCode::Char('a'), KeyModifiers::NONE, all), "\x1b[97u");
        assert_eq!(kitty(KeyCode::Enter, KeyModifiers::NONE, all), "\x1b[13u");
        assert_eq!(kitty(KeyCode::Char('A'), KeyModifiers::SHIFT, all | TermMode::REPORT_ASSOCIATED_TEXT), "\x1b[97;2;65u");
        let repeat = KeyEvent { kind: KeyEventKind::Repeat, ..KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL) };
        assert_eq!(String::from_utf8(encode(&repeat, d | TermMode::REPORT_EVENT_TYPES)).unwrap(), "\x1b[120;5:2u");
    }
}
