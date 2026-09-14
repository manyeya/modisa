import { test, expect } from "bun:test";
import { OwnedGroup } from "../../src/server/plugins";

// a stand-in for process.kill: records signals, and answers the existence probe (signal 0) from `state`
function os() {
  const signals: [number, string][] = [];
  const o = {
    state: "ours" as "ours" | "gone" | "someone else's",
    signals,
    kill(pid: number, signal: string | 0) {
      if (o.state === "gone") throw Object.assign(new Error("no such process"), { code: "ESRCH" });
      if (o.state === "someone else's") throw Object.assign(new Error("operation not permitted"), { code: "EPERM" });
      if (signal !== 0) signals.push([pid, signal]);
    },
  };
  return o;
}

test("a group seen gone is never signalled again, even once its id belongs to another group", () => {
  const fake = os();
  const group = new OwnedGroup(4242, fake);
  expect(group.alive()).toBe(true);
  group.signal("SIGTERM");
  expect(fake.signals).toEqual([[-4242, "SIGTERM"]]);

  fake.state = "gone";
  expect(group.alive()).toBe(false);
  fake.state = "ours"; // the id has been reused: a live group answers to it again
  fake.signals.length = 0;
  group.signal("SIGKILL");
  expect(group.alive()).toBe(false);
  expect(fake.signals).toEqual([]);
});

test("a group owned by another user is treated as not ours", () => {
  const fake = os();
  fake.state = "someone else's";
  const group = new OwnedGroup(99, fake);
  expect(group.alive()).toBe(false);
  fake.state = "ours";
  group.signal("SIGKILL");
  expect(fake.signals).toEqual([]);
});
