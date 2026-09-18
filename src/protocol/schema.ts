// Wire protocol: JSON-RPC 2.0, newline-delimited, over a unix socket (or ssh stdio for --remote).
// Requests get responses; the server pushes events as notifications ({method, params}, no id).
import { z } from "zod";
import { ERROR_CODES, type ErrorCode } from "./types";
import { LINK, globProblem, regexProblem } from "./links";

export type Msg = {
  jsonrpc: "2.0";
  id?: number;
  method?: string;
  params?: any;
  result?: any;
  error?: { code: number; message: string; data?: { code: ErrorCode } };
};

// plugin.json: who the plugin is, the protocol version it speaks, and how to start it (argv, run in the plugin's
// directory, no shell). Optionally what it offers the TUI: actions (listed before it connects; hello must offer each),
// panes it can open, keys under the prefix, and URL globs that Ctrl+click hands to an action.
const pluginId = z.string().regex(/^[a-z0-9][a-z0-9-]*$/, "use lowercase letters, digits and dashes");
const cells = z.union([z.number().int().positive(), z.string().regex(/^\d{1,3}%$/, 'a number of cells, or a percentage like "80%"')]);
export const pluginManifest = z
  .object({
    name: pluginId,
    protocol: z.number().int().positive(),
    run: z.array(z.string().min(1)).min(1),
    description: z.string().optional(),
    actions: z.array(z.strictObject({ id: pluginId, title: z.string().min(1).max(60), description: z.string().max(200).optional() })).optional(),
    panes: z.array(z.strictObject({ id: pluginId, title: z.string().min(1).max(60), run: z.array(z.string().min(1)).min(1), placement: z.enum(["overlay", "popup", "split", "tab", "zoomed"]).default("overlay"), width: cells.optional(), height: cells.optional() })).optional(),
    keys: z.array(z.strictObject({ key: z.string().min(1).max(12), action: pluginId.optional(), pane: pluginId.optional(), description: z.string().min(1).max(80) })).optional(),
    links: z.array(z.strictObject({ pattern: z.string().min(1).max(LINK.source).optional(), regex: z.string().min(1).max(LINK.source).optional(), action: pluginId })).max(LINK.perPlugin).optional(),
  })
  .superRefine((m, issues) => {
    const problem = (path: (string | number)[], message: string) => issues.addIssue({ code: "custom", path, message });
    for (const [list, label] of [[m.actions, "action"], [m.panes, "pane"]] as const) {
      const seen = new Set<string>();
      list?.forEach((item, i) => (seen.has(item.id) ? problem([`${label}s`, i, "id"], `a second ${label} with id ${item.id}`) : seen.add(item.id)));
    }
    const actions = new Set(m.actions?.map((a) => a.id));
    const panes = new Set(m.panes?.map((p) => p.id));
    const keys = new Set<string>();
    m.keys?.forEach((k, i) => {
      if (keys.has(k.key)) problem(["keys", i, "key"], `key ${k.key} is bound twice`);
      keys.add(k.key);
      if (!!k.action === !!k.pane) problem(["keys", i], `key ${k.key} needs exactly one of action or pane`);
      if (k.action && !actions.has(k.action)) problem(["keys", i, "action"], `key ${k.key} runs action ${k.action}, which isn't in actions`);
      if (k.pane && !panes.has(k.pane)) problem(["keys", i, "pane"], `key ${k.key} opens pane ${k.pane}, which isn't in panes`);
    });
    m.links?.forEach((l, i) => {
      if ((l.pattern === undefined) === (l.regex === undefined)) problem(["links", i], "a link needs exactly one of pattern (a URL glob) or regex");
      else if (l.pattern !== undefined) {
        const why = globProblem(l.pattern);
        if (why) problem(["links", i, "pattern"], why);
      } else {
        const why = regexProblem(l.regex!);
        if (why) problem(["links", i, "regex"], why);
      }
      if (!actions.has(l.action)) problem(["links", i, "action"], `links to action ${l.action}, which isn't in actions`);
    });
  });
