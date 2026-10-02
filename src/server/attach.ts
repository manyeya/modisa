// Single-pane attach (`modisa pane attach`): another terminal shows one pane full-screen and is its emulator, sent the
// pane's screen once and then its output as it comes. Takeover drives it: one connection per pane, the pane held at that
// terminal's size (Session.sizeLocks) and everyone else's typing dropped (rpc/client.ts `input`). Observe only watches,
// at the pane's own size. Watchers aren't attached clients: no views, prompts or focus. A watch ends when its connection
// closes, the pane's process exits, or the pane closes, and then a takeover gives the pane back its box.
import { b64, fail } from "../protocol/conn";
import type { Client, ServerContext } from "./context";
import type { Handlers } from "./rpc/dispatch";
import type { PtyPane } from "./session/pane";

type Mode = "takeover" | "observe";
type End = { reason: "exited" | "closed"; exitCode?: number };

// A watcher's terminal stays on its alternate screen (its own is the user's shell), so it drops a program's screen
// switches; a chunk with one is sent as the pane's whole screen redrawn instead, as it is after that chunk.
const SWITCH = /\x1b\[\?(?:1049|1047|47)[hl]/;
const latin1 = new TextDecoder("latin1");
export function forWatchers(p: PtyPane, bytes: Uint8Array, data: string) {
  return bytes.includes(0x1b) && SWITCH.test(latin1.decode(bytes)) ? b64(new TextEncoder().encode(`\x1b[H\x1b[2J${p.replay()}`)) : data;
}

export function createAttach(ctx: ServerContext) {
  const { s } = ctx;
  const watching = new Map<Client, { pane: string; mode: Mode }>(); // one pane per connection

  // `end` says why to a watcher still there to hear it
  const release = (c: Client, end?: End) => {
    const w = watching.get(c);
    if (!w) return;
    watching.delete(c);
    const set = ctx.watchers.get(w.pane);
    set?.delete(c);
    if (!set?.size) ctx.watchers.delete(w.pane);
    if (ctx.takeovers.get(w.pane) === c) {
      ctx.takeovers.delete(w.pane);
      s.sizeLocks.delete(w.pane);
      const p = s.panes.get(w.pane);
      if (p) delete p.info.takeover;
      s.layout(); // back to its box, and clients drop the border's note
    }
    if (end) c.conn.notify("attach.end", { pane: w.pane, ...end });
  };
  const endAll = (id: string, end: End) => {
    for (const c of [...(ctx.watchers.get(id) ?? [])]) release(c, end);
  };
  const live = (pane: PtyPane) => s.panes.get(pane.id) === pane && pane.info.status === "running";
  const busy = (pane: PtyPane) => {
    if (ctx.takeovers.has(pane.id)) throw fail("ui_busy", `${pane.id} is already taken over from another terminal; --observe watches it`);
  };

  const methods: Handlers = {
    "pane.attach": async (p, c) => {
      if (watching.has(c)) throw fail("usage", `this connection is already attached to ${watching.get(c)!.pane}`);
      const pane = ctx.subject(p.target, p.caller);
      // its own output would come straight back to it
      if (pane.id === p.caller) throw fail("usage", `${pane.id} is your own pane: attach to another one`);
      if (pane.info.status === "exited") throw new Error(`${pane.id} has exited (${pane.info.exitCode ?? "?"}): modisa pane read ${pane.id} shows what it left`);
      const mode: Mode = p.mode;
      if (mode === "takeover") {
        if (pane.info.popup) throw fail("usage", `${pane.id} is a plugin's popup, sized by the client that opened it; --observe watches it`);
        busy(pane);
        await ctx.permit(p.caller, "keys", pane, "take it over from another terminal (pane attach)");
        // the wait for an answer can outlast the pane, the connection, or someone else's takeover
        if (!live(pane)) throw fail("pane_gone", `${pane.id} closed or exited before it could be attached`);
        if (c.conn.closed) throw new Error("the connection closed");
        busy(pane);
      }
      // One synchronous step from here to the replay: every byte after it reaches this connection, and none before it.
      watching.set(c, { pane: pane.id, mode });
      ctx.watchers.set(pane.id, (ctx.watchers.get(pane.id) ?? new Set()).add(c));
      if (mode === "takeover") {
        ctx.takeovers.set(pane.id, c);
        pane.info.takeover = true;
        s.sizeLocks.set(pane.id, { cols: p.cols, rows: p.rows });
        s.layout(); // resized before the replay, so it's drawn at the terminal's size
      }
      return { pane: pane.id, instance: pane.info.instance, mode, cols: pane.info.cols, rows: pane.info.rows, data: b64(new TextEncoder().encode(pane.replay())) };
    },
    "pane.attach.resize": (p, c) => {
      const w = watching.get(c);
      if (w?.mode !== "takeover") throw fail("usage", "only the connection that took a pane over (pane.attach) sizes it");
      s.sizeLocks.set(w.pane, { cols: p.cols, rows: p.rows });
      s.layout();
      const pane = s.panes.get(w.pane)!;
      return { cols: pane.info.cols, rows: pane.info.rows };
    },
  };

  return {
    methods,
    disconnected: (c: Client) => release(c),
    paneExited: (p: PtyPane) => endAll(p.id, { reason: "exited", exitCode: p.info.exitCode }),
    // a shell pane closes itself when its process exits: that's an exit, with its code
    paneClosing: (id: string) => {
      const p = s.panes.get(id);
      endAll(id, p?.info.status === "exited" ? { reason: "exited", exitCode: p.info.exitCode } : { reason: "closed" });
    },
  };
}
