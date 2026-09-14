// The protocol plugins build on: every event is stamped (type, at, seq, epoch) and matches its published schema,
// real replies match the published result schemas, subscribing can hand back a snapshot that sits exactly where
// the live stream starts, a client that stops reading is cut off without holding others up, and a restarted
// server has a new epoch.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { z } from "zod";
import { sandbox, startServer } from "../support/harness";
import { installFakeAgent } from "../support/fake-agent";
import { connectUnix } from "../../src/protocol/transport";
import { events as eventSchemas, envelope, errorReply, PROTOCOL, results } from "../../src/protocol/schema";
import type { Conn } from "../../src/protocol/conn";

const sb = sandbox("protocol");
const S = "proto";
const sock = (s = S) => `${sb.root}/state/${s}.sock`;
const cli = (...args: string[]) => sb.cli(S, args);

// a raw socket client that records every event it's sent
async function subscriber(params: object, s = S) {
  const conn: Conn = await connectUnix(sock(s));
  const seen: any[] = [];
  conn.onMessage = (m) => m.method === "event" && seen.push(m.params);
  const sub = await conn.request<any>("events.subscribe", params);
  return { conn, seen, sub };
}
// one JSON-RPC exchange on a bare socket, to see the reply exactly as it's sent
function raw(message: object): Promise<any> {
  return new Promise((resolve) => {
    let buf = "";
    Bun.connect({
      unix: sock(),
      socket: {
        open: (s) => void s.write(JSON.stringify(message) + "\n"),
        data(s, d) {
          buf += new TextDecoder().decode(d);
          const nl = buf.indexOf("\n");
          if (nl >= 0) {
            s.end();
            resolve(JSON.parse(buf.slice(0, nl)));
          }
        },
      },
    });
  });
}
const until = async (what: string, ok: () => boolean | Promise<boolean>, ms = 15000) => {
  for (const end = Date.now() + ms; Date.now() < end; await Bun.sleep(100)) if (await ok()) return;
  throw new Error(`timed out waiting for ${what}`);
};
const matches = (schema: z.ZodType, value: unknown, what: string) => {
  const r = schema.safeParse(value);
  expect(r.success, `${what} doesn't match its schema: ${r.error?.message}\n${JSON.stringify(value)}`).toBe(true);
};

beforeAll(async () => {
  await installFakeAgent(sb.root);
  await startServer(sb, S);
}, 20000);

afterAll(async () => {
  await cli("kill", S);
  await cli("kill", "slow");
  await cli("kill", "tiny");
  await sb.cleanup();
});

test("every event carries the envelope and matches its schema, in increasing seq order after the snapshot", async () => {
  const { conn, seen, sub } = await subscriber({ snapshot: true, output: true });
  // plain checks: toMatchObject with asymmetric matchers can write the matchers back into `sub`
  expect(sub.protocol).toBe(PROTOCOL);
  expect(typeof sub.epoch).toBe("string");
  expect(typeof sub.seq).toBe("number");
  matches(results["events.subscribe"], sub, "the subscribe reply");
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
    matches(schema!, e, e.type);
    expect(e.epoch).toBe(sub.epoch);
  }
  const seqs = seen.map((e) => e.seq);
  expect(seqs[0]).toBeGreaterThan(sub.seq); // nothing in the snapshot is replayed as an event
  expect(seqs).toEqual([...seqs].sort((a, b) => a - b));
  expect(new Set(seqs).size).toBe(seqs.length);
  conn.close();
}, 60000);

test("replies match the published result schemas, and errors the error schema", async () => {
  const conn = await connectUnix(sock());
  const check = async (method: keyof typeof results, params: object = {}) => matches(results[method], await conn.request(method, params), method);
  await check("list");
  await check("events.subscribe", { snapshot: true });
  await check("pane.read", { target: "@fake" });
  await check("agent.list");
  await check("wait", { target: "@job", exited: true });
  await check("wait", { target: "@fake", state: "idle", timeout: 5 });
  await check("wait", { target: "@job", match: "out" });
  await check("send", { to: "@fake", body: "a result check" });
  conn.close();

  for (const [request, code] of [
    [{ method: "pane.read", params: { target: "nope" } }, "no_such_pane"],
    [{ method: "pane.read", params: { lines: -1 } }, "invalid_params"],
    [{ method: "no.such.method", params: {} }, "unknown_method"],
  ] as const) {
    const reply = await raw({ jsonrpc: "2.0", id: 7, ...request });
    matches(errorReply, reply.error, `the error for ${request.method}`);
    expect(reply.error.data.code).toBe(code);
  }
}, 30000);

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

