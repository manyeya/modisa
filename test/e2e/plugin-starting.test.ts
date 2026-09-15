// A plugin the server knows but hasn't launched yet is `starting`, never `failed`: a server holds its linked plugins
// before launching them (SHEPHERD_TEST_PLUGIN_HOLD, a test-only barrier), a client connecting at once sees them
// starting, and once released a good one runs and connects while a bad one fails with its reason. `plugin check`
// waits through starting instead of reporting it as a failure.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { MAIN, sandbox } from "../support/harness";
import { connectUnix } from "../../src/protocol/transport";

const sb = sandbox("plugin-starting");
const S = "starting";
const shepherd = (args: string[], env: Record<string, string> = {}) => sb.run("unused", args, env);
let server: Bun.Subprocess | undefined;

beforeAll(async () => {
  await Bun.$`mkdir -p ${sb.root}/config/plugins`.quiet();
  expect((await shepherd(["plugin", "new", "good", "--dir", `${sb.root}/good`])).code).toBe(0);
  expect((await shepherd(["plugin", "new", "bad", "--dir", `${sb.root}/bad`])).code).toBe(0);
  await Bun.write(`${sb.root}/bad/plugin.json`, JSON.stringify({ name: "bad", protocol: 99, run: ["bun", "plugin.ts"] }));
  for (const name of ["good", "bad"]) await Bun.$`ln -s ${sb.root}/${name} ${sb.root}/config/plugins/${name}`.quiet();
}, 30000);

afterAll(async () => {
  server?.kill("SIGKILL");
  await server?.exited;
  await sb.cleanup();
});

test("held before launch, linked plugins are starting from the first moment; released, the good one runs and the bad one fails with its reason", async () => {
  const barrier = `${sb.root}/release-server`;
  server = Bun.spawn(["bun", MAIN, "server", "-s", S], { env: { ...sb.env, SHEPHERD_TEST_PLUGIN_HOLD: barrier }, cwd: sb.root, stdout: "ignore", stderr: "ignore" });
  let conn;
  for (let i = 0; i < 1500 && !conn; i++) conn = await connectUnix(`${sb.root}/state/${S}.sock`).catch(() => Bun.sleep(10).then(() => undefined));
  expect(conn).toBeDefined();
  const plugins = async () => Object.fromEntries((await conn!.request<any[]>("plugin.list")).map((p) => [p.name, p]));

  // From the first moment, while held: a plugin is either not listed yet (the server lists its linked plugins just after
  // it starts listening) or starting, with no error — never failed. Both are seen starting before the release.
  const seenStarting = new Set<string>();
  for (let i = 0; i < 60 && seenStarting.size < 2; i++, await Bun.sleep(10)) {
    const now = await plugins();
    for (const name of ["good", "bad"]) {
      if (!now[name]) continue;
      expect(now[name]).toMatchObject({ status: "starting" });
      expect(now[name].error).toBeUndefined();
      seenStarting.add(name);
    }
  }
  expect([...seenStarting].sort()).toEqual(["bad", "good"]);
  for (let i = 0; i < 10; i++, await Bun.sleep(30)) {
    const now = await plugins();
    for (const name of ["good", "bad"]) expect(now[name]).toMatchObject({ status: "starting" }); // still held
  }
  // starting counts as already running: no second launch
  expect(await conn!.request("plugin.start", { name: "good" }).then(() => "started", (e) => e.code)).toBe("already_running");

  await Bun.write(barrier, "go");
  let seen: string[] = [];
  for (const end = Date.now() + 15000; Date.now() < end; await Bun.sleep(20)) {
    const now = await plugins();
    seen.push(now.good.status, now.bad.status);
    if (now.good.connected && now.bad.status === "failed") break;
  }
  const done = await plugins();
  expect(done.good).toMatchObject({ status: "running", connected: true });
  expect(done.bad).toMatchObject({ status: "failed" });
  expect(done.bad.error).toContain("protocol 99");
  expect(seen).not.toContain("exited");
  conn!.close();
}, 40000);

test("plugin check waits through starting, and passes once the plugin is released", async () => {
  const barrier = `${sb.root}/release-check`;
  const check = Bun.spawn(["bun", MAIN, "plugin", "check", `${sb.root}/good`], { env: { ...sb.env, SHEPHERD_TEST_PLUGIN_HOLD: barrier }, cwd: sb.root, stdout: "pipe", stderr: "pipe" });
  await Bun.sleep(2500);
  expect(check.exitCode).toBeNull(); // still waiting, not reporting a failure
  await Bun.write(barrier, "go");
  const [out, code] = await Promise.all([new Response(check.stdout).text(), check.exited]);
  expect(code, out).toBe(0);
  expect(out).toContain("✓ starts and connects");
}, 60000);
