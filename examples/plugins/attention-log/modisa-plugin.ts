// modisa-plugin.ts: modisa's client library for plugins (protocol 1). No dependencies.
// `modisa plugin new` copies it into each plugin; refresh a plugin's copy with `modisa plugin sdk > modisa-plugin.ts`.
//
// It handles the protocol so a plugin only writes its own logic: newline-delimited JSON-RPC framing, request ids,
// errors with modisa's stable codes, binding to the host (hello) with actions, and starting a subscription with no
// gap and no duplicates (subscribe). When the session's socket closes, `closed` resolves: exit then, because the next
// server starts the plugin again. runPlugin does all of that.

export const SDK_VERSION = 8;
export const PROTOCOL = 1;

export type AgentState = "working" | "blocked" | "done" | "idle";
// A pane: `id` (like p3) plus `instance`, unique to the process it was started with (ids come back after a restart).
// `agent` is what modisa's detection observed: its screen, or its integration's report. No `agent` means no agent
// is detected, not that there isn't one.
export type Pane = {
  id: string;
  instance: string;
  name?: string;
  title: string;
  cwd: string;
  command?: string;
  status: "running" | "exited";
  exitCode?: number;
  agent?: { harness: string; state: AgentState; source: "hook" | "screen" };
  workspace?: string;
  [field: string]: unknown;
};
// Every event has type, at (epoch ms), seq and epoch; `modisa plugin schema` has each type's fields.
export type Event = { type: string; at: number; seq: number; epoch: string; pane?: string; instance?: string; name?: string; harness?: string; from?: AgentState; to?: AgentState; [field: string]: unknown };
export type Snapshot = { protocol: number; epoch: string; seq: number; panes: Pane[] };
// An action gets its params and the call: `invocation` names it, and `signal` aborts if modisa gives up waiting
// (plugin.cancel, for that invocation only) or the connection goes. It's cooperative: an action that ignores it keeps
// running, the caller has already been told the outcome is unknown, and neither the abort nor a rejection proves
// that effects already started were undone.
// `target` is the pane the user took the action from (a menu entry, key or palette entry), kept apart from params and
// already checked by modisa to be that pane's current process. `link` is the URL the user Ctrl+clicked, when one of
// the manifest's links matched it (modisa checks the pattern); treat it as data, never as a command.
export type Action = (params: Record<string, unknown>, call: { invocation?: string; signal: AbortSignal; target?: { pane: string; instance: string }; link?: string }) => unknown;

// What a plugin shows in modisa's TUI (see Client.ui). Tones map to the user's theme.
export type Tone = "fg" | "dim" | "accent" | "warn";
// a row that focuses a pane names its instance too: clicking reaches that process, or tells the user it's gone
export type SidebarRow = { text: string; tone?: Tone; action?: string; pane?: string; instance?: string };
export type MenuItem = { id: string; title: string; action: string };
export type UiState = {
  plugin: string;
  run: string;
  actions: { id: string; title: string; description?: string }[];
  status: { id: string; text: string; tone: Tone; action?: string }[];
  sidebar?: { title: string; rows: { text: string; tone: Tone; action?: string; pane?: string; instance?: string }[] };
  badges: { pane: string; instance: string; text: string; tone: Tone }[];
  menu: MenuItem[];
  keys: { key: string; action?: string; pane?: string; description: string }[]; // as plugin.json declares them: each client binds them with its own config
  panes: { id: string; title: string; placement: "overlay" | "popup" | "split" | "tab" | "zoomed" }[];
};

export class ModisaError extends Error {
  constructor(message: string, readonly code: string) {
    super(message);
    this.name = "ModisaError";
  }
}

export class Client {
  private seq = 0;
  private pending = new Map<number, { resolve: (value: any) => void; reject: (error: Error) => void }>();
  private buf = "";
  private actions: Record<string, Action> = {};
  private running = new Map<string, AbortController>(); // invocation → its call's signal
  private listeners: ((e: Event) => void)[] = [];
  private cause?: Error;
  private onClosed!: (cause: Error) => void;
  /** Resolves, with the reason, once the connection to the session is gone. */
  readonly closed = new Promise<Error>((resolve) => (this.onClosed = resolve));

