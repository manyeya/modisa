// A plugin view's elements as OpenTUI renderables, drawn in the user's theme. An update reuses what hasn't changed:
// boxes and scroll areas are kept (rebuilt around their children when their own props change), and an element whose
// props are the same keeps its renderable, so what's typed in it, chosen in it and scrolled to stays. A changed element
// is built again, keeping the user's text, choice and scroll when the plugin didn't change those itself.
import {
  ASCIIFontRenderable, BoxRenderable, CodeRenderable, DiffRenderable, ImageRenderable, InputRenderable, InputRenderableEvents, LineNumberRenderable,
  MarkdownRenderable, RGBA, Renderable, ScrollBoxRenderable, SelectRenderable, SelectRenderableEvents, StyledText, SyntaxStyle, TabSelectRenderable,
  TabSelectRenderableEvents, TextRenderable, TextTableRenderable, TextareaRenderable, bold, dim, fg, italic, strikethrough, underline,
  type OptimizedBuffer, type RenderContext, type TextChunk,
} from "@opentui/core";
import { RASTER, TONES, type Tone, type ViewInline, type ViewLayout, type ViewNode } from "../../protocol/types";
import type { Theme } from "../../config/themes";
import { agentMark, mix } from "../design";
import type { App } from "../context";
import { columns, gauge, heatmap, lines, progress, type Grid, type Ink } from "./charts";

// What a view element tells its plugin: run `action` with the element's params and what it holds.
export type Fire = (action: string | undefined, params: Record<string, unknown> | undefined, ui: { key?: string; value?: string; index?: number }) => void;
// What Tab moves between: a button (pressed with Enter or Space), a field or list (OpenTUI's own focus, so it gets
// the keys), or something to scroll with the arrows.
// `keys`: an element that takes some keys itself while it has focus (a diff's line cursor), saying which it took.
export type Focusable = { key: string; kind: "button" | "field" | "list" | "scroll"; r: Renderable; press?: () => void; scroll?: (rows: number) => void; mark?: (on: boolean) => void; keys?: (name: string) => boolean };
export type Mounted = { node: ViewNode; sig: string; r: Renderable; kids: Mounted[]; focus?: Omit<Focusable, "key">; dispose?: () => void; blit?: (cells: string) => void };
export type Build = { app: App; fire: Fire; focusables: Focusable[]; rasters: Map<string, Mounted>; path: string };
type Dir = "row" | "column"; // the parent's direction: what filling it means

// Something a line tall that fills its parent: across a row it takes the room left; down a column it's stretched wide.
const line = (dir: Dir): Size => (dir === "row" ? { flexGrow: 1, flexShrink: 1, minWidth: 4 } : {});
// Something that takes the room left in its parent, whichever way that runs.
const area = (min = 3): Size => ({ flexGrow: 1, flexShrink: 1, minHeight: min, minWidth: 4 });

const color = (th: Theme, tone: Tone | undefined, fallback = th.fg) => (tone ? ({ fg: th.fg, dim: th.dim, accent: th.accent, warn: th.warn, working: th.working, blocked: th.blocked, done: th.done, idle: th.idle })[tone] : fallback);
const track = (th: Theme) => mix(th.bg, th.dim, 0.3);

// One syntax style per theme, for code, diffs and Markdown.
const styles = new Map<Theme, SyntaxStyle>();
export function syntax(th: Theme) {
  let s = styles.get(th);
  if (s) return s;
  s = SyntaxStyle.fromStyles({
    default: { fg: th.fg }, keyword: { fg: th.accent, bold: true }, "keyword.import": { fg: th.accent, bold: true }, string: { fg: th.done }, comment: { fg: th.dim, italic: true },
    number: { fg: th.warn }, boolean: { fg: th.warn }, constant: { fg: th.warn }, function: { fg: th.working }, "function.method": { fg: th.working }, type: { fg: th.idle },
    operator: { fg: th.dim }, punctuation: { fg: th.dim }, property: { fg: th.fg }, variable: { fg: th.fg },
    "markup.heading": { fg: th.accent, bold: true }, "markup.strong": { fg: th.fg, bold: true }, "markup.bold": { fg: th.fg, bold: true }, "markup.italic": { fg: th.fg, italic: true },
    "markup.list": { fg: th.accent }, "markup.quote": { fg: th.dim, italic: true }, "markup.raw": { fg: th.done }, "markup.raw.block": { fg: th.done },
    "markup.link": { fg: th.focus, underline: true }, "markup.link.url": { fg: th.focus, underline: true },
  });
  styles.set(th, s);
  return s;
}

