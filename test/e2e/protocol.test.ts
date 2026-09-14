// The protocol plugins build on: every event is stamped (type, at, seq, epoch) and matches its published schema,
// subscribing can hand back a snapshot that sits exactly where the live stream starts, protocol.describe
// publishes the request and event schemas, and a restarted server has a new epoch.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { z } from "zod";
import { sandbox, startServer } from "../support/harness";
import { installFakeAgent } from "../support/fake-agent";
import { connectUnix } from "../../src/protocol/transport";
import { events as eventSchemas, envelope, PROTOCOL } from "../../src/protocol/schema";
import type { Conn } from "../../src/protocol/conn";

const sb = sandbox("protocol");
const S = "proto";
const sock = `${sb.root}/state/${S}.sock`;
const cli = (...args: string[]) => sb.cli(S, args);

// a raw socket client that records every event it's sent
async function subscriber(params: object) {
  const conn: Conn = await connectUnix(sock);
  const seen: any[] = [];
  conn.onMessage = (m) => m.method === "event" && seen.push(m.params);
  const sub = await conn.request<any>("events.subscribe", params);
  return { conn, seen, sub };
}
const until = async (what: string, ok: () => boolean, ms = 15000) => {
  for (const end = Date.now() + ms; Date.now() < end; await Bun.sleep(100)) if (ok()) return;
  throw new Error(`timed out waiting for ${what}`);
};

beforeAll(async () => {
  await installFakeAgent(sb.root);
  await startServer(sb, S);
}, 20000);

afterAll(async () => {
  await cli("kill", S);
  await sb.cleanup();
});

test("every event carries the envelope and matches its schema, in increasing seq order after the snapshot", async () => {
  const { conn, seen, sub } = await subscriber({ snapshot: true, output: true });
  // plain checks: toMatchObject with asymmetric matchers can write the matchers back into `sub`
  expect(sub.protocol).toBe(PROTOCOL);
  expect(typeof sub.epoch).toBe("string");
  expect(typeof sub.seq).toBe("number");
  expect(sub.panes.find((p: any) => p.id === "p1")).toMatchObject({ id: "p1", instance: expect.any(String) });

  await cli("pane", "split", "--name", "job", "echo out; exit 3"); // pane.created, pane.output, process.exited
  await cli("agent", "spawn", "fakeagent", "--name", "fake"); // agent.state
  await cli("wait", "@fake", "--state", "working", "--timeout", "10");
  await cli("wait", "@fake", "--state", "idle", "--timeout", "15");
  await cli("send", "@fake", "hello"); // message.sent, then message.delivered
  await conn.request("attach", {}); // client.attached
  await until("every kind of event", () => Object.keys(eventSchemas).every((t) => seen.some((e) => e.type === t)));

  for (const e of seen) {
    const schema = (eventSchemas as Record<string, z.ZodType>)[e.type];
    expect(schema, `an undocumented event type: ${e.type}`).toBeDefined();
    const parsed = schema!.safeParse(e);
    expect(parsed.success, `${e.type} doesn't match its schema: ${JSON.stringify(e)} ${parsed.error?.message}`).toBe(true);
    expect(e.epoch).toBe(sub.epoch);
  }
  const seqs = seen.map((e) => e.seq);
  expect(seqs[0]).toBeGreaterThan(sub.seq); // nothing in the snapshot is replayed as an event
  expect(seqs).toEqual([...seqs].sort((a, b) => a - b));
  expect(new Set(seqs).size).toBe(seqs.length);
  conn.close();
}, 60000);

test("a snapshot has no gap: a pane created right after subscribing is either in it or arrives as an event", async () => {
  for (let i = 0; i < 5; i++) {
    const splitting = cli("pane", "split", "--name", `race${i}`, "sleep 30");
    const { conn, seen, sub } = await subscriber({ snapshot: true });
    const id = (await splitting).trim();
    await until(`race${i} to be accounted for`, () => sub.panes.some((p: any) => p.id === id) || seen.some((e) => e.type === "pane.created" && e.pane === id));
    expect(sub.panes.some((p: any) => p.id === id) && seen.some((e) => e.type === "pane.created" && e.pane === id)).toBe(false); // never both
    conn.close();
  }
}, 60000);

test("protocol.describe publishes a JSON Schema for every request and every event", async () => {
  const conn = await connectUnix(sock);
  const d = await conn.request<any>("protocol.describe", {});
  conn.close();
  expect(d.protocol).toBe(PROTOCOL);
  for (const method of ["list", "pane.read", "events.subscribe", "protocol.describe"]) expect(d.requests[method]).toMatchObject({ type: "object" });
  expect(Object.keys(d.events).sort()).toEqual(Object.keys(eventSchemas).sort());
  expect(d.envelope).toMatchObject({ type: "object" });
  expect(envelope.safeParse({ type: "x", at: 1, seq: 1, epoch: "e" }).success).toBe(true);
});

test("a restarted server has a new epoch, so a client knows to take a new snapshot", async () => {
  const before = (await subscriber({})).sub.epoch;
  expect(await cli("restart", S)).toContain("restarted");
  const after = await subscriber({ snapshot: true });
  expect(after.sub.epoch).not.toBe(before);
  after.conn.close();
}, 30000);
