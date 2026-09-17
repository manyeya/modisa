// The plugin host: link validates plugin.json; plugins start from their argv in their own process group, log, and
// bind their connection; broken, crashing, missing and wrong-protocol plugins show why; actions are callable, and a
// slow or silent one times out with its outcome unknown and nothing left pending; a run's token stops binding once
// the run is stopped, unlinked or has exited; a bound connection can't claim to be a pane; stopping the session ends
// each plugin's whole group, even one that ignores TERM.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { sandbox, startServer } from "../support/harness";
import { results } from "../../src/protocol/schema";
import { connectUnix } from "../../src/protocol/transport";

const sb = sandbox("plugin-host");
const S = "host";
const REPO = `${import.meta.dir}/../..`;
const sock = `${sb.root}/state/${S}.sock`;
const run = (...args: string[]) => sb.run(S, args);
const dir = (name: string) => `${sb.root}/plugins-src/${name}`;
const alive = (pid: number) => {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
};
const list = async () => sb.json<any[]>(S, ["plugin", "list"], { retry: "startup" });
const plugin = async (name: string) => (await list()).find((p) => p.name === name);
const until = async (what: string, ok: () => Promise<boolean>, ms = 15000) => {
  for (const end = Date.now() + ms; Date.now() < end; await Bun.sleep(100)) if (await ok()) return;
  throw new Error(`timed out waiting for ${what}: ${JSON.stringify(await list())}`);
};
const pidDead = async (file: string, ms = 6000) => {
  const pid = Number(await Bun.file(file).text());
  for (const end = Date.now() + ms; Date.now() < end && alive(pid); ) await Bun.sleep(100);
  return !alive(pid);
};
const token = (name: string) => Bun.file(`${dir(name)}/token`).text().then((t) => t.trim());
// try to bind a fresh connection with a token: "bound", or the error code
async function helloWith(t: string) {
  const conn = await connectUnix(sock);
  try {
    await conn.request("plugin.hello", { token: t });
    return "bound";
  } catch (e) {
    return (e as { code?: string }).code;
  } finally {
    conn.close();
  }
}

beforeAll(async () => {
  // a well-behaved plugin with a grandchild: sh starts `sleep`, then execs a client that binds and offers actions
  await Bun.write(`${dir("probe")}/plugin.json`, JSON.stringify({ name: "probe", protocol: 1, run: ["sh", "start.sh"] }));
  await Bun.write(`${dir("probe")}/start.sh`, `sleep 300 &\necho $! > child.pid\necho "$MODISA_PLUGIN_TOKEN" > token\necho "probe starting"\nexec bun client.ts\n`);
  await Bun.write(
    `${dir("probe")}/client.ts`,
    `import { connectUnix } from "${REPO}/src/protocol/transport";
const conn = await connectUnix(Bun.env.MODISA_SOCKET!);
let hungUp = false;
conn.onClose = () => { if (!hungUp) process.exit(0); };
const reply = (id: number, x: object) => conn.send({ jsonrpc: "2.0", id, ...x } as any);
conn.onMessage = async (m) => {
  if (m.method === "plugin.cancel") return console.log("cancel", m.params.invocation);
  if (m.method !== "plugin.action" || m.id === undefined) return;
  const { action, params } = m.params;
  if (action === "echo") reply(m.id, { result: params });
  else if (action === "slow") setTimeout(() => reply(m.id!, { result: "finished late" }), 1500);
  else if (action === "silent") return; // never answers
  else if (action === "impersonate") {
    const outcome = await conn.request("pane.keys", { caller: params.pane, target: params.pane, keys: ["x"] }).then(() => "accepted", (e) => e.code);
    reply(m.id, { result: { outcome } });
  } else if (action === "hangup") {
    reply(m.id, { result: "bye" });
    hungUp = true;
    setTimeout(() => conn.close(), 100); // the process stays; only its connection goes
  } else reply(m.id, { error: { code: -32000, message: "it broke on purpose" } });
};
console.log("hello", JSON.stringify(await conn.request("plugin.hello", { token: Bun.env.MODISA_PLUGIN_TOKEN, actions: ["echo", "boom", "slow", "silent", "impersonate", "hangup"] })));
setInterval(() => {}, 1 << 30);
`,
  );
  // one that ignores TERM, so only the KILL after the time limit ends it
  await Bun.write(`${dir("stubborn")}/plugin.json`, JSON.stringify({ name: "stubborn", protocol: 1, run: ["sh", "-c", "trap '' TERM; echo $$ > pid; while :; do sleep 1; done"] }));
  await Bun.write(`${dir("sleepy")}/plugin.json`, JSON.stringify({ name: "sleepy", protocol: 1, run: ["sh", "-c", `echo $$ > pid; echo "$MODISA_PLUGIN_TOKEN" > token; while :; do sleep 1; done`] }));
  await Bun.write(`${dir("crashy")}/plugin.json`, JSON.stringify({ name: "crashy", protocol: 1, run: ["sh", "-c", `echo "$MODISA_PLUGIN_TOKEN" > token; echo oops >&2; exit 3`] }));
  await Bun.write(`${dir("missing")}/plugin.json`, JSON.stringify({ name: "missing", protocol: 1, run: ["no-such-program-for-modisa"] }));
  await Bun.write(`${dir("future")}/plugin.json`, JSON.stringify({ name: "future", protocol: 99, run: ["true"] }));
  await Bun.write(`${dir("broken")}/plugin.json`, JSON.stringify({ name: "broken", protocol: 1 }));

  for (const name of ["probe", "stubborn", "sleepy", "crashy", "missing", "future"]) expect((await run("plugin", "link", dir(name))).code).toBe(0);
  await Bun.$`ln -s ${dir("broken")} ${sb.root}/config/plugins/broken`; // the CLI refuses to link it (see below)
  await startServer(sb, S, { MODISA_PLUGIN_INVOKE_MS: "1000" });
}, 30000);

