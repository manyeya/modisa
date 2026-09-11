// The session server: owns every PTY and speaks the protocol to TUI clients, the CLI, MCP and plugins.
// This file is the startup/shutdown order; the pieces live in context, rpc/, agents/, persist/.
import { DIR, codeVersion, cwd, socketPath } from "../core/paths";
import { socketConn } from "../protocol/conn";
import { connectUnix } from "../protocol/transport";
import { loadConfig, watchConfig } from "../config/config";
import { loadAdapters } from "../config/adapters";
import { prepareNative } from "../platform/native";
import { createContext, type Client } from "./context";
import { preparePaneEnv } from "./env";
import { installPermissions } from "./permissions";
import { startMonitor } from "./agents/monitor";
import { startPlugins } from "./plugins";
import { createDispatcher } from "./rpc/dispatch";
import { clientMethods } from "./rpc/client";
import { apiMethods } from "./rpc/api";
import { save, load, forget } from "./persist/store";
import { restore } from "./persist/restore";
import { applyTemplate } from "./persist/template";

export async function runServer(session: string) {
  await prepareNative();
  const sock = socketPath(session);
  // Two clients reconnecting after a restart can both start a server; the second one bows out.
  const other = await connectUnix(sock).catch(() => undefined);
  if (other) {
    other.close();
    console.log(`shepherd server "${session}" is already running`);
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
  const dispatch = createDispatcher({ ...clientMethods(ctx), ...apiMethods(ctx) });

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

  const plugins = startPlugins(ctx.cfg);

  ctx.shutdown = async (empty, why = "exit") => {
    if (ctx.down) return;
    ctx.down = true;
    ctx.cancelSave();
    if (empty) await forget(session);
    else await save(ctx.s, session).catch(() => {});
    for (const c of ctx.clients) c.conn.notify(why, {});
    plugins.stop();
    ctx.s.destroy();
    server.stop(true);
    await Bun.file(sock).delete().catch(() => {});
    monitor.stop(); // nothing left holding the event loop: the process ends here
  };

  // ---------- initial contents: saved session, else shepherd.toml, else a shell ----------
  const saved = await load(session);
  if (saved?.workspaces.length) restore(ctx.s, saved, ctx.adapters);
  else if (!(await applyTemplate(ctx.s, cwd(), ctx.adapters))) ctx.s.newWorkspace(undefined, cwd());
  console.log(`shepherd server "${session}" listening on ${sock}`);
}
