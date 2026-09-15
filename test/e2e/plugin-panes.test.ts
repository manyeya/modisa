// Plugin panes and keys. split, tab and zoomed open ordinary panes (with the plugin's environment) that outlive the
// plugin; a popup only opens from a TUI client; keys that are shepherd's, or wanted by two plugins, are off, and a
// [plugin_keys] remap wins; an overlay opens zoomed over its origin and gives focus and zoom back when it closes; a
// popup shows only in the client that opened it, one per session, keeps Escape for its program and closes on prefix x,
// on its process exiting, and when its plugin stops; a key runs its action for the focused pane.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { Screen, sandbox, startServer, borders } from "../support/harness";

const sb = sandbox("plugin-panes");
const S = "panes";
const REPO = `${import.meta.dir}/../..`;
const run = (...args: string[]) => sb.run(S, args);
const screens: Screen[] = [];
const list = async () => JSON.parse((await run("pane", "list", "--json")).stdout) as any[];
const plugins = async () => JSON.parse((await run("plugin", "list", "--json")).stdout) as any[];

async function plugin(name: string, manifest: object) {
  const dir = `${sb.root}/${name}`;
  await Bun.write(`${dir}/plugin.json`, JSON.stringify({ name, protocol: 1, run: ["bun", "plugin.ts"], ...manifest }));
  await Bun.write(`${dir}/shepherd-plugin.ts`, await Bun.file(`${REPO}/src/plugins/shepherd-plugin.ts`).text());
  await Bun.write(`${dir}/plugin.ts`, `import { runPlugin } from "./shepherd-plugin";\nrunPlugin(async (shepherd) => { await shepherd.hello({ hello: (p) => "hi " + (p.pane ?? "nobody") }); });\n`);
  expect((await run("plugin", "link", dir)).code).toBe(0);
}

beforeAll(async () => {
  await Bun.write(`${sb.root}/config/config.toml`, `[plugin_keys]\n"demo.pop" = "U"\n`); // the manifest says P
  await startServer(sb, S);
  await plugin("demo", {
    actions: [{ id: "hello", title: "Say hello" }],
    panes: [
      { id: "side", title: "Side", placement: "split", run: ["sh", "-c", "echo side:$SHEPHERD_PLUGIN:$(test -d \"$SHEPHERD_PLUGIN_CONFIG\" && echo config); sleep 120"] },
      { id: "board", title: "Board", placement: "tab", run: ["sh", "-c", "echo board; sleep 120"] },
      { id: "focus", title: "Focus", placement: "zoomed", run: ["sh", "-c", "echo zoomed-in; sleep 120"] },
      { id: "peek", title: "Peek", placement: "overlay", run: ["sh", "-c", "echo peeking; read x"] },
      { id: "pop", title: "Pop", placement: "popup", width: "60%", height: 12, run: ["sh", "-c", "echo in-popup; read x; echo got:$x; sleep 1"] },
    ],
    keys: [
      { key: "O", pane: "peek", description: "peek" },
      { key: "P", pane: "pop", description: "popup" },
      { key: "G", action: "hello", description: "greet" },
      { key: "Y", action: "hello", description: "wanted by the rival too" },
      { key: "v", action: "hello", description: "shepherd's split-right" },
    ],
  });
  await plugin("rival", { actions: [{ id: "hello", title: "Hello" }], keys: [{ key: "Y", action: "hello", description: "the same key" }] });
}, 40000);

afterAll(async () => {
  for (const s of screens) s.close();
  await run("kill", S);
  await sb.cleanup();
});

test("split, tab and zoomed open ordinary panes with the plugin's environment, and outlive the plugin", async () => {
  const side = JSON.parse((await run("plugin", "pane", "demo", "side", "--json")).stdout);
  expect(side).toMatchObject({ placement: "split", title: "Side" });
  expect((await run("wait", side.pane, "--match", "side:demo:config", "--timeout", "10")).code).toBe(0);
  expect((await list()).find((p) => p.id === side.pane)).toMatchObject({ createdBy: "plugin:demo", title: "Side" });
  const tabs = JSON.parse((await run("workspace", "list", "--json")).stdout)[0].tabs;
  expect(JSON.parse((await run("plugin", "pane", "demo", "board", "--json")).stdout).placement).toBe("tab");
  expect(JSON.parse((await run("workspace", "list", "--json")).stdout)[0].tabs).toBe(tabs + 1);
  expect(JSON.parse((await run("plugin", "pane", "demo", "focus", "--json")).stdout).placement).toBe("zoomed");

  expect((await run("plugin", "stop", "demo")).code).toBe(0);
  expect((await list()).some((p) => p.id === side.pane)).toBe(true); // the session's now
  expect((await run("plugin", "start", "demo")).code).toBe(0);
  for (let i = 0; i < 100 && !(await plugins()).find((p) => p.name === "demo")?.connected; i++) await Bun.sleep(100);
}, 40000);

