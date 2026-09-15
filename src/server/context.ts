// Everything a running session server holds, shared by its modules (RPC handlers, the agent monitor,
// permissions). Built once in server.ts; config and adapters are swapped in place on reload.
import type { Config } from "../config/config";
import type { Adapter } from "../config/adapters";
import type { Conn } from "../protocol/conn";
import { b64, fail } from "../protocol/conn";
import type { PluginUiView } from "../protocol/types";
import { Session, type SpawnOpts } from "./session/session";
import type { PtyPane } from "./session/pane";
import { Detector } from "./agents/detect";
import { Mailbox } from "./agents/mailbox";
import { save } from "./persist/store";
import { quote } from "./persist/template";

// plugin: set once a plugin's connection has said plugin.hello; it then acts as that plugin, never as a pane
export type Client = { conn: Conn; attached: boolean; events: boolean; output: boolean; plugin?: string };

export type ServerContext = {
  session: string;
  version: string;
  epoch: string; // this server run, new on every start
  seq: number; // the last event's seq

  cfg: Config;
  adapters: Adapter[];
  s: Session;
  mail: Mailbox;
  detector: Detector;
  clients: Set<Client>;
  down: boolean;
  // set by server.ts / permissions.ts / monitor.ts once they exist
  shutdown: (empty: boolean, why?: "exit" | "restart") => Promise<void>;
  permit: (caller: string | undefined, action: "keys" | "close" | "run", target: PtyPane, detail?: string) => Promise<void>;
  prompts: Map<number, (answer: string) => void>;
  tick: () => Promise<void>;
  attached(): Client[];
  broadcast(event: string, data: any, to?: Client[]): void;
  emit(type: string, data?: any): void;
  changed(): void;
  cancelSave(): void;
  name(id: string): string;
  need(target: string | undefined, caller?: string): PtyPane;
  agentOpts(harness: string, prompt?: string, name?: string, createdBy?: string): SpawnOpts;
  snapshot(p: PtyPane, lines?: number): PtyPane["info"] & { screen: string; recentOutput: string };
  pluginUi(): PluginUiView[]; // what plugins show in the TUI; set by server.ts (plugins.ts)
  paneExited(p: PtyPane): void; // after a pane's process ended (plugins.ts: overlays and popups)
};

export function createContext(session: string, version: string, cfg: Config, adapters: Adapter[]): ServerContext {
  const ctx = { session, version, epoch: crypto.randomUUID().slice(0, 8), seq: 0, cfg, adapters, clients: new Set<Client>(), down: false, prompts: new Map() } as ServerContext;

  ctx.pluginUi = () => [];
  ctx.paneExited = () => {};
  ctx.attached = () => [...ctx.clients].filter((c) => c.attached);
  ctx.broadcast = (event, data, to = ctx.attached()) => to.forEach((c) => c.conn.notify(event, data));
  // shapes: `events` in protocol/schema.ts (an e2e test checks every emitted event against them)
  ctx.emit = (type, data = {}) => {
    const ev = { type, at: Date.now(), seq: ++ctx.seq, epoch: ctx.epoch, ...data };
    for (const c of ctx.clients) if (c.events && (type !== "pane.output" || c.output)) c.conn.notify("event", ev);
  };

  // State changes: coalesce client updates into one view per tick, debounce saves by a second.
  let viewQueued = false;
  let saveTimer: Timer | undefined;
  ctx.changed = () => {
    if (!viewQueued) {
      viewQueued = true;
      queueMicrotask(() => {
        viewQueued = false;
        ctx.broadcast("view", { ...ctx.s.view(), paused: ctx.mail.paused, plugins: ctx.pluginUi() });
      });
    }
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => save(ctx.s, session, () => !ctx.down).catch(() => {}), 1000);
  };
  ctx.cancelSave = () => clearTimeout(saveTimer);

  ctx.s = new Session({
    output: (p, bytes) => {
      ctx.broadcast("output", { pane: p.id, data: b64(bytes) });
      ctx.emit("pane.output", { pane: p.id, instance: p.info.instance, text: new TextDecoder().decode(bytes) });
    },
    exited: (p) => {
      ctx.emit("process.exited", { pane: p.id, instance: p.info.instance, name: p.info.name, exitCode: p.info.exitCode });
      ctx.paneExited(p);
    },
    created: (p) => ctx.emit("pane.created", { pane: p.id, instance: p.info.instance, name: p.info.name, command: p.info.command }),
    changed: () => ctx.changed(),
    empty: () => ctx.shutdown(true),
  });
  ctx.detector = new Detector(() => ctx.adapters);
  ctx.mail = new Mailbox(() => ctx.cfg.messaging);

  ctx.name = (id) => (id === "user" ? "user" : ctx.s.panes.get(id)?.info.name ?? id);
  ctx.need = (target, caller) => {
    const p = ctx.s.resolve(target, caller);
    if (p) return p;
    if (/^p\d+:\w+$/.test(target ?? "")) throw fail("pane_gone", `${target!.split(":")[0]} has gone: that pane was closed or restarted since its message was sent`);
    throw fail("no_such_pane", `no such pane: ${target ?? "(none)"}`);
  };
  ctx.agentOpts = (harness, prompt, name, createdBy) => {
    const a = ctx.adapters.find((x) => x.id === harness);
    const command = a?.launch ? [a.launch, prompt && quote(prompt)].filter(Boolean).join(" ") : harness;
    return { command, harness: a?.id ?? "generic", name, createdBy };
  };
  ctx.snapshot = (p, lines = 50) => {
    const all = p.text().split("\n");
    return { ...p.info, screen: p.screen(), recentOutput: all.slice(-lines).join("\n") };
  };
  return ctx;
}
