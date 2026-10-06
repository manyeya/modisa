// Data that crosses the wire: what the server sends clients, and what the API returns.
import type { Node } from "../core/layout";

export type AgentState = "working" | "blocked" | "done" | "idle";

// Agent state changes the server tells clients about (the "notify" event), each configurable.
export type NotifyEvent = Exclude<AgentState, "idle">;

// An agent integration on the server's machine: whether it's installed and current, and whether the
// agent is there at all (on PATH, or its config directory exists).
export type IntegrationStatus = { id: string; name: string; kind: "lifecycle" | "session"; status: "current" | "outdated" | "none"; available: boolean; configured: boolean };

// A failed request's stable code (JSON-RPC error.data.code); the CLI maps some to exit statuses.
export const ERROR_CODES = ["error", "usage", "unreachable", "timeout", "invalid_params", "unknown_method", "no_such_pane", "pane_gone", "no_such_plugin", "no_such_action", "plugin_unavailable", "plugin_error", "already_running", "rate_limited", "ui_busy"] as const;
export type ErrorCode = (typeof ERROR_CODES)[number];

// A plugin as its host sees it. status: running; exited (code 0), failed (nonzero, or it couldn't start: a bad
// manifest, another protocol version, a missing program) or stopped (by modisa, when the session stopped).
export type PluginStatus = {
  name: string;
  source: "linked" | "config"; // a linked plugin.json, or a [[plugin]] run line in config.toml
  dir?: string;
  status: "starting" | "running" | "exited" | "failed" | "stopped"; // starting: known to the server, not launched yet
  pid?: number;
  exitCode?: number;
  signal?: string;
  error?: string;
  log: string;
  connected: boolean; // it has said plugin.hello on a connection that's still open
  actions: string[]; // what `modisa plugin run` can call
  group?: "running" | "gone"; // its process group: children can outlive the process modisa started
  invocations?: number; // action calls sent to it and not yet answered or timed out
  keys?: PluginKey[]; // its keys as the server's config binds them: active, or disabled and why
  install?: { source: string; ref: string | null; commit: string; marketplace?: string }; // fetched with `modisa plugin install` (from a marketplace)
};

// A plugin key: `key` after the prefix runs `action` or opens `pane`. Disabled when it's one of modisa's keys or
// reserved, when another plugin wants the same key (both are disabled), or when [plugin_keys] turns it off.
export type PluginKey = { key: string; action?: string; pane?: string; description: string; state: "active" | "disabled"; reason?: string };

export type PaneInfo = {
  id: string;
  instance: string; // random per spawned process: tells a pane apart from a later one given the same id or name
  name?: string;
  title: string;
  terminalTitle?: string; // the title the program last set (OSC 0/2), e.g. an agent's task: a named pane's `title` stays its name
  cwd: string;
  command?: string; // set for process/agent panes; undefined = interactive shell
  harness?: string; // adapter id when spawned as an agent
  createdBy: string; // pane id or "user"
  status: "running" | "exited";
  exitCode?: number;
  agent?: { harness: string; state: AgentState; source: "hook" | "screen" };
  session?: { agent: string; id: string; source: string }; // the agent's own session, for exact resume
  cols: number;
  rows: number;
  popup?: boolean; // a plugin's popup: no place in the layout, shown only by the client that opened it
  takeover?: boolean; // driven from another terminal (pane attach): at its size, and typing from elsewhere is dropped
};

// The pane a target names: "p3", "@coder", "coder", "@p3" (ids still work once a pane is named), "p3:1a2b3c4d" (only that
// instance of p3). The server resolves targets with it, and so does the CLI where it works from a snapshot.
export function findPane<T extends { id: string; instance: string; name?: string }>(panes: T[], target: string): T | undefined {
  const inst = /^(p\d+):(\w+)$/.exec(target);
  if (inst) return panes.find((p) => p.id === inst[1] && p.instance === inst[2]);
  const name = target.replace(/^@/, "");
  return panes.find((p) => p.id === target) ?? panes.find((p) => p.name === name) ?? panes.find((p) => p.id === name);
}

