import { test, expect } from "bun:test";
import { DEFAULTS, SAMPLE, findKey, withDefaultKeys, withTheme, withValue, withoutTable } from "../../src/config/config";
import { checkConfig } from "../../src/config/check";
import { toastExtras } from "../../src/client/notify";

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

test("a key is matched literally: dots and dashes in a quoted key aren't patterns", () => {
  const source = '[plugin_keys]\n"radarXlog" = "A"\n"radar.log" = "B"\n';
  expect(withValue(source, "plugin_keys", "radar.log", "C")).toBe('[plugin_keys]\n"radarXlog" = "A"\n"radar.log" = "C"\n');
  const lines = source.split("\n");
  expect(findKey(lines, "plugin_keys", "radar.log")).toEqual({ start: 1, end: 4, at: 2 });
  expect(findKey(lines, "plugin_keys", "radar.lo")).toMatchObject({ at: -1 });
  expect(findKey(lines, "keys")).toEqual({ start: -1, end: -1, at: -1 });
});

test("withoutTable takes a table and its settings out, keeping every comment and the other tables", () => {
  const source = '# mine\nprefix = "C-a" # p\n\n[keys]   # my keys\nzoom = "f"   # z\n# a note\nsplit-right = [\n  "v", # one\n  "|",\n]\n\n[sidebar]\nwidth = 30\n';
  const result = withoutTable(source, "keys");
  expect(result).toBe('# mine\nprefix = "C-a" # p\n\n# a note\n\n[sidebar]\nwidth = 30\n');
  expect(Bun.TOML.parse(result)).toEqual({ prefix: "C-a", sidebar: { width: 30 } });
  expect(withoutTable('theme = "ion"\n\n[keys]\nzoom = "f"\n', "keys")).toBe('theme = "ion"\n');
  expect(withoutTable('[keys]\nzoom = "f"\n\n[git]\nrepo = false\n', "keys")).toBe('[git]\nrepo = false\n');
  expect(withoutTable(source, "absent")).toBe(source);
  expect(() => withoutTable('keys = { zoom = "f" }\n', "keys")).toThrow("by hand"); // not a [keys] table: left alone
});

test("reset-keys takes out [keys] and [plugin_keys] and puts the prefix back, saying what changed", () => {
  const { result, changes } = withDefaultKeys('prefix = "C-a"   # mine\n\n[keys]\nzoom = "f"\n\n[plugin_keys]\n"a.b" = "Y"\n"c.d" = ""\n');
  expect(result).toBe('prefix = "C-b"   # mine\n');
  expect(changes).toEqual(["[keys] removed (1 setting)", "[plugin_keys] removed (2 settings)", 'prefix "C-a" → "C-b"']);
  expect(withDefaultKeys('theme = "ion"\n').changes).toEqual([]);
  expect(() => withDefaultKeys("a = \n")).toThrow("doesn't parse (line 1");
});

test("config check: the sample and an empty file are fine; mistakes are errors with their line, unknown settings warnings", () => {
  expect(checkConfig(SAMPLE)).toEqual([]);
  expect(checkConfig("")).toEqual([]);
  const problems = checkConfig('prefix = "Ctrl-b"\ntheme = "neon"\nshiny = true\n\n[sidebar]\nwidth = 60\ngit = false\n\n[notify]\nblocked = ["toast", "pager"]\n\n[sound]\ndone = "boom"\n\n[permissions]\nkeys_foreign = "maybe"\n\n[update]\nchannel = "nightly"\n\n[keys]\nzoom = "x"\nbogus = "q"\n');
  const at = (key: string) => problems.find((p) => p.key === key);
  expect(at("prefix")).toMatchObject({ level: "error", line: 1 });
  expect(at("theme")).toMatchObject({ level: "error", line: 2, message: expect.stringContaining('"neon"') });
  expect(at("shiny")).toMatchObject({ level: "warning", line: 3 });
  expect(at("sidebar.width")).toMatchObject({ level: "error", line: 6, message: "20 to 48 columns" });
  expect(at("sidebar.git")).toMatchObject({ level: "warning", line: 7, message: expect.stringContaining("[git] status") });
  expect(at("notify.blocked.1")).toMatchObject({ level: "error", line: 10 });
  expect(at("sound.done")).toMatchObject({ level: "error", line: 13 });
  expect(at("permissions.keys_foreign")).toMatchObject({ level: "error", line: 16 });
  expect(at("update.channel")).toMatchObject({ level: "error", line: 19 });
  expect(at("keys.zoom")).toMatchObject({ level: "error", line: 22, message: expect.stringContaining("reserved") });
  expect(at("keys.bogus")).toMatchObject({ level: "warning", line: 23 });
  expect(problems.map((p) => p.line)).toEqual([...problems.map((p) => p.line)].sort((a, b) => a! - b!)); // in file order
});

test("config check: a file that doesn't parse is one error, at its line and column", () => {
  expect(checkConfig('theme = "ion"\n[sidebar]\nwidth = [1, 2\n')).toEqual([{ level: "error", message: expect.any(String), line: 3, column: 14 }]);
});

test("a sent toast's system notification and sound happen only where the user's config has them on", () => {
  const cfg = (notify: object) => ({ ...DEFAULTS, notify: { blocked: [], done: [], working: [], ...notify } }) as typeof DEFAULTS;
  const all = { tone: "done" as const, system: true, sound: true };
  expect(toastExtras(cfg({}), all)).toEqual({ system: false, sound: undefined });
  expect(toastExtras(cfg({ blocked: ["system"] }), all)).toEqual({ system: true, sound: undefined });
  expect(toastExtras(cfg({ blocked: ["sound"], done: ["sound"] }), all)).toEqual({ system: false, sound: DEFAULTS.sound.done }); // its tone's
  expect(toastExtras(cfg({ blocked: ["sound"] }), all)).toEqual({ system: false, sound: DEFAULTS.sound.blocked }); // else the first that plays
  expect(toastExtras(cfg({ blocked: ["system", "sound"] }), { tone: "fg" })).toEqual({ system: false, sound: undefined }); // not asked
});
