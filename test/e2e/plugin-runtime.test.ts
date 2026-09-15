// Plugins load at runtime: `plugin link` registers a plugin for every session and starts it in the running session it
// reaches, without a restart, and says whether it connected. Linking or starting again never makes a second run; a
// plugin that can't start is reported as linked-but-failed; with no session running it's only registered.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { sandbox, startServer } from "../support/harness";
import { cliResults } from "../../src/protocol/schema";

const sb = sandbox("plugin-runtime");
const S = "runtime";
const run = (...args: string[]) => sb.run(S, args);
const list = async () => sb.json<any[]>(S, ["plugin", "list"], { retry: "startup" });
const linkJson = async (dir: string, session = S) => {
  const r = await sb.run(session, ["plugin", "link", dir, "--json"]);
  const parsed = JSON.parse(r.stdout);
  expect(cliResults["plugin link"].safeParse(parsed).success).toBe(true);
  return { code: r.code, result: parsed };
};

beforeAll(async () => {
  await startServer(sb, S);
  expect((await run("plugin", "new", "demo", "--dir", `${sb.root}/demo`)).code).toBe(0);
  expect((await run("plugin", "new", "broken", "--dir", `${sb.root}/broken`)).code).toBe(0);
  await Bun.write(`${sb.root}/broken/plugin.json`, JSON.stringify({ name: "broken", protocol: 1, run: ["bun", "missing.ts"] }));
}, 30000);

afterAll(async () => {
  await run("kill", S);
  await sb.cleanup();
});

test("link starts the plugin in the running session and it's usable at once, no restart", async () => {
  const { code, result } = await linkJson(`${sb.root}/demo`);
  expect(code).toBe(0);
  expect(result).toMatchObject({ name: "demo", linked: true, alreadyLinked: false, start: { session: S, state: "started" } });
  const status = await run("plugin", "run", "demo", "status");
  expect(status.code).toBe(0);
  expect(JSON.parse(status.stdout)).toHaveProperty("agents");
  expect((await run("plugin", "link", `${sb.root}/demo`)).stdout).toContain("every session starts it");
}, 30000);

test("linking or starting again never makes a second run", async () => {
  const before = (await list()).filter((p) => p.name === "demo");
  expect(before).toHaveLength(1);
  const again = await linkJson(`${sb.root}/demo`);
  expect(again.code).toBe(0);
  expect(again.result).toMatchObject({ alreadyLinked: true, start: { state: "already-running", pid: before[0].pid } });
  const start = await run("plugin", "start", "demo", "--json");
  expect(start.code).toBe(1);
  expect(JSON.parse(start.stderr).error.code).toBe("already_running");
  const after = (await list()).filter((p) => p.name === "demo");
  expect(after).toHaveLength(1);
  expect(after[0]).toMatchObject({ pid: before[0].pid, connected: true });
});

test("a plugin that can't start is linked, and says why it failed", async () => {
  const { code, result } = await linkJson(`${sb.root}/broken`);
  expect(code).toBe(1);
  expect(result).toMatchObject({ name: "broken", linked: true, start: { state: "failed" } });
  expect(result.start.reason).toContain("exited");
  expect(result.start.log).toContain("broken");
  expect((await run("plugin", "link", `${sb.root}/broken`)).stdout).toContain("linked, but failed to start in session");
}, 30000);

test("with no session running, link only registers the plugin", async () => {
  expect((await run("plugin", "unlink", "demo")).code).toBe(0);
  const { code, result } = await linkJson(`${sb.root}/demo`, "nobody-here");
  expect(code).toBe(0);
  expect(result).toMatchObject({ linked: true, start: { session: "nobody-here", state: "not-started" } });
  expect(await Bun.file(`${sb.root}/state/nobody-here.sock`).exists()).toBe(false); // no session was started
  // and the running session can start it now it's linked
  expect((await run("plugin", "start", "demo")).code).toBe(0);
});

test("unlink stops it in the session and removes the link", async () => {
  const r = await run("plugin", "unlink", "demo");
  expect(r.code).toBe(0);
  expect(r.stdout).toContain(`stopped it in session ${S}`);
  expect((await list()).find((p) => p.name === "demo")).toMatchObject({ status: "stopped", connected: false });
});
