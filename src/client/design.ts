import type { Rect } from "../core/layout";
import { brand, halfVariant, logo, logoHalves } from "../config/agents/brands";
import type { Theme } from "../config/themes";

// Measure terminal cells, not UTF-16 code units (paths and titles can contain emoji/CJK).
export function fit(text: string, width: number): string {
  const clean = text.replace(/[\x00-\x1f\x7f-\x9f]/g, " ");
  if (width <= 0) return "";
  if (Bun.stringWidth(clean) <= width) return clean;
  let result = "";
  for (const { segment } of new Intl.Segmenter(undefined, { granularity: "grapheme" }).segment(clean)) {
    if (Bun.stringWidth(result + segment) > width - 1) break;
    result += segment;
  }
  return result + "…";
}

// Blend two #rrggbb colors (t = 0 is a, 1 is b): hover and selection tints derived from any theme.
export function mix(a: string, b: string, t: number): string {
  const ch = (s: string, i: number) => parseInt(s.slice(1 + 2 * i, 3 + 2 * i), 16);
  return "#" + [0, 1, 2].map((i) => Math.round(ch(a, i) + (ch(b, i) - ch(a, i)) * t).toString(16).padStart(2, "0")).join("");
}

// WCAG contrast ratio of two #rrggbb colours (1 to 21).
export function contrast(a: string, b: string): number {
  const lum = (c: string) => {
    const [r, g, b] = [0, 1, 2].map((i) => parseInt(c.slice(1 + 2 * i, 3 + 2 * i), 16) / 255).map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
    return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!;
  };
  const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p);
  return (x! + 0.05) / (y! + 0.05);
}

// An agent's mark: its logo where the terminal shows modisa's logo font (else its glyph), in its brand colour, or in
// the theme's text colour when the brand has none or it would be faint on this theme's sidebar. `cells` is the room
// it takes, its gap after it included: a logo is drawn two cells wide over a one-cell character, so it's given the
// cell after it too, then the gap. `halves` are the logo to centre between two lines, for cells `cell` ems tall.
export function agentMark(th: Theme, agent: string, logos: boolean | "whole" | "halves" = false, cell = 1.2) {
  const { glyph, color } = brand(agent);
  const drawn = logos && logo(agent);
  const halves = drawn && logos !== "whole" ? logoHalves(agent, halfVariant(cell)) : undefined;
  return { glyph: drawn || glyph, color: color && contrast(color, th.bar) >= 3 ? color : th.fg, cells: drawn ? 3 : 2, halves };
}

// How tall a terminal's cells are in ems of its font, from its size in pixels: a monospace cell is about 0.6em wide.
export const cellEms = (pixels: { width: number; height: number }, cols: number, rows: number) => (0.6 * (pixels.height / rows)) / (pixels.width / cols);

// The line height of common monospace fonts, in ems (ascent + descent + line gap): for a terminal that doesn't say how
// big its cells are. Nerd Font builds of them are the same.
const LINE_EMS: Record<string, number> = {
  menlo: 1.164, sfmono: 1.193, jetbrainsmono: 1.32, firacode: 1.311, firamono: 1.2, cascadiacode: 1.172, cascadiamono: 1.172,
  hack: 1.164, sourcecodepro: 1.257, droidsansmono: 1.164, dejavusansmono: 1.164, ibmplexmono: 1.3, robotomono: 1.319,
  iosevka: 1.25, ubuntumono: 1.0, consolas: 1.172, monaco: 1.334, inconsolata: 1.1, victormono: 1.37, commitmono: 1.3, geistmono: 1.3,
};
// the first font of a CSS-style list, as a key: no case, spaces or quotes, and no Nerd Font suffix
export function fontEms(family: string | undefined, lineHeight = 1): number | undefined {
  const key = (family ?? "").split(",")[0]!.toLowerCase().replace(/['"\s]/g, "").replace(/(nerdfont(mono|propo)?|nfm|nfp|nf|nl)$/, "");
  return LINE_EMS[key] && LINE_EMS[key]! * lineHeight;
}

// The task an agent's terminal title names, without the spinner or mark it puts in front; "" when the title only
// names the agent (or isn't there).
export function agentTask(title: string | undefined, ...not: (string | undefined)[]) {
  const t = (title ?? "").replace(/^[^\p{L}\p{N}]+/u, "").trim();
  return t && !not.includes(t) ? t : "";
}

export function chrome(width: number, height: number, sidebar: boolean, preferred: number) {
  const top = 1; // tab bar
  const bottom = 1; // status row
  const side = sidebar && width >= 100 && height >= 22
    ? Math.min(Math.max(20, Number.isFinite(preferred) ? preferred : 26), 48, Math.floor(width / 3)) : 0;
  return { top, bottom, side, area: { x: side, y: top, w: Math.max(1, width - side), h: Math.max(1, height - top - bottom) } satisfies Rect };
}

export function floating(width: number, height: number, wantedWidth: number, wantedHeight: number, x?: number, y?: number): Rect {
  const w = Math.max(1, Math.min(wantedWidth, width));
  const h = Math.max(1, Math.min(wantedHeight, height));
  return { x: Math.max(0, Math.min(x ?? Math.floor((width - w) / 2), width - w)), y: Math.max(0, Math.min(y ?? Math.floor((height - h) / 3), height - h)), w, h };
}

// Keep the selected tab in view, with an equal cell budget for each visible tab.
export function tabWindow(count: number, active: number, width: number) {
  const visible = Math.max(1, Math.min(count, Math.floor(width / 18)));
  const start = Math.max(0, Math.min(active - Math.floor(visible / 2), count - visible));
  return { start, end: start + visible, width: Math.max(1, Math.floor(width / visible)) };
}

// Reserve the footer before assigning list rows. Agent rows always occupy exactly
// two lines; overflow controls are part of the budget rather than drawn over it.
export function sidebarBudget(height: number, agents: number) {
  const remaining = Math.max(0, height - 5 - 3); // the footer, then a blank line, the AGENTS heading and a blank line
  const moreAgents = agents * 2 > remaining;
  const agentRows = Math.min(agents, Math.max(0, Math.floor((remaining - Number(moreAgents)) / 2)));
  return { agentRows, moreAgents: agents > agentRows, lines: remaining }; // lines: what the agent list may use
}

// Keep priority order, but never strand the focused agent behind an overflow row.
export function sidebarAgents<T extends { id: string }>(agents: T[], focused: string, budget: number): T[] {
  if (budget <= 0) return [];
  const visible = agents.slice(0, budget);
  const active = agents.find((agent) => agent.id === focused);
  if (active && !visible.includes(active)) visible[visible.length - 1] = active;
  return visible;
}

// Right-hand state labels keep their cell budget; Unicode names fit the remainder.
export function sidebarColumns(left: string, right: string, width: number) {
  const tail = fit(right, Math.max(0, width));
  const head = fit(left, Math.max(0, width - Bun.stringWidth(tail) - Number(Boolean(tail))));
  return { left: head + " ".repeat(Math.max(0, width - Bun.stringWidth(head + tail))), right: tail };
}
