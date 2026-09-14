import { test, expect } from "bun:test";
import { Client, connect, writeQueue } from "../../src/plugins/shepherd-plugin";

test("a write that fails while draining drops the connection once, rejects what's pending, and doesn't escape", async () => {
  let handlers: Record<string, (...args: any[]) => void> = {};
  let draining = false;
  let ends = 0;
  const socket = {
    write: () => {
      if (!draining) return 0; // nothing fits: frames are queued
      throw new Error("drain write failed");
    },
    end: () => void ends++,
  };
  const client = await connect("/fake.sock", async (options) => {
    handlers = options.socket;
    return socket;
  });
  const settled = [client.request("list"), client.request("agent.list")].map((request) => request.then(() => "resolved", (error: Error) => error.message));
  draining = true;
  expect(() => handlers.drain!(socket)).not.toThrow();
  expect(await Promise.all(settled)).toEqual(["drain write failed", "drain write failed"]);
  expect((await client.closed).message).toBe("drain write failed");
  expect(ends).toBe(1);
  handlers.close!(socket); // the socket's own close arrives afterwards
  expect(ends).toBe(1);
});

test("the write queue keeps what a socket doesn't take, bytes in order across partial and zero-byte writes", () => {
  const received: number[] = [];
  let room = 1000;
  const out = writeQueue((bytes) => {
    const n = Math.min(room, bytes.length);
    for (const b of bytes.subarray(0, n)) received.push(b);
    room -= n;
    return n;
  });
  const first = JSON.stringify({ result: "é✓ 日本 ".repeat(800) }) + "\n"; // multi-byte characters cut mid-sequence
  const second = JSON.stringify({ id: 2, result: "after" }) + "\n";
  out.push(first);
  out.push(second);
  expect(out.queued).toBe(new TextEncoder().encode(first + second).length - 1000);
  for (let drain = 0; out.queued; drain++) {
    room = drain % 3 === 0 ? 0 : 333; // some drains take nothing
    out.flush();
  }
  expect(new TextDecoder().decode(new Uint8Array(received))).toBe(first + second);
});

test("the write queue refuses past its limit, and clear() empties it", () => {
  const out = writeQueue(() => 0, 100);
  out.push("x".repeat(60));
  expect(() => out.push("x".repeat(60))).toThrow("more than 100 bytes");
  out.clear();
  expect(out.queued).toBe(0);
});

test("a write that fails drops the connection and rejects what's pending", async () => {
  let closed = false;
  let fail = false;
  const client = new Client({ write: () => { if (fail) throw new Error("socket gone"); }, close: () => void (closed = true) });
  const first = client.request("list");
  fail = true;
  await expect(client.request("list")).rejects.toThrow("socket gone");
  await expect(first).rejects.toThrow("socket gone");
  expect(closed).toBe(true);
});

test("a stalled event handler hit by a burst disconnects once the backlog passes its limit", async () => {
  let closed = false;
  const sent: any[] = [];
  const client = new Client({ write: (l) => void sent.push(JSON.parse(l)), close: () => void (closed = true) });
  const line = (m: object) => client.feed(JSON.stringify({ jsonrpc: "2.0", ...m }) + "\n");
  const subscribing = client.subscribe({ onEvent: () => new Promise(() => {}) }, { maxBacklog: 5 });
  line({ id: sent[0].id, result: { protocol: 1, epoch: "e", seq: 0, panes: [] } });
  await subscribing;
  for (let seq = 1; seq <= 10; seq++) line({ method: "event", params: { type: "x", at: 1, seq, epoch: "e" } });
  const why = await client.closed;
  expect(why).toMatchObject({ code: "backlog" });
  expect(closed).toBe(true);
});

test("cancel aborts only its own invocation; a disconnect aborts the rest; a late cancel is harmless", async () => {
  const sent: any[] = [];
  const client = new Client({ write: (l) => void sent.push(JSON.parse(l)), close() {} });
  const line = (m: object) => client.feed(JSON.stringify({ jsonrpc: "2.0", ...m }) + "\n");
  const signals = new Map<string, AbortSignal>();
  const waitForAbort = (_p: unknown, call: { invocation?: string; signal: AbortSignal }) => {
    signals.set(call.invocation!, call.signal);
    return new Promise((resolve) => call.signal.addEventListener("abort", () => resolve("aborted")));
  };
  const hello = client.hello({ wait: waitForAbort, quick: () => "done" }, "token");
  line({ id: sent[0].id, result: {} });
  await hello;
  line({ id: 1, method: "plugin.action", params: { action: "wait", params: {}, invocation: "a" } });
  line({ id: 2, method: "plugin.action", params: { action: "wait", params: {}, invocation: "b" } });
  line({ id: 3, method: "plugin.action", params: { action: "quick", params: {}, invocation: "c" } });
  await Bun.sleep(5);
  line({ method: "plugin.cancel", params: { invocation: "a" } });
  line({ method: "plugin.cancel", params: { invocation: "c" } }); // already finished
  await Bun.sleep(5);
  expect(signals.get("a")!.aborted).toBe(true);
  expect(signals.get("b")!.aborted).toBe(false);
  expect(sent.find((m) => m.id === 3)).toMatchObject({ result: "done" });
  client.drop(new Error("gone"));
  expect(signals.get("b")!.aborted).toBe(true);
});

