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
// manifest, another protocol version, a missing program) or stopped (by shepherd, when the session stopped).
export type PluginStatus = {
  name: string;
  source: "linked" | "config"; // a linked plugin.json, or a [[plugin]] run line in config.toml
  dir?: string;
  status: "running" | "exited" | "failed" | "stopped";
  pid?: number;
  exitCode?: number;
  signal?: string;
  error?: string;
  log: string;
  connected: boolean; // it has said plugin.hello on a connection that's still open
  actions: string[]; // what `shepherd plugin run` can call
  group?: "running" | "gone"; // its process group: children can outlive the process shepherd started
  invocations?: number; // action calls sent to it and not yet answered or timed out
  keys?: PluginKey[]; // its keys as the server's config binds them: active, or disabled and why
  install?: { source: string; ref: string | null; commit: string }; // fetched with `shepherd plugin install`
};

// A plugin key: `key` after the prefix runs `action` or opens `pane`. Disabled when it's one of shepherd's keys or
// reserved, when another plugin wants the same key (both are disabled), or when [plugin_keys] turns it off.
export type PluginKey = { key: string; action?: string; pane?: string; description: string; state: "active" | "disabled"; reason?: string };

export type PaneInfo = {
  id: string;
  instance: string; // random per spawned process: tells a pane apart from a later one given the same id or name
  name?: string;
  title: string;
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
};

export type TabView = { id: string; name?: string; tree: Node; focused: string; zoomed: boolean };
export type WorkspaceView = { id: string; name: string; cwd: string; active: number; tabs: TabView[] };

// What a plugin's current run shows in the TUI, from its ui.* calls: drawn by shepherd, in the user's theme.
export type Tone = "fg" | "dim" | "accent" | "warn";
export type PluginUiView = {
  plugin: string;
  run: string; // the run that set it: an action taken from what it showed is refused once that run has ended
  actions: { id: string; title: string; description?: string }[]; // offered by the connected run: palette entries
  status: { id: string; text: string; tone: Tone; action?: string }[]; // status bar segments
  sidebar?: { title: string; rows: { text: string; tone: Tone; action?: string; pane?: string; instance?: string }[] }; // a sidebar section; a row's pane comes with its instance
  badges: { pane: string; instance: string; text: string; tone: Tone }[]; // labels on pane borders
  menu: { id: string; title: string; action: string }[]; // pane context menu entries
  keys: { key: string; action?: string; pane?: string; description: string }[]; // plugin.json's, under the prefix: each client binds them with its own [plugin_keys]
  panes: { id: string; title: string; placement: "overlay" | "popup" | "split" | "tab" | "zoomed" }[]; // it can open (plugin.pane.open)
  links: { pattern?: string; regex?: string; action: string }[]; // URLs Ctrl+click hands to an action, in manifest order (src/protocol/links.ts)
};

// The plugin UI a client understands, sent with attach: the server sends plugins' UI (in views, plugin toasts, popups)
// only to clients at this version or later, so an older client never gets what it can't draw. Bump on a change an
// older client would misdraw.
export const PLUGIN_UI = 1;

// Everything a client needs to draw the session.
export type View = { active: number; workspaces: WorkspaceView[]; panes: PaneInfo[]; plugins?: PluginUiView[] };
