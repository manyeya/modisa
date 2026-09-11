import { test, expect } from "bun:test";
import { Conn, ConnectionClosedError } from "../../src/protocol/conn";

test("disconnect rejects all pending requests and retains the socket error", async () => {
  const conn = new Conn(() => {}, () => {});
  const requests = [conn.request("attach"), conn.request("replay")];
  const results = Promise.allSettled(requests);
  const cause = new Error("ECONNRESET");
  let closed = 0;
  conn.onClose = () => closed++;
  conn.closedByPeer(cause);
  conn.closedByPeer();
  for (const result of await results) {
    expect(result.status).toBe("rejected");
    if (result.status === "rejected") {
      expect(result.reason).toBeInstanceOf(ConnectionClosedError);
      expect(result.reason.cause).toBe(cause);
    }
  }
  expect(closed).toBe(1);
  await expect(conn.request("after-close")).rejects.toThrow("ECONNRESET");
});

test("a synchronous socket write failure closes the connection without hanging requests", async () => {
  const conn = new Conn(() => { throw new Error("EPIPE"); }, () => {});
  await expect(conn.request("attach")).rejects.toThrow("EPIPE");
  expect(conn.closed).toBe(true);
  expect(() => conn.notify("area", {})).not.toThrow();
});

test("close ends a failed transport once and settles its requests", async () => {
  let ended = 0;
  const conn = new Conn(() => {}, () => { ended++; throw new Error("already ended"); });
  const pending = conn.request("attach");
  const result = Promise.allSettled([pending]);
  conn.closedByPeer();
  expect(() => { conn.close(); conn.close(); }).not.toThrow();
  expect(ended).toBe(1);
  expect((await result)[0]!.status).toBe("rejected");
});