// ---------- layout ----------

const ALIGN = { start: "flex-start", center: "center", end: "flex-end", stretch: "stretch" } as const;
const JUSTIFY = { start: "flex-start", center: "center", end: "flex-end", between: "space-between", around: "space-around", evenly: "space-evenly" } as const;
// An element's own layout over its kind's defaults (`size`: cells, or flex for one that fills its parent).
type Size = { width?: number | `${number}%`; height?: number | `${number}%`; flexGrow?: number; flexShrink?: number; minWidth?: number; minHeight?: number };
const layout = (n: ViewLayout, size: Size = {}) => {
  const o: Record<string, unknown> = { ...size };
  if (n.width !== undefined) o.width = n.width;
  if (n.height !== undefined) o.height = n.height;
  if ((n.width !== undefined || n.height !== undefined) && size.flexGrow) delete o.flexGrow; // a size it was given is the size it gets
  for (const [from, to] of [["minWidth", "minWidth"], ["maxWidth", "maxWidth"], ["minHeight", "minHeight"], ["maxHeight", "maxHeight"], ["grow", "flexGrow"], ["shrink", "flexShrink"]] as const) if (n[from] !== undefined) o[to] = n[from];
  return o;
};

// ---------- text ----------

type Look = { tone?: Tone; bold?: boolean; italic?: boolean; underline?: boolean; dim?: boolean; strike?: boolean };
function chunk(th: Theme, text: string, look: Look): TextChunk {
  let c = fg(color(th, look.tone))(text);
  if (look.bold) c = bold(c);
  if (look.italic) c = italic(c);
  if (look.underline) c = underline(c);
  if (look.dim) c = dim(c);
  if (look.strike) c = strikethrough(c);
  return c;
}
function chunks(app: App, xs: ViewInline[] | undefined, look: Look): TextChunk[] {
  return (xs ?? []).flatMap((x): TextChunk[] => {
    if (typeof x === "string") return [chunk(app.th, x, look)];
    if (x.type === "icon") {
      const { glyph, color: c, cells } = agentMark(app.th, x.agent, app.logos);
      return [fg(c)(glyph + " ".repeat(Math.max(0, cells - 2)))];
    }
    const { type: _t, children, ...own } = x;
    return chunks(app, children, { ...look, ...Object.fromEntries(Object.entries(own).filter(([, v]) => v !== undefined)) });
  });
}

// ---------- cells: charts and rasters ----------

// Draws a Grid each frame, sized by the layout: charts redraw to fit whatever room they get.
class CellsRenderable extends Renderable {
  constructor(ctx: RenderContext, options: Record<string, unknown>, public draw: (w: number, h: number) => Grid, private paint: (ink: Ink | undefined, fallback: RGBA) => RGBA, private base: RGBA) {
    super(ctx, options);
  }
  private cache?: { w: number; h: number; grid: Grid };
  invalidate() {
    this.cache = undefined;
    this.requestRender();
  }
  protected override renderSelf(buffer: OptimizedBuffer) {
    const w = this.width, h = this.height;
    if (w <= 0 || h <= 0) return;
    if (!this.cache || this.cache.w !== w || this.cache.h !== h) this.cache = { w, h, grid: this.draw(w, h) };
    this.cache.grid.forEach((row, y) => row.forEach((c, x) => x < w && y < h && buffer.setCell(this.x + x, this.y + y, c.ch, this.paint(c.fg, this.base), this.paint(c.bg, this.base))));
  }
}

