// What a plugin's view may hold, on top of the shapes the schema checks: how much of it (nodes, depth, bytes), that
// every action it names is one the plugin offered, its Rasters' and Images' bytes, and its text made safe to draw.
import { cleanText } from "../core/text";
import { fail } from "../protocol/conn";
import type { ViewInline, ViewKey, ViewNode } from "../protocol/types";

export const VIEW_LIMIT = { views: 4, sessionViews: 8, nodes: 5000, depth: 40, bytes: 2 * 1024 * 1024, label: 200, block: 512 * 1024, image: 4 * 1024 * 1024 };
export const BLITS = { burst: 120, perSecond: 60 }; // Raster repaints per run: animation, apart from ui.* updates

// Multi-line text (Markdown, code, a diff, a Text, what a field holds): no escape sequences or control characters, but
// newlines and tabs kept, and at most `chars` of it.
export const cleanBlock = (text: string, chars = VIEW_LIMIT.block) =>
  text
    .slice(0, chars * 2)
    .replace(/\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)?/g, "") // OSC
    .replace(/\x1b\[[0-9;?]*[ -/]*[@-~]/g, "") // CSI
    .replace(/[\x00-\x08\x0b-\x1f\x7f-\x9f]|\p{Cf}/gu, "")
    .slice(0, chars);

const PNG = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];

// A Raster's cells: exactly columns × rows [codePoint, fg, bg] triplets, each code point a printable width-1 character.
export function checkCells(cells: string, columns: number, rows: number) {
  const bytes = Buffer.from(cells, "base64");
  if (bytes.length !== columns * rows * 12) throw fail("invalid_params", `a ${columns}×${rows} raster has ${columns * rows * 12} bytes of cells, not ${bytes.length}`);
  const words = new Uint32Array(bytes.buffer, bytes.byteOffset, bytes.length / 4);
  for (let i = 0; i < words.length; i += 3) {
    const cp = words[i]!;
    const ok = cp >= 0x20 && !(cp >= 0x7f && cp <= 0x9f) && !(cp >= 0xd800 && cp <= 0xdfff) && cp <= 0xffff && Bun.stringWidth(String.fromCodePoint(cp)) === 1;
    if (!ok) throw fail("invalid_params", `raster cell ${i / 3}: U+${cp.toString(16).toUpperCase()} isn't a printable one-cell character`);
  }
}

// A view as the plugin sent it, checked and cleaned: what's drawn, and its Rasters' sizes by key (for blits).
export function checkView(root: ViewNode, keys: ViewKey[], close: string | undefined, offered: string[]) {
  const json = JSON.stringify(root);
  if (json.length > VIEW_LIMIT.bytes) throw fail("invalid_params", `a view is at most ${VIEW_LIMIT.bytes / 1024 / 1024} MB of JSON; this one is ${(json.length / 1024 / 1024).toFixed(1)} MB`);
  const actions = new Set(offered);
  const need = (action: string | undefined) => {
    if (action && !actions.has(action)) throw fail("no_such_action", `the view names action ${action}, which wasn't offered in hello (offered: ${offered.join(", ") || "none"})`);
  };
  for (const k of keys) need(k.action);
  need(close);
  const rasters = new Map<string, { columns: number; rows: number }>();
  let nodes = 0;
  const label = (s: string | undefined) => (s === undefined ? undefined : cleanText(s, VIEW_LIMIT.label));
  const inline = (x: ViewInline): ViewInline => {
    nodes++;
    if (typeof x === "string") return cleanBlock(x);
    if (x.type === "icon") return { type: "icon", agent: cleanText(x.agent, 40) };
    return { ...x, children: x.children?.map(inline) };
  };
  const walk = (n: ViewNode, depth: number): ViewNode => {
    if (++nodes > VIEW_LIMIT.nodes) throw fail("invalid_params", `a view has at most ${VIEW_LIMIT.nodes} elements`);
    if (depth > VIEW_LIMIT.depth) throw fail("invalid_params", `a view nests at most ${VIEW_LIMIT.depth} deep`);
    switch (n.type) {
      case "box":
        return { ...n, title: label(n.title), children: n.children?.map((c) => walk(c, depth + 1)) };
      case "scroll":
        return { ...n, children: n.children?.map((c) => walk(c, depth + 1)) };
      case "text":
        return { ...n, children: n.children?.map(inline) };
      case "markdown":
      case "code":
        return { ...n, content: cleanBlock(n.content) };
      case "diff":
        need(n.action);
        need(n.change);
        return { ...n, diff: cleanBlock(n.diff) };
      case "table":
        return { ...n, rows: n.rows.map((r) => r.map((c) => (typeof c === "string" ? cleanText(c, VIEW_LIMIT.label) : c.map(inline)))) };
      case "bigtext":
        return { ...n, text: cleanText(n.text, 40) };
      case "gauge":
      case "spinner":
        return { ...n, label: label(n.label) };
      case "raster":
        if (rasters.has(n.key)) throw fail("invalid_params", `two rasters in one view have the key ${n.key}`);
        checkCells(n.cells, n.columns, n.rows);
        rasters.set(n.key, { columns: n.columns, rows: n.rows });
        return n;
      case "image": {
        if (n.png.length > VIEW_LIMIT.image) throw fail("invalid_params", `an image is at most ${VIEW_LIMIT.image / 1024 / 1024} MB of base64`);
        const head = Buffer.from(n.png.slice(0, 12), "base64");
        if (!PNG.every((b, i) => head[i] === b)) throw fail("invalid_params", "an image's png isn't base64 of a PNG");
        return { ...n, alt: label(n.alt) };
      }
      case "button":
        need(n.action);
        return { ...n, label: cleanText(n.label, VIEW_LIMIT.label) };
      case "input":
      case "textarea":
        need(n.action);
        return { ...n, placeholder: label(n.placeholder), value: n.value === undefined ? undefined : cleanBlock(n.value, 100_000) };
      case "select":
        need(n.action);
        need(n.change);
        return { ...n, options: n.options.map((o) => ({ ...o, name: cleanText(o.name, VIEW_LIMIT.label), description: label(o.description) })) };
      case "tabs":
        need(n.action);
        return { ...n, options: n.options.map((o) => ({ ...o, name: cleanText(o.name, VIEW_LIMIT.label), description: label(o.description) })) };
      default:
        return n; // progress, sparkline, chart, heatmap: numbers only
    }
  };
  // JSON drops what's undefined, so a cleaned field that was absent stays absent on the wire
  return { root: walk(root, 0), keys: keys.map((k) => ({ ...k, description: label(k.description) })), rasters };
}
