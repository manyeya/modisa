// `modisa snapshot` and `modisa tab list`: what clients draw, read without attaching. Nothing changes for reading it:
// no client.attached event, the same area (so no pane is resized) and the same clients.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { sandbox, startServer } from "../support/harness";
import { connectUnix } from "../../src/protocol/transport";
import { cliResults, results } from "../../src/protocol/schema";
import type { Conn } from "../../src/protocol/conn";

const sb = sandbox("snapshot");
const S = "snap";
const area = { x: 26, y: 1, w: 90, h: 30 }; // what the attached client below draws in
let client: Conn;
const events: any[] = [];

beforeAll(async () => {
  await startServer(sb, S);
  await sb.cli(S, ["pane", "split", "--name", "job", "sleep 120"]);
  await sb.cli(S, ["tab", "create", "logs", "--command", "sleep 120"]);
  client = await connectUnix(`${sb.root}/state/${S}.sock`);
  client.onMessage = (m) => m.method === "event" && events.push(m.params);
  await client.request("attach", { area });
  await client.request("events.subscribe", {});
}, 30000);

afterAll(async () => {
  client?.close();
  await sb.run(S, ["kill", S]);
  await sb.cleanup();
});

test("snapshot prints session.info's snapshot as JSON, and reading it changes nothing", async () => {
  const before = await sb.json(S, ["pane", "list"]);
  const r = await sb.run(S, ["snapshot"]); // JSON with no --json
  expect(r.code).toBe(0);
  const snap = JSON.parse(r.stdout);
  const parsed = results["session.info"].safeParse(snap);
  expect(parsed.success, parsed.error?.message).toBe(true);
  expect(snap).toMatchObject({ session: S, clients: 1, area, active: 0 });
  expect(snap.workspaces[0].tabs.map((t: any) => t.name ?? null)).toEqual([null, "logs"]);
  expect(snap.workspaces[0].tabs[0].tree).toMatchObject({ dir: "row", a: { pane: "p1" }, b: { pane: "p2" } });
  expect(snap.panes.map((p: any) => p.id)).toEqual(["p1", "p2", "p3"]);

  await Bun.sleep(300);
  expect(events.filter((e) => e.type === "client.attached")).toEqual([]);
  const after = await sb.json(S, ["pane", "list"]);
  expect(after.map((p: any) => [p.id, p.cols, p.rows])).toEqual(before.map((p: any) => [p.id, p.cols, p.rows])); // nothing resized
  const again = JSON.parse((await sb.run(S, ["snapshot"])).stdout);
  expect(again).toMatchObject({ clients: 1, area });
}, 20000);

test("tab list: every space's tabs, and which one is on screen", async () => {
  const tabs = await sb.json<any[]>(S, ["tab", "list"]);
  const parsed = cliResults["tab list"].safeParse(tabs);
  expect(parsed.success, parsed.error?.message).toBe(true);
  expect(tabs.map((t) => [t.name ?? null, t.panes, t.current])).toEqual([[null, ["p1", "p2"], false], ["logs", ["p3"], true]]);
  expect(tabs[1]).toMatchObject({ workspaceId: "w1", focused: "p3", zoomed: false, active: true });
  const text = (await sb.run(S, ["tab", "list"])).stdout.split("\n");
  expect(text[0]).toMatch(/^ID\s+NAME\s+WORKSPACE\s+PANES\s+FOCUSED\s+ZOOMED\s+CURRENT$/);
  expect(text.find((l) => l.includes("logs"))).toMatch(/\bp3\b.*\*$/);
  expect(text.find((l) => l.includes("p1,p2"))).not.toContain("*");
}, 20000);