test("snapshots taken while an agent changes state agree with the events that follow them", async () => {
  const id = (await cli("agent", "spawn", "fakeagent", "--name", "flip")).trim(); // working for 4s, then idle
  const subs = [];
  for (let i = 0; i < 24; i++) {
    subs.push(await subscriber({ snapshot: true }));
    await Bun.sleep(250);
  }
  await cli("wait", "@flip", "--state", "idle", "--timeout", "15");
  await Bun.sleep(1000);
  let transitions = 0;
  for (const { conn, seen, sub } of subs) {
    let state = sub.panes.find((p: any) => p.id === id)?.agent?.state;
    for (const e of seen.filter((e) => e.type === "agent.state" && e.pane === id)) {
      expect(e.from).toBe(state); // each event continues from the state the snapshot (or the last event) left
      state = e.to;
      transitions++;
    }
    expect(state).toBe(JSON.parse(await cli("pane", "read", id, "--json")).agent?.state);
    conn.close();
  }
  expect(transitions).toBeGreaterThan(0); // some subscriptions really did straddle a change
}, 60000);

test("protocol.describe publishes JSON Schemas for requests, results, events and errors", async () => {
  const conn = await connectUnix(sock());
  const d = await conn.request<any>("protocol.describe", {});
  conn.close();
  expect(d.protocol).toBe(PROTOCOL);
  for (const method of ["list", "pane.read", "events.subscribe", "protocol.describe", "plugin.invoke"]) expect(d.requests[method]).toMatchObject({ type: "object" });
  expect(Object.keys(d.results).sort()).toEqual(Object.keys(results).sort());
  expect(Object.keys(d.events).sort()).toEqual(Object.keys(eventSchemas).sort());
  expect(d.envelope).toMatchObject({ type: "object" });
  expect(d.error).toMatchObject({ type: "object" });
  expect(envelope.safeParse({ type: "x", at: 1, seq: 1, epoch: "e" }).success).toBe(true);
});

test("a client that stops reading is disconnected once its queue passes the limit, and others carry on", async () => {
  await startServer(sb, "slow", { SHEPHERD_WRITE_QUEUE_LIMIT: String(256 * 1024) });
  // nc holds a subscription to all output (its stdin stays open), then is stopped: it reads nothing more
  const out = `${sb.root}/nc.out`;
  const stuck = Bun.spawn(["nc", "-U", sock("slow")], { stdin: "pipe", stdout: Bun.file(out), stderr: "ignore" });
  stuck.stdin.write(JSON.stringify({ jsonrpc: "2.0", id: 1, method: "events.subscribe", params: { output: true } }) + "\n");
  stuck.stdin.flush();
  await until("nc's subscription", async () => (await Bun.file(out).text().catch(() => "")).includes('"id":1'));
  // process.kill, not stuck.kill: the subprocess's kill("SIGSTOP") leaves nc running
  process.kill(stuck.pid, "SIGSTOP");
  await until("nc to be stopped", async () => (await Bun.$`ps -o stat= -p ${stuck.pid}`.nothrow().text()).includes("T"));

  const healthy = await subscriber({ output: true }, "slow");
  // the flood's output is emitted as the server works through it, well after the process has exited, so wait
  // for the marker printed after it to reach the healthy subscriber: by then every event has been sent
  const id = (await sb.cli("slow", ["pane", "split", "--name", "flood", "yes shepherd | head -c 2000000; echo; echo FLOOD-DONE-MARKER"])).trim();
  const flood = () => healthy.seen.filter((e) => e.type === "pane.output" && e.pane === id).map((e) => e.text).join("");
  await until("the whole flood to be emitted", () => flood().includes("FLOOD-DONE-MARKER"), 60000);
  const floodText = flood().length;

  // the server kept answering everyone else
  const started = Date.now();
  expect(await sb.cli("slow", ["pane", "list"])).toContain("flood");
  expect(Date.now() - started).toBeLessThan(5000);

  // resumed, nc reads what the kernel had buffered and then finds the connection closed
  process.kill(stuck.pid, "SIGCONT");
  const ended = await Promise.race([stuck.exited.then(() => true), Bun.sleep(15000).then(() => false)]);
  if (!ended) stuck.kill();
  expect(ended).toBe(true);
  expect((await Bun.file(out).text()).length).toBeLessThan(floodText); // it was cut off, not sent everything
  healthy.conn.close();
}, 120000);

test("a single message bigger than the limit closes that connection, even with nothing else waiting", async () => {
  await startServer(sb, "tiny", { SHEPHERD_WRITE_QUEUE_LIMIT: "4096" });
  await sb.run("tiny", ["pane", "run", "p1", "seq 1 3000"]);
  expect((await sb.run("tiny", ["wait", "p1", "--match", "^3000$", "--timeout", "10"])).code).toBe(0);
  const big = await sb.run("tiny", ["pane", "read", "p1", "--lines", "3000", "--json"]); // ~13 KB in one reply
  expect(big.code).toBe(3); // its connection closed under it
  expect((await sb.run("tiny", ["pane", "list"])).code).toBe(0); // a small reply on a new connection is fine
}, 30000);

test("a restarted server has a new epoch, so a client knows to take a new snapshot", async () => {
  const before = (await subscriber({})).sub.epoch;
  expect(await cli("restart", S)).toContain("restarted");
  const after = await subscriber({ snapshot: true });
  expect(after.sub.epoch).not.toBe(before);
  after.conn.close();
}, 30000);