afterAll(async () => {
  await run("kill", S);
  await sb.cleanup();
});

test("link checks plugin.json and says what's wrong", async () => {
  const r = await run("plugin", "link", dir("broken"));
  expect(r.code).toBe(1);
  expect(r.stderr).toContain("plugin.json: run:");
  expect((await run("plugin", "link", `${sb.root}/nowhere`)).code).toBe(1);
});

test("linked plugins start, log, and bind their connection; the rest show why they aren't running", async () => {
  await until("probe to connect", async () => (await plugin("probe"))?.connected === true);
  expect(results["plugin.list"].safeParse(await list()).success).toBe(true);
  expect(await plugin("probe")).toMatchObject({ source: "linked", status: "running", group: "running", invocations: 0 });
  const logs = await run("plugin", "logs", "probe");
  expect(logs.stdout).toContain("probe starting");
  expect(logs.stdout).toContain(`"name":"probe"`);

  await until("crashy to fail", async () => (await plugin("crashy"))?.status === "failed");
  expect(await plugin("crashy")).toMatchObject({ exitCode: 3, connected: false });
  expect((await run("plugin", "logs", "crashy")).stdout).toContain("oops");
  expect(await plugin("missing")).toMatchObject({ status: "failed" });
  expect((await plugin("missing")).error).toContain("no-such-program-for-modisa");
  expect((await plugin("future")).error).toContain("protocol 99");
  expect((await plugin("broken")).error).toContain("run");
}, 30000);

test("plugin run calls a connected plugin's action, and fails clearly otherwise", async () => {
  const echo = await run("plugin", "run", "probe", "echo", `{"x":1}`);
  expect(echo.code).toBe(0);
  expect(JSON.parse(echo.stdout)).toEqual({ x: 1 });
  const cases: [string[], number, string][] = [
    [["probe", "boom"], 1, "plugin_error"],
    [["probe", "nope"], 1, "no_such_action"],
    [["nobody", "echo"], 1, "no_such_plugin"],
    [["crashy", "echo"], 1, "plugin_unavailable"],
    [["probe", "echo", "{not json"], 2, "usage"],
  ];
  for (const [args, code, error] of cases) {
    const r = await run("plugin", "run", ...args, "--json");
    expect(r.code).toBe(code);
    expect(JSON.parse(r.stderr).error.code).toBe(error);
  }
  expect((await run("plugin", "run", "probe", "boom")).stderr).toContain("it broke on purpose");
});

