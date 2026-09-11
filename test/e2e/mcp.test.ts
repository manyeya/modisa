// `shepherd mcp`: the socket API as MCP tools over stdio.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { MAIN, sandbox, startServer } from "../support/harness";
import { installFakeAgent } from "../support/fake-agent";

const sb = sandbox("mcp");
const S = "mcp";
let server: Bun.Subprocess;

beforeAll(async () => {
  await installFakeAgent(sb.root);
  server = await startServer(sb, S);
  await sb.cli(S, ["agent", "spawn", "fakeagent", "--name", "fake"]);
  await sb.cli(S, ["wait", "@fake", "--state", "working", "--timeout", "10"]);
}, 30000);

afterAll(async () => {
  await sb.cli(S, ["kill", S]);
  await server?.exited;
  await sb.cleanup();
});

test("the MCP server lists tools and calls through to the API", async () => {
  const p = Bun.spawn(["bun", MAIN, "mcp", "-s", S], { env: sb.env, cwd: sb.root, stdin: "pipe", stdout: "pipe", stderr: "pipe" });
  const send = (m: object) => (p.stdin.write(JSON.stringify(m) + "\n"), p.stdin.flush());
  send({ jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "t", version: "0" } } });
  send({ jsonrpc: "2.0", method: "notifications/initialized" });
  send({ jsonrpc: "2.0", id: 2, method: "tools/list" });
  send({ jsonrpc: "2.0", id: 3, method: "tools/call", params: { name: "list_agents", arguments: {} } });
  const reader = p.stdout.getReader();
  let buf = "";
  const dec = new TextDecoder();
  while (!buf.includes('"id":3')) buf += dec.decode((await reader.read()).value);
  p.kill();
  expect(buf).toContain('"name":"split_pane"');
  expect(buf).toContain('"name":"send_message"');
  expect(buf).toContain("fakeagent");
}, 15000);