export type TabView = { id: string; name?: string; tree: Node; focused: string; zoomed: boolean };
// A space's repository, where its focused pane is: ahead/behind are only there when the branch has an upstream.
export type GitView = { repo: string; branch: string; ahead?: number; behind?: number; changes: number };
export type WorkspaceView = { id: string; name: string; cwd: string; active: number; tabs: TabView[]; git?: GitView };

// What a plugin's current run shows in the TUI, from its ui.* calls: drawn by modisa, in the user's theme.
// The theme's own colours: text, dim, accent, warning, and the four agent states.
export type Tone = "fg" | "dim" | "accent" | "warn" | "working" | "blocked" | "done" | "idle";
// A piece of a sidebar row: text in a tone (bold if asked), or an agent's mark: `icon` names a built-in agent
// (claude-code, codex, …) and modisa draws its glyph in its brand colour.
export type Span = { text: string; tone?: Tone; bold?: boolean } | { icon: string };
export type PluginUiView = {
  plugin: string;
  run: string; // the run that set it: an action taken from what it showed is refused once that run has ended
  actions: { id: string; title: string; description?: string }[]; // offered by the connected run: palette entries
  status: { id: string; text: string; tone: Tone; action?: string }[]; // status bar segments
  sidebar?: { title: string; rows: { text: string; tone: Tone; spans?: Span[]; action?: string; pane?: string; instance?: string }[] }; // a sidebar section; a row's pane comes with its instance, and `text` is its spans as plain text
  badges: { pane: string; instance: string; text: string; tone: Tone }[]; // labels on pane borders
  menu: { id: string; title: string; action: string }[]; // pane context menu entries
  keys: { key: string; action?: string; pane?: string; description: string }[]; // plugin.json's, under the prefix: each client binds them with its own [plugin_keys]
  panes: { id: string; title: string; placement: "overlay" | "popup" | "split" | "tab" | "zoomed" }[]; // it can open (plugin.pane.open)
  links: { pattern?: string; regex?: string; action: string }[]; // URLs Ctrl+click hands to an action, in manifest order (src/protocol/links.ts)
};

// ---------- plugin views: element trees a plugin shows, drawn by the client in the user's theme ----------

