// Where a pane is and what runs in it, from the CLI: pane layout, neighbor and edges, worked out from a session.info
// snapshot that changes nothing (no attach, no event, no new area), agree with the server's own idea of neighbours;
// pane process-info gives the pane's pid, the job in the foreground of its terminal, and its shell's cwd.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { sandbox, startServer } from "../support/harness";
import { connectUnix } from "../../src/protocol/transport";
import { cliResults } from "../../src/protocol/schema";

const sb = sandbox("pane-info");
const S = "info";
const run = (...args: string[]) => sb.run(S, args);
const json = (...args: string[]): Promise<any> => sb.json(S, args);
const focused = async () => (await json("pane", "list")).find((p: any) => p.focused)?.id;

beforeAll(async () => {
  await startServer(sb, S);
  // p1 | p2 over p3, on the 120×38 area at (0, 1) a server has before any client sets one
  await run("pane", "split", "--name", "two");
  await run("pane", "split", "--target", "p2", "--down");
}, 20000);

afterAll(async () => {
  await run("kill", S);
  await sb.cleanup();
});

test("pane layout: every box in the pane's tab, and which are shown", async () => {
  const layout = await json("pane", "layout");
  expect(cliResults["pane layout"].safeParse(layout).success).toBe(true);
  const { workspaceId, tabId } = (await json("pane", "list"))[0];
  expect(layout).toEqual({
    pane: "p1", workspaceId, tabId, area: { x: 0, y: 1, w: 120, h: 38 }, focused: "p1", zoomed: false,
    panes: [
      { id: "p1", x: 0, y: 1, w: 60, h: 38, shown: true },
      { id: "p2", name: "two", x: 60, y: 1, w: 60, h: 19, shown: true },
      { id: "p3", x: 60, y: 20, w: 60, h: 19, shown: true },
    ],
  });
  // from inside a pane it's that pane's; a zoomed tab shows only its focused pane
  expect((await sb.json(S, ["pane", "layout"], { env: { MODISA_PANE_ID: "p3" } })).pane).toBe("p3");
  await run("pane", "zoom", "p3", "--on");
  expect((await run("pane", "layout", "@two")).out).toBe(["ID  NAME  X   Y   W   H   SHOWN  FOCUSED", "p1        0   1   60  38  no", "p2  @two  60  1   60  19  no", "p3        60  20  60  19  yes    *"].join("\n"));
  await run("pane", "zoom", "p3", "--off");
  await run("pane", "focus", "p1");
  // another tab's pane: its own tab
  const p4 = (await run("tab", "create")).out;
  expect((await json("pane", "layout", p4)).panes).toEqual([{ id: p4, x: 0, y: 1, w: 120, h: 38, shown: true }]);
  await run("pane", "close", p4);
  expect(await run("pane", "layout", "nope")).toMatchObject({ code: 1, out: "modisa: no such pane: nope" });
}, 30000);

test("pane neighbor and pane edges agree with the server's neighbours", async () => {
  expect((await run("pane", "neighbor", "p3", "--direction", "up")).out).toBe("p2");
  expect((await run("pane", "neighbor", "p3", "--direction", "left")).out).toBe("p1");
  expect(await json("pane", "neighbor", "@two", "--direction", "left")).toMatchObject({ id: "p1", focused: true, tabId: expect.any(String) });
  // focusing that way goes to the same pane
  for (const [from, d] of [["p1", "right"], ["p2", "down"], ["p3", "left"]]) {
    const n = (await run("pane", "neighbor", from!, "--direction", d!)).out;
    await run("pane", "focus", from!, "--direction", d!);
    expect(await focused()).toBe(n);
  }
  const none = await run("pane", "neighbor", "p1", "--direction", "left", "--json");
  expect(none.code).toBe(1);
  expect(JSON.parse(none.stderr).error).toEqual({ code: "no_such_pane", message: "modisa: no pane left of p1" });
  expect((await run("pane", "neighbor", "p1")).code).toBe(2);
  expect((await run("pane", "neighbor", "p1", "--direction", "sideways")).code).toBe(2);

  const edges = await json("pane", "edges", "two");
  expect(edges).toEqual({ pane: "p2", left: "p1", right: null, up: null, down: "p3" });
  expect(cliResults["pane edges"].safeParse(edges).success).toBe(true);
  expect((await run("pane", "edges", "p1")).out.split("\n").map((l) => l.trimEnd())).toEqual(["SIDE   PANE", "left   (edge)", "right  p2", "up     (edge)", "down   (edge)"]);
}, 30000);

test("the snapshot they come from changes nothing: no attach, no event, no area", async () => {
  const conn = await connectUnix(`${sb.root}/state/${S}.sock`);
  const seen: any[] = [];
  conn.onMessage = (m) => m.method === "event" && seen.push(m.params);
  await conn.request("events.subscribe", {});
  const before = await conn.request<any>("session.info", { snapshot: true });
  for (const args of [["pane", "layout"], ["pane", "edges"], ["pane", "neighbor", "--direction", "right"]]) expect((await run(...args)).code).toBe(0);
  const after = await conn.request<any>("session.info", { snapshot: true });
  conn.close();
  expect(after).toEqual(before);
  expect(after.clients).toBe(0);
  expect(seen).toEqual([]);
}, 30000);

test("pane process-info: the pane's pid, the job in its foreground, and where its shell is", async () => {
  // a command pane's pid is its shell's
  const job = (await run("pane", "split", "--name", "job", "echo pid=$$; sleep 300")).out;
  const pid = Number(/pid=(\d+)/.exec((await run("wait", job, "--match", "pid=\\d+", "--timeout", "10")).out)![1]);
  expect((await json("pane", "process-info", job)).pid).toBe(pid);

  // a shell running a job somewhere else
  await Bun.$`mkdir -p ${sb.root}/elsewhere`.quiet();
  await run("pane", "run", "p1", `cd ${sb.root}/elsewhere && sleep 301`);
  let info: any;
  for (let i = 0; i < 50 && !(info = await json("pane", "process-info", "p1")).foreground?.args.includes("sleep 301"); i++) await Bun.sleep(100);
  expect(cliResults["pane process-info"].safeParse(info).success).toBe(true);
  expect(info.pane).toBe("p1");
  expect(info.foreground.args).toBe("sleep 301");
  expect(info.foreground.pid).not.toBe(info.pid);
  expect(info.cwd).toEndWith("/elsewhere");
  expect((await run("pane", "process-info", "p1")).out).toBe(`pid ${info.pid}\nforeground ${info.foreground.pid} sleep 301\ncwd ${info.cwd}`);
  await run("pane", "keys", "p1", "C-c");

  // an exited pane has only the pid it had; with no target it's the focused pane, as debug detect's is
  const done = (await run("pane", "split", "--name", "done", "exit 0")).out;
  await run("wait", done, "--exited", "--timeout", "10");
  expect(Object.keys(await json("pane", "process-info", done))).toEqual(["pane", "pid"]);
  expect((await json("pane", "process-info")).pane).toBe(await focused());
  expect((await json("debug", "detect")).pane).toBe(await focused());
}, 30000);
