// The modal machinery: a centred panel over a dimmed screen, open() which shows it, routes keys to it and resolves
// with its result (null when dismissed), and the parts every modal is drawn from: a header, a search field, rows,
// buttons and a footer of key hints — so they all look and behave alike.
import { BoxRenderable, RGBA, StyledText, TextAttributes, TextRenderable, fg, type KeyEvent, type MouseEvent, type TextChunk } from "@opentui/core";
import type { Rect } from "../../core/layout";
import type { App } from "../context";
import { fit, floating, mix } from "../design";
import { render } from "../render";

// how each open frame re-lays itself out on terminal resize, and where it is now (a box's own width and height
// are its last layout's, 0 until it has been drawn)
export const frameLayouts = new WeakMap<BoxRenderable, () => void>();
const rects = new WeakMap<BoxRenderable, Rect>();

// A panel `w` wide and `h` tall (both clamped to the terminal), centred unless placed at (x, y).
export function frame(app: App, h: number, w = 64, x?: number, y?: number) {
  const { r } = app;
  const box = new BoxRenderable(r, { position: "absolute", zIndex: 100, flexDirection: "column", overflow: "hidden", border: true, borderStyle: "rounded", paddingLeft: 1, paddingRight: 1 });
  const place = () => {
    const { th } = app;
    const rect = floating(r.width, r.height, w, h, x, y);
    rects.set(box, rect);
    Object.assign(box, { left: rect.x, top: rect.y, width: rect.w, height: rect.h, borderColor: mix(th.border, th.focus, 0.45), backgroundColor: th.bar });
  };
  place();
  frameLayouts.set(box, place);
  r.root.add(box);
  return box;
}

// what's inside a frame's border and padding
export const innerWidth = (box: BoxRenderable) => Math.max(1, rects.get(box)!.w - 4);
export const innerHeight = (box: BoxRenderable) => Math.max(1, rects.get(box)!.h - 2);

export function clear(box: BoxRenderable) {
  for (const child of box.getChildren()) child.destroyRecursively();
}

type TextOptions = { bg?: string; attributes?: number; width?: number; height?: number; run?: () => void; context?: (e: MouseEvent) => void; hover?: [string, string] };

// One line of text in `parent`; with `run` it's clickable, with `hover` it takes those colours [fg, bg] under the pointer.
export function text(app: App, parent: BoxRenderable, content: string | StyledText, color: string, o: TextOptions = {}) {
  const width = o.width ?? (typeof content === "string" ? Bun.stringWidth(content) : undefined);
  const node: TextRenderable = new TextRenderable(app.r, {
    content, fg: color, height: o.height ?? 1, flexShrink: 0, ...(width !== undefined && { width }), ...(o.bg && { bg: o.bg }), attributes: o.attributes ?? 0,
    ...(o.run || o.context ? {
      onMouseDown: (e: MouseEvent) => {
        e.stopPropagation();
        if (e.button === 0) o.run?.();
        else if (e.button === 2) o.context?.(e);
      },
    } : {}),
    ...(o.hover ? {
      onMouseOver: () => { node.fg = o.hover![0]; node.bg = o.hover![1]; },
      onMouseOut: () => { node.fg = color; node.bg = o.bg ?? "transparent"; },
    } : {}),
  });
  if (o.run) app.clickable.add(node);
  parent.add(node);
  return node;
}

export function row(app: App, parent: BoxRenderable, o: { bg?: string; height?: number } = {}) {
  const node = new BoxRenderable(app.r, { width: "100%", height: o.height ?? 1, flexShrink: 0, flexDirection: "row", ...(o.bg && { backgroundColor: o.bg }) });
  parent.add(node);
  return node;
}

export const spacer = (app: App, parent: BoxRenderable) => parent.add(new BoxRenderable(app.r, { flexGrow: 1, height: 1, minWidth: 0 }));
export const blank = (app: App, parent: BoxRenderable) => row(app, parent);

// The title in bold, and on the right something about what's below (a count, the prefix key) in the dim colour.
export function header(app: App, box: BoxRenderable, title: string, meta = "") {
  const { th } = app;
  const width = innerWidth(box);
  const line = row(app, box);
  const right = fit(meta, Math.max(0, Math.floor(width / 2)));
  text(app, line, fit(title, Math.max(1, width - Bun.stringWidth(right) - 1)), th.fg, { attributes: TextAttributes.BOLD });
  spacer(app, line);
  if (right) text(app, line, right, th.dim);
}

// The search field: what's typed so far and a caret, or the placeholder, on an inset strip.
export function searchField(app: App, box: BoxRenderable, query: string, placeholder: string) {
  const { th } = app;
  const width = innerWidth(box);
  const line = row(app, box, { bg: th.bg });
  text(app, line, " ⌕ ", query ? th.accent : th.dim, { bg: th.bg });
  const room = Math.max(1, width - 4);
  if (query) {
    const shown = Bun.stringWidth(query) > room - 1 ? "…" + [...query].slice(-(room - 2)).join("") : query;
    text(app, line, shown, th.fg, { bg: th.bg });
    text(app, line, "▏", th.focus, { bg: th.bg });
  } else {
    text(app, line, "▏", th.focus, { bg: th.bg });
    text(app, line, fit(placeholder, room - 1), th.dim, { bg: th.bg });
  }
}

// Key hints along the bottom: each key in the text colour, what it does dim.
export function footer(app: App, box: BoxRenderable, hints: [string, string][], right = "") {
  const { th } = app;
  const width = innerWidth(box);
  const line = row(app, box);
  let used = Bun.stringWidth(right) + 1;
  for (const [key, what] of hints) {
    const w = Bun.stringWidth(key) + Bun.stringWidth(what) + 3;
    if (used + w > width) break;
    used += w;
    text(app, line, new StyledText([fg(th.fg)(key), fg(th.dim)(` ${what}   `)]), th.dim, { width: w });
  }
  spacer(app, line);
  if (right) text(app, line, right, th.dim);
}

