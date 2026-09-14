// Plugins: programs started with the session server. Linked plugins (a plugin.json in a directory linked under
// ~/.config/shepherd/plugins) start from their argv in their own directory; [[plugin]] run lines from config.toml
// start through a login shell. Each gets $SHEPHERD_PLUGIN_DATA, a directory of its own, and a log file.
//
// Every start is a run: its own token, its own process group. Stopping a run (plugin stop, unlink, session stop) or
// its process exiting first revokes it: the token stops binding and its connection is closed. Then its group gets
// TERM, and KILL after STOP_MS, but only while the group is provably still that run's (see OwnedGroup).
//
// A run's connection binds with its token (plugin.hello) and can offer actions, which plugin.invoke (`shepherd plugin
// run`) calls. An action that doesn't answer in time has an unknown outcome: the plugin is told to cancel (advisory),
// a late reply is logged, and nothing retries. Nothing restarts a plugin either; plugin.start does, when asked.
import { DIR } from "../core/paths";
import { ConnectionClosedError, fail } from "../protocol/conn";
import { PROTOCOL } from "../protocol/schema";
import type { PluginStatus } from "../protocol/types";
import { linkedPlugins, readManifest } from "../config/plugins";
import type { Client, ServerContext } from "./context";
import type { Handlers } from "./rpc/dispatch";

const STOP_MS = 2000;
const INVOKE_MS = Number(Bun.env.SHEPHERD_PLUGIN_INVOKE_MS) || 30_000;
const LOG_LIMIT = 5 * 1024 * 1024; // per run; past it the rest is read and dropped, so the plugin never blocks on output

type Kill = { kill(pid: number, signal: NodeJS.Signals | 0): void };

// A run's process group, signalled only while it's provably still ours. Once it's seen gone (or owned by someone
// else) it's retired for good: the id can be reused by an unrelated group, which must never be signalled.
// ponytail: a small window remains. If the group's last process exits and its id is reused before the next probe
// (the 1s watch after the leader exits, or stop's own probe), that probe can't tell. Closing it needs pidfds.
export class OwnedGroup {
  private retired = false;
  constructor(readonly pgid: number, private os: Kill = process as Kill) {}
  alive() {
    if (this.retired) return false;
    try {
      this.os.kill(-this.pgid, 0);
      return true;
    } catch {
      this.retired = true; // gone, or (EPERM) not ours
      return false;
    }
  }
  signal(signal: "SIGTERM" | "SIGKILL") {
    if (!this.alive()) return;
    try {
      this.os.kill(-this.pgid, signal);
    } catch {}
  }
}

type Run = { group: OwnedGroup; token: string; revoked: boolean; note(line: string): void };
type Plugin = PluginStatus & { run?: Run; argv?: string[]; client?: Client; stopping?: boolean; starting?: boolean };

