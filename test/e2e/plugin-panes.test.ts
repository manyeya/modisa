// Plugin panes and keys. split, tab and zoomed open ordinary panes (with the plugin's environment) that outlive the
// plugin; a popup only opens from a TUI client; keys that are shepherd's, or wanted by two plugins, are off, and a
// [plugin_keys] remap wins; an overlay opens zoomed over its origin and gives focus and zoom back when it closes, but
// only if it still had the focus; a popup shows only in the client that opened it, one per session, keeps Escape for its
// program and closes on prefix x, on its process exiting, and when its plugin stops; output from the pane underneath
// never draws over it, through a resize, and the pane redraws after it closes; a key runs its action for the focused pane.
// Every popup test has its own session and TUI clients, so each passes alone and in any order.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { Screen, sandbox, startServer, borders } from "../support/harness";

const sb = sandbox("plugin-panes");
const S = "panes";
const REPO = `${import.meta.dir}/../..`;
const screens: Screen[] = [];
const sessions = new Set<string>([S]);
const cli = (session: string) => (...args: string[]) => sb.run(session, args);
const run = cli(S);
const list = async (session = S) => JSON.parse((await sb.run(session, ["pane", "list", "--json"])).stdout) as any[];
const plugins = async (session = S) => JSON.parse((await sb.run(session, ["plugin", "list", "--json"])).stdout) as any[];
const connected = async (name: string, session = S) => {
  for (let i = 0; i < 100 && !(await plugins(session)).find((p) => p.name === name)?.connected; i++) await Bun.sleep(100);
};
async function attach(session = S) {
  const ui = new Screen(["-s", session], sb.env, sb.root);
  screens.push(ui);
  await ui.until("attached", (s) => s.includes("SPACES"), 20000);
  return ui;
}
// a session of its own: the linked plugins start in it
async function fresh(name: string, env: Record<string, string> = {}) {
  sessions.add(name);
  await startServer(sb, name, env);
  await connected("demo", name);
  return cli(name);
}

