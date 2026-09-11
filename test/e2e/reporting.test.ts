// Integrations reporting to shepherd: a lifecycle report takes over a pane's state until it's
// released, and a reported session id brings the agent back into that exact session after a restart.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { sandbox, startServer } from "../support/harness";
import { installFakeAgent } from "../support/fake-agent";

const sb = sandbox("reporting");
const S = "rep";
const cli = (...args: string[]) => sb.cli(S, args);
const agentOf = async (name: string) => JSON.parse(await cli("agent", "list", "--json")).find((a: any) => a.name === name);

beforeAll(async () => {
  await installFakeAgent(sb.root);
  // the fake agent resumes a session the way real ones do: `fakeagent --resume <id>`
  await Bun.write(`${sb.root}/config/adapters/fakeagent.toml`, (await Bun.file(`${sb.root}/config/adapters/fakeagent.toml`).text()).replace('launch = "fakeagent"', 'launch = "fakeagent"\nresumeSession = "fakeagent --resume {id}"'));
  await startServer(sb, S);
  await cli("agent", "spawn", "fakeagent", "--name", "fake");
  await cli("wait", "@fake", "--state", "working", "--timeout", "10");
  await cli("wait", "@fake", "--state", "idle", "--timeout", "15");
}, 40000);

afterAll(async () => {
  await cli("kill", S);
  await sb.cleanup();
});

test("a lifecycle report is the pane's authority until released; unnamed reports are ignored", async () => {
  await cli("report", "@fake", "--state", "blocked"); // no --source: what old hooks send
  await Bun.sleep(700);
  expect((await agentOf("fake")).state).not.toBe("blocked");
  await cli("report", "@fake", "--source", "custom:test", "--state", "blocked", "--seq", "5");
  expect(await cli("wait", "@fake", "--state", "blocked", "--timeout", "5")).toBe("blocked");
  expect((await agentOf("fake")).source).toBe("hook");
  await cli("report", "@fake", "--source", "custom:test", "--state", "working", "--seq", "4"); // stale: ignored
  await Bun.sleep(700);
  expect((await agentOf("fake")).state).toBe("blocked");
  await cli("report", "@fake", "--source", "custom:test", "--release");
  await Bun.sleep(1200);
  expect(await agentOf("fake")).toMatchObject({ source: "screen" });
  expect((await agentOf("fake")).state).not.toBe("blocked");
}, 20000);

test("a reported session id is resumed exactly after a server restart", async () => {
  await cli("report", "@fake", "--session-id", "sess-42");
  expect(JSON.parse(await cli("pane", "list", "--json")).find((p: any) => p.name === "fake").session).toMatchObject({ agent: "fakeagent", id: "sess-42" });
  await Bun.sleep(1500); // debounced save
  await cli("restart");
  const launched = () => Bun.file(`${sb.root}/fakeagent.log`).text().catch(() => "");
  for (let i = 0; i < 120 && !(await launched()).includes("--resume"); i++) await Bun.sleep(250); // CI machines restart slowly
  expect(await launched()).toContain("--resume sess-42");
}, 60000);

test("agents' hooks report through `shepherd hook`: sessions for most, state for lifecycle agents", async () => {
  const pane = JSON.parse(await cli("pane", "list", "--json")).find((p: any) => p.name === "fake").id;
  const hook = (agent: string, action: string, input: object) => {
    const p = Bun.spawn(["bun", `${import.meta.dir}/../../src/main.ts`, "hook", agent, action], {
      env: { ...sb.env, SHEPHERD_PANE_ID: pane, SHEPHERD_SOCKET: `${sb.root}/state/${S}.sock` }, stdin: new TextEncoder().encode(JSON.stringify(input)), stdout: "pipe", stderr: "pipe",
    });
    return Promise.all([new Response(p.stdout).text(), p.exited]);
  };
  // session-only (Codex): the id is stored, the output stays empty (agents may read it), state untouched
  const [out, code] = await hook("codex", "session", { hook_event_name: "SessionStart", session_id: "cx-1", transcript_path: "/t" });
  expect([out, code]).toEqual(["", 0]);
  expect(JSON.parse(await cli("pane", "list", "--json")).find((p: any) => p.id === pane).session).toMatchObject({ agent: "codex", id: "cx-1" });
  // lifecycle (Kimi): the hook's state is the pane's state
  await hook("kimi", "blocked", { session_id: "k-1" });
  expect(await cli("wait", "@fake", "--state", "blocked", "--timeout", "5")).toBe("blocked");
  await cli("report", "@fake", "--source", "shepherd:kimi", "--release");
  // outside a pane it does nothing
  const p = Bun.spawn(["bun", `${import.meta.dir}/../../src/main.ts`, "hook", "kimi", "blocked"], { env: sb.env, stdin: new TextEncoder().encode("{}"), stdout: "pipe" });
  expect([await new Response(p.stdout).text(), await p.exited]).toEqual(["", 0]);
}, 30000);