export function createPluginHost(ctx: ServerContext) {
  const plugins = new Map<string, Plugin>();
  let invocations = 0;

  const add = (name: string, source: Plugin["source"], dir?: string): Plugin => {
    const pl: Plugin = { name, source, dir, status: "failed", log: `${DIR}/plugins/${ctx.session}.${name}.log`, connected: false, actions: [] };
    plugins.set(name, pl);
    return pl;
  };
  const need = (name: string) => {
    const pl = plugins.get(name);
    if (!pl) throw fail("no_such_plugin", `no plugin named ${name} (see shepherd plugin list)`);
    return pl;
  };

  // The run's token stops binding and its connection is closed, whether or not its group is still alive.
  const revoke = (pl: Plugin) => {
    if (pl.run) pl.run.revoked = true;
    const client = pl.client;
    pl.client = undefined;
    pl.actions = [];
    client?.conn.close();
  };

  const failed = async (pl: Plugin, error: string) => {
    Object.assign(pl, { status: "failed", error, pid: undefined });
    await Bun.$`mkdir -p ${DIR}/plugins`.quiet();
    await Bun.write(pl.log, `shepherd: ${error}\n`).catch(() => {});
  };

  // A linked plugin's manifest is read again on every start, so edits to plugin.json apply.
  const prepare = async (pl: Plugin) => {
    if (pl.source !== "linked") return true;
    const { manifest, error } = await readManifest(pl.dir!);
    const why = !manifest ? error!
      : manifest.name !== pl.name ? `linked as ${pl.name}, but plugin.json names it ${manifest.name}`
      : manifest.protocol !== PROTOCOL ? `plugin.json says protocol ${manifest.protocol}; this shepherd speaks protocol ${PROTOCOL}`
      : undefined;
    if (why) {
      await failed(pl, why);
      return false;
    }
    pl.argv = manifest!.run;
    return true;
  };

  const launch = async (pl: Plugin) => {
    const data = `${DIR}/plugins/${pl.name}`;
    await Bun.$`mkdir -p ${data}`.quiet();
    await Bun.write(pl.log, "");
    Object.assign(pl, { stopping: false, pid: undefined, exitCode: undefined, signal: undefined, error: undefined });
    const token = crypto.randomUUID();
    let proc: Bun.Subprocess<"ignore", "pipe", "pipe">;
    try {
      // detached: a new session and process group, led by the plugin, so the whole group can be signalled
      proc = Bun.spawn(pl.argv!, { cwd: pl.dir, env: { ...Bun.env, SHEPHERD_PLUGIN: pl.name, SHEPHERD_PLUGIN_TOKEN: token, SHEPHERD_PLUGIN_DATA: data }, stdio: ["ignore", "pipe", "pipe"], detached: true });
    } catch (e) {
      return failed(pl, `couldn't start ${pl.argv![0]}: ${(e as Error).message}`);
    }
    // both streams, and shepherd's own notes, into one log in the order they happen
    const sink = Bun.file(pl.log).writer();
    let written = 0;
    const write = (chunk: Uint8Array | string) => {
      if (written > LOG_LIMIT) return;
      written += chunk.length;
      try {
        sink.write(written > LOG_LIMIT ? `\n[shepherd: log truncated at ${LOG_LIMIT / 1024 / 1024} MB]\n` : chunk);
        sink.flush();
      } catch {} // the log was closed
    };
    const pump = async (stream: ReadableStream<Uint8Array>) => {
      for await (const chunk of stream) write(chunk);
    };
    const run: Run = { group: new OwnedGroup(proc.pid), token, revoked: false, note: (line) => write(`${line}\n`) };
    Object.assign(pl, { run, status: "running", pid: proc.pid });
    Promise.all([pump(proc.stdout), pump(proc.stderr)]).catch(() => {});
    proc.exited.then((code) => {
      if (pl.run !== run) return; // started again since
      revoke(pl);
      pl.exitCode = code;
      pl.signal = proc.signalCode ?? undefined;
      pl.status = pl.stopping ? "stopped" : code === 0 ? "exited" : "failed";
      if (pl.status === "failed") pl.error = `exited with ${proc.signalCode ?? code}; see ${pl.log}`;
      // retire the group's id as soon as it's gone (its children can outlive the leader)
      const watch = setInterval(() => run.group.alive() || clearInterval(watch), 1000);
    });
  };

  const start = async () => {
    for (const l of await linkedPlugins()) {
      const pl = add(l.name, "linked", l.dir);
      if (l.error) await failed(pl, l.error);
      else if (await prepare(pl)) await launch(pl);
    }
    for (const [i, c] of ctx.cfg.plugin.entries()) {
      const pl = add(`config-${i + 1}`, "config");
      pl.argv = [Bun.env.SHELL || "/bin/sh", "-lc", c.run];
      await launch(pl);
    }
  };

  const stop = async (only?: Plugin) => {
    const targets = only ? [only] : [...plugins.values()];
    for (const p of targets) {
      p.stopping = true;
      revoke(p);
    }
    const live = targets.filter((p) => p.run?.group.alive());
    for (const p of live) p.run!.group.signal("SIGTERM");
    for (const end = Date.now() + STOP_MS; Date.now() < end && live.some((p) => p.run!.group.alive()); ) await Bun.sleep(50);
    for (const p of live) p.run!.group.signal("SIGKILL");
  };

  const view = ({ run, argv: _argv, client, stopping: _stopping, starting: _starting, ...p }: Plugin): PluginStatus => ({
    ...p,
    connected: !!client,
    group: run ? (run.group.alive() ? "running" : "gone") : undefined,
    invocations: client?.conn.inFlight,
  });

  const methods: Handlers = {
    "plugin.list": () => [...plugins.values()].map(view),
    // stop one plugin: revoke its run, then end its group if that's still there
    "plugin.stop": async (p) => {
      const pl = need(p.name);
      await stop(pl);
      return view(pl);
    },
    // Start a plugin that isn't running, as a new run with a new token. Linked plugins are looked up again, so one
    // linked since this server started can be started too. Never a second run: a running or starting plugin is
    // already_running.
    "plugin.start": async (p) => {
      let pl = plugins.get(p.name);
      if (!pl || pl.source === "linked") {
        const linked = (await linkedPlugins()).find((l) => l.name === p.name);
        if (!linked) throw fail("no_such_plugin", `no plugin named ${p.name} is linked (shepherd plugin link <dir>)`);
        pl = plugins.get(p.name) ?? add(linked.name, "linked", linked.dir);
        pl.dir = linked.dir;
        if (linked.error && !(pl.run?.group.alive() || pl.starting)) {
          await failed(pl, linked.error);
          return view(pl);
        }
      }
      if (pl.run?.group.alive() || pl.starting) throw fail("already_running", `${p.name} is already running in this session (pid ${pl.pid}); not started again`);
      pl.starting = true;
      try {
        if (await prepare(pl)) await launch(pl);
      } finally {
        pl.starting = false;
      }
      return view(pl);
    },
    // The token says which run of which plugin is talking. It tells plugins apart; it isn't a permission boundary
    // against other code running as the user, which can reach the socket too.
    "plugin.hello": (p, c) => {
      const pl = [...plugins.values()].find((x) => x.run && !x.run.revoked && x.run.token === p.token);
      if (!pl) throw fail("plugin_unavailable", "that token doesn't belong to a running plugin of this server: it was stopped, exited, or never started");
      if (pl.client && pl.client !== c) pl.client.conn.close(); // one live connection per run
      const run = pl.run!;
      pl.client = c;
      c.plugin = pl.name;
      pl.actions = p.actions ?? [];
      c.conn.onLateReply = (m) => run.note(`shepherd: a late reply from ${pl.name}, after its invocation had timed out (dropped): ${JSON.stringify(m).slice(0, 500)}`);
      return { name: pl.name, protocol: PROTOCOL, session: ctx.session, epoch: ctx.epoch };
    },
    "plugin.invoke": async (p) => {
      const pl = need(p.plugin);
      const conn = pl.client?.conn;
      const run = pl.run;
      if (!conn || !run || run.revoked) throw fail("plugin_unavailable", `${p.plugin} isn't connected (${pl.status}${pl.error ? `: ${pl.error}` : ""})`);
      if (!pl.actions.includes(p.action)) throw fail("no_such_action", `${p.plugin} has no action ${p.action} (it offers: ${pl.actions.join(", ") || "none"})`);
      const invocation = `${pl.name}-${++invocations}`;
      try {
        return (await conn.request("plugin.action", { action: p.action, params: p.params ?? {}, invocation }, { timeoutMs: INVOKE_MS })) ?? null;
      } catch (e) {
        if ((e as { code?: string }).code === "timeout") {
          conn.notify("plugin.cancel", { invocation, action: p.action }); // advisory: nothing proves the action stopped
          run.note(`shepherd: ${p.action} (invocation ${invocation}) didn't answer within ${INVOKE_MS / 1000}s; its outcome is unknown`);
          throw fail("timeout", `${p.plugin} didn't answer ${p.action} within ${INVOKE_MS / 1000}s. Its outcome is unknown: it may still finish, and running it again can repeat its effects`);
        }
        if (e instanceof ConnectionClosedError) throw fail("plugin_unavailable", `${p.plugin} disconnected before answering ${p.action}; its outcome is unknown`);
        throw fail("plugin_error", `${p.plugin} ${p.action}: ${(e as Error).message}`);
      }
    },
  };

  const disconnected = (c: Client) => {
    for (const pl of plugins.values()) {
      if (pl.client !== c) continue;
      pl.client = undefined;
      pl.actions = [];
    }
  };

  return { methods, start, stop, disconnected };
}