  constructor(private transport: { write(line: string): void; close(): void }) {}

  /** Bytes from the socket. */
  feed(chunk: string) {
    if (this.cause) return;
    this.buf += chunk;
    for (let nl; (nl = this.buf.indexOf("\n")) >= 0; ) {
      const line = this.buf.slice(0, nl);
      this.buf = this.buf.slice(nl + 1);
      let m: any;
      try {
        m = JSON.parse(line);
      } catch {
        continue; // a malformed line never ends the plugin
      }
      if (!m || typeof m !== "object") continue;
      if (m.id !== undefined && m.method === undefined) {
        const p = this.pending.get(m.id);
        if (!p) continue;
        this.pending.delete(m.id);
        if (m.error) p.reject(new ModisaError(String(m.error.message), m.error.data?.code ?? "error"));
        else p.resolve(m.result);
      } else if (m.method === "event" && m.params) for (const listen of this.listeners) listen(m.params);
      else if (m.method === "plugin.action" && m.id !== undefined) void this.answer(m.id, m.params ?? {});
      else if (m.method === "plugin.cancel") this.running.get(m.params?.invocation)?.abort(new Error("modisa stopped waiting for this action"));
    }
  }

  /** The connection is gone. */
  drop(cause = new Error("the session closed the connection")) {
    if (this.cause) return;
    this.cause = cause;
    this.buf = "";
    for (const p of this.pending.values()) p.reject(cause);
    this.pending.clear();
    for (const controller of this.running.values()) controller.abort(cause); // every action still running
    this.running.clear();
    this.onClosed(cause);
  }

  close() {
    this.transport.close();
    this.drop(new Error("closed by the plugin"));
  }

  request<T = unknown>(method: string, params: Record<string, unknown> = {}): Promise<T> {
    if (this.cause) return Promise.reject(this.cause);
    const id = ++this.seq;
    return new Promise<T>((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.send({ jsonrpc: "2.0", id, method, params });
    });
  }

  /** This plugin's name, once hello has bound it. */
  name?: string;

  /**
   * Bind this connection to the plugin modisa started, offering actions to `modisa plugin run <name> <action>`,
   * the command palette, and the status segments, sidebar rows and menu entries that name them.
   */
  async hello(actions: Record<string, Action> = {}, token = Bun.env.MODISA_PLUGIN_TOKEN) {
    if (!token) throw new ModisaError("no $MODISA_PLUGIN_TOKEN: modisa starts plugins (modisa plugin link), not a shell", "usage");
    this.actions = actions;
    const bound = await this.request<{ name: string; protocol: number; session: string; epoch: string }>("plugin.hello", { token, actions: Object.keys(actions) });
    this.name = bound.name;
    return bound;
  }

  /**
   * What this plugin shows in modisa's TUI, drawn by modisa in the user's theme. Only after hello. Text is cleaned
   * of control characters and cut to length; an `action` must be one offered in hello; updates are rate-limited
   * (errors: rate_limited, no_such_action). Everything is cleared when the plugin stops.
   */
  readonly ui = {
    /** A status bar segment (up to 4); clicking it runs `action`. */
    status: (id: string, text: string, options: { tone?: Tone; action?: string } = {}) => this.request<UiState>("ui.status.set", { id, text, ...options }),
    clearStatus: (id: string) => this.request<UiState>("ui.status.clear", { id }),
    /** This plugin's sidebar section; a row runs its `action`, or focuses its `pane`. */
    sidebar: (title: string, rows: SidebarRow[]) => this.request<UiState>("ui.sidebar.set", { title, rows }),
    clearSidebar: () => this.request<UiState>("ui.sidebar.clear"),
    /** A label on a pane's border, for that pane's current process (`instance`) only. */
    badge: (pane: string, instance: string, text: string, tone?: Tone) => this.request<UiState>("ui.badge.set", { pane, instance, text, ...(tone && { tone }) }),
    clearBadge: (pane: string) => this.request<UiState>("ui.badge.clear", { pane }),
    /** Entries in the pane context menu; the action gets the pane it was opened on as `call.target` ({ pane, instance }). */
    menu: (items: MenuItem[]) => this.request<UiState>("ui.menu.set", { items }),
    /** A toast in every attached client (a system notification too, if the user has those on); a few per 10s. */
    toast: (text: string, options: { tone?: Tone; system?: boolean } = {}) => this.request<true>("ui.toast", { text, ...options }),
    /** Close this plugin's popup, if one is open. */
    closePopup: () => this.request<true>("ui.popup.close"),
    /** What this plugin shows now. */
    state: () => this.request<UiState>("ui.state", { plugin: this.name ?? "" }),
  };