// A size in cells, or a share of the parent ("50%").
export type ViewSize = number | `${number}%`;
// What every element takes: `key` keeps an element's state (focus, scroll, what's typed) across updates, and names it
// to the plugin when it's used; the rest place it in its parent's flex layout.
export type ViewLayout = { key?: string; width?: ViewSize; height?: ViewSize; minWidth?: number; maxWidth?: number; minHeight?: number; maxHeight?: number; grow?: number; shrink?: number };
// Styled inline text inside a Text: plain strings, spans, and agents' marks (`icon` names a built-in agent).
export type ViewInline = string | { type: "span"; tone?: Tone; bold?: boolean; italic?: boolean; underline?: boolean; dim?: boolean; strike?: boolean; children?: ViewInline[] } | { type: "icon"; agent: string };
// An option of a Select or Tabs: what it says, and the value an action gets when it's chosen (its name when absent).
export type ViewOption = { name: string; description?: string; value?: string };
// What a Button, Input, Select and Tabs run: one of the plugin's actions, with params of its own.
type ViewAct = { action?: string; params?: Record<string, unknown> };
export type ViewNode = ViewLayout &
  (
    | { type: "box"; direction?: "row" | "column"; gap?: number; padding?: number; paddingX?: number; paddingY?: number; align?: "start" | "center" | "end" | "stretch"; justify?: "start" | "center" | "end" | "between" | "around" | "evenly"; wrap?: boolean; border?: boolean | "single" | "double" | "rounded" | "heavy"; title?: string; tone?: Tone; bg?: Tone; children?: ViewNode[] }
    | { type: "scroll"; sticky?: "top" | "bottom"; children?: ViewNode[] }
    | { type: "text"; tone?: Tone; bold?: boolean; italic?: boolean; underline?: boolean; dim?: boolean; strike?: boolean; wrap?: "word" | "char" | "none"; children?: ViewInline[] }
    | { type: "markdown"; content: string }
    | { type: "code"; content: string; filetype?: string; lineNumbers?: boolean }
    | ({ type: "diff"; diff: string; view?: "unified" | "split"; filetype?: string; lineNumbers?: boolean; cursor?: boolean; marks?: number[]; change?: string } & ViewAct) // cursor: a line cursor (j/k), Enter runs action
    | { type: "table"; rows: (string | ViewInline[])[][]; header?: boolean; border?: boolean }
    | { type: "bigtext"; text: string; font?: "tiny" | "block" | "shade" | "slick" | "huge" | "grid" | "pallet"; tone?: Tone }
    | { type: "progress"; value: number; tone?: Tone }
    | { type: "sparkline"; values: number[]; tone?: Tone; min?: number; max?: number }
    | { type: "chart"; series: { values: number[]; tone?: Tone }[]; min?: number; max?: number }
    | { type: "gauge"; value: number; tone?: Tone; label?: string }
    | { type: "heatmap"; values: number[][]; tone?: Tone; min?: number; max?: number }
    | { type: "raster"; key: string; columns: number; rows: number; cells: string } // see RASTER below
    | { type: "image"; png: string; alt?: string; fit?: "fit" | "cover" | "fill" } // base64 PNG bytes
    | { type: "spinner"; tone?: Tone; label?: string }
    | ({ type: "button"; label: string; tone?: Tone } & ViewAct)
    | ({ type: "input"; placeholder?: string; value?: string; maxLength?: number } & ViewAct)
    | ({ type: "textarea"; placeholder?: string; value?: string } & ViewAct)
    | ({ type: "select"; options: ViewOption[]; selected?: number; change?: string } & ViewAct) // action on Enter, change on moving
    | ({ type: "tabs"; options: ViewOption[]; selected?: number } & ViewAct)
  );
// A Raster's cells: base64 of `columns * rows` little-endian u32 triplets [codePoint, fg, bg], row-major. A colour is
// 0x00RRGGBB, RASTER.DEFAULT for the cell's default, or RASTER.TONE | the index of a tone in TONES.
export const RASTER = { DEFAULT: 0x01000000, TONE: 0x02000000 } as const;
export const TONES: readonly Tone[] = ["fg", "dim", "accent", "warn", "working", "blocked", "done", "idle"];
// Keys a view binds while it has focus (and what's focused in it doesn't take the key): "j", "S-tab", "C-s", "enter".
export type ViewKey = { key: string; action: string; params?: Record<string, unknown>; description?: string };
// A view as the server holds it and sends it to clients. `placement` is where it opens: floating over everything
// (popup), or over the pane it's `from` (overlay; a popup while that pane isn't on screen). `close` is the action run
// when the user closes it; `focus` the element this update hands the keyboard to. `rev` rises with every change.
export type PluginViewState = {
  plugin: string;
  run: string;
  id: string;
  title: string;
  placement: "popup" | "overlay";
  width?: ViewSize;
  height?: ViewSize;
  from?: { pane: string; instance: string };
  keys: ViewKey[];
  close?: string;
  focus?: string; // the key of the element this rev gives the keyboard to
  root: ViewNode;
  rev: number;
};
// What a view's element tells its plugin when it's used: the view, the element's key, and the value it has (an
// Input's text, a Select's chosen option) as `call.ui` beside the action's params.
export type ViewEvent = { view: string; key?: string; value?: string; index?: number };

// The plugin UI a client understands, sent with attach: the server sends plugins' UI (in views, plugin toasts, popups)
// only to clients at this version or later, so an older client never gets what it can't draw. Bump on a change an
// older client would misdraw.
export const PLUGIN_UI = 2;

// Everything a client needs to draw the session.
export type View = { active: number; workspaces: WorkspaceView[]; panes: PaneInfo[]; plugins?: PluginUiView[] };
