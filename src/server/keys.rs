// tmux-style key names for `modisa pane keys`: Enter, Escape, C-c, M-x, Up, … or literal text.
pub fn key_bytes(k: &str) -> String {
    let named = match k {
        "Enter" => "\r",
        "Escape" | "Esc" => "\x1b",
        "Tab" => "\t",
        "BSpace" | "Backspace" => "\x7f",
        "Space" => " ",
        "Up" => "\x1b[A",
        "Down" => "\x1b[B",
        "Right" => "\x1b[C",
        "Left" => "\x1b[D",
        "Home" => "\x1b[H",
        "End" => "\x1b[F",
        "PageUp" => "\x1b[5~",
        "PageDown" => "\x1b[6~",
        "Delete" => "\x1b[3~",
        _ => "",
    };
    if !named.is_empty() {
        return named.into();
    }
    let mut chars = k.chars();
    match (chars.next(), chars.next(), chars.next(), chars.next()) {
        // the low five bits of its UTF-16 code unit, like JavaScript's charCodeAt & 0x1f
        (Some('C'), Some('-'), Some(c), None) => char::from_u32(c.to_lowercase().next().unwrap_or(c) as u32 & 0x1f).unwrap().to_string(),
        (Some('M'), Some('-'), Some(c), None) => format!("\x1b{c}"),
        _ => k.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(key_bytes("C-c"), "\x03");
        assert_eq!(key_bytes("C-C"), "\x03");
        assert_eq!(key_bytes("M-x"), "\x1bx");
        assert_eq!(key_bytes("Enter"), "\r");
        assert_eq!(key_bytes("hello"), "hello");
        assert_eq!(key_bytes("C-"), "C-");
    }
}
