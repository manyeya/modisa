// Data that crosses the wire: what the server sends clients, and what the API returns.
import type { Node } from "../core/layout";

export type AgentState = "working" | "blocked" | "done" | "idle";

// Agent state changes the server tells clients about (the "notify" event), each configurable.
export type NotifyEvent = Exclude<AgentState, "idle">;

// An agent integration on the server's machine: whether it's installed and current, and whether the
// agent is there at all (on PATH, or its config directory exists).
export type IntegrationStatus = { id: string; name: string; kind: "lifecycle" | "session"; status: "current" | "outdated" | "none"; available: boolean; configured: boolean };

// A failed request's stable code (JSON-RPC error.data.code); the CLI maps some to exit statuses.
export type ErrorCode = "error" | "usage" | "unreachable" | "timeout" | "invalid_params" | "unknown_method" | "no_such_pane" | "pane_gone";

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
};

export type TabView = { id: string; name?: string; tree: Node; focused: string; zoomed: boolean };
export type WorkspaceView = { id: string; name: string; cwd: string; active: number; tabs: TabView[] };

// Everything a client needs to draw the session.
export type View = { active: number; workspaces: WorkspaceView[]; panes: PaneInfo[] };