test("a popup only opens from a TUI client", async () => {
  const r = await run("plugin", "pane", "demo", "pop", "--json");
  expect(r.code).toBe(2);
  expect(JSON.parse(r.stderr).error.code).toBe("usage");
});

test("keys: shepherd's own and one two plugins want are off, and a [plugin_keys] remap wins", async () => {
  const all = await plugins();
  const keys = Object.fromEntries(all.find((p) => p.name === "demo").keys.map((k: any) => [`${k.key}`, k]));
  expect(keys.v).toMatchObject({ state: "disabled", reason: "shepherd's split-right" });
  expect(keys.Y).toMatchObject({ state: "disabled", reason: "also wanted by rival" });
  expect(all.find((p) => p.name === "rival").keys[0]).toMatchObject({ key: "Y", state: "disabled", reason: "also wanted by demo" });
  expect(keys.U).toMatchObject({ pane: "pop", state: "active" }); // remapped from P
  expect(keys.P).toBeUndefined();
  expect(keys.G).toMatchObject({ action: "hello", state: "active" });
  expect((await run("plugin", "list")).stdout).toContain("demo: key v (hello) is off: shepherd's split-right");
});

test("an overlay opens zoomed over the pane and gives focus and zoom back when it closes", async () => {
  const ui = new Screen(["-s", S], sb.env, sb.root);
  screens.push(ui);
  await ui.until("attached", (s) => s.includes("SPACES"), 20000);
  await run("pane", "focus", "p1");
  await ui.until("p1 focused, not zoomed", (s) => borders(s) > 1);
  ui.write("\x02O");
  await ui.until("the overlay, zoomed", (s) => s.includes("peeking") && borders(s) === 1);
  const overlay = (await list()).find((p) => p.title === "Peek");
  expect(overlay).toMatchObject({ focused: true, createdBy: "plugin:demo" });
  ui.write("\r"); // its program ends
  await ui.until("the overlay gone, the layout back", (s) => !s.includes("peeking") && borders(s) > 1);
  expect((await list()).find((p) => p.id === "p1")).toMatchObject({ focused: true });
  expect((await list()).some((p) => p.title === "Peek")).toBe(false);
}, 40000);

test("a popup shows only where it was opened, is one per session, keeps Escape for its program, and closes on prefix x", async () => {
  const [ui] = screens;
  const other = new Screen(["-s", S], sb.env, sb.root);
  screens.push(other);
  await other.until("a second client", (s) => s.includes("SPACES"), 20000);

  ui!.write("\x02U");
  await ui!.until("the popup", (s) => s.includes("in-popup") && s.includes("prefix x closes"));
  await Bun.sleep(500);
  expect(other.text()).not.toContain("in-popup");
  other.write("\x02U"); // the other client asks for one too
  await other.until("told a popup is already open", (s) => s.includes("a popup is already open"));

  ui!.write("\x1b"); // Escape belongs to the popup's program
  await Bun.sleep(400);
  expect(ui!.text()).toContain("in-popup");
  ui!.write("\x02x");
  await ui!.until("the popup gone", (s) => !s.includes("in-popup"));
  expect((await list()).some((p) => p.popup)).toBe(false);

  ui!.write("\x02U");
  await ui!.until("a popup again", (s) => s.includes("in-popup"));
  ui!.write("done\r"); // its program ends
  await ui!.until("it closes when its process exits", (s) => !s.includes("in-popup"), 10000);
}, 60000);

test("stopping the plugin closes its popup", async () => {
  const [ui] = screens;
  ui!.write("\x02U");
  await ui!.until("the popup", (s) => s.includes("in-popup"));
  expect((await run("plugin", "stop", "demo")).code).toBe(0);
  await ui!.until("the popup gone", (s) => !s.includes("in-popup"));
  expect((await run("plugin", "start", "demo")).code).toBe(0);
  for (let i = 0; i < 100 && !(await plugins()).find((p) => p.name === "demo")?.connected; i++) await Bun.sleep(100);
}, 40000);

test("a key runs its plugin's action for the focused pane", async () => {
  const [ui] = screens;
  await ui!.until("demo's keys back", () => true);
  await Bun.sleep(1000);
  await run("pane", "focus", "p1");
  ui!.write("\x02G");
  await ui!.until("the action's result, for p1", (s) => s.includes("demo: Say hello → hi p1"));
}, 30000);
