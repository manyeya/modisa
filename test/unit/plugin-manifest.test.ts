import { test, expect } from "bun:test";
import { pluginManifest } from "../../src/protocol/schema";
import { cleanText } from "../../src/server/plugins";

const base = { name: "demo", protocol: 1, run: ["bun", "plugin.ts"] };
const problems = (manifest: object) => {
  const r = pluginManifest.safeParse(manifest);
  return r.success ? [] : r.error.issues.map((i) => i.message);
};

test("a manifest with actions, panes, keys and links validates", () => {
  const r = pluginManifest.safeParse({
    ...base,
    actions: [{ id: "clear", title: "Clear the log" }],
    panes: [{ id: "log", title: "Attention log", run: ["less", "attention.log"], placement: "popup", width: "80%", height: 20 }],
    keys: [{ key: "A", pane: "log", description: "open the log" }, { key: "C", action: "clear", description: "clear it" }],
    links: [{ pattern: "^https://github\\.com/", action: "clear" }],
  });
  expect(r.success).toBe(true);
  expect(r.data?.panes?.[0]?.placement).toBe("popup");
  expect(pluginManifest.parse({ ...base, panes: [{ id: "p", title: "P", run: ["x"] }] }).panes?.[0]?.placement).toBe("overlay");
});

test("bad references, duplicates and regexes are refused with what to fix", () => {
  expect(problems({ ...base, actions: [{ id: "a", title: "A" }, { id: "a", title: "again" }] })).toContain("a second action with id a");
  expect(problems({ ...base, keys: [{ key: "A", action: "missing", description: "d" }] })).toContain("key A runs action missing, which isn't in actions");
  expect(problems({ ...base, panes: [{ id: "p", title: "P", run: ["x"] }], actions: [{ id: "a", title: "A" }], keys: [{ key: "A", action: "a", pane: "p", description: "d" }] })).toContain("key A needs exactly one of action or pane");
  expect(problems({ ...base, keys: [{ key: "A", description: "nothing" }] })).toContain("key A needs exactly one of action or pane");
  expect(problems({ ...base, actions: [{ id: "a", title: "A" }], keys: [{ key: "A", action: "a", description: "d" }, { key: "A", action: "a", description: "d" }] })).toContain("key A is bound twice");
  expect(problems({ ...base, actions: [{ id: "a", title: "A" }], links: [{ pattern: "([unclosed", action: "a" }] }).some((m) => m.startsWith("not a valid regular expression"))).toBe(true);
  expect(problems({ ...base, links: [{ pattern: "x", action: "nope" }] })).toContain("links to action nope, which isn't in actions");
  expect(problems({ ...base, panes: [{ id: "p", title: "P", run: ["x"], placement: "floating" }] }).length).toBeGreaterThan(0);
});

test("text shown in the TUI loses escape sequences and control characters, and is cut to length", () => {
  expect(cleanText("\x1b[31mred\x1b[0m\x07 bell\x1b]0;title\x07 ok", 100)).toBe("red bell ok");
  expect(cleanText("x".repeat(50), 10)).toBe("x".repeat(10));
});

test("invisible formatting characters that could reorder or hide text are removed", () => {
  expect(cleanText("safe‮exe.txt", 100)).toBe("safeexe.txt"); // right-to-left override
  expect(cleanText("⁦isolated⁩", 100)).toBe("isolated");
  expect(cleanText("zero​width‍﻿⁠", 100)).toBe("zerowidth");
});

test("text is cut by terminal cells, never through a character", () => {
  expect(cleanText("ab\u{1F600}cd", 3)).toBe("ab"); // the emoji is 2 cells: it doesn't fit, and isn't split
  expect(cleanText("ab\u{1F600}cd", 4)).toBe("ab\u{1F600}");
  expect(cleanText("日本語", 5)).toBe("日本"); // wide characters count 2
  expect(cleanText("éé", 1)).toBe("é"); // a combining accent stays with its letter
});
