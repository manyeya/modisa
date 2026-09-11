// Agents need approval (from whoever is attached) to act on panes they didn't create.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { Screen, sandbox } from "../support/harness";
import { installFakeAgent } from "../support/fake-agent";

const sb = sandbox("permissions");
const S = "perm";
const cli = (...args: string[]) => sb.cli(S, args);
let ui: Screen;

beforeAll(async () => {
  await installFakeAgent(sb.root);
  ui = new Screen(["-s", S], sb.env, sb.root);
  await ui.until("attached", (s) => s.includes("SPACES"), 15000);
  await cli("pane", "split", "--name", "server", "echo server-up; sleep 600");
  await cli("agent", "spawn", "fakeagent", "--name", "coder");
}, 30000);

afterAll(async () => {
  ui?.close();
  await cli("kill", S);
  await sb.cleanup();
});

test("agents need approval to type into panes they didn't create", async () => {
  const panes = JSON.parse(await cli("pane", "list", "--json"));
  const coder = panes.find((p: any) => p.name === "coder").id;
  const asCoder = { SHEPHERD_PANE_ID: coder };
  // approve
  const allowed = sb.cli(S, ["pane", "keys", "@server", "C-c"], asCoder);
  await ui.until("permission prompt", (s) => s.includes('@coder wants to keys pane "server"') && s.includes("[y] allow"));
  ui.write("y");
  expect(await allowed).toBe("");
  // deny
  const denied = sb.cli(S, ["pane", "run", "@server", "echo nope"], asCoder);
  await ui.until("second prompt", (s) => s.includes('@coder wants to run pane "server"'));
  ui.write("n");
  expect(await denied).toContain("denied by user");
  // panes it created itself need no approval
  const own = (await sb.cli(S, ["pane", "split", "--name", "mine", "sleep 600"], asCoder)).trim();
  expect(await sb.cli(S, ["pane", "keys", own, "C-c"], asCoder)).toBe("");
  expect(await sb.cli(S, ["pane", "close", own], asCoder)).toBe("");
}, 30000);
