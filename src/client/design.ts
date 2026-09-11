import type { Rect } from "../core/layout";

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

export function chrome(width: number, height: number, sidebar: boolean, preferred: number) {
  const top = 1; // tab bar
  const bottom = 1; // status row
  const side = sidebar && width >= 100 && height >= 22
    ? Math.min(Math.max(20, Number.isFinite(preferred) ? preferred : 26), 34, Math.floor(width * 0.24)) : 0;
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
