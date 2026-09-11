import { test, expect } from "bun:test";
import { withTheme, withValue } from "../../src/config/config";

test("theme saving preserves comments, tables and unrelated settings", () => {
  const source = '# my setup\nprefix = "C-a"\ntheme = "ion" # keep this\n\n[sidebar]\nvisible = false\nwidth = 30\n';
  const result = withTheme(source, "dracula");
  expect(result).toBe(source.replace('theme = "ion"', 'theme = "dracula"'));
  expect(Bun.TOML.parse(result)).toEqual({ prefix: "C-a", theme: "dracula", sidebar: { visible: false, width: 30 } });
  expect(Bun.TOML.parse(withTheme('[sidebar]\nvisible = false\n', "nord"))).toEqual({ theme: "nord", sidebar: { visible: false } });
  expect(() => withTheme(source, "unknown")).toThrow();
});

test("settings edit one key in place, in any table, keeping comments", () => {
  const source = '# mine\ntheme = "ion"\n\n[notify]            # kinds\nblocked = ["toast", "sound"]   # needs you\ndone = ["toast"]\n\n[sidebar]\nvisible = true\n';
  // an existing key inside a table: value replaced, its comment kept
  const a = withValue(source, "notify", "blocked", ["toast"]);
  expect(a).toBe(source.replace('blocked = ["toast", "sound"]', 'blocked = ["toast"]'));
  // a new key in an existing table goes at the end of that table, not the file
  const b = withValue(source, "notify", "working", []);
  expect(b).toContain('done = ["toast"]\nworking = []\n\n[sidebar]');
  // a new table is appended; booleans and numbers are written as TOML literals
  const c = withValue(withValue(source, "sound", "volume", 0.5), "indicators", "tab", false);
  expect(c.startsWith(source.trimEnd())).toBe(true);
  expect(Bun.TOML.parse(c)).toMatchObject({ theme: "ion", sound: { volume: 0.5 }, indicators: { tab: false }, sidebar: { visible: true } });
  // a # inside a string isn't mistaken for a comment
  expect(withValue('[sound]\nblocked = "a#b"  # c\n', "sound", "blocked", "chime")).toBe('[sound]\nblocked = "chime"  # c\n');
  // a multiline array is replaced whole, with the comment after it kept
  expect(withValue('[notify]\nblocked = [\n  "toast", # mine\n  "sound",\n]  # end\ndone = []\n', "notify", "blocked", ["bell"])).toBe('[notify]\nblocked = ["bell"]  # end\ndone = []\n');
});