// A button: filled in `tone` when it's the primary (or focused) one, else a quiet outline of the text colour.
export function button(app: App, parent: BoxRenderable, label: string, key: string, tone: string, filled: boolean, run: () => void) {
  const { th } = app;
  const [fgc, bgc] = filled ? [th.bg, tone] : [tone, mix(th.bar, th.fg, 0.08)];
  const name = ` ${label}  `; // no colour of its own: it takes the node's, which hover changes
  const chunks: TextChunk[] = [{ __isChunk: true, text: name, attributes: filled ? TextAttributes.BOLD : 0 }, fg(filled ? mix(tone, th.bg, 0.5) : th.dim)(`${key} `)];
  return text(app, parent, new StyledText(chunks), fgc, {
    bg: bgc, run, width: Bun.stringWidth(name + key) + 1, hover: [filled ? th.bg : th.fg, filled ? mix(tone, th.fg, 0.2) : mix(th.bar, th.fg, 0.16)],
  });
}

// Search: every word of the query somewhere in the text, case aside. Ranks what starts with the query first, then
// what has it in its name, then what only has it in its description.
export function matches<T>(items: T[], query: string, name: (t: T) => string, rest: (t: T) => string = () => ""): T[] {
  const q = query.trim().toLowerCase();
  if (!q) return items;
  const words = q.split(/\s+/);
  const rank = (t: T) => {
    const n = name(t).toLowerCase();
    if (!words.every((w) => `${n} ${rest(t).toLowerCase()}`.includes(w))) return -1;
    return n.startsWith(q) ? 0 : words.every((w) => n.includes(w)) ? 1 : 2;
  };
  return items.map((t, i) => ({ t, i, r: rank(t) })).filter((x) => x.r >= 0).sort((a, b) => a.r - b.r || a.i - b.i).map((x) => x.t);
}

// `s` with the query's words picked out in `hit`: the chunks of a line, the rest in `color`.
export function highlight(s: string, query: string, color: string, hit: string, attributes = 0): TextChunk[] {
  const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  const on = new Array<boolean>(s.length).fill(false);
  const lower = s.toLowerCase();
  for (const w of words) {
    const at = lower.indexOf(w);
    if (at >= 0) on.fill(true, at, at + w.length);
  }
  const chunks: TextChunk[] = [];
  for (let i = 0; i < s.length;) {
    let j = i;
    while (j < s.length && on[j] === on[i]) j++;
    const chunk = fg(on[i] ? hit : color)(s.slice(i, j));
    chunks.push(attributes || on[i] ? { ...chunk, attributes: attributes | (on[i] ? TextAttributes.BOLD : 0) } : chunk);
    i = j;
  }
  return chunks;
}

// Typing into a search: printable keys add, backspace takes one back, ctrl+w a word, ctrl+u everything.
// Returns the new query, or undefined when the key isn't one for the search.
export function typed(query: string, k: KeyEvent): string | undefined {
  if (k.name === "backspace") return query.slice(0, -1);
  if (k.ctrl && k.name === "u") return "";
  if (k.ctrl && k.name === "w") return query.replace(/\S*\s*$/, "");
  if (!k.ctrl && !k.meta && k.sequence.length === 1 && k.sequence >= " " && k.sequence !== "\x7f") return query + k.sequence;
  return undefined;
}

// A scrollbar for `total` rows, `shown` of them from `first`, `height` cells tall: the thumb's cells.
export function thumb(total: number, shown: number, first: number, height: number) {
  if (total <= shown) return undefined;
  const size = Math.max(1, Math.round((shown / total) * height));
  const top = Math.round((first / Math.max(1, total - shown)) * (height - size));
  return { top, size };
}

// Only a pointer that moved selects: a repaint rebuilds the rows, and the new row under a resting pointer gets a
// fresh mouseover that would undo the arrow key that caused the repaint.
export function pointerMoved() {
  let at = "";
  return (e: MouseEvent) => {
    const here = `${e.x},${e.y}`;
    if (here === at) return false;
    at = here;
    return true;
  };
}

// onClose runs before the closing redraw: state restored after it (a theme preview, say) can miss the
// frame OpenTUI is already drawing and stay stale on screen until something else redraws.
export function open<T>(app: App, box: BoxRenderable, setup: (done: (v: T | null) => void) => ((k: KeyEvent) => boolean) | void, onClose?: () => void): Promise<T | null> {
  app.modal?.close(null);
  return new Promise((resolve) => {
    // the screen behind, dimmed; a click on it dismisses
    const veil = new BoxRenderable(app.r, { position: "absolute", left: 0, top: 0, width: "100%", height: "100%", zIndex: 90, backgroundColor: RGBA.fromInts(0, 0, 0, 140) });
    app.r.root.add(veil);
    const done = (v: T | null) => {
      if (app.modal?.close !== done) return;
      app.modal = undefined;
      onClose?.();
      veil.destroyRecursively();
      box.destroyRecursively();
      resolve(v);
      render(app);
    };
    veil.onMouseDown = (e) => { e.preventDefault(); e.stopPropagation(); done(null); };
    veil.onMouse = (e) => { e.preventDefault(); e.stopPropagation(); };
    box.onMouse = (e) => e.stopPropagation();
    app.modal = { close: done, resize: () => frameLayouts.get(box)?.() };
    app.modal.keys = setup(done) ?? undefined;
    render(app);
  });
}
