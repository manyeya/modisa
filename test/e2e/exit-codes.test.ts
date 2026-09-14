// The CLI's exit status tells scripts what happened: failures exit nonzero, each kind with its own status
// and a stable error code under --json, and `wait --exited` passes along the pane's exit code. A child's own
// code can equal one of shepherd's; stdout (the child's result) and stderr (shepherd's error) tell them apart.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { sandbox, startServer } from "../support/harness";

const sb = sandbox("exit-codes");
const S = "exit";
const run = (...args: string[]) => sb.run(S, args);
let server: Bun.Subprocess;

beforeAll(async () => {
  server = await startServer(sb, S);
}, 15000);

afterAll(async () => {
  await sb.cli(S, ["kill", S]);
  await server?.exited;
  await sb.cleanup();
});

test("success exits 0", async () => {
  expect((await run("pane", "list")).code).toBe(0);
});

test("an unknown pane exits 1, with a stable code under --json", async () => {
  expect(await run("pane", "read", "no-such-pane")).toMatchObject({ code: 1, out: "shepherd: no such pane: no-such-pane" });
  const j = await run("pane", "read", "no-such-pane", "--json");
  expect(j).toMatchObject({ code: 1, stdout: "" });
  expect(JSON.parse(j.stderr).error.code).toBe("no_such_pane");
});

test("an unknown command exits 2", async () => {
  const r = await run("pane", "frobnicate");
  expect(r.code).toBe(2);
  expect(r.out).toContain("unknown command: pane frobnicate");
});

test("a wait that times out exits 124", async () => {
  expect(await run("wait", "p1", "--match", "never-printed-xyz", "--timeout", "1")).toMatchObject({ code: 124, out: "shepherd: timeout" });
});

test("an unreachable server exits 3", async () => {
  const r = await sb.run("no-server-here", ["pane", "list"]);
  expect(r.code).toBe(3);
  expect(r.out).toContain('no shepherd server for session "no-server-here"');
  expect(JSON.parse((await sb.run("no-server-here", ["pane", "list", "--json"])).stderr).error.code).toBe("unreachable");
});

test("wait --exited exits with the pane's exit code, in plain and --json output", async () => {
  await run("pane", "split", "--name", "job", "exit 7");
  expect(await run("wait", "@job", "--exited", "--timeout", "10")).toMatchObject({ code: 7, stdout: "exited 7", stderr: "" });
  const j = await run("wait", "@job", "--exited", "--json");
  expect(j).toMatchObject({ code: 7, stderr: "" });
  expect(JSON.parse(j.stdout)).toEqual({ exitCode: 7 });
});

test("a child exiting 1, 2, 3 or 124 is told apart from shepherd's own failures by stdout vs stderr", async () => {
  for (const code of [1, 2, 3, 124]) {
    await run("pane", "split", "--name", `child${code}`, `exit ${code}`);
    const j = await run("wait", `@child${code}`, "--exited", "--timeout", "10", "--json");
    expect(j).toMatchObject({ code, stderr: "" });
    expect(JSON.parse(j.stdout)).toEqual({ exitCode: code });
  }
  const own: [string, string[], number, string][] = [
    [S, ["pane", "read", "nope", "--json"], 1, "no_such_pane"],
    [S, ["pane", "frobnicate", "--json"], 2, "usage"],
    ["no-server-here", ["pane", "list", "--json"], 3, "unreachable"],
    [S, ["wait", "p1", "--match", "never-printed-xyz", "--timeout", "1", "--json"], 124, "timeout"],
  ];
  for (const [session, args, code, error] of own) {
    const r = await sb.run(session, args);
    expect(r).toMatchObject({ code, stdout: "" });
    expect(JSON.parse(r.stderr).error.code).toBe(error);
  }
}, 30000);

test("a wait whose pane closes before the condition is met exits 1 with pane_gone", async () => {
  await run("pane", "split", "--name", "sleeper", "sleep 30");
  const waiting = run("wait", "@sleeper", "--match", "never-printed-xyz", "--json");
  await Bun.sleep(1000);
  await run("pane", "close", "@sleeper");
  const r = await waiting;
  expect(r).toMatchObject({ code: 1, stdout: "" });
  expect(JSON.parse(r.stderr).error.code).toBe("pane_gone");
});

test("wait --exited on a running pane that gets closed exits 1 with pane_gone, not the close's SIGHUP", async () => {
  for (const json of [false, true]) {
    await run("pane", "split", "--name", "doomed", "sleep 100");
    const waiting = run("wait", "@doomed", "--exited", "--timeout", "10", ...(json ? ["--json"] : []));
    await Bun.sleep(1000);
    await run("pane", "close", "@doomed");
    const r = await waiting;
    expect(r).toMatchObject({ code: 1, stdout: "" });
    if (json) expect(JSON.parse(r.stderr).error.code).toBe("pane_gone");
    else expect(r.stderr).toContain("closed before the wait was met");
  }
}, 20000);

test("a pane that exits and then closes still gives wait --exited its exit code", async () => {
  const shell = (await run("pane", "split")).stdout; // a shell pane closes itself when it exits
  const waiting = run("wait", shell, "--exited", "--timeout", "10");
  await Bun.sleep(1000);
  await run("pane", "run", shell, "exit 5");
  expect(await waiting).toMatchObject({ code: 5, stdout: "exited 5" });
});
