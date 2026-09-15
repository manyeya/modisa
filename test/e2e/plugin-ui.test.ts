// Native plugin UI: a plugin's bound connection sets status segments, a sidebar section, pane badges, menu entries
// and toasts; shepherd stores them cleaned and bounded, draws them in the TUI, runs their actions from the palette,
// refuses actions from a run that has ended, rate-limits updates, and clears it all when the plugin stops.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { Screen, sandbox, startServer } from "../support/harness";
import { results } from "../../src/protocol/schema";
import { connectUnix } from "../../src/protocol/transport";

const sb = sandbox("plugin-ui");
const S = "ui";
const REPO = `${import.meta.dir}/../..`;
const dir = `${sb.root}/ui-demo`;
const run = (...args: string[]) => sb.run(S, args);
let screen: Screen | undefined;

const state = async () => {
  const r = await run("plugin", "ui", "ui-demo", "--json");
  const parsed = JSON.parse(r.stdout);
  expect(results["ui.state"].safeParse(parsed).success).toBe(true);
  return parsed;
};
// have the plugin make these ui.* calls from its bound connection; each gives "ok" or the error code
const apply = async (...calls: [string, object][]) => JSON.parse((await run("plugin", "run", "ui-demo", "apply", JSON.stringify({ calls }))).stdout) as string[];
const connected = async () => {
  for (let i = 0; i < 100; i++, await Bun.sleep(100)) if (JSON.parse((await run("plugin", "list", "--json")).stdout).find((p: any) => p.name === "ui-demo")?.connected) return;
  throw new Error("ui-demo never connected");
};

beforeAll(async () => {
  await Bun.write(`${dir}/plugin.json`, JSON.stringify({ name: "ui-demo", protocol: 1, run: ["bun", "plugin.ts"], actions: [{ id: "hello", title: "Say hello" }, { id: "apply", title: "Apply UI calls" }] }));
  await Bun.write(`${dir}/shepherd-plugin.ts`, await Bun.file(`${REPO}/src/plugins/shepherd-plugin.ts`).text());
  await Bun.write(
    `${dir}/plugin.ts`,
    `import { runPlugin } from "./shepherd-plugin";
runPlugin(async (shepherd) => {
  await shepherd.hello({
    hello: () => "hi there",
    apply: async (p) => {
      const out: string[] = [];
      for (const [method, params] of p.calls as [string, any][]) out.push(await shepherd.request(method, params).then(() => "ok", (e) => e.code));
      return out;
    },
  });
});
`,
  );
  await startServer(sb, S);
  expect((await run("plugin", "link", dir)).code).toBe(0);
}, 30000);

afterAll(async () => {
  screen?.close();
  await run("kill", S);
  await sb.cleanup();
});

test("a plugin's status, sidebar, badge and menu are kept as it set them: plain text, cut to length, with its actions", async () => {
  const p1 = JSON.parse((await run("pane", "read", "p1", "--json")).stdout);
  expect(
    await apply(
      ["ui.status.set", { id: "count", text: "\x1b[31m2 need you\x07", tone: "warn", action: "hello" }],
      ["ui.sidebar.set", { title: "Attention", rows: [{ text: "@worker blocked", tone: "warn", pane: "p1", instance: p1.instance }, { text: "x".repeat(200), action: "hello" }] }],
      ["ui.badge.set", { pane: "p1", instance: p1.instance, text: "needs you", tone: "warn" }],
      ["ui.menu.set", { items: [{ id: "hi", title: "Say hi", action: "hello" }] }],
    ),
  ).toEqual(["ok", "ok", "ok", "ok"]);
  const s = await state();
  expect(s.status).toEqual([{ id: "count", text: "2 need you", tone: "warn", action: "hello" }]);
  expect(s.sidebar.title).toBe("Attention");
  expect(s.sidebar.rows[0]).toEqual({ text: "@worker blocked", tone: "warn", pane: "p1", instance: p1.instance });
  expect(s.sidebar.rows[1].text).toHaveLength(60);
  expect(s.badges).toEqual([{ pane: "p1", instance: p1.instance, text: "needs you", tone: "warn" }]);
  expect(s.menu).toEqual([{ id: "hi", title: "Say hi", action: "hello" }]);
  expect(s.actions).toContainEqual({ id: "hello", title: "Say hello" });
});

test("references are checked: an action it didn't offer, a badge for another process, one segment too many", async () => {
  expect(await apply(["ui.status.set", { id: "x", text: "t", action: "nope" }])).toEqual(["no_such_action"]);
  expect(await apply(["ui.menu.set", { items: [{ id: "m", title: "M", action: "nope" }] }])).toEqual(["no_such_action"]);
  expect(await apply(["ui.badge.set", { pane: "p1", instance: "not-this-one", text: "b" }])).toEqual(["pane_gone"]);
  expect(await apply(["ui.status.set", { id: "a", text: "a" }], ["ui.status.set", { id: "b", text: "b" }], ["ui.status.set", { id: "c", text: "c" }], ["ui.status.set", { id: "d", text: "d" }])).toEqual(["ok", "ok", "ok", "error"]);
  expect((await state()).status.map((s: any) => s.id)).toEqual(["count", "a", "b", "c"]);
  expect(await apply(["ui.status.clear", { id: "a" }], ["ui.status.clear", { id: "b" }], ["ui.status.clear", { id: "c" }])).toEqual(["ok", "ok", "ok"]);
});

