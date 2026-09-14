// A running server nobody can reach (an agent's sandbox blocking its socket) is never replaced: its
// socket stays, no second server starts, and the CLI says why. And restarts leave exactly one server.
import { test, expect, afterAll } from "bun:test";
import { sandbox, startServer } from "../support/harness";

const sb = sandbox("unreachable");
const S = "unr";
const sock = `${sb.root}/state/${S}.sock`;
const cli = (...args: string[]) => sb.cli(S, args);
const servers = async () => (await Bun.$`pgrep -f ${`server -s ${S}$`}`.quiet().nothrow().text()).split("\n").filter(Boolean);

afterAll(async () => {
  await cli("kill", S);
  await sb.cleanup();
});

test("a running server whose socket can't be reached is left alone, and the CLI says why", async () => {
  const server = await startServer(sb, S);
  // stands in for a sandbox: connecting to the socket path fails while the server keeps running
  await Bun.$`mv ${sock} ${sock}.real && touch ${sock}`;
  const blocked = await sb.run(S, ["pane", "list"]);
  expect(blocked.out).toContain("a sandbox is blocking it");
  expect(blocked.code).toBe(3);
  const ls = await sb.run(S, ["ls"]);
  expect(ls.out).toContain(`${S}\trunning (pid ${server.pid})`);
  expect(ls.code).toBe(3);
  expect(await Bun.file(sock).exists()).toBe(true); // not deleted as a dead server's
  expect(await servers()).toEqual([String(server.pid)]); // and no second server started
  await Bun.$`mv ${sock}.real ${sock}`;
  expect(await cli("pane", "list")).toContain("p1");
});

test("every restart leaves exactly one server", async () => {
  for (let i = 0; i < 4; i++) {
    expect(await cli("restart", S)).toContain(`restarted ${S}`);
    await Bun.sleep(1000); // a server that finished shutting down has exited by now
    expect(await servers()).toHaveLength(1);
  }
  expect(await cli("pane", "list")).toContain("p1");
}, 60000);
