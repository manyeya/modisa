// Prefix bindings: modisa's defaults, what [keys] changes in them, the keys it can't, and plugin keys bound around them.
import { test, expect } from "bun:test";
import { ACTION_IDS, DEFAULT_KEYS, bindPluginKeys, bindings, modisaKey, resolveKeys } from "../../src/config/keys";

test("with no [keys], the bindings are the defaults, and every default runs a known action", () => {
  expect(bindings({})).toEqual(DEFAULT_KEYS);
  expect(bindings({ keys: {} })).toEqual(DEFAULT_KEYS);
  for (const action of Object.values(DEFAULT_KEYS)) expect(ACTION_IDS).toContain(action);
});

test("an action set in [keys] has only the keys given there, taken from whatever had them", () => {
  const t = bindings({ keys: { zoom: "f", "split-right": ["v", "|"], "split-down": "%" } });
  expect(t.f).toBe("zoom");
  expect(t.z).toBeUndefined(); // zoom's default went
  expect(t["|"]).toBe("split-right");
  expect(t["%"]).toBe("split-down"); // split-right's default, given to another action
  expect(t["-"]).toBeUndefined();
  expect(t.h).toBe("focus-left"); // untouched actions keep theirs
});

test('"" or [] leaves an action with no key', () => {
  for (const none of ["", []]) {
    const t = bindings({ keys: { help: none } });
    expect(Object.values(t)).not.toContain("help");
    expect(t["?"]).toBeUndefined();
  }
});

test("x, d and escape can't be given away, and x and d stay on close-pane and detach", () => {
  const { table, problems } = resolveKeys({ zoom: "x", palette: "escape", "close-pane": "q", detach: "" });
  expect(table.x).toBe("close-pane");
  expect(table.q).toBe("close-pane");
  expect(table.d).toBe("detach");
  expect(table.escape).toBeUndefined();
  expect(Object.values(table)).not.toContain("zoom"); // asked only for x, which it can't have
  expect(problems.map((p) => [p.action, p.level])).toEqual([["zoom", "error"], ["palette", "error"]]);
  expect(resolveKeys({ "close-pane": ["x", "q"] }).problems).toEqual([]); // its own key, said again
});

test("what [keys] gets wrong is reported: an unknown action, something that isn't a key, a key given twice", () => {
  const { table, problems } = resolveKeys({ nope: "q", zoom: "ctrl-z", help: "g", settings: "g", "copy-mode": 3 });
  expect(problems).toEqual([
    { level: "warning", action: "nope", message: expect.stringContaining("no action nope") },
    { level: "error", action: "zoom", message: expect.stringContaining("isn't a key") },
    { level: "error", action: "settings", message: expect.stringContaining("given to help too") },
    { level: "error", action: "copy-mode", message: expect.stringContaining("a key is a string") },
  ]);
  expect(table.g).toBe("settings"); // the last one wins
  expect(table["["]).toBe("copy-mode"); // a value that isn't keys changes nothing
  expect(resolveKeys({ zoom: ["Z", "f5", "pageup", "é"] }).problems).toEqual([]);
});

test("plugin keys are refused for the keys these bindings use, and given the ones they free", () => {
  expect(modisaKey("z")).toBe("modisa's zoom");
  expect(modisaKey("x")).toContain("reserved");
  const table = bindings({ keys: { zoom: "Y" } });
  expect(modisaKey("z", table)).toBeUndefined();
  expect(modisaKey("Y", table)).toBe("modisa's zoom");
  const declared = [{ plugin: "p", key: "z", action: "a", description: "a" }, { plugin: "q", key: "Y", action: "b", description: "b" }];
  expect(bindPluginKeys(declared).map((k) => k.state)).toEqual(["disabled", "active"]);
  expect(bindPluginKeys(declared, {}, table).map((k) => [k.state, k.reason])).toEqual([["active", undefined], ["disabled", "modisa's zoom"]]);
});
