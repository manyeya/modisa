// Charts a plugin view asks for by their data, drawn here at whatever size the layout gives them, as cells of text:
// eighth blocks for bars (8 steps a cell), half blocks for heatmaps (2 rows a cell), braille for lines and arcs (2×4
// dots a cell). Pure: the view renderer paints the cells in the theme's colours.
import type { Tone } from "../../protocol/types";

// What a cell is painted in: a tone of the theme, `track` (the dim groove under a bar or arc), or `mix` of track and a
// tone (0: track, 1: the tone) for heatmaps; `rgb` (#rrggbb) for a colour a plugin's Raster gives itself.
export type Ink = { tone: Tone | "track"; mix?: number; rgb?: string };
export type Cell = { ch: string; fg?: Ink; bg?: Ink };
export type Grid = Cell[][]; // rows of cells

const EIGHTHS = ["", "▏", "▎", "▍", "▌", "▋", "▊", "▉"]; // left-aligned, for horizontal bars
const LEVELS = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"]; // bottom-aligned, for columns
const clamp = (n: number, lo = 0, hi = 1) => (Number.isFinite(n) ? Math.min(hi, Math.max(lo, n)) : lo);
const blank = (w: number, h: number): Grid => Array.from({ length: h }, () => Array.from({ length: w }, () => ({ ch: " " })));

// A horizontal bar `width` cells long, filled to `value` (0-1) at an eighth of a cell, over a track.
export function progress(value: number, width: number, tone: Tone): Cell[] {
  const eighths = Math.round(clamp(value) * width * 8);
  const full = Math.floor(eighths / 8), part = eighths % 8;
  return Array.from({ length: Math.max(0, width) }, (_, i) =>
    i < full ? { ch: "█", fg: { tone } } : i === full && part ? { ch: EIGHTHS[part]!, fg: { tone }, bg: { tone: "track" } } : { ch: " ", bg: { tone: "track" } },
  );
}

// Columns rising from the bottom of a `width`×`height` box, one a value, the last `width` values; `min`/`max` default
// to 0 and the largest value. One row high, it's a sparkline.
export function columns(values: number[], width: number, height: number, tone: Tone, min = 0, max?: number): Grid {
  const shown = values.slice(-width);
  const top = max ?? Math.max(min, ...shown);
  const grid = blank(width, height);
  const start = width - shown.length; // right-aligned: the newest value is at the right edge
  shown.forEach((v, i) => {
    let level = Math.round(clamp(top > min ? (v - min) / (top - min) : 0) * height * 8);
    for (let row = height - 1; row >= 0 && level > 0; row--, level -= 8) grid[row]![start + i] = { ch: LEVELS[Math.min(8, level)]!, fg: { tone } };
  });
  return grid;
}

// Rows of values as half-block cells, two values a cell (top, bottom), each coloured from the track up to `tone` by
// where it sits between 0 (or `min`) and the largest value.
export function heatmap(values: number[][], tone: Tone, min = 0, max?: number): Grid {
  const top = max ?? Math.max(min, ...values.flat());
  const level = (v?: number): Ink | undefined => (v === undefined ? undefined : { tone, mix: clamp(top > min ? (v - min) / (top - min) : 0) });
  const width = Math.max(0, ...values.map((r) => r.length));
  const grid: Grid = [];
  for (let r = 0; r < values.length; r += 2) {
    grid.push(Array.from({ length: width }, (_, c) => {
      const up = level(values[r]![c]), down = level(values[r + 1]?.[c]);
      return { ch: "▀", fg: up ?? { tone: "track", mix: 0 }, ...(down && { bg: down }) };
    }));
  }
  return grid;
}

// ---------- braille ----------

const DOT = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]]; // DOT[y % 4][x % 2]

