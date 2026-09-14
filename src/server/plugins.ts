// Plugins: programs started with the session server. Linked plugins (a plugin.json in a directory linked under
// ~/.config/shepherd/plugins) start from their argv in their own directory; [[plugin]] run lines from config.toml
// start through a login shell. Shepherd owns each one's process group: stopping sends TERM to the whole group, then
// KILL after STOP_MS, so a plugin's children don't outlive the session. Output goes to a log file per plugin.
// A plugin's connection binds to it with the token in $SHEPHERD_PLUGIN_TOKEN (plugin.hello) and can offer actions,
// which plugin.invoke (`shepherd plugin run`) calls. Nothing restarts a plugin that exits.
import { DIR } from "../core/paths";
import { ConnectionClosedError, fail } from "../protocol/conn";
import { PROTOCOL } from "../protocol/schema";
import type { PluginStatus } from "../protocol/types";
import { linkedPlugins } from "../config/plugins";
import type { Client, ServerContext } from "./context";
import type { Handlers } from "./rpc/dispatch";

const STOP_MS = 2000;
const INVOKE_MS = 30_000;
const LOG_LIMIT = 5 * 1024 * 1024; // per start; past it the rest is read and dropped, so the plugin never blocks on output

type Plugin = PluginStatus & { token: string; client?: Client; stopping?: boolean };

const groupAlive = (pgid: number) => {
  try {
    process.kill(-pgid, 0);
    return true;
  } catch (e) {
    return (e as { code?: string }).code === "EPERM";
  }
};
const signalGroup = (pgid: number, signal: "SIGTERM" | "SIGKILL") => {
  try {
    process.kill(-pgid, signal);
  } catch {} // already gone
};