export type PluginManifest = z.infer<typeof pluginManifest>;


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
  "process.exited": ev("process.exited", { ...paneRef, name: z.string().optional(), exitCode: z.number().int() }), // killed by signal n: 128+n
  // A detected agent's state changed. It's what detection observed (its screen, or an integration's report), not
  // the agent's own account; no harness means no agent is detected in the pane any more.
  "agent.state": ev("agent.state", { ...paneRef, name: z.string().optional(), harness: z.string().optional(), from: state.optional(), to: state }),
  "message.sent": ev("message.sent", { id: z.number(), from: z.string(), to: z.string(), hops: z.number().int() }),
  "message.delivered": ev("message.delivered", { id: z.number(), from: z.string(), to: z.string() }),
  "client.attached": ev("client.attached", {}),
};

// ---------- results: what the supported requests return (an e2e test checks real replies against these) ----------
// A pane's agent is what detection observed (its screen, or an integration's report). Absent means no agent is
// detected in the pane, not that it's known to have none.
const agentInfo = z.strictObject({ harness: z.string(), state, source: z.enum(["hook", "screen"]) });
export const paneInfo = z.strictObject({
  id: z.string(), instance: z.string(), // the same pair events call `pane` and `instance`
  name: z.string().optional(), title: z.string(), cwd: z.string(), command: z.string().optional(), harness: z.string().optional(), createdBy: z.string(),
  status: z.enum(["running", "exited"]), exitCode: z.number().int().optional(), // set once exited; signal n: 128+n
  agent: agentInfo.optional(),
  session: z.strictObject({ agent: z.string(), id: z.string(), source: z.string() }).optional(), // the agent's own session, reported by its integration
  cols: z.number().int(), rows: z.number().int(),
  popup: z.boolean().optional(), // a plugin's popup: no place in the layout, shown only by the client that opened it
});
const listedPane = paneInfo.extend({ focused: z.boolean(), workspace: z.string().optional() });
const pluginKey = z.strictObject({ key: z.string(), action: z.string().optional(), pane: z.string().optional(), description: z.string(), state: z.enum(["active", "disabled"]), reason: z.string().optional() });
const pluginStatus = z.strictObject({
  keys: z.array(pluginKey).optional(),
  name: z.string(), source: z.enum(["linked", "config"]), dir: z.string().optional(), status: z.enum(["starting", "running", "exited", "failed", "stopped"]),
  pid: z.number().int().optional(), exitCode: z.number().int().optional(), signal: z.string().optional(), error: z.string().optional(), log: z.string(),
  connected: z.boolean(), actions: z.array(z.string()), group: z.enum(["running", "gone"]).optional(), invocations: z.number().int().optional(),
  install: z.strictObject({ source: z.string(), ref: z.string().nullable(), commit: z.string() }).optional(),
});
const tone = z.enum(["fg", "dim", "accent", "warn"]);
export const pluginUiView = z.strictObject({
  plugin: z.string(),
  run: z.string(),
  actions: z.array(z.strictObject({ id: z.string(), title: z.string(), description: z.string().optional() })),
  status: z.array(z.strictObject({ id: z.string(), text: z.string(), tone, action: z.string().optional() })),
  sidebar: z.strictObject({ title: z.string(), rows: z.array(z.strictObject({ text: z.string(), tone, action: z.string().optional(), pane: z.string().optional(), instance: z.string().optional() })) }).optional(),
  badges: z.array(z.strictObject({ pane: z.string(), instance: z.string(), text: z.string(), tone })),
  menu: z.array(z.strictObject({ id: z.string(), title: z.string(), action: z.string() })),
  keys: z.array(z.strictObject({ key: z.string(), action: z.string().optional(), pane: z.string().optional(), description: z.string() })),
  panes: z.array(z.strictObject({ id: z.string(), title: z.string(), placement: z.enum(["overlay", "popup", "split", "tab", "zoomed"]) })),
  links: z.array(z.strictObject({ pattern: z.string().optional(), regex: z.string().optional(), action: z.string() })),
});
export const results = {
  list: z.array(listedPane),
  "ui.state": pluginUiView,
  "plugin.pane.open": z.strictObject({ pane: z.string(), instance: z.string(), placement: z.enum(["overlay", "popup", "split", "tab", "zoomed"]), title: z.string(), width: z.union([z.number(), z.string()]).optional(), height: z.union([z.number(), z.string()]).optional() }),
  "events.subscribe": z.strictObject({ protocol: z.number().int(), epoch: z.string(), seq: z.number().int().nonnegative(), panes: z.array(listedPane).optional() }),
  "pane.read": paneInfo.extend({ screen: z.string(), recentOutput: z.string() }),
  "agent.list": z.array(z.strictObject({ id: z.string(), name: z.string().optional(), title: z.string(), harness: z.string(), state, source: z.enum(["hook", "screen"]), workspace: z.string().optional() })),
  wait: z.union([z.strictObject({ exitCode: z.number().int() }), z.strictObject({ state }), z.strictObject({ match: z.string() })]),
  send: z.strictObject({ id: z.number(), queued: z.literal(true), delivered: z.literal(false), recipientState: state.optional() }),
  "plugin.list": z.array(pluginStatus),
  "plugin.hello": z.strictObject({ name: z.string(), protocol: z.number().int(), session: z.string(), epoch: z.string() }),
};
// A failed request's `error`: code is JSON-RPC's (-32601 unknown method, -32602 invalid params, -32000 the request
// failed); data.code is modisa's stable reason
export const errorReply = z.strictObject({ code: z.number().int(), message: z.string(), data: z.strictObject({ code: z.enum(ERROR_CODES) }).optional() });