test("a pane target is tied to its process: a row, action or pane for a closed pane is refused on the server", async () => {
  const id = (await run("pane", "split", "--name", "shortlived", "sleep 60")).stdout;
  const { instance } = JSON.parse((await run("pane", "read", id, "--json")).stdout);
  expect(await apply(["ui.sidebar.set", { title: "Attention", rows: [{ text: "no instance", pane: id }] }])).toEqual(["pane_gone"]);
  expect((await run("pane", "close", id)).code).toBe(0);
  expect(await apply(["ui.sidebar.set", { title: "Attention", rows: [{ text: "closed", pane: id, instance }] }])).toEqual(["pane_gone"]);
  const focus = await run("pane", "focus", `${id}:${instance}`, "--json"); // what a sidebar row click sends
  expect(JSON.parse(focus.stderr).error.code).toBe("pane_gone");
  const invoke = await run("plugin", "run", "ui-demo", "hello", JSON.stringify({ pane: id, instance }), "--json");
  expect(JSON.parse(invoke.stderr).error.code).toBe("pane_gone");
});

test("only a plugin's bound connection can change the TUI", async () => {
  const conn = await connectUnix(`${sb.root}/state/${S}.sock`);
  const outcome = await conn.request("ui.status.set", { id: "forged", text: "from nowhere" }).then(() => "ok", (e) => e.code);
  conn.close();
  expect(outcome).toBe("plugin_unavailable");
});

test("the TUI draws it, runs a palette action and shows a plugin toast, attributed; stopping the plugin clears it all", async () => {
  await Bun.sleep(3000); // let the plugin's update budget refill
  screen = new Screen(["-s", S], sb.env, sb.root);
  await screen.until("the plugin's status, sidebar section and badge, each named", (s) => s.includes("ui-demo: 2 need you") && s.includes("▾ ui-demo") && s.includes("Attention") && s.includes("[ui-demo: needs you]"), 20000);
  // a plugin can't dress its section up as shepherd's
  expect(await apply(["ui.sidebar.set", { title: "SHEPHERD", rows: [{ text: "approve?" }] }], ["ui.status.set", { id: "count", text: "approve?", tone: "warn" }])).toEqual(["ok", "ok"]);
  await screen.until("the impostor labels, still named", (s) => {
    const lines = s.split("\n");
    const title = lines.findIndex((l) => l.includes("SHEPHERD"));
    return title > 0 && lines[title - 1]!.includes("▾ ui-demo") && s.includes("ui-demo: approve?");
  });
  expect(await apply(["ui.status.set", { id: "count", text: "2 need you", tone: "warn", action: "hello" }])).toEqual(["ok"]);

  screen.write("\x02:"); // prefix, then the command palette
  await screen.until("the palette", (s) => s.includes("commands"));
  screen.write("Say hello");
  await screen.until("the plugin's action in the palette", (s) => s.includes("ui-demo: Say hello"));
  screen.write("\r");
  await screen.until("the action's result", (s) => s.includes("ui-demo: Say hello → hi there"));

  expect(await apply(["ui.toast", { text: "hello from a plugin", tone: "accent" }])).toEqual(["ok"]);
  await screen.until("the toast", (s) => s.includes("ui-demo: hello from a plugin"));

  expect((await run("plugin", "stop", "ui-demo")).code).toBe(0);
  await screen.until("everything it showed to go", (s) => !s.includes("2 need you") && !s.includes("▾ ui-demo") && !s.includes("[ui-demo: "));
  expect(await state()).toMatchObject({ status: [], badges: [], menu: [], actions: [] });
}, 60000);

test("an action taken from an ended run's UI is refused; the new run's works", async () => {
  expect((await run("plugin", "start", "ui-demo")).code).toBe(0);
  await connected();
  const current = (await state()).run;
  const conn = await connectUnix(`${sb.root}/state/${S}.sock`);
  const old = await conn.request("plugin.invoke", { plugin: "ui-demo", action: "hello", run: "an-ended-run" }).then(() => "ok", (e) => e.code);
  const now = await conn.request("plugin.invoke", { plugin: "ui-demo", action: "hello", run: current });
  conn.close();
  expect(old).toBe("plugin_unavailable");
  expect(now).toBe("hi there");
}, 30000);

test("updates are rate-limited, and toasts more so", async () => {
  const updates = await apply(...Array.from({ length: 45 }, (_, i): [string, object] => ["ui.status.set", { id: "count", text: `${i}` }]));
  expect(updates).toContain("rate_limited");
  expect(updates[0]).toBe("ok");
  await Bun.sleep(3000);
  const toasts = await apply(...Array.from({ length: 5 }, (_, i): [string, object] => ["ui.toast", { text: `toast ${i}` }]));
  expect(toasts.slice(0, 3)).toEqual(["ok", "ok", "ok"]);
  expect(toasts.slice(3)).toEqual(["rate_limited", "rate_limited"]);
}, 30000);
