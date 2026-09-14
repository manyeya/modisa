// The session server: owns every PTY and speaks the protocol to TUI clients, the CLI, the integrations and plugins.
// This file is the startup/shutdown order; the pieces live in context, rpc/, agents/, persist/.
import { DIR, codeVersion, cwd, socketPath } from "../core/paths";
import { socketConn } from "../protocol/conn";
import { connectUnix, runningPid, unreachable } from "../protocol/transport";
import { loadConfig, watchConfig } from "../config/config";
import { loadAdapters } from "../config/adapters";
import { prepareNative } from "../platform/native";
import { createContext, type Client } from "./context";
import { preparePaneEnv } from "./env";
import { installPermissions } from "./permissions";
import { startMonitor } from "./agents/monitor";
import { createPluginHost } from "./plugins";
import { createDispatcher } from "./rpc/dispatch";
import { clientMethods } from "./rpc/client";
import { apiMethods } from "./rpc/api";
import { save, load, forget } from "./persist/store";
import { restore } from "./persist/restore";
import { applyTemplate } from "./persist/template";

export async function runServer(session: string) {
  await prepareNative();
  const sock = socketPath(session);
  const pidFile = sock.replace(/\.sock$/, ".pid");
  // Two clients reconnecting after a restart can both start a server; the second one bows out.
  const other = await connectUnix(sock).catch(() => undefined);
  if (other) {
    other.close();
    console.log(`shepherd server "${session}" is already running`);
    return;
  }
  // Running but unreachable from here (a sandbox): taking its socket over would orphan every pane in it.
  const running = await runningPid(sock);
  if (running) {
    console.error(unreachable(running));
    return;
  }
  preparePaneEnv(session, sock);
  await Bun.$`mkdir -p ${DIR}`.quiet();

  const cfg = await loadConfig();
  const ctx = createContext(session, await codeVersion(), cfg, await loadAdapters(cfg));
  watchConfig(async () => {
    ctx.cfg = await loadConfig();
    ctx.adapters = await loadAdapters(ctx.cfg);
    ctx.broadcast("config", {});
  });
  installPermissions(ctx);
  const monitor = startMonitor(ctx);
  const plugins = createPluginHost(ctx);
  const dispatch = createDispatcher({ ...clientMethods(ctx), ...apiMethods(ctx), ...plugins.methods });

  // ---------- socket ----------
  await Bun.file(sock).delete().catch(() => {});
  const dec = new TextDecoder();
  const server = Bun.listen<{ client: Client; flush: () => void }>({
    unix: sock,
    socket: {
      open(sk) {
        const { conn, flush } = socketConn(sk);
        const client: Client = { conn, attached: false, events: false, output: false };
        conn.onMessage = (m) => dispatch(client, m);
        conn.onClose = () => {
          ctx.clients.delete(client);
          plugins.disconnected(client);
          ctx.s.dragEnd(client);
          ctx.changed();
        };
        ctx.clients.add(client);
        sk.data = { client, flush };
      },
      data(sk, d) { sk.data.client.conn.feed(dec.decode(d, { stream: true })); },
      drain(sk) { sk.data.flush(); },
      end(sk) { sk.end(); },
      close(sk) { sk.data?.client.conn.closedByPeer(); },
      error(sk) { sk.data?.client.conn.closedByPeer(); },
    },
  });

  await Bun.write(pidFile, String(process.pid)); // lets clients tell "running but unreachable" from "dead"
  await plugins.start();

  ctx.shutdown = async (empty, why = "exit") => {
    if (ctx.down) return;
    ctx.down = true;
    ctx.cancelSave();
    if (empty) await forget(session);
    else await save(ctx.s, session).catch(() => {});
    for (const c of ctx.clients) c.conn.notify(why, {});
    await plugins.stop(); // each plugin's whole process group, within its time limit
    ctx.s.destroy();
    // The pid file goes first, so nothing mistakes this exiting server for a running one it can't reach.
    // Then the socket file, while it's still ours: once the listener stops, `shepherd restart` starts the
    // next server at this same path, and deleting it after that would cut the new server off.
    if ((await Bun.file(pidFile).text().catch(() => "")) === String(process.pid)) await Bun.file(pidFile).delete().catch(() => {});
    await Bun.file(sock).delete().catch(() => {});
    server.stop(true);
    monitor.stop();
    // A client connection closing mid-shutdown (the restart command's own) can keep the event loop alive
    // for good, leaving a server that holds nothing, so exit instead of waiting for the loop to drain.
    setTimeout(() => process.exit(0), 200);
  };

  // ---------- initial contents: saved session, else shepherd.toml, else a shell ----------
  const saved = await load(session);
  if (saved?.workspaces.length) restore(ctx.s, saved, ctx.adapters);
  else if (!(await applyTemplate(ctx.s, cwd(), ctx.adapters))) ctx.s.newWorkspace(undefined, cwd());
  console.log(`shepherd server "${session}" listening on ${sock}`);
}