// A Raster's cells decoded: what plugins paint themselves.
function rasterGrid(th: Theme, cells: string, columns: number, rows: number): Grid {
  const bytes = Buffer.from(cells, "base64");
  const words = new Uint32Array(bytes.buffer, bytes.byteOffset, Math.floor(bytes.length / 4));
  const ink = (v: number): Ink | undefined => {
    if (v === RASTER.DEFAULT) return undefined;
    if (v & RASTER.TONE) return { tone: TONES[v & 0xff] ?? "fg" };
    return { tone: "fg", rgb: `#${(v & 0xffffff).toString(16).padStart(6, "0")}` };
  };
  const grid: Grid = [];
  for (let y = 0; y < rows; y++) {
    const row = [];
    for (let x = 0; x < columns; x++) {
      const i = (y * columns + x) * 3;
      const cp = words[i] ?? 0x20;
      row.push({ ch: cp >= 0x20 ? String.fromCodePoint(cp) : " ", fg: ink(words[i + 1] ?? RASTER.DEFAULT), bg: ink(words[i + 2] ?? RASTER.DEFAULT) });
    }
    grid.push(row);
  }
  return grid;
}

const paintWith = (th: Theme) => (ink: Ink | undefined, fallback: RGBA) => {
  if (!ink) return fallback;
  if (ink.rgb) return RGBA.fromHex(ink.rgb);
  if (ink.tone === "track") return RGBA.fromHex(track(th));
  const c = color(th, ink.tone);
  return RGBA.fromHex(ink.mix === undefined ? c : mix(track(th), c, ink.mix));
};

// ---------- building ----------

const SCROLLING = new Set(["scroll", "code", "diff", "markdown"]);
const sig = (n: ViewNode) => JSON.stringify(n, (k, v) => (k === "children" && (n.type === "box" || n.type === "scroll") ? undefined : v));
const SPIN = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏";

// A diff's body lines (+, -, context) in order, as the unified view draws them: row i is body line i.
export function diffBody(diff: string) {
  const out: string[] = [];
  let inHunk = false;
  for (const line of diff.split("\n")) {
    if (line.startsWith("@@")) inHunk = true;
    else if (line.startsWith("diff --git ")) inHunk = false;
    else if (inHunk && (line[0] === "+" || line[0] === "-" || line[0] === " ")) out.push(line);
  }
  return out;
}

// ponytail: DiffRenderable scrolls through its inner code panes (leftCodeRenderable, rightCodeRenderable), which
// OpenTUI doesn't publish; use its own API when it has one.
const diffScroll = (d: DiffRenderable) => {
  const panes = [(d as any).leftCodeRenderable, (d as any).rightCodeRenderable].filter(Boolean) as { scrollY: number }[];
  return { get: () => panes[0]?.scrollY ?? 0, set: (y: number) => panes.forEach((p) => (p.scrollY = Math.max(0, y))) };
};