// A `width`×`height` cell box as 2×4 dots a cell. A cell has one colour: the last dot set in it decides it.
export class Braille {
  private bits: number[][];
  private ink: (Ink | undefined)[][];
  constructor(readonly width: number, readonly height: number) {
    this.bits = Array.from({ length: height }, () => Array(width).fill(0));
    this.ink = Array.from({ length: height }, () => Array(width).fill(undefined));
  }
  get dotsWide() {
    return this.width * 2;
  }
  get dotsHigh() {
    return this.height * 4;
  }
  set(x: number, y: number, ink: Ink) {
    x = Math.round(x);
    y = Math.round(y);
    if (x < 0 || y < 0 || x >= this.dotsWide || y >= this.dotsHigh) return;
    const cx = x >> 1, cy = y >> 2;
    this.bits[cy]![cx]! |= DOT[y & 3]![x & 1]!;
    this.ink[cy]![cx] = ink;
  }
  line(x0: number, y0: number, x1: number, y1: number, ink: Ink) {
    const steps = Math.max(1, Math.ceil(Math.max(Math.abs(x1 - x0), Math.abs(y1 - y0))));
    for (let i = 0; i <= steps; i++) this.set(x0 + ((x1 - x0) * i) / steps, y0 + ((y1 - y0) * i) / steps, ink);
  }
  grid(): Grid {
    return this.bits.map((row, y) => row.map((b, x) => (b ? { ch: String.fromCharCode(0x2800 + b), fg: this.ink[y]![x] } : { ch: " " })));
  }
}

// Series as lines across a `width`×`height` box, every series on one scale (`min`/`max` default to the data's).
export function lines(series: { values: number[]; tone: Tone }[], width: number, height: number, min?: number, max?: number): Grid {
  const all = series.flatMap((s) => s.values).filter(Number.isFinite);
  const lo = min ?? Math.min(0, ...all), hi = max ?? Math.max(lo + 1e-9, ...all);
  const b = new Braille(width, height);
  const y = (v: number) => (b.dotsHigh - 1) * (1 - clamp((v - lo) / (hi - lo)));
  for (const s of series) {
    const n = s.values.length;
    if (!n) continue;
    const x = (i: number) => (n === 1 ? 0 : (i * (b.dotsWide - 1)) / (n - 1));
    if (n === 1) b.set(0, y(s.values[0]!), { tone: s.tone });
    for (let i = 1; i < n; i++) b.line(x(i - 1), y(s.values[i - 1]!), x(i), y(s.values[i]!), { tone: s.tone });
  }
  return b.grid();
}

// A gauge: an arc over 240°, open at the bottom, its track dim and filled clockwise to `value` (0-1), with `label`
// (the percentage when absent) in the middle. Sized to the box: as big a circle as fits.
export function gauge(value: number, width: number, height: number, tone: Tone, label?: string): Grid {
  const b = new Braille(width, height);
  // the arc reaches r above its centre and r·sin 30° = r/2 below: 1.5r tall, centred in the box that way
  const cx = (b.dotsWide - 1) / 2;
  const r = Math.max(1, Math.min(cx, (b.dotsHigh - 1) / 1.5)), thick = Math.max(1, Math.round(r / 5));
  const cy = r + (b.dotsHigh - 1 - 1.5 * r) / 2;
  const from = (210 * Math.PI) / 180, sweep = (240 * Math.PI) / 180; // from lower left, clockwise over the top
  const filled = clamp(value) * sweep;
  const steps = Math.ceil(sweep * r * 2);
  // the track first, then the fill over it: a cell both reach is the fill's colour
  for (const [upto, ink] of [[sweep, { tone: "track" } as Ink], [filled, { tone } as Ink]] as const) {
    for (let i = 0; i <= steps; i++) {
      const a = (i / steps) * sweep;
      if (a > upto) break;
      const t = from - a;
      for (let k = 0; k < thick; k++) b.set(cx + (r - k) * Math.cos(t), cy - (r - k) * Math.sin(t), ink);
    }
  }
  const grid = b.grid();
  const text = label ?? `${Math.round(clamp(value) * 100)}%`;
  const row = Math.round(cy / 4), col = Math.max(0, Math.floor((width - text.length) / 2));
  [...text].slice(0, width).forEach((ch, i) => grid[row] && col + i < width && (grid[row]![col + i] = { ch, fg: { tone } }));
  return grid;
}
