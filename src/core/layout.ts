// Split tree for one tab. "row" = children side by side, "col" = stacked.
export type Node = { pane: string } | { dir: "row" | "col"; ratio: number; a: Node; b: Node };
export type Rect = { x: number; y: number; w: number; h: number };
export type Dir = "left" | "right" | "up" | "down";

const MIN = 3; // smallest pane edge: two border cells + one content cell
// Below this a pane is too small to use, and the tab shows only its focused pane (see displayRects).
export const USABLE = { w: 24, h: 6 };

export function split(tree: Node, id: string, dir: "row" | "col", newId: string): Node {
  if ("pane" in tree) return tree.pane === id ? { dir, ratio: 0.5, a: tree, b: { pane: newId } } : tree;
  return { ...tree, a: split(tree.a, id, dir, newId), b: split(tree.b, id, dir, newId) };
}

export function remove(tree: Node, id: string): Node | null {
  if ("pane" in tree) return tree.pane === id ? null : tree;
  const a = remove(tree.a, id);
  const b = remove(tree.b, id);
  if (!a) return b;
  if (!b) return a;
  return { ...tree, a, b };
}

export function panes(tree: Node): string[] {
  return "pane" in tree ? [tree.pane] : [...panes(tree.a), ...panes(tree.b)];
}

function cut(n: Exclude<Node, { pane: string }>, r: Rect): [Rect, Rect] {
  if (n.dir === "row") {
    const aw = clamp(Math.round(r.w * n.ratio), Math.min(MIN, r.w - 1), Math.max(r.w - MIN, 1));
    return [{ ...r, w: aw }, { ...r, x: r.x + aw, w: r.w - aw }];
  }
  const ah = clamp(Math.round(r.h * n.ratio), Math.min(MIN, r.h - 1), Math.max(r.h - MIN, 1));
  return [{ ...r, h: ah }, { ...r, y: r.y + ah, h: r.h - ah }];
}

export function rects(tree: Node, r: Rect, out = new Map<string, Rect>()): Map<string, Rect> {
  if ("pane" in tree) return out.set(tree.pane, r);
  const [ra, rb] = cut(tree, r);
  rects(tree.a, ra, out);
  rects(tree.b, rb, out);
  return out;
}

// Preserve the split tree while temporarily showing the focused pane at small sizes.
// Shared by the client and server so PTY dimensions always match what is displayed.
export function displayRects(tree: Node, area: Rect, focused: string, zoomed = false): Map<string, Rect> {
  const rs = rects(tree, area);
  if (zoomed || [...rs.values()].some((r) => r.w < USABLE.w || r.h < USABLE.h)) return new Map([[focused, area]]);
  return rs;
}

// Nearest pane in a direction that shares an edge; ties go to the one closest to our centre.
export function neighbor(rs: Map<string, Rect>, id: string, dir: Dir): string | undefined {
  const c = rs.get(id);
  if (!c) return;
  let best: string | undefined;
  let bestDist = Infinity;
  for (const [other, o] of rs) {
    if (other === id) continue;
    const touches =
      dir === "right" ? o.x === c.x + c.w :
      dir === "left" ? o.x + o.w === c.x :
      dir === "down" ? o.y === c.y + c.h :
      o.y + o.h === c.y;
    const overlaps = dir === "left" || dir === "right"
      ? o.y < c.y + c.h && o.y + o.h > c.y
      : o.x < c.x + c.w && o.x + o.w > c.x;
    if (!touches || !overlaps) continue;
    const dist = dir === "left" || dir === "right"
      ? Math.abs(o.y + o.h / 2 - (c.y + c.h / 2))
      : Math.abs(o.x + o.w / 2 - (c.x + c.w / 2));
    if (dist < bestDist) [best, bestDist] = [other, dist];
  }
  return best;
}

// Smallest box a subtree needs so every pane in it stays usable.
function need(n: Node): { w: number; h: number } {
  if ("pane" in n) return USABLE;
  const a = need(n.a), b = need(n.b);
  return n.dir === "row" ? { w: a.w + b.w, h: Math.max(a.h, b.h) } : { w: Math.max(a.w, b.w), h: a.h + b.h };
}

// Keep a divider where both sides stay usable. If the space is too small for that anyway (the
// terminal shrank), leave the ratio alone; displayRects shows one pane until there's room again.
function settle(n: Exclude<Node, { pane: string }>, r: Rect, ratio: number) {
  const size = n.dir === "row" ? r.w : r.h;
  const key = n.dir === "row" ? "w" : "h";
  const lo = need(n.a)[key], hi = size - need(n.b)[key];
  if (lo > hi) return;
  n.ratio = clamp(ratio, lo / size, hi / size);
}

// Move the nearest divider on the pane's `dir` side by `cells`. Mutates ratios in place.
export function resize(tree: Node, r: Rect, id: string, dir: Dir, cells: number): boolean {
  if ("pane" in tree) return false;
  const [ra, rb] = cut(tree, r);
  const inA = panes(tree.a).includes(id);
  if (inA ? resize(tree.a, ra, id, dir, cells) : resize(tree.b, rb, id, dir, cells)) return true;
  const axis = dir === "left" || dir === "right" ? "row" : "col";
  // the divider sits right/below of `a` and left/above of `b`
  const facing = inA ? dir === "right" || dir === "down" : dir === "left" || dir === "up";
  if (tree.dir !== axis || !facing) return false;
  const size = axis === "row" ? r.w : r.h;
  const sign = dir === "right" || dir === "down" ? 1 : -1;
  settle(tree, r, tree.ratio + (sign * cells) / size);
  return true;
}

// Split node whose divider is under (x, y): the two border cells either side of the cut.
export function dividerAt(tree: Node, r: Rect, x: number, y: number): { node: Exclude<Node, { pane: string }>; rect: Rect } | undefined {
  if ("pane" in tree) return;
  const [ra, rb] = cut(tree, r);
  if (tree.dir === "row" && (x === rb.x - 1 || x === rb.x) && y >= r.y && y < r.y + r.h) return { node: tree, rect: r };
  if (tree.dir === "col" && (y === rb.y - 1 || y === rb.y) && x >= r.x && x < r.x + r.w) return { node: tree, rect: r };
  const inA = x < ra.x + ra.w && y < ra.y + ra.h;
  return dividerAt(inA ? tree.a : tree.b, inA ? ra : rb, x, y);
}

export function dragTo(hit: { node: Exclude<Node, { pane: string }>; rect: Rect }, x: number, y: number) {
  const { node, rect } = hit;
  settle(node, rect, node.dir === "row" ? (x - rect.x + 0.5) / rect.w : (y - rect.y + 0.5) / rect.h);
}

const clamp = (v: number, lo: number, hi: number) => Math.min(Math.max(v, lo), hi);
