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
  await ui.until("attached", (s) => s.includes("AGENTS"), 15000);
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
  const asCoder = { MODISA_PANE_ID: coder };
  // approve
  const allowed = sb.cli(S, ["pane", "keys", "@server", "C-c"], asCoder);
  await ui.until("permission prompt", (s) => s.includes('@coder wants to keys pane "server"') && s.includes("Allow  y"));
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

// Typing at another agent is what the mailbox is for. If the refusal doesn't say so, agents keep
// reaching for `pane keys` and every exchange interrupts the user.
test("refusing to type at another agent points at send", async () => {
  const panes = JSON.parse(await cli("pane", "list", "--json"));
  const coder = panes.find((p: any) => p.name === "coder").id;
  // a second agent, so the target is an agent pane the caller didn't create
  await cli("agent", "spawn", "fakeagent", "--name", "reviewer");
  const denied = sb.cli(S, ["pane", "run", "@reviewer", "echo hi"], { MODISA_PANE_ID: coder });
  await ui.until("prompt for the agent pane", (s) => s.includes('@coder wants to run pane "reviewer"'));
  ui.write("n");
  expect(await denied).toContain('modisa send @reviewer');
  // messaging itself never asks
  expect(await sb.cli(S, ["send", "@reviewer", "over the mailbox"], { MODISA_PANE_ID: coder })).not.toContain("denied");
}, 30000);