// ---------- CLI results: what `modisa plugin … --json` prints (an e2e test checks them) ----------
// Starting a plugin in the one session a command reaches: started (and connected), already running, not started (no
// session running), failed (it didn't start, or exited), or no-hello (started, but never connected in time).
export const pluginStart = z.strictObject({
  session: z.string(),
  state: z.enum(["started", "already-running", "not-started", "failed", "no-hello"]),
  reason: z.string().optional(),
  pid: z.number().int().optional(),
  log: z.string().optional(),
  disabledKeys: z.array(z.string()).optional(), // "<key>: <why>"
});
export const cliResults = {
  // registering is global (every session starts it); starting is only in `start.session`
  "plugin link": z.strictObject({ name: z.string(), dir: z.string(), linked: z.literal(true), alreadyLinked: z.boolean(), start: pluginStart }),
  // installed: the checkout, record and link are in place (then `start` says whether it started). Not installed:
  // `stage` and `reason` say where it failed, and nothing was left behind.
  "plugin install": z.strictObject({
    installed: z.boolean(),
    alreadyInstalled: z.boolean().optional(),
    name: z.string().optional(),
    source: z.string(), // without credentials
    ref: z.string().nullable(), // as requested; null: the default branch
    commit: z.string().optional(), // what the ref resolved to
    checkout: z.string().optional(),
    dir: z.string().optional(), // the plugin's directory (the checkout, or --subdir inside it)
    start: pluginStart.optional(),
    hints: z.array(z.string()).optional(),
    stage: z.enum(["source", "git", "clone", "ref", "subdir", "manifest", "collision"]).optional(),
    reason: z.string().optional(),
  }),
  // managed: installed with `plugin install` (stopped in every reachable session; its checkout deleted only if none
  // still runs it). Otherwise a directory you linked: stopped in the session reached, and never deleted.
  // repositories with the modisa-tui-plugin topic, most starred first; text is stripped of control characters
  "plugin search": z.strictObject({
    query: z.string(),
    total: z.number().int().nonnegative(),
    results: z.array(z.strictObject({ name: z.string(), repo: z.string(), url: z.string(), description: z.string(), stars: z.number().int().nonnegative(), updated: z.string(), created: z.string(), archived: z.boolean(), install: z.string() })),
  }),
  "plugin unlink": z.strictObject({
    name: z.string(),
    unlinked: z.literal(true),
    managed: z.boolean(),
    stoppedIn: z.array(z.string()),
    stillUsing: z.array(z.string()),
    unreachable: z.array(z.string()),
    checkout: z.strictObject({ path: z.string(), deleted: z.boolean() }).optional(),
  }),
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
  "plugin.list": z.object({ caller }),
  "plugin.stop": z.object({ caller, name: z.string().min(1) }),
  "plugin.start": z.object({ caller, name: z.string().min(1) }),
  // a plugin's own connection says which plugin it is (the token it was started with) and what actions it offers
  "plugin.hello": z.object({ caller, token: z.string().min(1), actions: z.array(z.string().min(1)).optional() }),
  // call an action a connected plugin offers; modisa sends it a plugin.action request ({ action, params })
  // run: the run whose UI the action was taken from (ui.state's `run`); refused if that run has since ended
  // target: the pane the action is for (a menu entry, key or palette entry), a complete pane + instance pair kept apart
  // from the plugin's own params; checked when invoked, and handed to the action as call.target
  "plugin.invoke": z.object({ caller, plugin: z.string().min(1), action: z.string().min(1), params: z.record(z.string(), z.unknown()).optional(), run: z.string().optional(), target: z.strictObject({ pane: z.string().min(1), instance: z.string().min(1) }).optional(), link: z.string().min(1).max(2048).optional() }),
  // A plugin's own TUI contributions, only on its bound connection (after plugin.hello). Text is cleaned of control
  // characters and cut to length; actions must be ones the plugin offered in hello; updates are rate-limited.
  "ui.status.set": z.object({ caller, id: z.string().min(1).max(40), text: z.string(), tone: tone.default("fg"), action: z.string().min(1).optional() }),
  "ui.status.clear": z.object({ caller, id: z.string().min(1).max(40) }),
  // a row with `pane` needs that pane's `instance`: clicking it reaches that process or nothing
  "ui.sidebar.set": z.object({ caller, title: z.string(), rows: z.array(z.object({ text: z.string(), tone: tone.default("fg"), action: z.string().min(1).optional(), pane: z.string().min(1).optional(), instance: z.string().min(1).optional() })).max(50) }),
  "ui.sidebar.clear": z.object({ caller }),
  "ui.toast": z.object({ caller, text: z.string(), tone: tone.default("fg"), system: z.boolean().optional() }),
  "ui.badge.set": z.object({ caller, pane: z.string().min(1), instance: z.string().min(1), text: z.string(), tone: tone.default("accent") }),
  "ui.badge.clear": z.object({ caller, pane: z.string().min(1) }),
  "ui.menu.set": z.object({ caller, items: z.array(z.object({ id: z.string().min(1).max(40), title: z.string(), action: z.string().min(1) })).max(20) }),
  // what a plugin shows now (any client may read it: plugin tests, plugin check)
  "ui.state": z.object({ caller, plugin: z.string().min(1) }),
  // Open one of a plugin's panes (plugin.json `panes`) from a key, the palette, an action or the CLI. `from` is the pane
  // it's opened over or next to, resolved when it was asked for. A popup opens only from an attached TUI client.
  "plugin.pane.open": z.object({ caller, plugin: z.string().min(1), pane: z.string().min(1), params: z.record(z.string(), z.unknown()).optional(), run: z.string().optional(), from: z.object({ pane: z.string().min(1), instance: z.string().optional() }).optional() }),
  // the client showing a popup: its size, and closing it
  "plugin.popup.resize": z.object({ caller, pane: z.string().min(1), cols: z.number().int().min(10).max(1000), rows: z.number().int().min(3).max(500) }),
  "plugin.popup.close": z.object({ caller, pane: z.string().min(1) }),
  // a plugin closing its own popup (bound connection)
  "ui.popup.close": z.object({ caller }),
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