  /**
   * Start watching. `onSnapshot` gets every pane as of the moment the subscription starts. `onEvent` then gets each
   * later event once, in order, from the same server run, and never an event already reflected in the snapshot.
   * Handlers run one at a time. Events waiting for them are capped at `maxBacklog`: past it the connection is dropped
   * (a gap) rather than letting a stalled handler grow memory. A disconnect ends the stream (see `closed`); changes
   * that came and went while disconnected are lost, so don't promise a complete history.
   */
  async subscribe(handlers: { onSnapshot?: (snapshot: Snapshot) => unknown; onEvent: (event: Event) => unknown }, options: { output?: boolean; maxBacklog?: number } = {}) {
    const max = options.maxBacklog ?? 10_000;
    let ready = false;
    let epoch = "";
    let last = 0;
    let waiting = 0; // events handed to the queue and not handled yet
    const early: Event[] = [];
    let queue: Promise<unknown> = Promise.resolve();
    const overflow = () => {
      this.transport.close();
      this.drop(new ModisaError(`more than ${max} events waiting for the plugin's handlers: disconnected, so everything after is a gap`, "backlog"));
    };
    const deliver = (e: Event) => {
      if (e.epoch !== epoch || e.seq <= last) return; // another server run, in the snapshot already, or seen
      last = e.seq;
      if (++waiting > max) return overflow();
      queue = queue
        .then(() => handlers.onEvent(e))
        .catch((error) => console.error(`plugin: onEvent failed: ${error instanceof Error ? error.stack : error}`))
        .finally(() => waiting--);
    };
    this.listeners.push((e) => {
      if (ready) deliver(e);
      else if (early.push(e) > max) overflow();
    });
    const snapshot = await this.request<Snapshot>("events.subscribe", { snapshot: true, output: !!options.output });
    epoch = snapshot.epoch;
    last = snapshot.seq;
    await handlers.onSnapshot?.(snapshot);
    for (const e of early.sort((a, b) => a.seq - b.seq)) deliver(e);
    ready = true;
    return snapshot;
  }

  private send(message: object) {
    if (this.cause) return;
    try {
      this.transport.write(JSON.stringify(message) + "\n");
    } catch (error) {
      this.transport.close();
      this.drop(error instanceof Error ? error : new Error(String(error)));
    }
  }

  private async answer(id: number, { action, params, invocation, target, link }: { action?: string; params?: Record<string, unknown>; invocation?: string; target?: { pane: string; instance: string }; link?: string }) {
    const reply = (x: object) => this.send({ jsonrpc: "2.0", id, ...x });
    const run = action ? this.actions[action] : undefined;
    if (!run) return reply({ error: { code: -32601, message: `no action ${action}` } });
    const controller = new AbortController();
    if (invocation) this.running.set(invocation, controller);
    try {
      reply({ result: (await run(params ?? {}, { invocation, signal: controller.signal, ...(target && { target }), ...(link && { link }) })) ?? null });
    } catch (error) {
      reply({ error: { code: -32000, message: error instanceof Error ? error.message : String(error) } });
    } finally {
      if (invocation) this.running.delete(invocation);
    }
  }
}

/**
 * Bytes waiting for the socket, in order. Bun's socket.write takes what fits (possibly nothing) and returns how many
 * bytes; the rest is sent as the socket drains. Past `limit` queued bytes, push throws: the session isn't reading.
 */
