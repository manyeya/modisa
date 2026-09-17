// The shared JSON CLI helper tests lean on: it retries only while a session isn't answering yet, fails at once on
// anything else, and says what went wrong (command, exit status, stdout, stderr).
import { test, expect, beforeAll, afterAll } from "bun:test";
import { sandbox, startServer } from "../support/harness";

const sb = sandbox("harness-json");
const S = "hj";

beforeAll(async () => {
  await startServer(sb, S);
}, 30000);
afterAll(async () => {
  await sb.run(S, ["kill", S]);
  await sb.cleanup();
});

test("a good call is parsed", async () => {
  const panes = await sb.json<any[]>(S, ["pane", "list"]);
  expect(panes.some((p) => p.id === "p1")).toBe(true);
});

test("an unreachable session fails at once without retry, and after the deadline with it, saying so with stderr", async () => {
  const quick = Date.now();
  const once = await sb.json("nobody-home", ["pane", "list"]).then(() => "ok", (e: Error) => e.message);
  expect(Date.now() - quick).toBeLessThan(3000);
  expect(once).toContain("modisa -s nobody-home pane list --json: unreachable");
  expect(once).toContain("exit status: 3");
  expect(once).toContain("stderr:");

  const waited = Date.now();
  const retried = await sb.json("nobody-home", ["pane", "list"], { retry: "startup", ms: 1200 }).then(() => "ok", (e: Error) => e.message);
  expect(Date.now() - waited).toBeGreaterThanOrEqual(1200);
  expect(retried).toContain("still unreachable after 1200ms");
  expect(retried).toMatch(/"code":\s*"unreachable"/);
});

test("anything but a session still starting fails at once, even with startup retry", async () => {
  const started = Date.now();
  const usage = await sb.json(S, ["pane", "read"], { retry: "startup", ms: 10000 }).then(() => "ok", (e: Error) => e.message); // no target
  expect(Date.now() - started).toBeLessThan(4000);
  expect(usage).toContain("failed");
  expect(usage).not.toContain("exit status: 0");
});
