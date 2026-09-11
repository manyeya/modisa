// Agent messaging: a message is queued, then typed into the recipient once it's idle.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { sandbox, startServer } from "../support/harness";
import { installFakeAgent } from "../support/fake-agent";

const sb = sandbox("messaging");
const S = "msg";
const cli = (...args: string[]) => sb.cli(S, args);
let server: Bun.Subprocess;

beforeAll(async () => {
  await installFakeAgent(sb.root);
  server = await startServer(sb, S);
  await cli("agent", "spawn", "fakeagent", "--name", "fake");
  // let it start up (working) and finish (idle/done) so the message lands at its prompt
  await cli("wait", "@fake", "--state", "working", "--timeout", "10");
  await cli("wait", "@fake", "--state", "idle", "--timeout", "15");
}, 30000);

afterAll(async () => {
  await cli("kill", S);
  await server?.exited;
  await sb.cleanup();
});

test("a message is typed into the idle agent", async () => {
  expect(await cli("send", "@fake", "ping-123")).toContain("queued for @fake");
  await Bun.sleep(2500);
  expect(await cli("pane", "read", "@fake")).toContain("got: ping-123");
  expect(await cli("messages")).toContain("ping-123");
}, 15000);