test("an action gets its invocation, and plugin.cancel aborts its signal", async () => {
  const sent: any[] = [];
  const client = new Client({ write: (l) => void sent.push(JSON.parse(l)), close() {} });
  const line = (m: object) => client.feed(JSON.stringify({ jsonrpc: "2.0", ...m }) + "\n");
  let seen: { invocation?: string; signal: AbortSignal } | undefined;
  const hello = client.hello({ wait: (_p, call) => { seen = call; return new Promise((resolve) => call.signal.addEventListener("abort", () => resolve("stopped"))); } }, "token");
  line({ id: sent[0].id, result: {} });
  await hello;
  line({ id: 7, method: "plugin.action", params: { action: "wait", params: {}, invocation: "demo-1" } });
  await Bun.sleep(5);
  expect(seen?.invocation).toBe("demo-1");
  expect(seen?.signal.aborted).toBe(false);
  line({ method: "plugin.cancel", params: { invocation: "demo-1", action: "wait" } });
  await Bun.sleep(5);
  expect(seen?.signal.aborted).toBe(true);
  expect(sent.find((m) => m.id === 7)).toMatchObject({ result: "stopped" });
});

// a Client on a fake socket: what it sends, and ways to feed it replies and events
function fake() {
  const sent: any[] = [];
  const client = new Client({ write: (line) => void sent.push(JSON.parse(line)), close() {} });
  const line = (m: object) => client.feed(JSON.stringify({ jsonrpc: "2.0", ...m }) + "\n");
  return { client, sent, reply: (id: number, result: unknown) => line({ id, result }), event: (e: object) => line({ method: "event", params: { type: "x", at: 1, ...e } }), line };
}

test("subscribe holds early events for the snapshot, and drops old, repeated and other-epoch events, in order", async () => {
  const { client, sent, reply, event } = fake();
  const got: number[] = [];
  const snapshots: unknown[] = [];
  const subscribing = client.subscribe({ onSnapshot: (s) => void snapshots.push(s), onEvent: (e) => void got.push(e.seq) });
  expect(sent[0]).toMatchObject({ method: "events.subscribe", params: { snapshot: true, output: false } });
  event({ seq: 12, epoch: "e1" }); // arrives before the reply
  event({ seq: 10, epoch: "e1" }); // at the snapshot's seq: already in it
  reply(sent[0].id, { protocol: 1, epoch: "e1", seq: 10, panes: [] });
  await subscribing;
  event({ seq: 12, epoch: "e1" }); // a repeat
  event({ seq: 11, epoch: "e1" }); // older than one already delivered
  event({ seq: 13, epoch: "e2" }); // another server run
  event({ seq: 14, epoch: "e1" });
  await Bun.sleep(10);
  expect(snapshots).toHaveLength(1);
  expect(got).toEqual([12, 14]);
});

test("event handlers run one at a time, in order, and one that throws doesn't stop the rest", async () => {
  const { client, sent, reply, event } = fake();
  const order: string[] = [];
  const subscribing = client.subscribe({
    onEvent: async (e) => {
      order.push(`start ${e.seq}`);
      await Bun.sleep(e.seq === 1 ? 20 : 0);
      order.push(`end ${e.seq}`);
      if (e.seq === 2) throw new Error("handler bug");
    },
  });
  reply(sent[0].id, { protocol: 1, epoch: "e", seq: 0, panes: [] });
  await subscribing;
  const errors = console.error;
  console.error = () => {};
  for (const seq of [1, 2, 3]) event({ seq, epoch: "e" });
  await Bun.sleep(60);
  console.error = errors;
  expect(order).toEqual(["start 1", "end 1", "start 2", "end 2", "start 3", "end 3"]);
});

test("malformed lines are ignored, and errors carry shepherd's code", async () => {
  const { client, sent, line } = fake();
  const reading = client.request("pane.read", { target: "nope" });
  client.feed("not json\n\n42\n");
  line({ id: sent[0].id, error: { code: -32000, message: "no such pane: nope", data: { code: "no_such_pane" } } });
  await expect(reading).rejects.toMatchObject({ name: "ShepherdError", code: "no_such_pane", message: "no such pane: nope" });
});

test("actions answer with their result or error, and an unknown action gets an error", async () => {
  const { client, sent, reply, line } = fake();
  const hello = client.hello({ echo: (p) => ({ echoed: p }), broken: () => { throw new Error("nope"); } }, "token");
  expect(sent[0]).toMatchObject({ method: "plugin.hello", params: { token: "token", actions: ["echo", "broken"] } });
  reply(sent[0].id, { name: "demo", protocol: 1, session: "s", epoch: "e" });
  await hello;
  line({ id: 100, method: "plugin.action", params: { action: "echo", params: { x: 1 } } });
  line({ id: 101, method: "plugin.action", params: { action: "broken", params: {} } });
  line({ id: 102, method: "plugin.action", params: { action: "missing", params: {} } });
  await Bun.sleep(10);
  expect(sent.find((m) => m.id === 100)).toMatchObject({ result: { echoed: { x: 1 } } });
  expect(sent.find((m) => m.id === 101)).toMatchObject({ error: { message: "nope" } });
  expect(sent.find((m) => m.id === 102)).toMatchObject({ error: { code: -32601 } });
});

test("a dropped connection rejects pending requests and resolves closed", async () => {
  const { client } = fake();
  const pending = client.request("list");
  client.drop(new Error("gone"));
  await expect(pending).rejects.toThrow("gone");
  expect((await client.closed).message).toBe("gone");
  await expect(client.request("list")).rejects.toThrow("gone");
});
