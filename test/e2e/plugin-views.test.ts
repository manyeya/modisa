// Plugin views: element trees a plugin's bound connection opens (ui.view.set), checked and cleaned by the server, sent
// only to clients that draw them, drawn by the TUI in a frame named for the plugin, with the keyboard: Tab between
// elements, buttons, fields and lists running the plugin's actions with what they hold, Escape closing it (and telling
// the plugin), Rasters repainted in place (ui.blit), and everything gone when the plugin stops.
import { test, expect, beforeAll, beforeEach, afterAll } from "bun:test";
import { Screen, sandbox, startServer } from "../support/harness";
import { results } from "../../src/protocol/schema";
import { connectUnix } from "../../src/protocol/transport";
import { PLUGIN_UI } from "../../src/protocol/types";

const sb = sandbox("plugin-views");
const S = "views";
const REPO = `${import.meta.dir}/../..`;
const dir = `${sb.root}/view-demo`;
const run = (...args: string[]) => sb.run(S, args);
let screen: Screen | undefined;

const state = async () => {
  const parsed = JSON.parse((await run("plugin", "ui", "view-demo", "--json")).stdout);
  expect(results["ui.state"].safeParse(parsed).success).toBe(true);
  return parsed;
};
// the plugin makes these calls from its bound connection; each gives "ok" or the error code
const apply = async (...calls: [string, object][]) => sb.json<string[]>(S, ["plugin", "run", "view-demo", "apply", JSON.stringify({ calls })]);
const log = async () => sb.json<{ action: string; params: object; ui?: object }[]>(S, ["plugin", "run", "view-demo", "log"]);
const waitFor = async (what: string, ok: () => Promise<boolean>, ms = 8000) => {
  for (const end = Date.now() + ms; Date.now() < end; await Bun.sleep(100)) if (await ok()) return;
  throw new Error(`timed out waiting for ${what}: ${JSON.stringify(await log())}\n${screen?.text()}`);
};
const connected = async () => {
  for (let i = 0; i < 100; i++, await Bun.sleep(100)) if ((await sb.json(S, ["plugin", "list"], { retry: "startup" })).find((p: any) => p.name === "view-demo")?.connected) return;
  throw new Error("view-demo never connected");
};
// a Raster's cells: one character over and over, in the default colours
const cells = (ch: string, n: number) => Buffer.from(new Uint32Array(Array.from({ length: n }, () => [ch.codePointAt(0)!, 0x01000000, 0x01000000]).flat()).buffer).toString("base64");

const demo = {
  type: "box", direction: "column", gap: 1, children: [
    { type: "text", children: ["Hello ", { type: "span", tone: "accent", bold: true, children: ["\x1b[31mviews"] }] },
    { type: "progress", value: 0.5, width: 20 },
    { type: "raster", key: "pix", columns: 3, rows: 1, cells: cells("a", 3) },
    { type: "button", key: "go", label: "Press me", action: "pressed", params: { n: 1 } },
    { type: "input", key: "note", placeholder: "say something", action: "typed" },
    { type: "select", key: "pick", options: [{ name: "first" }, { name: "second", value: "two" }], action: "chose" },
  ],
};

beforeAll(async () => {
  const actions = ["apply", "pressed", "typed", "chose", "closed", "log"].map((id) => ({ id, title: id }));
  await Bun.write(`${dir}/plugin.json`, JSON.stringify({ name: "view-demo", protocol: 1, run: ["bun", "plugin.ts"], actions }));
  await Bun.write(`${dir}/modisa-plugin.ts`, await Bun.file(`${REPO}/src/plugins/modisa-plugin.ts`).text());
  await Bun.write(
    `${dir}/plugin.ts`,
    `import { runPlugin } from "./modisa-plugin";
const seen: unknown[] = [];
const note = (action: string) => (params: unknown, call: { ui?: unknown }) => void seen.push({ action, params, ...(call.ui ? { ui: call.ui } : {}) });
runPlugin(async (modisa) => {
  await modisa.hello({
    apply: async (p) => {
      const out: string[] = [];
      for (const [method, params] of p.calls as [string, any][]) out.push(await modisa.request(method, params).then(() => "ok", (e) => e.code));
      return out;
    },
    pressed: note("pressed"), typed: note("typed"), chose: note("chose"), closed: note("closed"),
    log: () => seen,
  });
});
`,
  );
  await startServer(sb, S);
  expect((await run("plugin", "link", dir)).code).toBe(0);
}, 30000);

