// Plugins: programs started with the session server. Linked plugins (a plugin.json in a directory linked under
// ~/.config/shepherd/plugins) start from their argv in their own directory; [[plugin]] run lines from config.toml
// start through a login shell. Each gets $SHEPHERD_PLUGIN_DATA, a directory of its own, and a log file.
//
// Every start is a run: its own token, its own process group. Stopping a run (plugin stop, unlink, session stop) or
// its process exiting first revokes it: the token stops binding, its connection is closed, and everything it showed in
// the TUI is cleared. Then its group gets TERM, and KILL after STOP_MS, but only while the group is provably still that
// run's (see OwnedGroup).
//
// A run's connection binds with its token (plugin.hello) and can offer actions, which plugin.invoke (`shepherd plugin
// run`, the palette, a status segment, a sidebar row, a menu entry) calls. An action that doesn't answer in time has an
// unknown outcome: the plugin is told to cancel (advisory), a late reply is logged, and nothing retries. The bound
// connection can also put data into the TUI (ui.*): status segments, a sidebar section, pane badges, menu entries and
// toasts, which shepherd draws itself. Nothing restarts a plugin; plugin.start does, when asked.
import { DIR } from "../core/paths";
import { CONFIG_DIR } from "../config/config";
import { shepherdKey } from "../config/keys";
import { ConnectionClosedError, fail } from "../protocol/conn";
import { PROTOCOL, type PluginManifest } from "../protocol/schema";
import { linkMatches } from "../protocol/links";
import type { PluginKey, PluginStatus, PluginUiView, Tone } from "../protocol/types";
import { linkedPlugins, readManifest } from "../config/plugins";
import type { Client, ServerContext } from "./context";
import type { Handlers } from "./rpc/dispatch";
import type { PtyPane } from "./session/pane";
import type { SpawnOpts } from "./session/session";
import { quote } from "./persist/template";

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

// ---------- what a run shows in the TUI ----------
// Limits keep a plugin from crowding out shepherd's own chrome or flooding clients with redraws.
const LIMIT = { statusSegments: 4, statusText: 32, sidebarTitle: 30, sidebarRows: 20, rowText: 60, badgeText: 12, badges: 50, menuItems: 8, menuTitle: 40, toastText: 120 };
const UPDATES = { burst: 30, perSecond: 10 }; // ui.* calls per run
const TOASTS = { burst: 3, windowMs: 10_000 };
// and across all the session's plugins together, so many plugins, each within its own limits, can't do it either:
// first come, first served
const SESSION = { statusSegments: 12, badgesPerPane: 4, sidebarSections: 6, menuItems: 24, toasts: 6, burst: 60, perSecond: 30 };

type UiState = {
  status: Map<string, PluginUiView["status"][number]>;
  sidebar?: PluginUiView["sidebar"];
  badges: Map<string, PluginUiView["badges"][number]>;
  menu: PluginUiView["menu"];
  tokens: number;
  refilled: number;
  toasts: number[];
};
const emptyUi = (): UiState => ({ status: new Map(), badges: new Map(), menu: [], tokens: UPDATES.burst, refilled: Date.now(), toasts: [] });

// Text a plugin sends is shown in the TUI: no escape sequences, control characters, or invisible formatting characters
// (bidi overrides and isolates, zero-width marks) that could reorder or hide part of a label; and at most `cells`
// terminal cells, cut between whole characters (a wide character counts 2).
const graphemes = new Intl.Segmenter();
export const cleanText = (text: string, cells: number) => {
  const plain = text
    .replace(/\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)?/g, "") // OSC
    .replace(/\x1b\[[0-9;?]*[ -/]*[@-~]/g, "") // CSI
    .replace(/[\x00-\x1f\x7f-\x9f]|\p{Cf}/gu, "");
  let out = "";
  let width = 0;
  for (const { segment } of graphemes.segment(plain)) {
    const w = Bun.stringWidth(segment);
    if (width + w > cells) break;
    out += segment;
    width += w;
  }
  return out;
};

// id: public, shown with the run's UI so an action taken from it can be refused once the run has ended
type Run = { id: string; group: OwnedGroup; token: string; revoked: boolean; note(line: string): void };
type Plugin = PluginStatus & { run?: Run; argv?: string[]; manifest?: PluginManifest; client?: Client; stopping?: boolean; starting?: boolean; ui: UiState };

