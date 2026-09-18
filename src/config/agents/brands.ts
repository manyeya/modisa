// Each built-in agent's mark: its logo, drawn from modisa's logo font (marks.ttf, see src/platform/logos.ts) where the
// terminal can show it, else one single-width Unicode glyph (text presentation, never an emoji, so it lines up in any
// monospace font); and its brand colour. No colour means a monochrome brand: drawn in the theme's text colour. A plugin
// names an agent (`{ icon: "claude-code" }`) and modisa draws the mark, so no plugin picks colours of its own.
export type Brand = { glyph: string; color?: string };

export const BRANDS: Record<string, Brand> = {
  "claude-code": { glyph: "✳", color: "#d97757" },
  codex: { glyph: "◎" },
  gemini: { glyph: "✦", color: "#4796e3" },
  "cursor-agent": { glyph: "◈" },
  copilot: { glyph: "◉", color: "#a371f7" },
  opencode: { glyph: "□" },
  pi: { glyph: "π", color: "#c792ea" },
  omp: { glyph: "ω", color: "#6fb3ff" },
  droid: { glyph: "▲", color: "#ee6018" },
  amp: { glyph: "»", color: "#f34e3f" },
  kiro: { glyph: "◒", color: "#9046ff" },
  kimi: { glyph: "ĸ", color: "#1783ff" },
  kilo: { glyph: "▦", color: "#e2b714" },
  devin: { glyph: "✤", color: "#1fa5a0" },
  grok: { glyph: "⊘" },
  hermes: { glyph: "☿", color: "#c9a227" },
  qodercli: { glyph: "◐", color: "#7b61ff" },
  qwen: { glyph: "❖", color: "#615ced" },
  antigravity: { glyph: "Λ", color: "#4285f4" },
  cline: { glyph: "⊡" },
  mastracode: { glyph: "ℳ" },
  maki: { glyph: "◍", color: "#e0655e" },
  muse: { glyph: "♪", color: "#e0a0ff" },
  aider: { glyph: "≻", color: "#14b014" },
};

export const GENERIC: Brand = { glyph: "•" };
export const brand = (agent: string): Brand => BRANDS[agent] ?? GENERIC;

// The agents with a logo in marks.ttf, and the Lobe Icons (MIT) mark each is drawn from; the font has them from
// U+F5A00 in this order (a private-use range no common icon font uses). Append only: a codepoint that has shipped keeps
// its logo, since installed fonts outlive the binary that installed them.
export const LOGOS: [agent: string, icon: string][] = [
  ["claude-code", "claude"], ["codex", "codex"], ["gemini", "gemini"], ["cursor-agent", "cursor"], ["copilot", "githubcopilot"],
  ["opencode", "opencode"], ["pi", "pi"], ["amp", "amp"], ["kiro", "kiro"], ["kimi", "kimi"], ["kilo", "kilocode"],
  ["devin", "devin"], ["grok", "grok"], ["hermes", "nousresearch"], ["qodercli", "qoder"], ["qwen", "qwen"],
  ["antigravity", "antigravity"], ["cline", "cline"], ["mastracode", "mastra"],
];
export const LOGO_FIRST = 0xf5a00;
export function logo(agent: string): string | undefined {
  const i = LOGOS.findIndex(([a]) => a === agent);
  return i < 0 ? undefined : String.fromCodePoint(LOGO_FIRST + i);
}

// Each logo also comes in halves, to sit centred between an agent row's two lines: the top half on the first line, the
// bottom half on the second, one cell-height apart. Where they meet depends on the terminal's cell height (in ems of
// its font), so the halves come in HALF_VARIANTS sizes, for cells from 1.10em to 1.50em tall in steps of 0.02em;
// variant v's top halves are at HALVES_FIRST + v * 0x100 + i, its bottom halves 0x80 after.
export const HALVES_FIRST = 0xf6000;
export const HALF_VARIANTS = 21;
export const halfCell = (v: number) => 1.1 + 0.02 * v;
export function logoHalves(agent: string, variant: number): [top: string, bottom: string] | undefined {
  const i = LOGOS.findIndex(([a]) => a === agent);
  if (i < 0) return undefined;
  const at = HALVES_FIRST + Math.max(0, Math.min(HALF_VARIANTS - 1, variant)) * 0x100 + i;
  return [String.fromCodePoint(at), String.fromCodePoint(at + 0x80)];
}
// the variant for a terminal whose cells are `cell` ems tall
export const halfVariant = (cell: number) => Math.max(0, Math.min(HALF_VARIANTS - 1, Math.round((cell - 1.1) / 0.02)));

// every character the logo font has: what a terminal's codepoint map covers
export const LOGO_RANGE = `U+${LOGO_FIRST.toString(16).toUpperCase()}-U+${(HALVES_FIRST + HALF_VARIANTS * 0x100 - 1).toString(16).toUpperCase()}`;