test("a slow action times out with its outcome unknown: the plugin is told to cancel, and its late reply is logged", async () => {
  const r = await run("plugin", "run", "probe", "slow");
  expect(r.code).toBe(124);
  expect(r.stderr).toContain("outcome is unknown");
  await until("the late reply", async () => (await run("plugin", "logs", "probe")).stdout.includes("finished late"));
  const logs = (await run("plugin", "logs", "probe")).stdout;
  expect(logs).toContain("late reply");
  expect(logs).toMatch(/cancel probe-\d+/);
  expect((await run("plugin", "run", "probe", "echo", `{"still":"fine"}`)).code).toBe(0);
});

test("an action that never answers leaves nothing pending, however often it's called", async () => {
  for (let i = 0; i < 5; i++) expect((await run("plugin", "run", "probe", "silent")).code).toBe(124);
  expect((await plugin("probe")).invocations).toBe(0);
}, 30000);

test("a plugin's bound connection can't claim to be a pane", async () => {
  // (a second, unbound connection would be trusted as the local user: this checks only the bound one)
  const r = await run("plugin", "run", "probe", "impersonate", `{"pane":"p1"}`);
  expect(JSON.parse(r.stdout)).toEqual({ outcome: "invalid_params" });
});

test("a run that exited can't be bound again, and stopping it when nothing is left is harmless", async () => {
  expect(await helloWith(await token("crashy"))).toBe("plugin_unavailable");
  const stopped = await run("plugin", "stop", "crashy", "--json");
  expect(stopped.code).toBe(0);
  expect(JSON.parse(stopped.stdout)).toMatchObject({ status: "failed", group: "gone" });
});

test("a plugin whose connection closes is unavailable, though its process runs on", async () => {
  expect((await run("plugin", "run", "probe", "hangup")).code).toBe(0);
  await until("probe to disconnect", async () => (await plugin("probe"))?.connected === false);
  expect(await plugin("probe")).toMatchObject({ status: "running" });
  const r = await run("plugin", "run", "probe", "echo", "--json");
  expect(JSON.parse(r.stderr).error.code).toBe("plugin_unavailable");
});

test("stopping a plugin revokes its run: the old token no longer binds and its actions are unavailable; start makes a new run", async () => {
  const old = await token("probe");
  expect((await run("plugin", "stop", "probe")).code).toBe(0);
  expect(await helloWith(old)).toBe("plugin_unavailable");
  expect(JSON.parse((await run("plugin", "run", "probe", "echo", "--json")).stderr).error.code).toBe("plugin_unavailable");

  expect((await run("plugin", "start", "probe")).code).toBe(0);
  await until("probe to connect again", async () => (await plugin("probe"))?.connected === true);
  expect(await token("probe")).not.toBe(old);
  expect((await run("plugin", "run", "probe", "echo", "{}")).code).toBe(0);
}, 30000);

test("unlink stops the plugin in this session, revokes its token, and never removes its directory", async () => {
  await until("sleepy to write its pid", async () => Bun.file(`${dir("sleepy")}/token`).exists());
  const old = await token("sleepy");
  const r = await run("plugin", "unlink", "sleepy");
  expect(r.code).toBe(0);
  expect(r.stdout).toContain("stopped it in session");
  expect(r.stdout).toContain("other running sessions keep");
  expect(await pidDead(`${dir("sleepy")}/pid`)).toBe(true);
  expect(await plugin("sleepy")).toMatchObject({ status: "stopped" });
  expect(await helloWith(old)).toBe("plugin_unavailable");
  expect(await Bun.file(`${dir("sleepy")}/plugin.json`).exists()).toBe(true);
  expect((await run("plugin", "unlink", "sleepy")).code).toBe(1);
}, 20000);

test("stopping the session ends each plugin's whole process group, even one that ignores TERM", async () => {
  await until("stubborn to write its pid", async () => Bun.file(`${dir("stubborn")}/pid`).exists());
  const grandchild = `${dir("probe")}/child.pid`;
  expect(alive(Number(await Bun.file(grandchild).text()))).toBe(true);
  await run("kill", S);
  expect(await pidDead(grandchild)).toBe(true);
  expect(await pidDead(`${dir("stubborn")}/pid`)).toBe(true);
}, 20000);
