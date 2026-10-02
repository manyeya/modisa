// Wire protocol: JSON-RPC 2.0, newline-delimited, over a unix socket (or ssh stdio for --remote).
// Requests get responses; the server pushes events as notifications ({method, params}, no id).
import { z } from "zod";
import { ERROR_CODES, type ErrorCode } from "./types";
import { LINK, globProblem, regexProblem } from "./links";
import type { Node } from "../core/layout";

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
const direction = z.enum(["left", "right", "up", "down"]);
const state = z.enum(["working", "blocked", "done", "idle"]);
// pane.read: the visible screen, or the scrollback's tail as the pane wraps it or with soft wraps joined; plain text, or
// with colours and styles (ANSI SGR)
const readSource = z.enum(["visible", "recent", "recent-unwrapped"]);
const readFormat = z.enum(["text", "ansi"]);
// Variables a new pane gets on top of the server's environment. Saved with the session (a restart keeps them), so
// they're on disk. MODISA_ ones are modisa's: MODISA_PANE_ID is always the pane's own.
const envKey = z.string().regex(/^[A-Za-z_]\w*$/, "isn't a variable name (letters, digits and _)").refine((k) => !k.startsWith("MODISA_"), "is modisa's: MODISA_ variables can't be set");
const env = z.record(envKey, z.string(), { error: (i) => (i.code === "invalid_key" ? i.issues[0]?.message : undefined) }).optional();

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
  name: z.string().optional(), title: z.string(), terminalTitle: z.string().optional(), cwd: z.string(), command: z.string().optional(), harness: z.string().optional(), createdBy: z.string(),
  status: z.enum(["running", "exited"]), exitCode: z.number().int().optional(), // set once exited; signal n: 128+n
  agent: agentInfo.optional(),
  session: z.strictObject({ agent: z.string(), id: z.string(), source: z.string() }).optional(), // the agent's own session, reported by its integration
  cols: z.number().int(), rows: z.number().int(),
  popup: z.boolean().optional(), // a plugin's popup: no place in the layout, shown only by the client that opened it
});
const listedPane = paneInfo.extend({ focused: z.boolean(), workspace: z.string().optional(), workspaceId: z.string().optional(), tabId: z.string().optional() }); // where it is: a popup has no place
const created = paneInfo.extend({ workspaceId: z.string(), tabId: z.string() }); // a new pane, and where it is
// A tab's split tree: a pane, or two subtrees side by side (row) or stacked (col), `a` with `ratio` of the room.
const tree: z.ZodType<Node> = z.lazy(() => z.union([z.strictObject({ pane: z.string() }), z.strictObject({ dir: z.enum(["row", "col"]), ratio: z.number(), a: tree, b: tree })]));
const rect = z.strictObject({ x: z.number().int(), y: z.number().int(), w: z.number().int(), h: z.number().int() });
// A space as clients draw it: `active` is its tab on screen, and a zoomed tab shows only its focused pane.
const workspaceView = z.strictObject({
  id: z.string(), name: z.string(), cwd: z.string(), active: z.number().int(),
  tabs: z.array(z.strictObject({ id: z.string(), name: z.string().optional(), tree, focused: z.string(), zoomed: z.boolean() })),
  git: z.strictObject({ repo: z.string(), branch: z.string(), ahead: z.number().int().optional(), behind: z.number().int().optional(), changes: z.number().int() }).optional(),
});
const sessionInfo = { session: z.string(), clients: z.number().int(), paused: z.boolean(), version: z.string() };
const pluginKey = z.strictObject({ key: z.string(), action: z.string().optional(), pane: z.string().optional(), description: z.string(), state: z.enum(["active", "disabled"]), reason: z.string().optional() });
const pluginStatus = z.strictObject({
  keys: z.array(pluginKey).optional(),
  name: z.string(), source: z.enum(["linked", "config"]), dir: z.string().optional(), status: z.enum(["starting", "running", "exited", "failed", "stopped"]),
  pid: z.number().int().optional(), exitCode: z.number().int().optional(), signal: z.string().optional(), error: z.string().optional(), log: z.string(),
  connected: z.boolean(), actions: z.array(z.string()), group: z.enum(["running", "gone"]).optional(), invocations: z.number().int().optional(),
  install: z.strictObject({ source: z.string(), ref: z.string().nullable(), commit: z.string() }).optional(),
});
const tone = z.enum(["fg", "dim", "accent", "warn", "working", "blocked", "done", "idle"]);
const span = z.union([z.strictObject({ text: z.string(), tone: tone.optional(), bold: z.boolean().optional() }), z.strictObject({ icon: z.string() })]);
export const pluginUiView = z.strictObject({
  plugin: z.string(),
  run: z.string(),
  actions: z.array(z.strictObject({ id: z.string(), title: z.string(), description: z.string().optional() })),
  status: z.array(z.strictObject({ id: z.string(), text: z.string(), tone, action: z.string().optional() })),
  sidebar: z.strictObject({ title: z.string(), rows: z.array(z.strictObject({ text: z.string(), tone, spans: z.array(span).optional(), action: z.string().optional(), pane: z.string().optional(), instance: z.string().optional() })) }).optional(),
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
  // content: what source and format asked for; screen and recentOutput: the visible screen and recent text, as always
  "pane.read": paneInfo.extend({ screen: z.string(), recentOutput: z.string(), content: z.string(), source: readSource, format: readFormat }),
  "pane.split": created,
  "agent.spawn": created,
  "tab.create": created,
  "workspace.create": created,
  // snapshot: what clients draw (every space's tabs with their trees, focus and zoom, in an `area` of cells) and every
  // pane, where `panes` and `workspaces` are otherwise counts
  "session.info": z.union([
    z.strictObject({ ...sessionInfo, panes: z.number().int(), workspaces: z.number().int() }),
    z.strictObject({ ...sessionInfo, active: z.number().int(), area: rect, workspaces: z.array(workspaceView), panes: z.array(listedPane) }),
  ]),
  "pane.move": z.strictObject({ ...paneRef, workspaceId: z.string(), tabId: z.string() }), // where it is now
  "pane.resize": z.strictObject({ changed: z.boolean() }), // false: no border on that side, or it's as far as it goes
  "pane.zoom": z.strictObject({ zoomed: z.boolean() }),
  "agent.list": z.array(z.strictObject({ id: z.string(), name: z.string().optional(), title: z.string(), harness: z.string(), state, source: z.enum(["hook", "screen"]), workspace: z.string().optional() })),
  wait: z.union([z.strictObject({ exitCode: z.number().int() }), z.strictObject({ state }), z.strictObject({ match: z.string() })]),
  send: z.strictObject({ id: z.number(), queued: z.literal(true), delivered: z.literal(false), recipientState: state.optional() }),
  "plugin.list": z.array(pluginStatus),
  notify: z.strictObject({ clients: z.number().int().nonnegative() }), // the TUI clients it reached: 0 when none is attached
  "plugin.hello": z.strictObject({ name: z.string(), protocol: z.number().int(), session: z.string(), epoch: z.string() }),
};
// A failed request's `error`: code is JSON-RPC's (-32601 unknown method, -32602 invalid params, -32000 the request
// failed); data.code is modisa's stable reason
export const errorReply = z.strictObject({ code: z.number().int(), message: z.string(), data: z.strictObject({ code: z.enum(ERROR_CODES) }).optional() });

// ---------- CLI results: what `modisa … --json` prints where the CLI makes it (an e2e test checks them) ----------
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
  // A pane's process (its shell, or the command it started with), the job in the foreground of its terminal, and
  // where it is now (its working directory). An exited pane has only the pid it had.
  "pane process-info": z.strictObject({ pane: z.string(), pid: z.number().int(), foreground: z.strictObject({ pid: z.number().int(), args: z.string() }).optional(), cwd: z.string().optional() }),
  // The tab a pane is in, from a session.info snapshot: each pane's box in cells, borders included (where the split
  // tree puts it), and whether it's shown when the tab is (a zoomed tab, or one too small for its panes, shows only
  // its focused pane).
  "pane layout": z.strictObject({
    pane: z.string(), workspaceId: z.string(), tabId: z.string(), area: rect, focused: z.string(), zoomed: z.boolean(),
    panes: z.array(z.strictObject({ id: z.string(), name: z.string().optional(), ...rect.shape, shown: z.boolean() })),
  }),
  // the pane on each side of it in its tab; null: that side is the tab's edge
  "pane edges": z.strictObject({ pane: z.string(), left: z.string().nullable(), right: z.string().nullable(), up: z.string().nullable(), down: z.string().nullable() }),
  // every space's tabs, from a session.info snapshot: active is the tab its space shows, current the one on screen
  "tab list": z.array(z.strictObject({ id: z.string(), name: z.string().optional(), workspaceId: z.string(), workspace: z.string(), panes: z.array(z.string()), focused: z.string(), zoomed: z.boolean(), active: z.boolean(), current: z.boolean() })),
  // config.toml checked where the command runs, no session needed. A problem's key is the setting's dotted path (none:
  // the file doesn't parse); line and column count from 1. ok: no errors (warnings are settings modisa ignores).
  "config check": z.strictObject({
    file: z.string(), exists: z.boolean(), ok: z.boolean(),
    problems: z.array(z.strictObject({ level: z.enum(["error", "warning"]), key: z.string().optional(), message: z.string(), line: z.number().int().positive().optional(), column: z.number().int().positive().optional() })),
  }),
};

