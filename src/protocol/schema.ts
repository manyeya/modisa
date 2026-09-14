// Wire protocol: JSON-RPC 2.0, newline-delimited, over a unix socket (or ssh stdio for --remote).
// Requests get responses; the server pushes events as notifications ({method, params}, no id).
import { z } from "zod";
import type { ErrorCode } from "./types";

export type Msg = {
  jsonrpc: "2.0";
  id?: number;
  method?: string;
  params?: any;
  result?: any;
  error?: { code: number; message: string; data?: { code: ErrorCode } };
};

// The public API: the server validates params with these; the CLI builds params from them.
const target = z.string().min(1);
const caller = z.string().optional();
const dir = z.enum(["right", "down"]).default("right");
const state = z.enum(["working", "blocked", "done", "idle"]);

// The protocol version: bumped when a request, result or event changes incompatibly. Plugins declare the one
// they speak.
export const PROTOCOL = 1;

// Every event is stamped with what it is, when (epoch ms), its order (seq rises with every event this server
// run emits; a subscriber only sees the events it asked for, so seqs it sees can skip) and which server run
// (epoch: new on every start, so ids and seqs from an earlier epoch mean nothing now).
// Within one connection, events arrive in seq order and none is dropped. There's no replay: after a disconnect,
// or when epoch changes, subscribe again with `snapshot: true`.
export const envelope = z.object({ type: z.string(), at: z.number(), seq: z.number().int().positive(), epoch: z.string() });
const ev = <T extends z.ZodRawShape>(type: string, shape: T) => z.strictObject({ ...envelope.shape, type: z.literal(type), ...shape });
// A pane is addressed by id (stable for its life, and reused by a later pane only after a restart) plus instance
// (unique to the process the pane was started with).
const paneRef = { pane: z.string(), instance: z.string() };
export const events = {
  "pane.created": ev("pane.created", { ...paneRef, name: z.string().optional(), command: z.string().optional() }),
  "pane.output": ev("pane.output", { ...paneRef, text: z.string() }), // only with events.subscribe { output: true }
  "process.exited": ev("process.exited", { ...paneRef, name: z.string().optional(), exitCode: z.number().int().optional() }), // no exitCode: killed by a signal
  // A detected agent's state changed. It's what detection observed (its screen, or an integration's report), not
  // the agent's own account; no harness means no agent is detected in the pane any more.
  "agent.state": ev("agent.state", { ...paneRef, name: z.string().optional(), harness: z.string().optional(), from: state.optional(), to: state }),
  "message.sent": ev("message.sent", { id: z.number(), from: z.string(), to: z.string(), hops: z.number().int() }),
  "message.delivered": ev("message.delivered", { id: z.number(), from: z.string(), to: z.string() }),
  "client.attached": ev("client.attached", {}),
};

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
  // snapshot: also return every pane as of the moment the subscription starts (see envelope)
  "events.subscribe": z.object({ caller, output: z.boolean().optional(), snapshot: z.boolean().optional() }),
  "protocol.describe": z.object({ caller }),
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