beforeEach(async () => {
  await run("plugin", "stop", "view-demo");
  await run("plugin", "start", "view-demo");
  await connected();
}, 20000);

afterAll(async () => {
  screen?.close();
  await sb.cleanup();
});

test("a view is checked, cleaned, kept with a rising rev, and closed", async () => {
  expect(await apply(["ui.view.set", { id: "main", title: "Demo", root: demo, keys: [{ key: "x", action: "pressed" }], close: "closed" }])).toEqual(["ok"]);
  const [v] = (await state()).views;
  expect(v).toMatchObject({ plugin: "view-demo", id: "main", title: "Demo", placement: "popup", rev: 1, close: "closed" });
  expect(v.root.children[0].children[1].children).toEqual(["views"]); // the escape sequence is gone
  expect(await apply(["ui.view.set", { id: "main", root: { type: "text", children: ["again"] } }])).toEqual(["ok"]);
  expect((await state()).views[0]).toMatchObject({ title: "Demo", rev: 2, root: { type: "text", children: ["again"] } }); // the title stays

  // what it may hold
  expect(
    await apply(
      ["ui.view.set", { id: "bad", root: { type: "button", label: "x", action: "nope" } }],
      ["ui.view.set", { id: "bad", root: { type: "box", children: [], keys: [] } }],
      ["ui.view.set", { id: "bad", root: { type: "raster", key: "r", columns: 2, rows: 1, cells: cells("a", 3) } }],
      ["ui.view.set", { id: "bad", root: { type: "raster", key: "r", columns: 1, rows: 1, cells: cells("\x07", 1) } }],
      ["ui.view.set", { id: "bad", root: { type: "image", png: "aGVsbG8=" } }],
      ["ui.view.set", { id: "bad", root: { type: "box", children: Array.from({ length: 5001 }, () => ({ type: "text" })) } }],
      ["ui.view.set", { id: "bad", placement: "overlay", from: { pane: "p1", instance: "nope" }, root: { type: "text" } }],
      ["ui.blit", { view: "main", key: "nope", cells: cells("b", 3) }],
    ),
  ).toEqual(["no_such_action", "invalid_params", "invalid_params", "invalid_params", "invalid_params", "invalid_params", "pane_gone", "invalid_params"]);
  expect((await state()).views.map((x: { id: string }) => x.id)).toEqual(["main"]);

  expect(await apply(["ui.view.close", { id: "main" }])).toEqual(["ok"]);
  expect((await state()).views).toEqual([]);
}, 30000);

test("only a client that draws views is sent them; stopping the plugin closes them", async () => {
  const sock = `${sb.env.MODISA_DIR}/${S}.sock`;
  const older = await connectUnix(sock), current = await connectUnix(sock);
  const got = { older: [] as string[], current: [] as string[] };
  older.onMessage = (m) => m.method?.startsWith("plugin.") && got.older.push(m.method);
  current.onMessage = (m) => m.method?.startsWith("plugin.") && got.current.push(m.method);
  expect((await older.request<any>("attach", { ui: 1 })).views).toBeUndefined();
  expect((await current.request<any>("attach", { ui: PLUGIN_UI })).views).toEqual([]);

  expect(await apply(["ui.view.set", { id: "main", root: demo }], ["ui.blit", { view: "main", key: "pix", cells: cells("b", 3) }])).toEqual(["ok", "ok"]);
  expect((await state()).views[0].root.children[2].cells).toBe(cells("b", 3)); // a client attaching now draws the new cells
  expect((await run("plugin", "stop", "view-demo")).code).toBe(0);
  for (let i = 0; i < 50 && !got.current.includes("plugin.view.closed"); i++) await Bun.sleep(100);
  expect(got.current).toEqual(["plugin.view", "plugin.blit", "plugin.view.closed"]);
  expect(got.older).toEqual([]);
  older.close();
  current.close();
}, 30000);

