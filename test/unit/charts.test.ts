import { test, expect } from "bun:test";
import { Braille, columns, gauge, heatmap, lines, progress, type Grid } from "../../src/client/views/charts";

const text = (g: Grid) => g.map((r) => r.map((c) => c.ch).join(""));

test("a progress bar fills to an eighth of a cell, over a track", () => {
  expect(progress(0.23, 20, "accent").map((c) => c.ch).join("")).toBe("████▋" + " ".repeat(15));
  expect(progress(1, 4, "accent").every((c) => c.ch === "█")).toBe(true);
  expect(progress(-3, 3, "accent").every((c) => c.ch === " " && c.bg?.tone === "track")).toBe(true);
  expect(progress(0.5, 3, "warn")[1]).toEqual({ ch: "▌", fg: { tone: "warn" }, bg: { tone: "track" } });
});

test("columns rise from the bottom, newest at the right; one row is a sparkline", () => {
  expect(text(columns([0, 4, 8], 5, 1, "accent"))).toEqual(["   ▄█"]);
  expect(text(columns([8, 16], 2, 2, "accent", 0, 16))).toEqual([" █", "██"]);
  expect(text(columns([1, 2, 3], 2, 1, "accent"))).toEqual(["▅█"]); // only the last `width` values
});

test("braille: 2×4 dots a cell, the last dot's ink colours the cell", () => {
  const b = new Braille(2, 1);
  b.set(0, 0, { tone: "accent" });
  b.set(3, 3, { tone: "warn" });
  b.set(9, 9, { tone: "warn" }); // outside: ignored
  expect(b.grid()).toEqual([[{ ch: "⠁", fg: { tone: "accent" } }, { ch: "⢀", fg: { tone: "warn" } }]]);
});

test("lines, gauges and heatmaps fill the box they're given", () => {
  const l = lines([{ values: [0, 10], tone: "accent" }], 4, 2);
  expect(l).toHaveLength(2);
  expect(text(l)[1]![0]).not.toBe(" "); // starts bottom left
  expect(text(l)[0]![3]).not.toBe(" "); // ends top right
  const g = gauge(0.5, 12, 5, "accent");
  expect(g).toHaveLength(5);
  expect(text(g).join("\n")).toContain("50%");
  expect(g.flat().some((c) => c.fg?.tone === "track")).toBe(true); // the unfilled half
  expect(text(gauge(0.9, 20, 8, "blocked", "5h 90%")).join("")).toContain("5h 90%");
  expect(heatmap([[0, 10], [10, 0]], "accent")).toEqual([[
    { ch: "▀", fg: { tone: "accent", mix: 0 }, bg: { tone: "accent", mix: 1 } },
    { ch: "▀", fg: { tone: "accent", mix: 1 }, bg: { tone: "accent", mix: 0 } },
  ]]);
});
