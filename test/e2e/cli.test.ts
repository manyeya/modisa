// The CLI agents use: split a command pane, wait for it, read it, send keys, close it; bad params
// are rejected by the API schema.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { Screen, sandbox, borders } from "../support/harness";

const sb = sandbox("cli");
const S = "cli";
const cli = (...args: string[]) => sb.cli(S, args);
let ui: Screen;

beforeAll(async () => {
  await Bun.$`mkdir -p ${sb.root}`.quiet();
  ui = new Screen(["-s", S], sb.env, sb.root);
  await ui.until("first pane", (s) => s.includes("AGENTS") && borders(s) === 1);
}, 20000);

afterAll(async () => {
  ui?.close();
  await cli("kill", S);
  await sb.cleanup();
});

test("split a command pane, wait for it, read it, send keys, close it", async () => {
  expect(await cli("pane", "split", "--name", "job", "echo hello-from-job; exit 3")).toMatch(/^p\d+$/);
  expect(await cli("wait", "@job", "--exited", "--timeout", "10")).toBe("exited 3");
  expect(await cli("pane", "read", "@job")).toContain("hello-from-job");
  await ui.until("pane shows exit code", (s) => s.includes("@job") && s.includes("[exited 3]"));
  expect(await cli("pane", "list")).toContain("@job");
  expect(await cli("pane", "keys", "@job", "x")).toBe("");
  expect(await cli("pane", "close", "@job")).toBe("");
  await ui.until("job pane closed", (s) => !s.includes("@job"));
}, 20000);

test("the API validates params with zod", async () => {
  expect(await cli("wait", "@nope", "--state", "sleepy")).toContain("invalid params");
});
