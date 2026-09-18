// Each built-in agent's mark: one single-width Unicode glyph (text presentation, never an emoji, so it lines up in any
// monospace font) and its brand colour. They stand in for the vendors' logos, which a terminal can't draw without
// installing a font. No colour means a monochrome brand: drawn in the theme's text colour. A plugin names an agent
// (`{ icon: "claude-code" }`) and modisa draws the mark, so no plugin picks colours of its own.
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
