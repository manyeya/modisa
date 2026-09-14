import { test, expect } from "bun:test";
import { Client } from "../../src/plugins/shepherd-plugin";

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
