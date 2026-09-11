import { test, expect } from "bun:test";
import { Mailbox } from "../../src/server/agents/mailbox";

test("reply chains count hops and stop at the limit", () => {
  const mb = new Mailbox(() => ({ max_hops: 3, per_minute: 100 }));
  let from = "a", to = "b";
  for (let i = 0; i < 3; i++) {
    const m = mb.send(from, from, to, to, "ping");
    expect(m.hops).toBe(i);
    mb.markDelivered(m);
    [from, to] = [to, from];
  }
  expect(() => mb.send(from, from, to, to, "ping")).toThrow(/hop limit/);
  expect(mb.send("user", "user", "a", "a", "stop").hops).toBe(0);
});

test("per-pair rate limit", () => {
  const mb = new Mailbox(() => ({ max_hops: 100, per_minute: 2 }));
  mb.send("a", "a", "b", "b", "1");
  mb.send("a", "a", "b", "b", "2");
  expect(() => mb.send("a", "a", "b", "b", "3")).toThrow(/rate limit/);
  expect(() => mb.send("a", "a", "c", "c", "1")).not.toThrow();
});

test("take drains pending", () => {
  const mb = new Mailbox(() => ({ max_hops: 10, per_minute: 10 }));
  mb.send("user", "user", "a", "a", "hi");
  expect(mb.take("a").length).toBe(1);
  expect(mb.pending("a").length).toBe(0);
});
