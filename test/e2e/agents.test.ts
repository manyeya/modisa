// Agents: a modisa.toml starts them, detection tracks working → done → needs you, and the sidebar
// and status row show it.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { Screen, sandbox } from "../support/harness";
import { installFakeAgent } from "../support/fake-agent";
import { click } from "../support/mouse";

const sb = sandbox("agents");
const S = "agents";
const cli = (...args: string[]) => sb.cli(S, args);
let ui: Screen;

beforeAll(async () => {
  await installFakeAgent(sb.root);
  // project template: a server pane and an agent pane
  await Bun.write(`${sb.root}/proj/modisa.toml`, `name = "proj"\n\n[[pane]]\nname = "server"\nrun = "echo server-up; sleep 600"\n\n[[pane]]\nname = "coder"\nagent = "fakeagent"\n`);
});

afterAll(async () => {
  ui?.close();
  await cli("kill", S);
  await sb.cleanup();
});

test("modisa.toml template lays out named panes and starts the agent", async () => {
  ui = new Screen(["-s", S], { ...sb.env, PWD: `${sb.root}/proj` }, `${sb.root}/proj`);
  await ui.until("template panes", (s) => s.includes("@server") && s.includes("@coder") && s.includes("server-up"), 15000);
  expect(await cli("wait", "@coder", "--state", "working", "--timeout", "10")).toBe("working");
  expect(await cli("workspace", "list")).toContain("proj");
}, 30000);

test("an unfocused agent finishing shows a toast", async () => {
  await ui.until("done toast", (s) => s.includes("@coder is done"), 15000);
}, 20000);

test("detection: working, then done while unfocused", async () => {
  expect(await cli("agent", "spawn", "fakeagent", "--name", "fake")).toMatch(/^p\d+$/);
  await ui.until("agent in sidebar", (s) => s.includes("@fake"));
  expect(await cli("wait", "@fake", "--state", "working", "--timeout", "10")).toBe("working");
  expect(await cli("wait", "@fake", "--state", "done", "--timeout", "10")).toBe("done");
  expect(await cli("agent", "list")).toMatch(/@fake\s+fakeagent\s+done/);
  expect(await cli("debug", "detect", "@fake")).toContain('"harness": "fakeagent"');
}, 25000);

test("need-you counts agents in a dialog; the status buttons list only that state", async () => {
  const status = () => { const lines = ui.lines(); return { line: lines.at(-1)!, y: lines.length - 1 }; };
  await cli("pane", "split", "--name", "asker", "fakeagent --ask");
  expect(await cli("wait", "@asker", "--state", "blocked", "--timeout", "10")).toBe("blocked");
  await ui.until("need-you count", (s) => s.split("\n").at(-1)!.includes("! 1 need you"));
  let { line, y } = status();
  click(ui, 0, line.indexOf("need you"), y);
  await ui.until("only blocked agents listed", (s) => s.includes("Agents that need you") && s.includes("@asker"));
  expect(ui.lines().filter((l) => l.includes("@coder") && l.includes("fakeagent ·")).length).toBe(0);
  ui.write("\x1b");
  await ui.until("picker closed", (s) => !s.includes("Agents that need you"));
  ({ line, y } = status());
  if (line.includes("◆ 0 working")) {
    click(ui, 0, line.indexOf("working"), y);
    await ui.until("nothing working", (s) => s.includes("no agents are working"));
  }
  await cli("pane", "close", "@asker");
}, 30000);

test("the sidebar lists agents and toggles", async () => {
  await ui.until("agent in sidebar", (s) => s.includes("AGENTS") && s.includes("@coder"));
  ui.write("\x02b");
  await ui.until("sidebar hidden", (s) => !s.includes("AGENTS"));
  ui.write("\x02b");
  await ui.until("sidebar back", (s) => s.includes("AGENTS"));
}, 10000);

test("the sidebar and status row only count the current space's agents", async () => {
  await cli("workspace", "create", "empty");
  await ui.until("empty space has no agents", (s) => s.includes("◈ empty") && /AGENTS +0/.test(s) && s.includes("◆ 0 working"));
  await cli("workspace", "close", "empty");
  await ui.until("back to the agents' space", (s) => s.includes("◈ proj") && s.includes("@coder") && !/AGENTS +0/.test(s));
}, 15000);

test("closing a pane while detection is reading the process table doesn't crash the server", async () => {
  const { PtyPane } = await import("../../src/server/session/pane");
  const { Detector } = await import("../../src/server/agents/detect");
  const { loadAdapters } = await import("../../src/config/adapters");
  const { DEFAULTS } = await import("../../src/config/config");
  const adapters = await loadAdapters(DEFAULTS);
  const noop = () => {};
  const pane = new PtyPane({ id: "p1", cwd: sb.root, harness: "claude-code", createdBy: "user", cols: 80, rows: 24 }, { output: noop, exit: noop, title: noop });
  const tick = new Detector(() => adapters).tick([pane], () => false); // now awaiting `ps`
  pane.dispose();
  expect(await tick).toEqual([]);
});