function create(b: Build, n: ViewNode, old: Mounted | undefined, dir: Dir): Mounted {
  const { app } = b;
  const r = app.r, th = app.th;
  const same = old?.node.type === n.type ? (old!.node as any) : undefined; // carry the user's state over from it
  const m: Mounted = { node: n, sig: sig(n), r: undefined as unknown as Renderable, kids: [] };
  const fire = (action: string | undefined, ui: { value?: string; index?: number } = {}) => b.fire(action, (n as { params?: Record<string, unknown> }).params, { ...(n.key && { key: n.key }), ...ui });
  switch (n.type) {
    case "box":
      m.r = new BoxRenderable(r, {
        ...layout(n), flexDirection: n.direction ?? "column", ...(n.gap !== undefined && { gap: n.gap }), ...(n.padding !== undefined && { padding: n.padding }),
        ...(n.paddingX !== undefined && { paddingX: n.paddingX }), ...(n.paddingY !== undefined && { paddingY: n.paddingY }), ...(n.align && { alignItems: ALIGN[n.align] }),
        ...(n.justify && { justifyContent: JUSTIFY[n.justify] }), ...(n.wrap && { flexWrap: "wrap" }),
        ...(n.border && { border: true, borderStyle: typeof n.border === "string" ? n.border : "rounded", borderColor: color(th, n.tone, th.border) }),
        ...(n.border && n.title && { title: ` ${n.title} `, titleColor: color(th, n.tone, th.dim) }), ...(n.bg && { backgroundColor: mix(th.bg, color(th, n.bg), 0.15) }),
      });
      break;
    case "scroll": {
      const s = new ScrollBoxRenderable(r, { ...layout(n, area()), scrollY: true, ...(n.sticky && { stickyScroll: true, stickyStart: n.sticky }) });
      if (old?.r instanceof ScrollBoxRenderable) s.scrollTop = old.r.scrollTop;
      m.r = s;
      m.focus = { kind: "scroll", r: s, scroll: (d) => s.scrollBy(d) };
      break;
    }
    case "text":
      m.r = new TextRenderable(r, { ...layout(n), content: new StyledText(chunks(app, n.children, n)), wrapMode: n.wrap ?? "word" });
      break;
    case "markdown":
      m.r = new MarkdownRenderable(r, { ...layout(n), content: n.content, syntaxStyle: syntax(th), fg: th.fg, conceal: true });
      break;
    case "code": {
      const code = new CodeRenderable(r, { ...layout(n), content: n.content, syntaxStyle: syntax(th), fg: th.fg, wrapMode: "none", ...(n.filetype && { filetype: n.filetype }) });
      m.r = n.lineNumbers ? new LineNumberRenderable(r, { ...layout(n), target: code, fg: th.dim }) : code;
      if (same) (code as any).scrollY = (old!.r as any).scrollY ?? 0;
      m.focus = { kind: "scroll", r: m.r, scroll: (d) => ((code as any).scrollY = Math.max(0, ((code as any).scrollY ?? 0) + d)) };
      break;
    }
    case "diff": {
      const added = mix(th.bg, th.done, 0.16), removed = mix(th.bg, th.blocked, 0.16);
      const d = new DiffRenderable(r, {
        ...layout(n, area()), diff: n.diff, view: n.cursor ? "unified" : (n.view ?? "unified"), syntaxStyle: syntax(th), fg: th.fg, showLineNumbers: n.lineNumbers ?? true, lineNumberFg: th.dim,
        addedBg: added, removedBg: removed, addedSignColor: th.done, removedSignColor: th.blocked, ...(n.filetype && { filetype: n.filetype }), ...(n.cursor && { wrapMode: "none" }),
      });
      const scroll = diffScroll(d);
      const was = same?.diff === n.diff ? (old as Mounted & { cursor?: number; top?: number }) : undefined;
      m.r = d;
      m.focus = { kind: "scroll", r: d, scroll: (delta) => scroll.set(scroll.get() + delta) };
      if (!n.cursor) {
        setTimeout(() => was?.top !== undefined && scroll.set(was.top), 0); // where the one it replaces was scrolled to (known once that's unmounted)
        m.dispose = () => ((m as Mounted & { top?: number }).top = scroll.get());
        break;
      }
      // the line cursor: j/k (and the arrows, PageUp/PageDown, g/G) move it over the body lines, Enter runs `action`
      // with the line (`index` into the body, `value` the line as the diff has it), moving runs `change`
      const body = diffBody(n.diff);
      const marks = new Set(n.marks ?? []);
      const base = (i: number) => ({ gutter: RGBA.fromValues(0, 0, 0, 0), content: RGBA.fromHex(body[i]?.[0] === "+" ? added : body[i]?.[0] === "-" ? removed : th.bg) });
      const lit = (i: number, c: string) => ({ gutter: RGBA.fromHex(c), content: RGBA.fromHex(c) });
      const cm = m as Mounted & { cursor?: number; top?: number };
      cm.cursor = Math.min(was?.cursor ?? 0, Math.max(0, body.length - 1));
      const paint = () => {
        for (const i of marks) d.setLineColor(i, lit(i, mix(th.bg, th.warn, 0.22)));
        d.setLineColor(cm.cursor!, lit(cm.cursor!, mix(th.bg, th.focus, 0.32)));
      };
      const show = () => {
        const h = Math.max(1, d.height), y = scroll.get();
        if (cm.cursor! < y) scroll.set(cm.cursor!);
        else if (cm.cursor! >= y + h) scroll.set(cm.cursor! - h + 1);
      };
      const move = (to: number) => {
        const next = Math.max(0, Math.min(body.length - 1, to));
        if (next === cm.cursor) return;
        d.setLineColor(cm.cursor!, marks.has(cm.cursor!) ? lit(cm.cursor!, mix(th.bg, th.warn, 0.22)) : base(cm.cursor!));
        cm.cursor = next;
        paint();
        show();
        fire(n.change, { index: next, value: body[next] });
      };
      setTimeout(() => (paint(), was?.top !== undefined ? scroll.set(was.top) : show()), 0); // once the diff is laid out
      m.dispose = () => (cm.top = scroll.get());
      const STEP: Record<string, number> = { j: 1, down: 1, k: -1, up: -1, pagedown: 10, pageup: -10, "C-d": 10, "C-u": -10 };
      m.focus.keys = (name) => {
        if (STEP[name]) return (move(cm.cursor! + STEP[name]!), true);
        if (name === "g" || name === "home") return (move(0), true);
        if (name === "G" || name === "end") return (move(body.length - 1), true);
        if (name === "enter" && body.length) return (fire(n.action, { index: cm.cursor!, value: body[cm.cursor!] }), true);
        return false;
      };
      break;
    }
    case "table":
      m.r = new TextTableRenderable(r, {
        ...layout(n), content: n.rows.map((row, i) => row.map((c) => chunks(app, typeof c === "string" ? [c] : c, { bold: !!n.header && i === 0 }))),
        border: n.border ?? true, outerBorder: n.border ?? true, showBorders: n.border ?? true, borderColor: th.border, borderStyle: "rounded", fg: th.fg, wrapMode: "word", cellPaddingX: 1,
      });
      break;
    case "bigtext":
      m.r = new ASCIIFontRenderable(r, { ...layout(n), text: n.text, font: n.font ?? "tiny", color: color(th, n.tone, th.accent) });
      break;
    case "progress":
    case "sparkline":
    case "chart":
    case "gauge":
    case "heatmap": {
      const tone = ("tone" in n && n.tone) || "accent";
      const draw: (w: number, h: number) => Grid =
        n.type === "progress" ? (w) => [progress(n.value, w, tone)]
        : n.type === "sparkline" ? (w, h) => columns(n.values, w, h, tone, n.min, n.max)
        : n.type === "chart" ? (w, h) => lines(n.series.map((s) => ({ values: s.values, tone: s.tone ?? "accent" })), w, h, n.min, n.max)
        : n.type === "gauge" ? (w, h) => gauge(n.value, w, h, tone, n.label)
        : () => heatmap(n.values, tone, n.min, n.max);
      const size: Size =
        n.type === "progress" || n.type === "sparkline" ? { ...line(dir), height: 1 }
        : n.type === "chart" ? { ...line(dir), height: 6 }
        : n.type === "gauge" ? { width: 14, height: 5, flexShrink: 0 }
        : { width: Math.max(0, ...n.values.map((v) => v.length)), height: Math.ceil(n.values.length / 2), flexShrink: 0 };
      m.r = new CellsRenderable(r, layout(n, size), draw, paintWith(th), RGBA.fromHex(th.bg));
      break;
    }
    case "raster": {
      let grid = rasterGrid(th, n.cells, n.columns, n.rows);
      const cells = new CellsRenderable(r, { ...layout(n), width: n.columns, height: n.rows, flexShrink: 0 }, () => grid, paintWith(th), RGBA.fromHex(th.bg));
      m.r = cells;
      m.blit = (data) => {
        grid = rasterGrid(th, data, n.columns, n.rows);
        cells.invalidate();
      };
      break;
    }
    case "image": {
      const box = new BoxRenderable(r, { ...layout(n, { width: 40, height: 12 }), flexShrink: 0 });
      const img = new ImageRenderable(r, { width: "100%", height: "100%", source: new Uint8Array(Buffer.from(n.png, "base64")), fit: n.fit ?? "fit", onError: () => {
        box.remove(img);
        box.add(new TextRenderable(r, { content: n.alt ?? "[image]", fg: th.dim }));
      } });
      box.add(img);
      m.r = box;
      break;
    }
    case "spinner": {
      let i = 0;
      const t = new TextRenderable(r, { ...layout(n), content: "" });
      const tick = () => (t.content = new StyledText([fg(color(th, n.tone, th.working))(SPIN[i++ % SPIN.length]!), ...(n.label ? [fg(th.dim)(` ${n.label}`)] : [])]));
      tick();
      const timer = setInterval(tick, 80);
      m.dispose = () => clearInterval(timer);
      m.r = t;
      break;
    }
    case "button": {
      const tone = color(th, n.tone, th.accent);
      const box = new BoxRenderable(r, { ...layout(n), height: 1, flexShrink: 0, flexDirection: "row" });
      const label = new TextRenderable(r, { content: ` ${n.label} ` });
      box.add(label);
      const mark = (on: boolean) => ((label.content = new StyledText([on ? bold(fg(th.bg)(` ${n.label} `)) : fg(tone)(` ${n.label} `)])), (box.backgroundColor = on ? tone : mix(th.bg, tone, 0.15)));
      mark(false);
      const press = () => fire(n.action);
      box.onMouseDown = (e) => (e.preventDefault(), press());
      app.clickable.add(box);
      m.focus = { kind: "button", r: box, press, mark };
      m.r = box;
      break;
    }
    case "input": {
      const typed = same && same.value === n.value ? (old!.r as InputRenderable).value : n.value;
      const input = new InputRenderable(r, {
        ...layout(n, line(dir)), value: typed ?? "", placeholder: n.placeholder ?? "", ...(n.maxLength && { maxLength: n.maxLength }),
        backgroundColor: th.bar, focusedBackgroundColor: mix(th.bar, th.focus, 0.12), textColor: th.fg, focusedTextColor: th.fg, cursorColor: th.focus, placeholderColor: th.dim,
      });
      input.on(InputRenderableEvents.ENTER, (value: string) => fire(n.action, { value }));
      m.focus = { kind: "field", r: input };
      m.r = input;
      break;
    }
    case "textarea": {
      const typed = same && same.value === n.value ? (old!.r as TextareaRenderable).plainText : n.value;
      const area: TextareaRenderable = new TextareaRenderable(r, {
        ...layout(n, { ...line(dir), height: 5 }), initialValue: typed ?? "", placeholder: n.placeholder ?? null, placeholderColor: th.dim,
        backgroundColor: th.bar, focusedBackgroundColor: mix(th.bar, th.focus, 0.12), textColor: th.fg, focusedTextColor: th.fg, cursorColor: th.focus,
        keyBindings: [{ name: "return", ctrl: true, action: "submit" }, { name: "s", ctrl: true, action: "submit" }],
        onSubmit: () => fire(n.action, { value: area.plainText }),
      });
      m.focus = { kind: "field", r: area };
      m.r = area;
      break;
    }
    case "select": {
      const at = same && same.selected === n.selected ? (old!.r as SelectRenderable).getSelectedIndex() : (n.selected ?? 0);
      const described = n.options.some((o) => o.description);
      const sel = new SelectRenderable(r, {
        ...layout(n, { ...line(dir), height: Math.max(1, Math.min(n.options.length, 12)) * (described ? 2 : 1) }),
        options: n.options.map((o) => ({ name: o.name, description: o.description ?? "", value: o.value ?? o.name })), selectedIndex: Math.min(at, Math.max(0, n.options.length - 1)),
        backgroundColor: th.bg, textColor: th.fg, focusedBackgroundColor: th.bg, focusedTextColor: th.fg, selectedBackgroundColor: mix(th.bg, th.focus, 0.22), selectedTextColor: th.fg,
        descriptionColor: th.dim, selectedDescriptionColor: th.dim, showDescription: described, showScrollIndicator: n.options.length > 12, wrapSelection: false,
      });
      const value = (i: number) => ({ index: i, value: n.options[i]?.value ?? n.options[i]?.name });
      sel.on(SelectRenderableEvents.ITEM_SELECTED, (i: number) => fire(n.action, value(i)));
      sel.on(SelectRenderableEvents.SELECTION_CHANGED, (i: number) => fire(n.change, value(i)));
      m.focus = { kind: "list", r: sel };
      m.r = sel;
      break;
    }
    case "tabs": {
      const at = same && same.selected === n.selected ? (old!.r as TabSelectRenderable).getSelectedIndex() : (n.selected ?? 0);
      const tabs = new TabSelectRenderable(r, {
        ...layout(n, line(dir)), height: 1, options: n.options.map((o) => ({ name: o.name, description: o.description ?? "", value: o.value ?? o.name })),
        tabWidth: Math.min(30, Math.max(8, ...n.options.map((o) => Bun.stringWidth(o.name) + 4))), showDescription: false, showUnderline: false, showScrollArrows: true,
        backgroundColor: th.bg, textColor: th.dim, focusedBackgroundColor: th.bg, focusedTextColor: th.fg, selectedBackgroundColor: mix(th.bg, th.focus, 0.22), selectedTextColor: th.fg,
      });
      tabs.setSelectedIndex(Math.min(at, Math.max(0, n.options.length - 1)));
      tabs.on(TabSelectRenderableEvents.SELECTION_CHANGED, (i: number) => fire(n.action, { index: i, value: n.options[i]?.value ?? n.options[i]?.name }));
      m.focus = { kind: "list", r: tabs };
      m.r = tabs;
      break;
    }
  }
  if (SCROLLING.has(n.type) && !m.focus) m.focus = { kind: "scroll", r: m.r };
  return m;
}

