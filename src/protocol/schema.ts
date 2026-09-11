// Wire protocol: JSON-RPC 2.0, newline-delimited, over a unix socket (or ssh stdio for --remote).
// Requests get responses; the server pushes events as notifications ({method, params}, no id).
import { z } from "zod";

export type Msg = {
  jsonrpc: "2.0";
  id?: number;
  method?: string;
  params?: any;
  result?: any;
  error?: { code: number; message: string };
};

// The public API: the server validates params with these; the CLI and MCP build params from them.
const target = z.string().min(1);
const caller = z.string().optional();
const dir = z.enum(["right", "down"]).default("right");
const state = z.enum(["working", "blocked", "done", "idle"]);

export const api = {
  list: z.object({ caller }),
  "session.info": z.object({ caller }),
  "workspace.list": z.object({ caller }),
  "workspace.create": z.object({ caller, name: z.string().optional(), cwd: z.string().optional(), command: z.string().optional() }),
  "workspace.rename": z.object({ caller, workspace: z.string().min(1), name: z.string().trim().min(1) }),
  "workspace.close": z.object({ caller, workspace: z.string().min(1) }),
  "tab.create": z.object({ caller, name: z.string().optional(), workspace: z.string().optional(), command: z.string().optional(), paneName: z.string().optional(), cwd: z.string().optional() }),
  "pane.split": z.object({ caller, target: target.optional(), dir, name: z.string().optional(), cwd: z.string().optional(), command: z.string().optional(), focus: z.boolean().optional() }),
  "pane.run": z.object({ caller, target, command: z.string() }),
  "pane.read": z.object({ caller, target: target.optional(), lines: z.number().int().positive().max(10_000).default(50) }),
  "pane.keys": z.object({ caller, target, keys: z.array(z.string()).min(1) }),
  "pane.close": z.object({ caller, target: target.optional() }),
  "pane.rename": z.object({ caller, target: target.optional(), name: z.string() }),
  "pane.focus": z.object({ caller, target }),
  "agent.spawn": z.object({ caller, harness: z.string(), name: z.string().optional(), prompt: z.string().optional(), target: target.optional(), dir, tab: z.boolean().optional(), focus: z.boolean().optional() }),
  "agent.list": z.object({ caller }),
  wait: z.object({ caller, target, exited: z.boolean().optional(), state: state.optional(), match: z.string().optional(), timeout: z.number().positive().optional() }),
  "events.subscribe": z.object({ caller, output: z.boolean().optional() }),
  // from integrations: lifecycle state (authoritative for the pane until released or the agent exits),
  // the agent's own session id (for exact resume), or both
  report: z.object({
    caller, pane: z.string().optional(), source: z.string().min(1).optional(), agent: z.string().optional(),
    state: state.optional(), seq: z.number().optional(), session: z.string().min(1).optional(), release: z.boolean().optional(),
  }),
  send: z.object({ caller, to: target, body: z.string().min(1) }),
  inbox: z.object({ caller }),
  messages: z.object({ caller }),
  "messaging.pause": z.object({ caller, paused: z.boolean().optional() }),
  "debug.detect": z.object({ caller, target }),
  integrations: z.object({ caller }),
  integration: z.object({ caller, id: z.string().min(1), install: z.boolean() }),
  kill: z.object({ caller }),
  restart: z.object({ caller }),
} as const;
