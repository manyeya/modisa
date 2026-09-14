// The plugin host: link validates plugin.json; linked plugins start from their argv in their own process group,
// log to a file and bind their connection; broken, crashing and wrong-protocol plugins show why; actions are
// callable with `shepherd plugin run`; stopping the session ends each plugin's whole group, even one that
// ignores TERM; unlink removes only the link.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { sandbox, startServer } from "../support/harness";

const sb = sandbox("plugin-host");
const S = "host";
const REPO = `${import.meta.dir}/../..`;
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
const list = async () => JSON.parse((await run("plugin", "list", "--json")).stdout) as any[];
const plugin = async (name: string) => (await list()).find((p) => p.name === name);
const until = async (what: string, ok: () => Promise<boolean>, ms = 15000) => {
  for (const end = Date.now() + ms; Date.now() < end; await Bun.sleep(100)) if (await ok()) return;
  throw new Error(`timed out waiting for ${what}: ${JSON.stringify(await list())}`);
};

beforeAll(async () => {
  // a well-behaved plugin with a grandchild: sh starts `sleep`, then execs a client that binds and offers actions
  await Bun.write(`${dir("probe")}/plugin.json`, JSON.stringify({ name: "probe", protocol: 1, run: ["sh", "start.sh"] }));
  await Bun.write(`${dir("probe")}/start.sh`, `sleep 300 &\necho $! > child.pid\necho "probe starting"\nexec bun client.ts\n`);
  await Bun.write(
    `${dir("probe")}/client.ts`,
    `import { connectUnix } from "${REPO}/src/protocol/transport";
const conn = await connectUnix(Bun.env.SHEPHERD_SOCKET!);
conn.onClose = () => process.exit(0);
conn.onMessage = (m) => {
  if (m.method !== "plugin.action" || m.id === undefined) return;
  if (m.params.action === "echo") conn.send({ jsonrpc: "2.0", id: m.id, result: m.params.params });
  else conn.send({ jsonrpc: "2.0", id: m.id, error: { code: -32000, message: "it broke on purpose" } });
};
console.log("hello", JSON.stringify(await conn.request("plugin.hello", { token: Bun.env.SHEPHERD_PLUGIN_TOKEN, actions: ["echo", "boom"] })));
`,
  );
  // one that ignores TERM, so only the KILL after the time limit ends it
  await Bun.write(`${dir("stubborn")}/plugin.json`, JSON.stringify({ name: "stubborn", protocol: 1, run: ["sh", "-c", "trap '' TERM; echo $$ > pid; while :; do sleep 1; done"] }));
  await Bun.write(`${dir("crashy")}/plugin.json`, JSON.stringify({ name: "crashy", protocol: 1, run: ["sh", "-c", "echo oops >&2; exit 3"] }));
  await Bun.write(`${dir("future")}/plugin.json`, JSON.stringify({ name: "future", protocol: 99, run: ["true"] }));
  await Bun.write(`${dir("broken")}/plugin.json`, JSON.stringify({ name: "broken", protocol: 1 }));

  for (const name of ["probe", "stubborn", "crashy", "future"]) expect((await run("plugin", "link", dir(name))).code).toBe(0);
  await Bun.$`ln -s ${dir("broken")} ${sb.root}/config/plugins/broken`; // the CLI refuses to link it (see below)
  await startServer(sb, S);
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
  expect(await plugin("probe")).toMatchObject({ source: "linked", status: "running", group: "running", actions: ["echo", "boom"] });
  const logs = await run("plugin", "logs", "probe");
  expect(logs.stdout).toContain("probe starting");
  expect(logs.stdout).toContain(`"name":"probe"`);

  await until("crashy to fail", async () => (await plugin("crashy"))?.status === "failed");
  expect(await plugin("crashy")).toMatchObject({ exitCode: 3, connected: false });
  expect((await run("plugin", "logs", "crashy")).stdout).toContain("oops");
  expect((await plugin("future")).error).toContain("protocol 99");
  expect((await plugin("broken")).error).toContain("run");
  expect((await run("plugin", "list")).stdout).toContain("probe");
}, 30000);

test("plugin run calls a connected plugin's action, and fails clearly otherwise", async () => {
  const echo = await run("plugin", "run", "probe", "echo", `{"x":1}`);
  expect(echo.code).toBe(0);
  expect(JSON.parse(echo.stdout)).toEqual({ x: 1 });
  const cases: [string[], number, string][] = [
    [["probe", "boom"], 1, "plugin_error"],
    [["probe", "nope"], 1, "no_such_action"],
    [["missing", "echo"], 1, "no_such_plugin"],
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

test("stopping the session ends each plugin's whole process group, even one that ignores TERM", async () => {
  await until("stubborn to write its pid", async () => Bun.file(`${dir("stubborn")}/pid`).exists());
  const grandchild = Number(await Bun.file(`${dir("probe")}/child.pid`).text());
  const stubborn = Number(await Bun.file(`${dir("stubborn")}/pid`).text());
  expect(alive(grandchild) && alive(stubborn)).toBe(true);
  await run("kill", S);
  for (const end = Date.now() + 6000; Date.now() < end && (alive(grandchild) || alive(stubborn)); ) await Bun.sleep(100);
  expect(alive(grandchild)).toBe(false);
  expect(alive(stubborn)).toBe(false);
}, 20000);

test("unlink removes the link, never the plugin's directory", async () => {
  expect((await run("plugin", "unlink", "probe")).code).toBe(0);
  expect(await Bun.file(`${dir("probe")}/plugin.json`).exists()).toBe(true);
  expect((await run("plugin", "unlink", "probe")).code).toBe(1);
});