// Mount `n` where `old` was: reuse it when it's the same element with the same props; containers keep their children
// either way.
export function mount(b: Build, n: ViewNode, old: Mounted | undefined, path: string, dir: Dir = "column"): Mounted {
  const key = n.key ? `#${n.key}` : path;
  const container = n.type === "box" || n.type === "scroll";
  const s = `${dir}|${sig(n)}`; // where it sits decides how it fills: moved into a row, it's laid out again
  let m: Mounted;
  if (old && old.node.type === n.type && old.sig === s) {
    m = old; // the same element: kept, with all the user did in it
    m.node = n;
  } else if (old && old.node.type === n.type && container) {
    m = create(b, n, old, dir); // its own props changed: a new box around the same children
    for (const k of old.kids) old.r.remove(k.r);
    old.r.destroy();
    m.kids = old.kids;
  } else {
    m = create(b, n, old, dir);
    if (old) unmount(old);
  }
  m.sig = s;
  // this pass's focus order (depth first, as drawn) and rasters by key
  if (m.focus) b.focusables.push({ ...m.focus, key });
  if (n.type === "raster") b.rasters.set(n.key, m);
  if (container) {
    const children = (n as { children?: ViewNode[] }).children ?? [];
    const byKey = new Map(m.kids.filter((k) => k.node.key).map((k) => [k.node.key!, k]));
    const used = new Set<Mounted>();
    const next = children.map((c, i) => {
      const prev = c.key ? byKey.get(c.key) : m.kids[i] && !m.kids[i]!.node.key ? m.kids[i] : undefined;
      const reuse = prev && !used.has(prev) ? prev : undefined;
      if (reuse) used.add(reuse);
      return mount(b, c, reuse, `${path}.${i}`, n.type === "box" && n.direction === "row" ? "row" : "column");
    });
    for (const k of m.kids) if (!used.has(k)) unmount(k);
    const same = next.length === m.kids.length && next.every((k, i) => k === m.kids[i]) && m.r.getChildren().length === next.length;
    if (!same) {
      for (const child of [...m.r.getChildren()]) m.r.remove(child);
      for (const k of next) m.r.add(k.r);
    }
    m.kids = next;
  }
  return m;
}


export function unmount(m: Mounted) {
  for (const k of m.kids) unmount(k);
  m.dispose?.();
  if (!m.r.isDestroyed) m.r.destroyRecursively();
}
