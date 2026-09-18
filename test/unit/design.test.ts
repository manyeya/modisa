import { test, expect } from "bun:test";
import { chrome, fit, floating, mix, tabWindow } from "../../src/client/design";
import { displayRects, split } from "../../src/core/layout";

test("chrome fits wide, compact and short terminals", () => {
  for (const [width, height] of [[160, 48], [100, 24], [80, 24], [44, 14], [20, 6]]) {
    const { area, side } = chrome(width!, height!, true, 999);
    expect(area.x + area.w).toBe(width!);
    expect(area.y + area.h).toBeLessThan(height!);
    expect(side).toBeLessThanOrEqual(Math.min(48, Math.floor(width! / 3)));
    if (width! < 100) expect(side).toBe(0);
  }
});

test("small layouts preserve the tree and restore every split", () => {
  const tree = split({ pane: "a" }, "a", "row", "b");
  const original = structuredClone(tree);
  const small = { x: 0, y: 1, w: 44, h: 12 };
  expect([...displayRects(tree, small, "b")]).toEqual([["b", small]]);
  expect([...displayRects(tree, small, "a").keys()]).toEqual(["a"]);
  expect(displayRects(tree, { ...small, w: 120 }, "b").size).toBe(2);
  expect(tree).toEqual(original);
});

test("long Unicode titles fit terminal cells without splitting graphemes", () => {
  for (const text of ["a very long pane title", "日本語のターミナル", "👨‍👩‍👧‍👦 agent", "e\u0301e\u0301e\u0301", "\x1b[31mtitle\nnext"]) {
    for (let width = 0; width < 20; width++) expect(Bun.stringWidth(fit(text, width))).toBeLessThanOrEqual(width);
  }
  expect(fit("👨‍👩‍👧‍👦 agent", 3)).toBe("👨‍👩‍👧‍👦…");
});

test("menus stay inside the terminal near every edge", () => {
  for (const [width, height] of [[140, 40], [44, 14], [18, 7]]) {
    const rect = floating(width!, height!, 34, 17, width! - 1, height! - 1);
    expect(rect.x).toBeGreaterThanOrEqual(0);
    expect(rect.y).toBeGreaterThanOrEqual(0);
    expect(rect.x + rect.w).toBeLessThanOrEqual(width!);
    expect(rect.y + rect.h).toBeLessThanOrEqual(height!);
  }
});

test("tab overflow always includes the selected tab", () => {
  for (let active = 0; active < 40; active++) {
    const window = tabWindow(40, active, 63);
    expect(window.start).toBeLessThanOrEqual(active);
    expect(window.end).toBeGreaterThan(active);
    expect((window.end - window.start) * window.width).toBeLessThanOrEqual(63);
  }
});

test("hover and selection tints blend theme colors", () => {
  expect(mix("#000000", "#ffffff", 0)).toBe("#000000");
  expect(mix("#000000", "#ffffff", 1)).toBe("#ffffff");
  expect(mix("#101b2c", "#5ee7ef", 0.5)).toBe("#37818e");
});

test("an agent's mark is its brand colour, or the theme's text where that would be faint; its task drops the spinner", async () => {
  const { agentMark, agentTask } = await import("../../src/client/design");
  const { THEMES } = await import("../../src/config/themes");
  expect(agentMark(THEMES.ion!, "claude-code")).toEqual({ glyph: "✳", color: "#d97757", cells: 2 }); // mark, gap
  expect(agentMark(THEMES.ion!, "claude-code", true)).toEqual({ glyph: String.fromCodePoint(0xf5a00), color: "#d97757", cells: 3 }); // a logo's two cells, gap
  expect(agentMark(THEMES.ion!, "aider", true).cells).toBe(2); // no logo: its glyph, one cell
  expect(agentMark(THEMES.ion!, "codex").color).toBe(THEMES.ion!.fg); // a monochrome brand
  expect(agentMark(THEMES["bearded-solarized-light"]!, "kilo").color).toBe(THEMES["bearded-solarized-light"]!.fg); // yellow on cream
  expect(agentMark(THEMES.ion!, "who-knows").glyph).toBe("•");
  expect(agentTask("✳ Audit the token cache")).toBe("Audit the token cache");
  expect(agentTask("⠋ codex", "codex")).toBe("");
  expect(agentTask(undefined)).toBe("");
});
