// Attaching to the server, handling what it pushes, reconnecting after drops and restarts, and quitting.
import { codeVersion } from "../core/paths";
import { unb64 } from "../protocol/conn";
import type { App, ServerView } from "./context";
import { notify } from "./notify";
import { permission } from "./modals/permission";
import { render } from "./render";

export function quit(app: App, why: "detached" | "exited" | string) {
  if (app.quitting) return;
  app.quitting = true;
  const conn = app.conn;
  if (why === "detached" && conn && !conn.closed) conn.request("detach").catch(() => {});
  app.modal?.close(null);
  app.clearToast();
  for (const undo of app.cleanup) undo();
  app.r.destroy();
  conn?.close();
  console.log(why === "detached" ? `[detached from ${app.opts.session}]` : why === "exited" ? "[shepherd exited]" : why);
}

async function attach(app: App, spawn: boolean) {
  const conn = (app.conn = await app.opts.connect(spawn));
  if (app.quitting) { conn.close(); return; }
  let ready = false;
  conn.onMessage = (m) => {
    const d = m.params;
    switch (m.method) {
      case "view":
        app.view = d;
        render(app);
        break;
      case "output":
        app.panes.get(d.pane)?.term.write(unb64(d.data));
        break;
      case "notify":
        notify(app, d.state, d.text);
        break;
      case "prompt":
        permission(app, d.id, d.text);
        break;
      case "prompt.done":
        if (app.promptIds.has(d.id)) {
          app.promptIds.delete(d.id);
          app.modal?.close(null);
        }
        break;
      case "detach":
        quit(app, "detached");
        break;
      case "exit":
        quit(app, "exited");
        break;
      case "restart":
        app.restarting = true;
        app.toast("server restarting…", app.th.warn);
        break;
    }
  };
  conn.onClose = () => {
    // During startup, the awaited attach/replay request owns the error. Once
    // ready, one reconnect loop owns it; failed attempts must not spawn loops.
    if (!app.quitting && ready && conn === app.conn) {
      conn.close();
      void connectWithRetry(app, true);
    }
  };
  const res = await conn.request<ServerView & { prompts: number[]; version?: string }>("attach", { area: app.area() });
  for (const p of app.panes.values()) p.destroy(); // reconnect: rebuild from the replay
  app.panes.clear();
  app.view = res;
  render(app);
  for (const { pane, data } of await conn.request<{ pane: string; data: string }[]>("replay")) app.panes.get(pane)?.term.write(unb64(data));
  for (const id of res.prompts) permission(app, id, "(pending permission request)");
  ready = true;
  if (res.version !== (await codeVersion())) app.toast(`this client and the session's server run different shepherd builds · detach and reattach, or ${app.cfg.prefix.replace("C-", "^").toUpperCase()} : → Restart server`, app.th.warn, 12000);
}

let connecting = false;
export async function connectWithRetry(app: App, reconnecting = false) {
  if (connecting || app.quitting) return;
  connecting = true;
  let failure: unknown;
  try {
    for (let attempt = 0; !app.quitting && (app.opts.remote || attempt < (app.restarting ? 40 : 3)); attempt++) {
      if (reconnecting || attempt > 0) {
        if (!app.restarting) app.toast("connection lost — reconnecting…", app.th.warn);
        await Bun.sleep(app.opts.remote ? 1000 : 350);
        if (app.quitting) return;
      }
      try {
        // first attach starts the server; after a restart, the client that asked starts it (others wait ~2s first)
        await attach(app, !reconnecting || app.restartedByUs || (app.restarting && attempt >= 6));
        if (app.restarting) app.toast("server restarted", app.th.done);
        else if (reconnecting || attempt > 0) app.toast("reconnected", app.th.done);
        app.restarting = app.restartedByUs = false;
        return;
      } catch (error) {
        failure = error;
        // Avoid onClose scheduling another loop when disposing a failed attempt.
        if (app.conn) { app.conn.onClose = () => {}; app.conn.close(); }
      }
    }
    if (!app.quitting) {
      process.exitCode = 1;
      const reason = failure instanceof Error ? failure.message : String(failure);
      quit(app, `[could not connect to ${app.opts.session}: ${reason}]`);
    }
  } finally {
    connecting = false;
  }
}
