import { test, expect } from "bun:test";
import { split, remove, rects, neighbor, resize, dividerAt, dragTo, panes, displayRects, USABLE, type Node } from "../../src/core/layout";

const area = { x: 0, y: 1, w: 100, h: 40 };

// a | b
//   | c
const tree = (): Node => split(split({ pane: "a" }, "a", "row", "b"), "b", "col", "c");

test("rects tile the area exactly", () => {
  const rs = rects(tree(), area);
  expect([...rs.keys()]).toEqual(["a", "b", "c"]);
  let cells = 0;
  const seen = new Set<string>();
  for (const r of rs.values())
    for (let x = r.x; x < r.x + r.w; x++)
      for (let y = r.y; y < r.y + r.h; y++) {
        const k = `${x},${y}`;
        expect(seen.has(k)).toBe(false);
        seen.add(k);
        cells++;
      }
  expect(cells).toBe(area.w * area.h);
});

test("remove collapses the parent split", () => {
  expect(remove(tree(), "b")).toEqual({ dir: "row", ratio: 0.5, a: { pane: "a" }, b: { pane: "c" } });
  expect(remove({ pane: "a" }, "a")).toBeNull();
  expect(panes(remove(tree(), "a")!)).toEqual(["b", "c"]);
});

test("neighbor finds adjacent panes", () => {
  const rs = rects(tree(), area);
  expect(neighbor(rs, "a", "right")).toBeDefined();
  expect(neighbor(rs, "b", "down")).toBe("c");
  expect(neighbor(rs, "c", "up")).toBe("b");
  expect(neighbor(rs, "c", "left")).toBe("a");
  expect(neighbor(rs, "a", "left")).toBeUndefined();
});

test("resize moves the right divider and clamps", () => {
  const t = tree();
  expect(resize(t, area, "a", "right", 10)).toBe(true);
  expect(rects(t, area).get("a")!.w).toBe(60);
  expect(resize(t, area, "c", "up", 4)).toBe(true); // c grows up = divider between b and c moves up
  expect(rects(t, area).get("b")!.h).toBe(16);
  expect(resize(t, area, "a", "left", 10)).toBe(false); // no divider on a's left
  resize(t, area, "a", "right", 1000);
  expect(rects(t, area).get("a")!.w).toBe(100 - USABLE.w); // the right column keeps a usable width
});

test("dividerAt + dragTo move a border with the mouse", () => {
  const t = tree();
  const hit = dividerAt(t, area, 50, 10)!;
  expect(hit.node.dir).toBe("row");
  dragTo(hit, 30, 10);
  expect(rects(t, area).get("a")!.w).toBe(31);
  expect(dividerAt(t, area, 10, 10)).toBeUndefined();
});

test("dragging a border can't shrink a pane into the one-pane compact view", () => {
  // a short terminal, like VS Code's panel: two panes stacked
  const short = { x: 0, y: 1, w: 140, h: 18 };
  const t: Node = split({ pane: "top" }, "top", "col", "bottom");
  const hit = dividerAt(t, short, 10, rects(t, short).get("bottom")!.y)!;
  for (const y of [short.y + short.h - 1, short.y, 100, -100]) {
    dragTo(hit, 10, y);
    const rs = rects(t, short);
    expect(rs.get("top")!.h).toBeGreaterThanOrEqual(USABLE.h);
    expect(rs.get("bottom")!.h).toBeGreaterThanOrEqual(USABLE.h);
    expect(displayRects(t, short, "top").size).toBe(2);
  }
  // keyboard resizing obeys the same limit
  for (let i = 0; i < 20; i++) resize(t, short, "top", "down", 3);
  expect(rects(t, short).get("bottom")!.h).toBe(USABLE.h);
  // a nested stack needs room for all of its panes
  const n: Node = split(split({ pane: "a" }, "a", "row", "b"), "b", "col", "c");
  const wide = { x: 0, y: 0, w: 100, h: 20 };
  dragTo(dividerAt(n, wide, rects(n, wide).get("b")!.x, 5)!, 99, 5);
  expect(rects(n, wide).get("b")!.w).toBe(USABLE.w);
  // too small to keep both usable: the ratio is left alone rather than forced
  const tiny = { x: 0, y: 0, w: 140, h: 10 };
  const m: Node = split({ pane: "p" }, "p", "col", "q");
  dragTo(dividerAt(m, tiny, 5, rects(m, tiny).get("q")!.y)!, 5, 9);
  expect((m as any).ratio).toBe(0.5);
});
