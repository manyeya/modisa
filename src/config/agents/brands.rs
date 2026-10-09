// Each built-in agent's mark: its logo, drawn from modisa's logo font (marks.ttf, see src/platform/logos.ts) where the
// terminal can show it, else one single-width Unicode glyph (text presentation, never an emoji, so it lines up in any
// monospace font); and its brand colour. No colour means a monochrome brand: drawn in the theme's text colour. A plugin
// names an agent (`{ icon: "claude-code" }`) and modisa draws the mark, so no plugin picks colours of its own.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Brand {
    pub glyph: &'static str,
    pub color: Option<&'static str>,
}

const fn b(glyph: &'static str, color: Option<&'static str>) -> Brand {
    Brand { glyph, color }
}

pub const BRANDS: &[(&str, Brand)] = &[
    ("claude-code", b("✳", Some("#d97757"))),
    ("codex", b("◎", None)),
    ("gemini", b("✦", Some("#4796e3"))),
    ("cursor-agent", b("◈", None)),
    ("copilot", b("◉", Some("#a371f7"))),
    ("opencode", b("□", None)),
    ("pi", b("π", Some("#c792ea"))),
    ("omp", b("ω", Some("#6fb3ff"))),
    ("droid", b("▲", Some("#ee6018"))),
    ("amp", b("»", Some("#f34e3f"))),
    ("kiro", b("◒", Some("#9046ff"))),
    ("kimi", b("ĸ", Some("#1783ff"))),
    ("kilo", b("▦", Some("#e2b714"))),
    ("devin", b("✤", Some("#1fa5a0"))),
    ("grok", b("⊘", None)),
    ("hermes", b("☿", Some("#c9a227"))),
    ("qodercli", b("◐", Some("#7b61ff"))),
    ("qwen", b("❖", Some("#615ced"))),
    ("antigravity", b("Λ", Some("#4285f4"))),
    ("cline", b("⊡", None)),
    ("mastracode", b("ℳ", None)),
    ("maki", b("◍", Some("#e0655e"))),
    ("muse", b("♪", Some("#e0a0ff"))),
    ("aider", b("≻", Some("#14b014"))),
];

pub const GENERIC: Brand = b("•", None);

pub fn brand(agent: &str) -> Brand {
    BRANDS.iter().find(|(a, _)| *a == agent).map_or(GENERIC, |(_, b)| *b)
}

// The agents with a logo in marks.ttf, and the Lobe Icons (MIT) mark each is drawn from; the font has them from
// U+F5A00 in this order (a private-use range no common icon font uses). Append only: a codepoint that has shipped keeps
// its logo, since installed fonts outlive the binary that installed them.
// omp (oh-my-pi, a fork of pi) has no Lobe mark and none under a licence we could check: it's pi's, in omp's colour.
pub const LOGOS: &[(&str, &str)] = &[
    ("claude-code", "claude"),
    ("codex", "codex"),
    ("gemini", "gemini"),
    ("cursor-agent", "cursor"),
    ("copilot", "githubcopilot"),
    ("opencode", "opencode"),
    ("pi", "pi"),
    ("amp", "amp"),
    ("kiro", "kiro"),
    ("kimi", "kimi"),
    ("kilo", "kilocode"),
    ("devin", "devin"),
    ("grok", "grok"),
    ("hermes", "nousresearch"),
    ("qodercli", "qoder"),
    ("qwen", "qwen"),
    ("antigravity", "antigravity"),
    ("cline", "cline"),
    ("mastracode", "mastra"),
    ("omp", "pi"),
];
pub const LOGO_FIRST: u32 = 0xf5a00;

fn logo_index(agent: &str) -> Option<u32> {
    LOGOS.iter().position(|(a, _)| *a == agent).map(|i| i as u32)
}

pub fn logo(agent: &str) -> Option<char> {
    char::from_u32(LOGO_FIRST + logo_index(agent)?)
}

// Each logo also comes in halves, to sit centred between an agent row's two lines: the top half on the first line, the
// bottom half on the second, one cell-height apart. Where they meet depends on the terminal's cell height (in ems of
// its font), so the halves come in HALF_VARIANTS sizes, for cells from 1.10em to 1.50em tall in steps of 0.02em;
// variant v's top halves are at HALVES_FIRST + v * 0x100 + i, its bottom halves 0x80 after.
pub const HALVES_FIRST: u32 = 0xf6000;
pub const HALF_VARIANTS: u32 = 21;

#[cfg(test)]
pub fn half_cell(v: u32) -> f64 {
    1.1 + 0.02 * v as f64
}

// (top, bottom)
pub fn logo_halves(agent: &str, variant: i64) -> Option<(char, char)> {
    let i = logo_index(agent)?;
    let at = HALVES_FIRST + variant.clamp(0, HALF_VARIANTS as i64 - 1) as u32 * 0x100 + i;
    Some((char::from_u32(at)?, char::from_u32(at + 0x80)?))
}

// the variant for a terminal whose cells are `cell` ems tall
pub fn half_variant(cell: f64) -> i64 {
    // Math.round: halves round up
    (((cell - 1.1) / 0.02 + 0.5).floor() as i64).clamp(0, HALF_VARIANTS as i64 - 1)
}

// every character the logo font has: what a terminal's codepoint map covers (a test checks it against the constants)
pub const LOGO_RANGE: &str = "U+F5A00-U+F74FF";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_agent_with_a_logo_gets_its_own_character_in_order() {
        assert_eq!(logo("claude-code"), char::from_u32(0xf5a00));
        assert_eq!(logo("mastracode"), char::from_u32(0xf5a00 + 18)); // append only: shipped codepoints keep their logos
        assert_eq!(logo("omp"), char::from_u32(0xf5a00 + LOGOS.len() as u32 - 1)); // pi's mark, drawn in omp's colour
        assert_eq!(LOGOS.last(), Some(&("omp", "pi")));
        assert_eq!(logo("aider"), None); // no logo: its glyph instead
        let unique: std::collections::HashSet<_> = LOGOS.iter().map(|(a, _)| a).collect();
        assert_eq!(unique.len(), LOGOS.len());
    }

    #[test]
    fn halves_are_sized_for_the_cells() {
        assert_eq!([half_variant(1.0), half_variant(1.164), half_variant(1.32), half_variant(9.0)], [0, 3, 11, 20]); // clamped
        let (top, bottom) = logo_halves("codex", 3).unwrap();
        assert_eq!((top as u32, bottom as u32), (HALVES_FIRST + 0x300 + 1, HALVES_FIRST + 0x300 + 0x81));
        assert_eq!(logo_halves("codex", 99), logo_halves("codex", 20));
        assert_eq!(logo_halves("aider", 3), None);
        assert!((half_cell(20) - 1.5).abs() < 1e-9);
        assert_eq!(LOGO_RANGE, format!("U+{:X}-U+{:X}", LOGO_FIRST, HALVES_FIRST + HALF_VARIANTS * 0x100 - 1));
    }

    #[test]
    fn brands_fall_back_to_the_generic_mark() {
        assert_eq!(brand("claude-code"), Brand { glyph: "✳", color: Some("#d97757") });
        assert_eq!(brand("codex").color, None); // a monochrome brand
        assert_eq!(brand("who-knows").glyph, "•");
        // every built-in agent but the catch-all has a brand
        for a in super::super::builtin_agents() {
            assert_eq!(BRANDS.iter().any(|(id, _)| *id == a.id), a.id != "generic", "{}", a.id);
        }
    }
}
