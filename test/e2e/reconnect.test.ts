import { test, expect } from "bun:test";
import { Screen, sandbox } from "../support/harness";
import { socketConn, type Conn } from "../../src/protocol/conn";

// A socket fixture drops requests at the exact startup stages from the reported crash.
async function fixture(name: string, drop: "attach" | "replay" | "adapters", recover: boolean) {
  const { root, env } = sandbox(`disconnect-${name}`);
  await Bun.write(`${root}/config/config.toml`, "");
  await Bun.$`mkdir -p ${root}/state`.quiet();
  let attempts = 0;
  let dropped = false;
  const clients = new Set<Conn>();
  const view = {
    active: 0, paused: false, prompts: [],
    workspaces: [{ id: "w1", name: "fixture", cwd: root, active: 0, tabs: [{ id: "t1", tree: { pane: "p1" }, focused: "p1", zoomed: false }] }],
    panes: [{ id: "p1", title: "shell", cwd: root, createdBy: "user", status: "running", cols: 112, rows: 33 }],
  };
  const server = Bun.listen<{ conn: Conn; flush: () => void }>({
    unix: `${root}/state/${name}.sock`,
    socket: {
      open(socket) {
        const { conn, flush } = socketConn(socket);
        socket.data = { conn, flush };
        clients.add(conn);
        conn.onMessage = (message) => {
          if (message.method === "attach") attempts++;
          if (message.method === drop && (!dropped || !recover)) {
            dropped = true;
            conn.close();
            return;
          }
          if (message.id !== undefined) conn.send({ jsonrpc: "2.0", id: message.id, result: message.method === "attach" ? view : [] });
        };
      },
      data(socket, data) { socket.data.conn.feed(new TextDecoder().decode(data)); },
      drain(socket) { socket.data.flush(); },
      close(socket) { socket.data.conn.closedByPeer(); clients.delete(socket.data.conn); },
    },
  });
  const ui = new Screen(["-s", name], env, root);
  return {
    ui, attempts: () => attempts,
    async cleanup() {
      ui.close();
      for (const conn of clients) conn.close();
      server.stop(true);
      await Bun.$`rm -rf ${root}`.quiet();
    },
  };
}

for (const stage of ["attach", "replay"] as const) test(`a dropped ${stage} request reconnects without an uncaught error`, async () => {
  const f = await fixture(`recover-${stage}`, stage, true);
  try {
    await f.ui.until("reconnected", (s) => s.includes("reconnected") && s.includes("SPACES"));
    expect(f.attempts()).toBe(2);
    f.ui.write("\x02d");
    expect(await f.ui.proc.exited).toBe(0);
    expect(f.ui.text()).not.toContain("ConnectionClosedError");
  } finally { await f.cleanup(); }
}, 12000);

test("a repeatedly dropped startup connection exits cleanly after bounded retries", async () => {
  const f = await fixture("unavailable", "attach", false);
  try {
    expect(await Promise.race([f.ui.proc.exited, Bun.sleep(5000).then(() => "timeout")])).toBe(1);
    expect(f.attempts()).toBe(3);
    expect(f.ui.text()).toContain("could not connect to unavailable: connection closed");
    expect(f.ui.text()).not.toContain("closedByPeer");
  } finally { await f.cleanup(); }
}, 12000);

test("disconnect during a command palette lookup recovers without an unhandled action promise", async () => {
  const f = await fixture("palette-drop", "adapters", true);
  try {
    await f.ui.until("initial frame", (s) => s.includes("SPACES"));
    await Bun.sleep(300);
    f.ui.write("\x02:");
    await f.ui.until("reconnected", (s) => s.includes("reconnected"));
    expect(f.attempts()).toBe(2);
    f.ui.write("\x02d");
    expect(await f.ui.proc.exited).toBe(0);
  } finally { await f.cleanup(); }
}, 12000);