test("the TUI draws it framed and named, and its elements run the plugin's actions with what they hold", async () => {
  expect(await apply(["ui.view.set", { id: "main", title: "Demo", root: demo, keys: [{ key: "x", action: "pressed", params: { from: "key" }, description: "press" }], close: "closed" }])).toEqual(["ok"]);
  screen?.close();
  screen = new Screen(["-s", S], sb.env, sb.root);
  await screen.until("the view, framed and named", (s) => s.includes("view-demo · Demo") && s.includes("Hello views") && s.includes("Press me") && s.includes("aaa") && s.includes("██████████"), 20000);
  expect(screen.text()).toContain("x press"); // its keys, at the bottom of the frame

  // the input has the keyboard first (a field before a list or a button): type, Enter
  screen.write("hi there\r");
  await waitFor("what was typed", async () => (await log()).some((e) => e.action === "typed"));
  // Tab to the list, down, Enter
  screen.write("\t");
  await Bun.sleep(200);
  screen.write("\x1b[B\r");
  await waitFor("the choice", async () => (await log()).some((e) => e.action === "chose"));
  // Tab wraps around to the button (first in the view), Enter
  screen.write("\t");
  await Bun.sleep(200);
  screen.write("\r");
  await waitFor("the press", async () => (await log()).filter((e) => e.action === "pressed").length === 1);
  screen.write("x"); // the view's own key
  await waitFor("the key", async () => (await log()).filter((e) => e.action === "pressed").length === 2);
  expect(await log()).toEqual([
    { action: "typed", params: {}, ui: { view: "main", key: "note", value: "hi there" } },
    { action: "chose", params: {}, ui: { view: "main", key: "pick", value: "two", index: 1 } },
    { action: "pressed", params: { n: 1 }, ui: { view: "main", key: "go" } },
    { action: "pressed", params: { from: "key" }, ui: { view: "main" } },
  ]);

  // a blit repaints the raster in place
  expect(await apply(["ui.blit", { view: "main", key: "pix", cells: cells("z", 3) }])).toEqual(["ok"]);
  await screen.until("the new cells", (s) => s.includes("zzz"));

  // Escape closes it, for everyone, and the plugin hears of it
  screen.write("\x1b");
  await screen.until("the view to go", (s) => !s.includes("view-demo · Demo"));
  await waitFor("the close action", async () => (await log()).some((e) => e.action === "closed"));
  expect((await state()).views).toEqual([]);
}, 60000);

test("a diff's line cursor moves without the plugin and runs its action with the line; a plugin can hand the keyboard over", async () => {
  const diff = "diff --git a/a.ts b/a.ts\n--- a/a.ts\n+++ b/a.ts\n@@ -1,3 +1,4 @@\n const a = 1;\n-const b = 2;\n+const b = 3;\n+const c = 4;\n const d = 5;\n";
  const root = (focus = false) => ({ type: "box", children: [{ type: "diff", key: "d", diff, filetype: "typescript", cursor: true, marks: [1], action: "pressed" }, ...(focus ? [{ type: "input", key: "note", action: "typed" }] : [])] });
  expect(await apply(["ui.view.set", { id: "main", title: "Diff", root: root() }])).toEqual(["ok"]);
  screen?.close();
  screen = new Screen(["-s", S], sb.env, sb.root);
  await screen.until("the diff", (s) => s.includes("view-demo · Diff") && s.includes("const c = 4;"), 20000);
  screen.write("jj\r");
  await waitFor("the line", async () => (await log()).some((e) => e.action === "pressed"));
  expect((await log()).find((e) => e.action === "pressed")).toEqual({ action: "pressed", params: {}, ui: { view: "main", key: "d", index: 2, value: "+const b = 3;" } });

  // the plugin opens a field and gives it the keyboard; the cursor stays where it was
  expect(await apply(["ui.view.set", { id: "main", root: root(true), focus: "note" }])).toEqual(["ok"]);
  await Bun.sleep(300);
  screen.write("on line 3\r");
  await waitFor("the note", async () => (await log()).some((e) => e.action === "typed"));
  expect((await log()).find((e) => e.action === "typed")?.ui).toEqual({ view: "main", key: "note", value: "on line 3" });
  screen.write("\t\r"); // back to the diff: Enter on the same line
  await waitFor("the same line again", async () => (await log()).filter((e) => e.action === "pressed").length === 2);
  expect((await log()).filter((e) => e.action === "pressed")[1]?.ui).toMatchObject({ index: 2 });
}, 60000);