export const api = {
  list: z.object({ caller }),
  "session.info": z.object({ caller, snapshot: z.boolean().optional() }), // snapshot: read only, attaches nothing
  "workspace.list": z.object({ caller }),
  "workspace.create": z.object({ caller, name: z.string().optional(), cwd: z.string().optional(), command: z.string().optional(), env }),
  "workspace.rename": z.object({ caller, workspace: z.string().min(1), name: z.string().trim().min(1) }),
  "workspace.close": z.object({ caller, workspace: z.string().min(1) }),
  "tab.create": z.object({ caller, name: z.string().optional(), workspace: z.string().optional(), command: z.string().optional(), paneName: z.string().optional(), cwd: z.string().optional(), env }),
  // ratio: the new pane's share of the room
  "pane.split": z.object({ caller, target: target.optional(), dir, ratio: z.number().min(0.1).max(0.9).default(0.5), name: z.string().optional(), cwd: z.string().optional(), command: z.string().optional(), focus: z.boolean().optional(), env }),
  "pane.run": z.object({ caller, target, command: z.string() }),
  "pane.read": z.object({ caller, target: target.optional(), lines: z.number().int().positive().max(10_000).default(50), source: readSource.default("recent"), format: readFormat.default("text") }),
  "pane.keys": z.object({ caller, target, keys: z.array(z.string()).min(1) }),
  "pane.close": z.object({ caller, target: target.optional() }),
  "pane.rename": z.object({ caller, target: target.optional(), name: z.string() }),
  // dir: focus the pane on that side of the target instead
  "pane.focus": z.object({ caller, target: target.optional(), dir: direction.optional() }),
  // Exactly one destination: beside a pane (`beside`, else the focused pane of `tab`; `dir` side, `ratio` its share),
  // alone in a new tab (`newTab`, in `workspace` or its own space) or in a new space (`newWorkspace`, at the pane's
  // cwd). `name` names the new tab or space; `focus` moves the view to it.
  "pane.move": z
    .object({ caller, target: target.optional(), tab: z.string().min(1).optional(), beside: target.optional(), newTab: z.boolean().optional(), workspace: z.string().min(1).optional(), newWorkspace: z.boolean().optional(), name: z.string().optional(), dir, ratio: z.number().min(0.1).max(0.9).default(0.5), focus: z.boolean().optional() })
    .refine((p) => [p.tab !== undefined || p.beside !== undefined, !!p.newTab, !!p.newWorkspace].filter(Boolean).length === 1, "exactly one destination: tab and/or beside, newTab, or newWorkspace"),
  // with another pane, or with the target's neighbour on the `dir` side
  "pane.swap": z.object({ caller, target: target.optional(), with: target.optional(), dir: direction.optional() }).refine((p) => (p.with === undefined) !== (p.dir === undefined), "exactly one of with or dir"),
  "pane.resize": z.object({ caller, target: target.optional(), dir: direction, amount: z.number().int().positive().max(1000).default(2) }), // amount: cells
  "pane.zoom": z.object({ caller, target: target.optional(), mode: z.enum(["on", "off", "toggle"]).default("toggle") }),
  "agent.spawn": z.object({ caller, harness: z.string(), name: z.string().optional(), prompt: z.string().optional(), target: target.optional(), dir, tab: z.boolean().optional(), focus: z.boolean().optional(), env }),
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
  "ui.sidebar.set": z.object({ caller, title: z.string(), rows: z.array(z.object({ text: z.string().default(""), tone: tone.default("fg"), spans: z.array(z.union([z.object({ text: z.string(), tone: tone.optional(), bold: z.boolean().optional() }), z.object({ icon: z.string().max(40) })])).max(16).optional(), action: z.string().min(1).optional(), pane: z.string().min(1).optional(), instance: z.string().min(1).optional() })).max(50) }),
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
  // title: what the pane is called while it has no name, over the terminal title its program sets ("" stops); not saved
  report: z.object({
    caller, pane: z.string().optional(), source: z.string().min(1).optional(), agent: z.string().optional(),
    state: state.optional(), seq: z.number().optional(), session: z.string().min(1).optional(), release: z.boolean().optional(), title: z.string().optional(),
  }),
  // A toast in every attached TUI, titled with who sent it (the calling pane, else "notify"); system and sound ask for
  // those too, which each client gives only where its user has them on for some event. Rate-limited: 3 every 10s from
  // one sender, 6 from all of them together.
  notify: z.object({ caller, title: z.string().min(1), body: z.string().optional(), tone: tone.default("fg"), system: z.boolean().optional(), sound: z.boolean().optional() }),
  send: z.object({ caller, to: target, body: z.string().min(1) }),
  inbox: z.object({ caller }),
  messages: z.object({ caller }),
  "messaging.pause": z.object({ caller, paused: z.boolean().optional() }),
  "debug.detect": z.object({ caller, target: target.optional() }), // also the pane's process: see cliResults "pane process-info"
  integrations: z.object({ caller }),
  integration: z.object({ caller, id: z.string().min(1), install: z.boolean() }),
  kill: z.object({ caller }),
  restart: z.object({ caller }),
} as const;