export function writeQueue(write: (bytes: Uint8Array) => number, limit = 64 * 1024 * 1024) {
  const queue: Uint8Array[] = [];
  const encoder = new TextEncoder();
  let queued = 0;
  const flush = () => {
    while (queue.length) {
      const chunk = queue[0]!;
      const n = Math.max(0, write(chunk));
      queued -= n;
      if (n < chunk.length) {
        queue[0] = chunk.subarray(n);
        return;
      }
      queue.shift();
    }
  };
  return {
    push(line: string) {
      const bytes = encoder.encode(line);
      if (queued + bytes.length > limit) throw new Error(`more than ${limit} bytes waiting to be sent: the session isn't reading`);
      queue.push(bytes);
      queued += bytes.length;
      flush();
    },
    flush,
    clear() {
      queue.length = 0;
      queued = 0;
    },
    get queued() {
      return queued;
    },
  };
}

type Socket = { write(data: Uint8Array): number; end(): void };
type Connector = (options: { unix: string; socket: Record<string, (...args: any[]) => void> }) => Promise<Socket>;

/** Connect to the session this plugin was started for ($MODISA_SOCKET). (`open` is for tests: a fake socket.) */
export async function connect(socket = Bun.env.MODISA_SOCKET, open: Connector = Bun.connect as unknown as Connector): Promise<Client> {
  if (!socket) throw new ModisaError("no $MODISA_SOCKET: modisa starts plugins with it set", "usage");
  let sock: Socket | undefined;
  let ended = false;
  const out = writeQueue((bytes) => sock?.write(bytes) ?? 0);
  // buffers cleared and the socket ended, once, however the connection goes
  const shut = () => {
    out.clear();
    if (ended || !sock) return;
    ended = true;
    sock.end();
  };
  const client = new Client({ write: (line) => out.push(line), close: shut });
  const gone = (error?: Error) => {
    shut();
    client.drop(error);
  };
  const decoder = new TextDecoder();
  sock = await open({
    unix: socket,
    socket: {
      // a write that fails while draining ends the connection like one that fails when sent
      drain: () => {
        try {
          out.flush();
        } catch (error) {
          gone(error instanceof Error ? error : new Error(String(error)));
        }
      },
      data: (_s, d) => client.feed(decoder.decode(d, { stream: true })),
      end: () => gone(),
      close: () => gone(),
      error: (_s, error) => gone(error),
    },
  });
  return client;
}

/** Run a plugin: connect, run `main`, and exit once the session's connection closes (the next server starts it again). */
export async function runPlugin(main: (modisa: Client) => unknown) {
  try {
    const modisa = await connect();
    await main(modisa);
    const why = await modisa.closed;
    console.log(`session connection closed (${why.message}); exiting`);
    process.exit(0);
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exit(1);
  }
}

/**
 * For a plugin's own tests, run by `modisa plugin check`: the throwaway session it started with this plugin running.
 * `modisa(...args)` runs the CLI against it, `json(...args)` parses a --json reply, `data` is the plugin's data
 * directory, `until` waits for a condition.
 */
export function checkSession() {
  const raw = Bun.env.MODISA_CHECK;
  if (!raw) throw new Error("run these tests with `modisa plugin check <dir>`: it starts a throwaway session with this plugin running");
  const { bin, session, env, plugin, data } = JSON.parse(raw) as { bin: string[]; session: string; env: Record<string, string>; plugin: string; data: string };
  const modisa = async (...args: string[]) => {
    const p = Bun.spawn([...bin, "-s", session, ...args], { env: { ...Bun.env, ...env, MODISA_CHECK: "" }, stdout: "pipe", stderr: "pipe" });
    const [stdout, stderr] = await Promise.all([new Response(p.stdout).text(), new Response(p.stderr).text()]);
    return { code: await p.exited, stdout: stdout.trim(), stderr: stderr.trim() };
  };
  const json = async <T = any>(...args: string[]): Promise<T> => {
    const r = await modisa(...args, "--json");
    if (r.code !== 0) throw new Error(`modisa ${args.join(" ")} exited ${r.code}: ${r.stderr}`);
    return JSON.parse(r.stdout);
  };
  const until = async (what: string, ok: () => unknown, ms = 10_000) => {
    for (const end = Date.now() + ms; Date.now() < end; await Bun.sleep(100)) if (await ok()) return;
    throw new Error(`timed out waiting for ${what}`);
  };
  /** What the plugin shows in the TUI now (status, sidebar, badges, menu, palette actions). */
  const ui = () => json<UiState>("plugin", "ui", plugin);
  return { plugin, session, data, modisa, json, until, ui };
}