// A plugin's own directories: DATA for its state (under shepherd's state directory), CONFIG for settings the user
// edits (under ~/.config/shepherd/plugin-config).
const dirsOf = (name: string) => ({ SHEPHERD_PLUGIN: name, SHEPHERD_PLUGIN_DATA: `${DIR}/plugins/${name}`, SHEPHERD_PLUGIN_CONFIG: `${CONFIG_DIR}/plugin-config/${name}` });

// Placements. split, tab and zoomed are ordinary panes in the layout (zoomed: split, then the tab zoomed); they belong
// to the session and outlive the plugin. overlay is a temporary zoomed pane over its origin, closed when its process
// exits or its plugin stops; closing puts focus (and zoom) back on the origin if it still exists and the user hasn't
// gone to another tab, and otherwise leaves focus where closing put it. popup is a terminal with no place in the layout,
// owned by the one TUI client that opened it (others never see it), one per session, closed when its process exits,
// its client closes it (prefix x), the plugin closes it, or the plugin stops.
// hadFocus: the overlay was the focused pane of the tab on screen when it closed. Only then does focus go back: if the
// user focused another pane or switched tabs while it was open, focus stays where they put it.
type Overlay = { plugin: string; origin?: string; instance?: string; wasZoomed: boolean; hadFocus?: boolean };
type Popup = { plugin: string; client: Client };