async function plugin(name: string, manifest: object) {
  const dir = `${sb.root}/${name}`;
  await Bun.write(`${dir}/plugin.json`, JSON.stringify({ name, protocol: 1, run: ["bun", "plugin.ts"], ...manifest }));
  await Bun.write(`${dir}/shepherd-plugin.ts`, await Bun.file(`${REPO}/src/plugins/shepherd-plugin.ts`).text());
  await Bun.write(`${dir}/plugin.ts`, `import { runPlugin } from "./shepherd-plugin";\nrunPlugin(async (shepherd) => { await shepherd.hello({ hello: (_p, call) => "hi " + (call.target?.pane ?? "nobody") }); });\n`);
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
  for (const s of sessions) await sb.run(s, ["kill", s]);
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
  await connected("demo");
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
  const ui = await attach();
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

test("an overlay doesn't take focus back if the user focused another pane in that tab meanwhile", async () => {
  const other = (await run("pane", "split", "--name", "elsewhere", "sleep 120")).stdout;
  await run("pane", "focus", "p1");
  const overlay = JSON.parse((await run("plugin", "pane", "demo", "peek", "--json")).stdout).pane;
  await run("pane", "focus", other); // the user moves on, in the same tab
  await run("pane", "close", overlay);
  await Bun.sleep(500);
  expect((await list()).find((p) => p.id === other)).toMatchObject({ focused: true });
  await run("pane", "close", other);
}, 30000);

test("an overlay doesn't take focus back if the user switched tabs meanwhile", async () => {
  await run("pane", "focus", "p1");
  const overlay = JSON.parse((await run("plugin", "pane", "demo", "peek", "--json")).stdout).pane;
  await run("tab", "create", "away"); // switches to the new tab
  const away = (await list()).find((p) => p.focused);
  expect(away.id).not.toBe(overlay);
  await run("pane", "close", overlay);
  await Bun.sleep(500);
  expect((await list()).find((p) => p.id === away.id)).toMatchObject({ focused: true });
  await run("pane", "close", away.id);
}, 30000);

test("a popup shows only where it was opened, is one per session, keeps Escape for its program, and closes on prefix x", async () => {
  const session = "popup-one";
  const here = await fresh(session);
  const ui = await attach(session);
  const other = await attach(session);

  ui.write("\x02U");
  await ui.until("the popup", (s) => s.includes("in-popup") && s.includes("Pop · prefix x closes"));
  await Bun.sleep(500);
  expect(other.text()).not.toContain("in-popup");
  other.write("\x02U"); // the other client asks for one too
  await other.until("told a popup is already open", (s) => s.includes("a popup is already open"));

  ui.write("\x1b"); // Escape belongs to the popup's program
  await Bun.sleep(400);
  expect(ui.text()).toContain("in-popup");
  ui.write("\x02x");
  await ui.until("the popup gone", (s) => !s.includes("in-popup"));
  expect((await list(session)).some((p) => p.popup)).toBe(false);

  ui.write("\x02U");
  await ui.until("a popup again", (s) => s.includes("in-popup"));
  ui.write("done\r"); // its program ends
  await ui.until("it closes when its process exits", (s) => !s.includes("in-popup"), 10000);
  expect((await here("plugin", "list")).code).toBe(0);
}, 60000);

test("a popup is opaque: every cell it covers is its own, over a pane full of text", async () => {
  const session = "popup-opaque";
  const here = await fresh(session, { TERM_PROGRAM: "Apple_Terminal" }); // its shell startup prints under the popup too
  const full = (await here("pane", "split", "--name", "full", "awk 'BEGIN{for(i=0;i<20000;i++) printf \"X\"}'; sleep 120")).stdout;
  await here("pane", "close", "p1"); // the full pane under the whole popup
  await here("pane", "focus", full);
  const ui = await attach(session);
  await ui.until("the pane full of X", (s) => s.split("\n").filter((l) => l.includes("X".repeat(60))).length > 20);
  ui.write("\x02U");
  await ui.until("the popup", (s) => s.includes("in-popup") && s.includes("Pop · prefix x closes"));
  await Bun.sleep(300);

  const lines = ui.lines();
  const top = lines.findIndex((l) => l.includes("Pop · prefix x closes"));
  const left = lines[top]!.indexOf("╭");
  const right = lines[top]!.indexOf("╮", left);
  const row = (i: number) => lines[i]!.slice(left, right + 1);
  expect(row(top)).toMatch(/^╭─ Pop · prefix x closes ─+╮$/);
  expect(row(top + 1)).toMatch(/^│in-popup +│$/);
  for (let i = top + 2; i < top + 11; i++) expect(row(i)).toMatch(/^│ +│$/);
  expect(row(top + 11)).toMatch(/^╰─+╯$/);
  ui.write("\x02x");
  await ui.until("the popup gone, the X back", (s) => !s.includes("in-popup") && s.split("\n")[top]!.includes("X".repeat(60)));
}, 60000);

test("output from the pane underneath never draws over a popup, through a resize, and the pane redraws after it closes", async () => {
  const session = "popup-noise";
  const here = await fresh(session, { TERM_PROGRAM: "Apple_Terminal" });
  const noisy = (await here("pane", "split", "--name", "noisy", "while :; do printf 'noise noise noise\\033]7;file://host/tmp\\a\\033]0;title\\a\\033[31mred\\033[0m\\n'; sleep 0.05; done")).stdout;
  await here("pane", "focus", noisy);
  const ui = await attach(session);
  await ui.until("the noise", (s) => s.includes("noise noise"));
  ui.write("\x02U");
  await ui.until("the popup", (s) => s.includes("in-popup") && s.includes("Pop · prefix x closes"));

  const intact = () => {
    const lines = ui.lines();
    const top = lines.findIndex((l) => l.includes("Pop · "));
    expect(top).toBeGreaterThanOrEqual(0);
    expect(lines[top]).toContain("Pop · prefix x closes");
    const left = lines[top]!.indexOf("╭");
    const right = lines[top]!.lastIndexOf("╮");
    const body = lines.slice(top + 1, top + 9).map((l) => l.slice(left + 1, right));
    expect(body.join("\n")).toContain("in-popup");
    expect(body.join("\n")).not.toContain("noise");
    expect(ui.text()).not.toContain("file://host"); // OSC 7 is never text
  };
  for (let i = 0; i < 8; i++) {
    await Bun.sleep(200); // the pane underneath keeps printing
    intact();
  }
  ui.resize(110, 32);
  await ui.until("the popup redrawn at the new size", (s) => s.includes("Pop · prefix x closes") && s.includes("in-popup"));
  for (let i = 0; i < 5; i++) {
    await Bun.sleep(200);
    intact();
  }

  ui.write("\x02x");
  await ui.until("the popup gone and the pane redrawn", (s) => !s.includes("in-popup") && !s.includes("Pop · ") && s.includes("noise noise"));
}, 60000);

test("stopping the plugin closes its popup", async () => {
  const session = "popup-stop";
  const here = await fresh(session);
  const ui = await attach(session);
  ui.write("\x02U");
  await ui.until("the popup", (s) => s.includes("in-popup"));
  expect((await here("plugin", "stop", "demo")).code).toBe(0);
  await ui.until("the popup gone", (s) => !s.includes("in-popup"));
}, 40000);

test("a key runs its plugin's action for the focused pane", async () => {
  const ui = await attach();
  await connected("demo");
  await run("pane", "focus", "p1");
  ui.write("\x02G");
  await ui.until("the action's result, for p1", (s) => s.includes("demo: Say hello → hi p1"));
}, 30000);
