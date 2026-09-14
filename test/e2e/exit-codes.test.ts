// The CLI's exit status tells scripts what happened: failures exit nonzero, each kind with its own status
// and a stable error code under --json, and `wait --exited` passes along the pane's exit code.
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
  const r = await run("pane", "read", "no-such-pane");
  expect(r).toEqual({ code: 1, out: "shepherd: no such pane: no-such-pane" });
  const j = await run("pane", "read", "no-such-pane", "--json");
  expect(j.code).toBe(1);
  expect(JSON.parse(j.out).error.code).toBe("no_such_pane");
});

test("an unknown command exits 2", async () => {
  const r = await run("pane", "frobnicate");
  expect(r.code).toBe(2);
  expect(r.out).toContain("unknown command: pane frobnicate");
});

test("a wait that times out exits 124", async () => {
  const r = await run("wait", "p1", "--match", "never-printed-xyz", "--timeout", "1");
  expect(r).toEqual({ code: 124, out: "shepherd: timeout" });
});

test("an unreachable server exits 3", async () => {
  const r = await sb.run("no-server-here", ["pane", "list"]);
  expect(r.code).toBe(3);
  expect(r.out).toContain('no shepherd server for session "no-server-here"');
  expect(JSON.parse((await sb.run("no-server-here", ["pane", "list", "--json"])).out).error.code).toBe("unreachable");
});

test("wait --exited exits with the pane's exit code", async () => {
  await run("pane", "split", "--name", "job", "exit 7");
  expect(await run("wait", "@job", "--exited", "--timeout", "10")).toEqual({ code: 7, out: "exited 7" });
});