export function createPluginHost(ctx: ServerContext) {
  const plugins = new Map<string, Plugin>();
  const overlays = new Map<string, Overlay>();
  const popups = new Map<string, Popup>();
  let invocations = 0;

  const add = (name: string, source: Plugin["source"], dir?: string): Plugin => {
    const pl: Plugin = { name, source, dir, status: "failed", log: `${DIR}/plugins/${ctx.session}.${name}.log`, connected: false, actions: [], ui: emptyUi() };
    plugins.set(name, pl);
    return pl;
  };
  const need = (name: string) => {
    const pl = plugins.get(name);
    if (!pl) throw fail("no_such_plugin", `no plugin named ${name} (see shepherd plugin list)`);
    return pl;
  };

  // The run's token stops binding, its connection is closed and its TUI contributions go, whether or not its group is
  // still alive.
  const revoke = (pl: Plugin) => {
    if (pl.run) pl.run.revoked = true;
    const client = pl.client;
    pl.client = undefined;
    pl.actions = [];
    pl.ui = emptyUi();
    client?.conn.close();
    // its transient panes go with it; split, tab and zoomed panes are the session's and stay
    for (const [id, overlay] of overlays) if (overlay.plugin === pl.name) ctx.s.close(id);
    for (const [id, popup] of popups) if (popup.plugin === pl.name) ctx.s.dropHidden(id);
    ctx.changed();
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
    pl.manifest = manifest;
    return true;
  };

  const launch = async (pl: Plugin) => {
    const dirs = dirsOf(pl.name);
    await Bun.$`mkdir -p ${dirs.SHEPHERD_PLUGIN_DATA} ${dirs.SHEPHERD_PLUGIN_CONFIG}`.quiet();
    await Bun.write(pl.log, "");
    Object.assign(pl, { stopping: false, pid: undefined, exitCode: undefined, signal: undefined, error: undefined });
    const token = crypto.randomUUID();
    let proc: Bun.Subprocess<"ignore", "pipe", "pipe">;
    try {
      // detached: a new session and process group, led by the plugin, so the whole group can be signalled
      proc = Bun.spawn(pl.argv!, { cwd: pl.dir, env: { ...Bun.env, ...dirs, SHEPHERD_PLUGIN_TOKEN: token }, stdio: ["ignore", "pipe", "pipe"], detached: true });
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
    const run: Run = { id: crypto.randomUUID().slice(0, 8), group: new OwnedGroup(proc.pid), token, revoked: false, note: (line) => write(`${line}\n`) };
    Object.assign(pl, { run, status: "running", pid: proc.pid, ui: emptyUi() });
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

  // Keys for this session's running plugins: plugin.json's, remapped by [plugin_keys]. shepherd's own and reserved keys
  // are refused, and when two plugins want one key both are disabled, so nothing depends on which started first.
  const keyTable = () => {
    const wanted = [...plugins.values()]
      .filter((pl) => pl.run && !pl.run.revoked && pl.manifest?.keys?.length)
      .flatMap((pl) =>
        pl.manifest!.keys!.map((k) => {
          const remap = ctx.cfg.plugin_keys?.[`${pl.name}.${k.action ?? k.pane}`];
          return { plugin: pl.name, key: remap ?? k.key, action: k.action, pane: k.pane, description: k.description, remapped: remap !== undefined };
        }),
      );
    const byKey = new Map<string, string[]>();
    for (const w of wanted) if (w.key) byKey.set(w.key, [...(byKey.get(w.key) ?? []), w.plugin]);
    return wanted.map(({ plugin, remapped, ...w }): PluginKey & { plugin: string } => {
      const others = (byKey.get(w.key) ?? []).filter((p) => p !== plugin);
      const reason = !w.key ? "turned off in [plugin_keys]" : shepherdKey(w.key) ?? (others.length || (byKey.get(w.key)?.length ?? 0) > 1 ? `also wanted by ${others.length ? others.join(", ") : `another key of ${plugin}`}` : undefined);
      return { plugin, ...w, ...(remapped && !reason && { description: w.description }), state: reason ? "disabled" : "active", ...(reason && { reason }) };
    });
  };
  const keysOf = (name: string) => keyTable().filter((k) => k.plugin === name).map(({ plugin: _plugin, ...k }) => k);

  const view = ({ run, argv: _argv, manifest: _manifest, client, stopping: _stopping, starting: _starting, ui: _ui, ...p }: Plugin): PluginStatus => ({
    ...p,
    connected: !!client,
    group: run ? (run.group.alive() ? "running" : "gone") : undefined,
    invocations: client?.conn.inFlight,
    ...(p.name && run && !run.revoked && { keys: keysOf(p.name) }),
  });

  // what one plugin shows in the TUI now
  const uiOf = (pl: Plugin): PluginUiView => {
    const titles = new Map(pl.manifest?.actions?.map((a) => [a.id, a]) ?? []);
    return {
      plugin: pl.name,
      run: pl.run?.id ?? "",
      actions: pl.client ? pl.actions.map((id) => ({ id, title: titles.get(id)?.title ?? id, ...(titles.get(id)?.description && { description: titles.get(id)!.description }) })) : [],
      status: [...pl.ui.status.values()],
      ...(pl.ui.sidebar && { sidebar: pl.ui.sidebar }),
      badges: [...pl.ui.badges.values()],
      menu: pl.ui.menu,
      keys: pl.run && !pl.run.revoked ? keysOf(pl.name) : [],
      panes: pl.run && !pl.run.revoked ? (pl.manifest?.panes ?? []).map(({ id, title, placement }) => ({ id, title, placement })) : [],
      links: pl.client ? (pl.manifest?.links ?? []).filter((l) => pl.actions.includes(l.action)) : [],
    };
  };
  const uiView = () =>
    [...plugins.values()].filter((pl) => pl.run && !pl.run.revoked).map(uiOf).filter((v) => v.actions.length || v.status.length || v.sidebar || v.badges.length || v.menu.length || v.keys.length || v.panes.length || v.links.length);

  // a pane is being closed: whether an overlay still had the focus then
  const paneClosing = (id: string, focused: boolean) => {
    const overlay = overlays.get(id);
    if (overlay) overlay.hadFocus = focused;
  };

  // an overlay's or popup's process ended
  const paneExited = (p: PtyPane) => {
    const overlay = overlays.get(p.id);
    if (overlay) {
      overlays.delete(p.id);
      const origin = overlay.origin ? ctx.s.panes.get(overlay.origin) : undefined;
      const where = origin && origin.info.instance === overlay.instance ? ctx.s.locate(origin.id) : undefined;
      if (overlay.hadFocus && where && where.tab === ctx.s.tab) {
        ctx.s.focusPane(origin!.id);
        where.tab.zoomed = overlay.wasZoomed;
        ctx.s.layout();
      }
    }
    if (popups.delete(p.id)) {
      ctx.s.dropHidden(p.id);
      ctx.changed();
    }
  };

  // the other running plugins, and the session's shared update and toast budgets
  const others = (pl: Plugin) => [...plugins.values()].filter((x) => x !== pl && x.run && !x.run.revoked);
  const sessionUi = { tokens: SESSION.burst, refilled: Date.now(), toasts: [] as number[] };

  // A ui.* call: from the plugin's bound connection, within its rate, naming only actions it offered.
  const uiCall = (c: Client, action?: string) => {
    const pl = [...plugins.values()].find((x) => x.client === c && x.run && !x.run.revoked);
    if (!pl) throw fail("plugin_unavailable", "ui calls work only on a plugin's bound connection: call plugin.hello first");
    const now = Date.now();
    pl.ui.tokens = Math.min(UPDATES.burst, pl.ui.tokens + ((now - pl.ui.refilled) / 1000) * UPDATES.perSecond);
    pl.ui.refilled = now;
    sessionUi.tokens = Math.min(SESSION.burst, sessionUi.tokens + ((now - sessionUi.refilled) / 1000) * SESSION.perSecond);
    sessionUi.refilled = now;
    // both checked before either is spent, so an update one refuses doesn't use up the other
    if (pl.ui.tokens < 1) throw fail("rate_limited", `too many ui updates from ${pl.name}: at most ${UPDATES.perSecond} a second`);
    if (sessionUi.tokens < 1) throw fail("rate_limited", `too many ui updates from the session's plugins: at most ${SESSION.perSecond} a second`);
    pl.ui.tokens -= 1;
    sessionUi.tokens -= 1;
    if (action && !pl.actions.includes(action)) throw fail("no_such_action", `${pl.name} didn't offer action ${action} in hello (it offers: ${pl.actions.join(", ") || "none"})`);
    return pl;
  };
  const changed = (pl: Plugin) => {
    ctx.changed();
    return uiOf(pl);
  };

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
      ctx.changed(); // its actions reach the palette
      return { name: pl.name, protocol: PROTOCOL, session: ctx.session, epoch: ctx.epoch };
    },
    "plugin.invoke": async (p) => {
      const pl = need(p.plugin);
      const conn = pl.client?.conn;
      const run = pl.run;
      if (!conn || !run || run.revoked) throw fail("plugin_unavailable", `${p.plugin} isn't connected (${pl.status}${pl.error ? `: ${pl.error}` : ""})`);
      if (p.run && p.run !== run.id) throw fail("plugin_unavailable", `${p.plugin} has restarted since that was shown; use what it shows now`);
      // an action aimed at a pane (from a menu, key or the palette) reaches that process or nothing
      if (p.target && ctx.s.panes.get(p.target.pane)?.info.instance !== p.target.instance) throw fail("pane_gone", `pane ${p.target.pane} has closed or restarted since`);
      if (!pl.actions.includes(p.action)) throw fail("no_such_action", `${p.plugin} has no action ${p.action} (it offers: ${pl.actions.join(", ") || "none"})`);
      // a clicked URL reaches only an action whose link pattern matches it
      if (p.link && !(pl.manifest?.links ?? []).some((l) => l.action === p.action && linkMatches(l, p.link!))) throw fail("invalid_params", `${p.plugin}'s ${p.action} doesn't handle that link`);
      const invocation = `${pl.name}-${++invocations}`;
      try {
        return (await conn.request("plugin.action", { action: p.action, params: p.params ?? {}, invocation, ...(p.target && { target: p.target }), ...(p.link && { link: p.link }) }, { timeoutMs: INVOKE_MS })) ?? null;
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

    // ---------- the TUI ----------
    "ui.status.set": (p, c) => {
      const pl = uiCall(c, p.action);
      if (!pl.ui.status.has(p.id) && pl.ui.status.size >= LIMIT.statusSegments) throw fail("error", `at most ${LIMIT.statusSegments} status segments per plugin`);
      if (!pl.ui.status.has(p.id) && others(pl).reduce((n, x) => n + x.ui.status.size, pl.ui.status.size) >= SESSION.statusSegments) throw fail("error", `at most ${SESSION.statusSegments} status segments across the session's plugins`);
      pl.ui.status.set(p.id, { id: p.id, text: cleanText(p.text, LIMIT.statusText), tone: p.tone as Tone, ...(p.action && { action: p.action }) });
      return changed(pl);
    },
    "ui.status.clear": (p, c) => {
      const pl = uiCall(c);
      pl.ui.status.delete(p.id);
      return changed(pl);
    },
    "ui.sidebar.set": (p, c) => {
      const pl = uiCall(c);
      for (const row of p.rows) if (row.action && !pl.actions.includes(row.action)) throw fail("no_such_action", `${pl.name} didn't offer action ${row.action} in hello`);
      if (!pl.ui.sidebar && others(pl).filter((x) => x.ui.sidebar).length >= SESSION.sidebarSections) throw fail("error", `at most ${SESSION.sidebarSections} plugins' sidebar sections in a session`);
      // a row that focuses a pane names the process it's for; clicking it later reaches that process or nothing
      for (const row of p.rows) if (row.pane && ctx.s.panes.get(row.pane)?.info.instance !== row.instance) throw fail("pane_gone", `no pane ${row.pane} with instance ${row.instance ?? "(none given: a row's pane needs its instance)"}`);
      pl.ui.sidebar = {
        title: cleanText(p.title, LIMIT.sidebarTitle),
        rows: p.rows.slice(0, LIMIT.sidebarRows).map((row: any) => ({ text: cleanText(row.text, LIMIT.rowText), tone: row.tone, ...(row.action && { action: row.action }), ...(row.pane && { pane: row.pane, instance: row.instance }) })),
      };
      return changed(pl);
    },
    "ui.sidebar.clear": (_p, c) => {
      const pl = uiCall(c);
      pl.ui.sidebar = undefined;
      return changed(pl);
    },
    "ui.badge.set": (p, c) => {
      const pl = uiCall(c);
      if (ctx.s.panes.get(p.pane)?.info.instance !== p.instance) throw fail("pane_gone", `no pane ${p.pane} with instance ${p.instance}: it closed or was restarted`);
      if (!pl.ui.badges.has(p.pane) && pl.ui.badges.size >= LIMIT.badges) throw fail("error", `at most ${LIMIT.badges} badges per plugin`);
      if (!pl.ui.badges.has(p.pane) && others(pl).filter((x) => x.ui.badges.has(p.pane)).length >= SESSION.badgesPerPane) throw fail("error", `at most ${SESSION.badgesPerPane} plugins' badges on one pane`);
      pl.ui.badges.set(p.pane, { pane: p.pane, instance: p.instance, text: cleanText(p.text, LIMIT.badgeText), tone: p.tone });
      return changed(pl);
    },
    "ui.badge.clear": (p, c) => {
      const pl = uiCall(c);
      pl.ui.badges.delete(p.pane);
      return changed(pl);
    },
    "ui.menu.set": (p, c) => {
      const pl = uiCall(c);
      for (const item of p.items) if (!pl.actions.includes(item.action)) throw fail("no_such_action", `${pl.name} didn't offer action ${item.action} in hello`);
      if (others(pl).reduce((n, x) => n + x.ui.menu.length, Math.min(p.items.length, LIMIT.menuItems)) > SESSION.menuItems) throw fail("error", `at most ${SESSION.menuItems} menu entries across the session's plugins`);
      pl.ui.menu = p.items.slice(0, LIMIT.menuItems).map((item: any) => ({ id: item.id, title: cleanText(item.title, LIMIT.menuTitle), action: item.action }));
      return changed(pl);
    },
    // not stored: every attached client shows it as a toast (and a system notification, if asked and the user has
    // those on), at most a few per plugin in a window
    "ui.toast": (p, c) => {
      const pl = uiCall(c);
      const now = Date.now();
      pl.ui.toasts = pl.ui.toasts.filter((at) => now - at < TOASTS.windowMs);
      if (pl.ui.toasts.length >= TOASTS.burst) throw fail("rate_limited", `too many toasts from ${pl.name}: at most ${TOASTS.burst} every ${TOASTS.windowMs / 1000}s`);
      sessionUi.toasts = sessionUi.toasts.filter((at) => now - at < TOASTS.windowMs);
      if (sessionUi.toasts.length >= SESSION.toasts) throw fail("rate_limited", `too many toasts from the session's plugins: at most ${SESSION.toasts} every ${TOASTS.windowMs / 1000}s`);
      pl.ui.toasts.push(now);
      sessionUi.toasts.push(now);
      ctx.broadcast("plugin.toast", { plugin: pl.name, text: cleanText(p.text, LIMIT.toastText), tone: p.tone, system: !!p.system });
      return true;
    },
    "ui.state": (p) => uiOf(need(p.plugin)),

    // ---------- panes ----------
    "plugin.pane.open": (p, c) => {
      const pl = need(p.plugin);
      if (!pl.run || pl.run.revoked) throw fail("plugin_unavailable", `${p.plugin} isn't running`);
      if (p.run && p.run !== pl.run.id) throw fail("plugin_unavailable", `${p.plugin} has restarted since that was shown; use what it shows now`);
      const def = pl.manifest?.panes?.find((x) => x.id === p.pane);
      if (!def) throw fail("no_such_action", `${p.plugin} has no pane ${p.pane} (its plugin.json panes: ${pl.manifest?.panes?.map((x) => x.id).join(", ") || "none"})`);
      const { s } = ctx;
      // the pane it's for, as it was when asked for: never whatever pane has that id now
      if (p.from?.instance && s.panes.get(p.from.pane)?.info.instance !== p.from.instance) throw fail("pane_gone", `pane ${p.from.pane} has closed or restarted since`);
      const origin = p.from ? s.panes.get(p.from.pane) : s.focusedId ? s.panes.get(s.focusedId) : undefined;
      const context = { plugin: pl.name, pane: origin?.id, instance: origin?.info.instance, params: p.params ?? {} };
      const opts: SpawnOpts = { command: def.run.map(quote).join(" "), cwd: pl.dir, createdBy: `plugin:${pl.name}`, env: { ...dirsOf(pl.name), SHEPHERD_PLUGIN_CONTEXT: JSON.stringify(context) } };
      const inLayout = origin && s.locate(origin.id) ? origin.id : s.focusedId;
      let pane: PtyPane | undefined;
      switch (def.placement) {
        case "tab":
          pane = s.newTab(def.title, opts);
          break;
        case "split":
        case "zoomed":
        case "overlay": {
          const wasZoomed = inLayout ? (s.locate(inLayout)?.tab.zoomed ?? false) : false;
          pane = s.split("row", { ...opts, ephemeral: def.placement === "overlay" }, inLayout, true);
          if (!pane) throw fail("error", "there's no pane to open it next to");
          if (def.placement !== "split") {
            s.locate(pane.id)!.tab.zoomed = true;
            s.layout();
          }
          if (def.placement === "overlay") overlays.set(pane.id, { plugin: pl.name, origin: origin?.id, instance: origin?.info.instance, wasZoomed });
          break;
        }
        case "popup":
          if (!c.attached) throw fail("usage", "a popup opens in a TUI client (a key or the palette); from the CLI use a split, tab or zoomed pane");
          if (popups.size) throw fail("ui_busy", "a popup is already open in this session");
          pane = s.spawnHidden(opts);
          popups.set(pane.id, { plugin: pl.name, client: c });
          break;
      }
      pane!.info.title = def.title;
      ctx.changed();
      return { pane: pane!.id, instance: pane!.info.instance, placement: def.placement, title: def.title, ...(def.width !== undefined && { width: def.width }), ...(def.height !== undefined && { height: def.height }) };
    },
    "plugin.popup.resize": (p, c) => {
      const popup = popups.get(p.pane);
      if (!popup || popup.client !== c) throw fail("no_such_pane", `no popup ${p.pane} opened by this client`);
      ctx.s.panes.get(p.pane)?.resize(p.cols, p.rows);
      return true;
    },
    "plugin.popup.close": (p, c) => {
      const popup = popups.get(p.pane);
      if (!popup || popup.client !== c) throw fail("no_such_pane", `no popup ${p.pane} opened by this client`);
      popups.delete(p.pane);
      ctx.s.dropHidden(p.pane);
      return true;
    },
    "ui.popup.close": (_p, c) => {
      const pl = uiCall(c);
      for (const [id, popup] of popups) {
        if (popup.plugin !== pl.name) continue;
        popups.delete(id);
        ctx.s.dropHidden(id);
      }
      return true;
    },
  };

  const disconnected = (c: Client) => {
    for (const pl of plugins.values()) {
      if (pl.client !== c) continue;
      pl.client = undefined;
      pl.actions = [];
      ctx.changed(); // its palette actions go; what it put in the TUI stays until its run ends
    }
    // a popup is its client's: that client is gone
    for (const [id, popup] of popups) {
      if (popup.client !== c) continue;
      popups.delete(id);
      ctx.s.dropHidden(id);
    }
  };

  return { methods, start, stop, disconnected, uiView, paneExited, paneClosing };
}