export function createPluginHost(ctx: ServerContext) {
  const plugins = new Map<string, Plugin>();

  const add = (name: string, source: Plugin["source"], dir?: string): Plugin => {
    const pl: Plugin = { name, source, dir, status: "failed", log: `${DIR}/plugins/${ctx.session}.${name}.log`, token: crypto.randomUUID(), connected: false, actions: [] };
    plugins.set(name, pl);
    return pl;
  };

  const launch = async (pl: Plugin, argv: string[], cwd?: string) => {
    await Bun.$`mkdir -p ${DIR}/plugins`.quiet();
    await Bun.write(pl.log, "");
    try {
      // detached: a new session and process group, led by the plugin, so the whole group can be signalled
      const proc = Bun.spawn(argv, { cwd, env: { ...Bun.env, SHEPHERD_PLUGIN: pl.name, SHEPHERD_PLUGIN_TOKEN: pl.token }, stdio: ["ignore", "pipe", "pipe"], detached: true });
      Object.assign(pl, { status: "running", pid: proc.pid, error: undefined });
      // both streams into one log, in the order they arrive
      const sink = Bun.file(pl.log).writer();
      let written = 0;
      const pump = async (stream: ReadableStream<Uint8Array>) => {
        for await (const chunk of stream) {
          if (written > LOG_LIMIT) continue;
          written += chunk.length;
          sink.write(written > LOG_LIMIT ? `\n[shepherd: log truncated at ${LOG_LIMIT / 1024 / 1024} MB]\n` : chunk);
          sink.flush();
        }
      };
      Promise.all([pump(proc.stdout), pump(proc.stderr)]).catch(() => {}).finally(() => sink.end());
      proc.exited.then((code) => {
        pl.exitCode = proc.signalCode ? undefined : code;
        pl.signal = proc.signalCode ?? undefined;
        pl.status = pl.stopping ? "stopped" : code === 0 ? "exited" : "failed";
        if (pl.status === "failed") pl.error = `exited with ${proc.signalCode ?? code}; see ${pl.log}`;
      });
    } catch (e) {
      pl.error = `couldn't start ${argv[0]}: ${(e as Error).message}`;
    }
  };

  const start = async () => {
    for (const l of await linkedPlugins()) {
      const pl = add(l.name, "linked", l.dir);
      if (!l.manifest) pl.error = l.error;
      else if (l.manifest.protocol !== PROTOCOL) pl.error = `plugin.json says protocol ${l.manifest.protocol}; this shepherd speaks protocol ${PROTOCOL}`;
      else await launch(pl, l.manifest.run, l.dir);
      if (pl.error && pl.status === "failed") await Bun.write(pl.log, `shepherd: ${pl.error}\n`).catch(() => {});
    }
    for (const [i, c] of ctx.cfg.plugin.entries()) await launch(add(`config-${i + 1}`, "config"), [Bun.env.SHELL || "/bin/sh", "-lc", c.run]);
  };

  const stop = async (only?: Plugin) => {
    const live = (only ? [only] : [...plugins.values()]).filter((p) => p.pid && groupAlive(p.pid));
    for (const p of live) {
      p.stopping = true;
      p.client?.conn.close(); // its connection no longer speaks for it
      signalGroup(p.pid!, "SIGTERM");
    }
    for (const end = Date.now() + STOP_MS; Date.now() < end && live.some((p) => groupAlive(p.pid!)); ) await Bun.sleep(50);
    for (const p of live) if (groupAlive(p.pid!)) signalGroup(p.pid!, "SIGKILL");
  };

  const view = ({ token: _token, client, stopping: _stopping, ...p }: Plugin): PluginStatus => ({
    ...p,
    connected: !!client,
    group: p.pid ? (groupAlive(p.pid) ? "running" : "gone") : undefined,
  });

  const methods: Handlers = {
    "plugin.list": () => [...plugins.values()].map(view),
    // stop one plugin (its whole group) until the next server start; `shepherd plugin unlink` calls it
    "plugin.stop": async (p) => {
      const pl = plugins.get(p.name);
      if (!pl) throw fail("no_such_plugin", `no plugin named ${p.name} (see shepherd plugin list)`);
      await stop(pl);
      return view(pl);
    },
    // The token says which plugin this server started is talking. It tells plugins apart; it isn't a permission
    // boundary against other code running as the user, which can reach the socket too.
    "plugin.hello": (p, c) => {
      const pl = [...plugins.values()].find((x) => x.token === p.token);
      if (!pl) throw fail("no_such_plugin", "that token doesn't belong to a plugin this server started");
      if (pl.client && pl.client !== c) pl.client.conn.close(); // one live connection per plugin
      pl.client = c;
      c.plugin = pl.name;
      pl.actions = p.actions ?? [];
      return { name: pl.name, protocol: PROTOCOL, session: ctx.session, epoch: ctx.epoch };
    },
    "plugin.invoke": async (p) => {
      const pl = plugins.get(p.plugin);
      if (!pl) throw fail("no_such_plugin", `no plugin named ${p.plugin} (see shepherd plugin list)`);
      if (!pl.client) throw fail("plugin_unavailable", `${p.plugin} isn't connected (${pl.status}${pl.error ? `: ${pl.error}` : ""})`);
      if (!pl.actions.includes(p.action)) throw fail("no_such_action", `${p.plugin} has no action ${p.action} (it offers: ${pl.actions.join(", ") || "none"})`);
      let timer: Timer | undefined;
      const timeout = new Promise<never>((_, reject) => {
        timer = setTimeout(() => reject(fail("timeout", `${p.plugin} didn't answer ${p.action} within ${INVOKE_MS / 1000}s`)), INVOKE_MS);
      });
      try {
        return (await Promise.race([pl.client.conn.request("plugin.action", { action: p.action, params: p.params ?? {} }), timeout])) ?? null;
      } catch (e) {
        if ((e as { code?: string }).code === "timeout") throw e;
        if (e instanceof ConnectionClosedError) throw fail("plugin_unavailable", `${p.plugin} disconnected before answering ${p.action}`);
        throw fail("plugin_error", `${p.plugin} ${p.action}: ${(e as Error).message}`);
      } finally {
        clearTimeout(timer);
      }
    },
  };

  const disconnected = (c: Client) => {
    for (const pl of plugins.values()) if (pl.client === c) pl.client = undefined;
  };

  return { methods, start, stop, disconnected };
}
