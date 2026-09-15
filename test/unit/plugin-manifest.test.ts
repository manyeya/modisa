import { test, expect } from "bun:test";
import { pluginManifest } from "../../src/protocol/schema";
import { urlMatches } from "../../src/protocol/links";
import { cleanText } from "../../src/server/plugins";
import { urlAt } from "../../src/client/panes/pane";

test("the URL under a column: the whole URL, without trailing punctuation, or nothing", () => {
  const line = "see https://example.com/a?b=1, and (http://x.io/y).";
  expect(urlAt(line, 4)).toBe("https://example.com/a?b=1");
  expect(urlAt(line, 28)).toBe("https://example.com/a?b=1");
  expect(urlAt(line, 29)).toBeUndefined(); // the comma
  expect(urlAt(line, 40)).toBe("http://x.io/y");
  expect(urlAt(line, 0)).toBeUndefined();
  expect(urlAt(`https://e.com/${"a".repeat(5000)}`, 3)).toHaveLength(2048);
});

test("a closing bracket stays when the URL opened it, and goes when it didn't", () => {
  const wiki = "see https://en.wikipedia.org/wiki/Foo_(bar) now";
  expect(urlAt(wiki, 10)).toBe("https://en.wikipedia.org/wiki/Foo_(bar)");
  expect(urlAt("(see https://x.dev/a)", 8)).toBe("https://x.dev/a");
  expect(urlAt("[https://x.dev/a[1]].", 3)).toBe("https://x.dev/a[1]");
  expect(urlAt("https://x.dev/(a)).", 3)).toBe("https://x.dev/(a)");
  expect(urlAt("https://x.dev/a_(b)_c", 3)).toBe("https://x.dev/a_(b)_c"); // inside the URL
  expect(urlAt("https://x.dev/a).", 3)).toBe("https://x.dev/a"); // outside it
  expect(urlAt("https://x.dev/a],", 3)).toBe("https://x.dev/a");
  expect(urlAt("https://x.dev/a‮gpj.exe", 3)).toBe("https://x.dev/a"); // a bidi override ends it
});

test("the column is a terminal cell: wide characters before the URL take two", () => {
  const cjk = "日本語 https://example.com/x"; // the URL starts at cell 7 and ends at cell 27
  expect(urlAt(cjk, 7)).toBe("https://example.com/x");
  expect(urlAt(cjk, 27)).toBe("https://example.com/x");
  expect(urlAt(cjk, 28)).toBeUndefined();
  expect(urlAt(cjk, 5)).toBeUndefined();
  const emoji = "\u{1F642} https://e.dev/a"; // 2 cells, a space, then the URL
  expect(urlAt(emoji, 3)).toBe("https://e.dev/a");
  expect(urlAt(emoji, 2)).toBeUndefined();
  // a combining accent is one cell with its letter, before the URL and inside it
  const cafe = "café https://e.dev/café"; // "café " is 5 cells; the URL's 18 cells are 5..22
  expect(urlAt(cafe, 5)).toBe("https://e.dev/café");
  expect(urlAt(cafe, 22)).toBe("https://e.dev/café");
  expect(urlAt(cafe, 23)).toBeUndefined();
  expect(urlAt(cafe, 4)).toBeUndefined();
  // a ZWJ family is one grapheme of two cells, before the URL and inside it
  const family = "\u{1F468}‍\u{1F469}‍\u{1F467}";
  const zwj = `${family} https://e.dev/${family}/p`; // the URL is cells 3..20: "https://e.dev/" 3..16, the family 17..18, "/p" 19..20
  expect(urlAt(zwj, 3)).toBe(`https://e.dev/${family}/p`);
  expect(urlAt(zwj, 18)).toBe(`https://e.dev/${family}/p`);
  expect(urlAt(zwj, 20)).toBe(`https://e.dev/${family}/p`);
  expect(urlAt(zwj, 21)).toBeUndefined();
  expect(urlAt(zwj, 2)).toBeUndefined();
});

test("a link glob ignores case in the scheme and host, not in the path", () => {
  expect(urlMatches("https://github.com/*", "https://GitHub.com/x")).toBe(true);
  expect(urlMatches("https://GITHUB.com/*", "HTTPS://github.COM/x")).toBe(true);
  expect(urlMatches("https://*.example.com/*", "https://API.Example.com/v1")).toBe(true);
  expect(urlMatches("https://x.dev/A*", "https://x.dev/a")).toBe(false);
  expect(urlMatches("https://x.dev/*?q=A", "https://X.dev/p?q=a")).toBe(false);
});

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
    links: [{ pattern: "https://github.com/*", action: "clear" }],
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
  expect(problems({ ...base, links: [{ pattern: "https://x/*", action: "nope" }] })).toContain("links to action nope, which isn't in actions");
  // link patterns are URL globs, never regular expressions: a regex, or any other scheme, is refused
  const glob = (pattern: string) => problems({ ...base, actions: [{ id: "a", title: "A" }], links: [{ pattern, action: "a" }] }).some((m) => m.includes("URL glob"));
  for (const bad of ["^(a|aa)+$", "(a+)+$", "^https://github\\.com/", "file:///*", "javascript:*", "*"]) expect(glob(bad)).toBe(true);
  for (const ok of ["https://github.com/*/pull/*", "http://localhost:*/*"]) expect(glob(ok)).toBe(false);
});

test("a link glob matches the whole URL, * spanning anything, and takes bounded time on any input", () => {
  expect(urlMatches("https://github.com/*/pull/*", "https://github.com/o/r/pull/12")).toBe(true);
  expect(urlMatches("https://github.com/*/pull/*", "https://github.com/o/r/issues/12")).toBe(false);
  expect(urlMatches("https://github.com/*", "https://github.com.evil.dev/x")).toBe(false); // literal, whole-URL
  expect(urlMatches("https://x.dev/a", "https://x.dev/a/b")).toBe(false);
  expect(urlMatches("https://x.dev/*", "https://x.dev/")).toBe(true);
  expect(urlMatches("https://x.dev/a.b", "https://x.dev/aXb")).toBe(false); // . is a dot, not "any character"
  // the worst case for a wildcard matcher: many stars and a URL that almost matches, at the maximum sizes
  const start = performance.now();
  expect(urlMatches(`https://${"a*".repeat(245)}b`, `https://${"a".repeat(2040)}`)).toBe(false);
  expect(performance.now() - start).toBeLessThan(100);
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
