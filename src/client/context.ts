// The client's shared state: renderer, connection, what the server last sent, config/theme and UI
// state, plus the small queries every module uses. One App per TUI process; modules take it as `app`.
import { BoxRenderable, TextRenderable, type CliRenderer, type KeyEvent, type MouseEvent } from "@opentui/core";
import { panes as treePanes, type Rect } from "../core/layout";
import type { Conn } from "../protocol/conn";
import type { AgentState, View } from "../protocol/types";
import { parsePrefix, type Config, type IndicatorStyle } from "../config/config";
import { theme, type Theme } from "../config/themes";
import { cellEms, chrome, fit } from "./design";
import type { ClientPane } from "./panes/pane";
import type { Manifest } from "../cli/update";

// connect(spawn): spawn = start the server if it isn't running (the first attach, or whoever asked for a restart)
export type ClientOptions = { session: string; connect: (spawn: boolean) => Promise<Conn>; remote?: boolean };
export type ServerView = View & { paused: boolean };
export type Option = { name: string; description: string; value: string; key?: string; context?: (e: MouseEvent) => void }; // key: a keycap on the right; context: its right-click menu
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

const TOASTS = 3; // cards shown at once; an older one beyond that goes

export class App {
  conn!: Conn; // set by the first attach
  view: ServerView | undefined;
  th: Theme;
  prefix: { ctrl: boolean; name: string };
  readonly panes = new Map<string, ClientPane>();
  readonly ui: { tabBar: BoxRenderable; side: BoxRenderable; telemetry: BoxRenderable; toasts: { box: BoxRenderable; text: TextRenderable }[] };
  actions: Record<string, Action> = {};

  // UI state
  sidebar: boolean;
  prefixArmed = false;
  mode: "normal" | "copy" = "normal";
  modal: Modal | undefined;
  search: { matches: number[]; total: number; i: number } | undefined;
  resizing: { x: number; y: number; sawButtonMotion: boolean; sidebar?: boolean } | undefined; // a pane border, or the sidebar's edge, being dragged
  pointerShape: PointerShape = "default";
  logos: false | "whole" | "halves" = false; // agents' logos, as much of modisa's logo font as this terminal has: see ./logos.ts
  cellGuess = 1.2; // this terminal's cell height in ems, from its font, when it doesn't say its size in pixels
  // this terminal's cell height in ems of its font: where a logo's halves meet
  cellEms() {
    const px = this.r.resolution;
    return px && px.width > 0 && px.height > 0 ? cellEms(px, this.r.width, this.r.height) : this.cellGuess;
  }
  readonly clickable = new WeakSet<object>(); // renderables that get the hand pointer
  readonly promptIds = new Set<number>(); // open permission prompts
  readonly collapsedPlugins = new Set<string>(); // plugins' sidebar sections the user folded
  readonly collapsedTabs = new Set<string>(); // tabs (by id) whose agents the user folded in the sidebar's graph
  popup: { pane: string; title: string; width?: number | string; height?: number | string } | undefined; // a plugin popup this client opened
  chromeSig = ""; // what the tab bar, sidebar and status row last drew
  quitting = false;
  update: Manifest | undefined; // a newer modisa release, when one is out
  restarting = false; // the server told us it's restarting: wait for the new one instead of giving up
  restartedByUs = false; // we asked for it, so we start the new server
  readonly cleanup: (() => void)[] = []; // run on quit
  private toasts: { text: string; color: Theme["fg"]; title?: string; timer: Timer }[] = []; // newest first

  constructor(readonly r: CliRenderer, readonly opts: ClientOptions, public cfg: Config, readonly debug: (line: string) => void) {
    this.th = theme(cfg);
    this.prefix = parsePrefix(cfg.prefix);
    this.sidebar = cfg.sidebar.visible;
    this.ui = {
      tabBar: new BoxRenderable(r, { position: "absolute", left: 0, top: 0, width: "100%", height: 1, flexDirection: "row", zIndex: 5 }),
      side: new BoxRenderable(r, { position: "absolute", left: 0, top: 1, flexDirection: "column", zIndex: 5, paddingLeft: 1 }),
      telemetry: new BoxRenderable(r, { position: "absolute", left: 0, bottom: 0, width: "100%", height: 1, zIndex: 5, flexDirection: "row" }),
      // a stack of cards at the top right, the newest on top: one toast doesn't wipe out another. Over dialogs and
      // popups too, so news shows wherever you are
      toasts: Array.from({ length: TOASTS }, () => {
        const box = new BoxRenderable(r, { position: "absolute", right: 1, top: 1, height: 3, zIndex: 300, border: true, borderStyle: "rounded", paddingLeft: 1, paddingRight: 1, visible: false });
        const text = new TextRenderable(r, { content: "" });
        box.add(text);
        return { box, text };
      }),
    };
    const { toasts, ...rest } = this.ui;
    for (const x of [...Object.values(rest), ...toasts.map((t) => t.box)]) r.root.add(x);
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

  // A card in the toast stack, in `color`'s border; `title` names who it's from (a plugin). Warnings and what needs
  // you stay twice as long as the rest.
  toast(text: string, color = this.th.fg, ms?: number, title?: string) {
    if (this.quitting) return;
    ms ??= color === this.th.warn || color === this.th.blocked ? 10000 : 5000;
    const t = { text, color, title, timer: setTimeout(() => ((this.toasts = this.toasts.filter((x) => x !== t)), this.drawToasts()), ms) };
    this.toasts.unshift(t);
    for (const old of this.toasts.splice(TOASTS)) clearTimeout(old.timer);
    this.drawToasts();
  }

  clearToast() {
    for (const t of this.toasts) clearTimeout(t.timer);
    this.toasts = [];
    this.drawToasts();
  }

  drawToasts() {
    let top = this.metrics().top;
    this.ui.toasts.forEach(({ box, text }, i) => {
      const t = this.toasts[i];
      box.visible = !!t;
      if (!t) return;
      const content = fit(t.text, Math.max(1, this.r.width - 6));
      const width = Math.max(Bun.stringWidth(content), Bun.stringWidth(t.title ?? "") + 2) + 4; // border and padding
      Object.assign(box, { top, width, borderColor: t.color, backgroundColor: this.th.bar, title: t.title ? ` ${t.title} ` : undefined, titleColor: t.color });
      Object.assign(text, { content, fg: this.th.fg });
      top += 3;
    });
  }

  setConfig(cfg: Config) {
    this.cfg = cfg;
    this.th = theme(cfg);
    this.prefix = parsePrefix(cfg.prefix);
    this.paintBackground();
  }

  // The theme's background in modisa's own cells, and as the terminal's default background (OSC 11), so the window
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
