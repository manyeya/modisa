// The client's shared state: renderer, connection, what the server last sent, config/theme and UI
// state, plus the small queries every module uses. One App per TUI process; modules take it as `app`.
import { BoxRenderable, TextRenderable, type CliRenderer, type KeyEvent } from "@opentui/core";
import { panes as treePanes, type Rect } from "../core/layout";
import type { Conn } from "../protocol/conn";
import type { AgentState, View } from "../protocol/types";
import { parsePrefix, type Config, type IndicatorStyle } from "../config/config";
import { theme, type Theme } from "../config/themes";
import { chrome, fit } from "./design";
import type { ClientPane } from "./panes/pane";
import type { Manifest } from "../cli/update";

// connect(spawn): spawn = start the server if it isn't running (the first attach, or whoever asked for a restart)
export type ClientOptions = { session: string; connect: (spawn: boolean) => Promise<Conn>; remote?: boolean };
export type ServerView = View & { paused: boolean };
export type Option = { name: string; description: string; value: string };
// keepEscape: Escape goes to what's in the modal (a plugin popup's program), not to closing it
export type Modal = { close: (v: any) => void; keys?: (k: KeyEvent) => boolean; resize: () => void; keepEscape?: boolean };
export type Action = { label: string; run: () => any };
export type PointerShape = "default" | "pointer" | "move";

// Agent-state glyphs, one set per [indicators] style.
export const INDICATORS: Record<IndicatorStyle, Record<AgentState, string>> = {
  symbols: { blocked: "!", working: "◆", done: "✓", idle: "○" },
  dots: { blocked: "●", working: "●", done: "●", idle: "○" },
  letters: { blocked: "B", working: "W", done: "D", idle: "I" },
};

export class App {
  conn!: Conn; // set by the first attach
  view: ServerView | undefined;
  th: Theme;
  prefix: { ctrl: boolean; name: string };
  readonly panes = new Map<string, ClientPane>();
  readonly ui: { tabBar: BoxRenderable; side: BoxRenderable; telemetry: BoxRenderable; toastBox: TextRenderable };
  actions: Record<string, Action> = {};

  // UI state
  sidebar: boolean;
  prefixArmed = false;
  mode: "normal" | "copy" = "normal";
  modal: Modal | undefined;
  search: { matches: number[]; total: number; i: number } | undefined;
  editing: { index: number; draft: string } | undefined; // a space being renamed in the sidebar
  lastSpaceClick = { index: -1, at: 0 };
  resizing: { x: number; y: number; sawButtonMotion: boolean } | undefined; // a pane border being dragged
  pointerShape: PointerShape = "default";
  readonly clickable = new WeakSet<object>(); // renderables that get the hand pointer
  readonly promptIds = new Map<number, BoxRenderable>(); // open permission prompts
  readonly collapsedPlugins = new Set<string>(); // plugins' sidebar sections the user folded
  popup: { pane: string; title: string; width?: number | string; height?: number | string } | undefined; // a plugin popup this client opened
  chromeSig = ""; // what the tab bar, sidebar and status row last drew
  quitting = false;
  update: Manifest | undefined; // a newer shepherd release, when one is out
  restarting = false; // the server told us it's restarting: wait for the new one instead of giving up
  restartedByUs = false; // we asked for it, so we start the new server
  readonly cleanup: (() => void)[] = []; // run on quit
  private toastTimer: Timer | undefined;

  constructor(readonly r: CliRenderer, readonly opts: ClientOptions, public cfg: Config, readonly debug: (line: string) => void) {
    this.th = theme(cfg);
    this.prefix = parsePrefix(cfg.prefix);
    this.sidebar = cfg.sidebar.visible;
    this.ui = {
      tabBar: new BoxRenderable(r, { position: "absolute", left: 0, top: 0, width: "100%", height: 1, flexDirection: "row", zIndex: 5 }),
      side: new BoxRenderable(r, { position: "absolute", left: 0, top: 1, flexDirection: "column", zIndex: 5, paddingLeft: 1 }),
      telemetry: new BoxRenderable(r, { position: "absolute", left: 0, bottom: 0, width: "100%", height: 1, zIndex: 5, flexDirection: "row" }),
      toastBox: new TextRenderable(r, { position: "absolute", right: 1, top: 1, zIndex: 50, content: "", visible: false }),
    };
    for (const x of Object.values(this.ui)) r.root.add(x);
  }

  // ---------- geometry ----------
  metrics() {
    return chrome(this.r.width, this.r.height, this.sidebar, this.cfg.sidebar.width);
  }
  sideWidth() {
    return this.metrics().side;
  }
  area(): Rect {
    return this.metrics().area;
  }

  // ---------- the server's view ----------
  ws() {
    return this.view!.workspaces[this.view!.active]!;
  }
  tab() {
    return this.ws().tabs[this.ws().active]!;
  }
  info(id: string) {
    return this.view!.panes.find((p) => p.id === id);
  }
  focusedPane() {
    return this.view ? this.panes.get(this.tab().focused) : undefined;
  }
  icon(state: AgentState) {
    return (INDICATORS[this.cfg.indicators.style] ?? INDICATORS.symbols)[state];
  }
  // The current space's agents, what needs you first. Other spaces' agents reach you as notifications.
  sortedAgents() {
    const order = { blocked: 0, done: 1, working: 2, idle: 3 };
    const here = new Set(this.ws().tabs.flatMap((t) => treePanes(t.tree)));
    return this.view!.panes.filter((p) => p.agent && here.has(p.id)).sort((a, b) => order[a.agent!.state] - order[b.agent!.state]);
  }

  // ---------- talking to the user and the server ----------
  call(name: string, args: any = {}) {
    return this.conn.request("cmd", { name, args }).catch((e) => this.toast(String(e.message ?? e), this.th.blocked));
  }

  toast(text: string, color = this.th.fg, ms = 4000) {
    if (this.quitting) return;
    const { toastBox } = this.ui;
    const content = fit(` ${text} `, Math.max(1, this.r.width - 2));
    toastBox.content = content;
    toastBox.top = this.metrics().top;
    toastBox.width = Math.max(1, Bun.stringWidth(content));
    toastBox.fg = color;
    toastBox.bg = this.th.bar;
    toastBox.visible = true;
    clearTimeout(this.toastTimer);
    this.toastTimer = setTimeout(() => (toastBox.visible = false), ms);
  }

  clearToast() {
    clearTimeout(this.toastTimer);
    this.ui.toastBox.visible = false;
  }

  setConfig(cfg: Config) {
    this.cfg = cfg;
    this.th = theme(cfg);
    this.prefix = parsePrefix(cfg.prefix);
    this.paintBackground();
  }

  // The theme's background in shepherd's own cells, and as the terminal's default background (OSC 11), so the window
  // padding a terminal draws around its cells matches the TUI instead of framing it in the terminal's own colour.
  // The terminal gets its own colour back (OSC 111) when the client exits: OpenTUI's destroy() doesn't do it.
  private backgroundResetOnExit = false;
  paintBackground() {
    this.r.setBackgroundColor(this.th.bg);
    const hex = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(this.th.bg);
    if (!hex) return;
    // ponytail: writeOut is OpenTUI's private output path (its own OSC sequences use it), so this can't split a frame
    (this.r as any).writeOut(`\x1b]11;rgb:${hex[1]}/${hex[2]}/${hex[3]}\x07`);
    if (this.backgroundResetOnExit) return;
    this.backgroundResetOnExit = true;
    process.once("exit", () => process.stdout.write("\x1b]111\x07")); // detach, quit, or any exit that runs its hooks
  }
}
