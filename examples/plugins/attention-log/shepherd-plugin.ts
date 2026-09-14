// shepherd-plugin.ts: shepherd's client library for plugins (protocol 1). No dependencies.
// `shepherd plugin new` copies it into each plugin; refresh a plugin's copy with `shepherd plugin sdk > shepherd-plugin.ts`.
//
// It handles the protocol so a plugin only writes its own logic: newline-delimited JSON-RPC framing, request ids,
// errors with shepherd's stable codes, binding to the host (hello) with actions, and starting a subscription with no
// gap and no duplicates (subscribe). When the session's socket closes, `closed` resolves: exit then, because the next
// server starts the plugin again. runPlugin does all of that.

export const SDK_VERSION = 1;
export const PROTOCOL = 1;

export type AgentState = "working" | "blocked" | "done" | "idle";
// A pane: `id` (like p3) plus `instance`, unique to the process it was started with (ids come back after a restart).
// `agent` is what shepherd's detection observed: its screen, or its integration's report. No `agent` means no agent
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
// Every event has type, at (epoch ms), seq and epoch; `shepherd plugin schema` has each type's fields.
export type Event = { type: string; at: number; seq: number; epoch: string; pane?: string; instance?: string; name?: string; harness?: string; from?: AgentState; to?: AgentState; [field: string]: unknown };
export type Snapshot = { protocol: number; epoch: string; seq: number; panes: Pane[] };
export type Action = (params: Record<string, unknown>) => unknown;

export class ShepherdError extends Error {
  constructor(message: string, readonly code: string) {
    super(message);
    this.name = "ShepherdError";
  }
}

export class Client {
  private seq = 0;
  private pending = new Map<number, { resolve: (value: any) => void; reject: (error: Error) => void }>();
  private buf = "";
  private actions: Record<string, Action> = {};
  private listeners: ((e: Event) => void)[] = [];
  private cause?: Error;
  private onClosed!: (cause: Error) => void;
  /** Resolves, with the reason, once the connection to the session is gone. */
  readonly closed = new Promise<Error>((resolve) => (this.onClosed = resolve));

  constructor(private transport: { write(line: string): void; close(): void }) {}

  /** Bytes from the socket. */
  feed(chunk: string) {
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
        if (m.error) p.reject(new ShepherdError(String(m.error.message), m.error.data?.code ?? "error"));
        else p.resolve(m.result);
      } else if (m.method === "event" && m.params) for (const listen of this.listeners) listen(m.params);
      else if (m.method === "plugin.action" && m.id !== undefined) void this.answer(m.id, m.params ?? {});
    }
  }

  /** The connection is gone. */
  drop(cause = new Error("the session closed the connection")) {
    if (this.cause) return;
    this.cause = cause;
    for (const p of this.pending.values()) p.reject(cause);
    this.pending.clear();
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

  /** Bind this connection to the plugin shepherd started, offering actions to `shepherd plugin run <name> <action>`. */
  hello(actions: Record<string, Action> = {}, token = Bun.env.SHEPHERD_PLUGIN_TOKEN) {
    if (!token) throw new ShepherdError("no $SHEPHERD_PLUGIN_TOKEN: shepherd starts plugins (shepherd plugin link, then shepherd restart), not a shell", "usage");
    this.actions = actions;
    return this.request<{ name: string; protocol: number; session: string; epoch: string }>("plugin.hello", { token, actions: Object.keys(actions) });
  }

  /**
   * Start watching. `onSnapshot` gets every pane as of the moment the subscription starts. `onEvent` then gets each
   * later event once, in order, from the same server run, and never an event already reflected in the snapshot.
   * Handlers run one at a time. A disconnect ends the stream (see `closed`); changes that came and went while
   * disconnected are lost, so don't promise a complete history.
   */
  async subscribe(handlers: { onSnapshot?: (snapshot: Snapshot) => unknown; onEvent: (event: Event) => unknown }, options: { output?: boolean } = {}) {
    let ready = false;
    let epoch = "";
    let last = 0;
    const early: Event[] = [];
    let queue: Promise<unknown> = Promise.resolve();
    const deliver = (e: Event) => {
      if (e.epoch !== epoch || e.seq <= last) return; // another server run, in the snapshot already, or seen
      last = e.seq;
      queue = queue.then(() => handlers.onEvent(e)).catch((error) => console.error(`plugin: onEvent failed: ${error instanceof Error ? error.stack : error}`));
    };
    this.listeners.push((e) => (ready ? deliver(e) : early.push(e)));
    const snapshot = await this.request<Snapshot>("events.subscribe", { snapshot: true, output: !!options.output });
    epoch = snapshot.epoch;
    last = snapshot.seq;
    await handlers.onSnapshot?.(snapshot);
    for (const e of early.sort((a, b) => a.seq - b.seq)) deliver(e);
    ready = true;
    return snapshot;
  }

  private send(message: object) {
    if (!this.cause) this.transport.write(JSON.stringify(message) + "\n");
  }

  private async answer(id: number, { action, params }: { action?: string; params?: Record<string, unknown> }) {
    const reply = (x: object) => this.send({ jsonrpc: "2.0", id, ...x });
    const run = action ? this.actions[action] : undefined;
    if (!run) return reply({ error: { code: -32601, message: `no action ${action}` } });
    try {
      reply({ result: (await run(params ?? {})) ?? null });
    } catch (error) {
      reply({ error: { code: -32000, message: error instanceof Error ? error.message : String(error) } });
    }
  }
}

/** Connect to the session this plugin was started for ($SHEPHERD_SOCKET). */
export async function connect(socket = Bun.env.SHEPHERD_SOCKET): Promise<Client> {
  if (!socket) throw new ShepherdError("no $SHEPHERD_SOCKET: shepherd starts plugins with it set", "usage");
  let sock: { write(data: string): number; end(): void } | undefined;
  const client = new Client({ write: (line) => void sock?.write(line), close: () => sock?.end() }); // ponytail: plugin writes are small, no write queue
  const decoder = new TextDecoder();
  sock = await Bun.connect({
    unix: socket,
    socket: {
      data: (_s, d) => client.feed(decoder.decode(d, { stream: true })),
      end: () => client.drop(),
      close: () => client.drop(),
      error: (_s, error) => client.drop(error),
    },
  });
  return client;
}

/** Run a plugin: connect, run `main`, and exit once the session's connection closes (the next server starts it again). */
export async function runPlugin(main: (shepherd: Client) => unknown) {
  try {
    const shepherd = await connect();
    await main(shepherd);
    const why = await shepherd.closed;
    console.log(`session connection closed (${why.message}); exiting`);
    process.exit(0);
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exit(1);
  }
}

/**
 * For a plugin's own tests, run by `shepherd plugin check`: the throwaway session it started with this plugin running.
 * `shepherd(...args)` runs the CLI against it, `json(...args)` parses a --json reply, `data` is the plugin's data
 * directory, `until` waits for a condition.
 */
export function checkSession() {
  const raw = Bun.env.SHEPHERD_CHECK;
  if (!raw) throw new Error("run these tests with `shepherd plugin check <dir>`: it starts a throwaway session with this plugin running");
  const { bin, session, env, plugin, data } = JSON.parse(raw) as { bin: string[]; session: string; env: Record<string, string>; plugin: string; data: string };
  const shepherd = async (...args: string[]) => {
    const p = Bun.spawn([...bin, "-s", session, ...args], { env: { ...Bun.env, ...env, SHEPHERD_CHECK: "" }, stdout: "pipe", stderr: "pipe" });
    const [stdout, stderr] = await Promise.all([new Response(p.stdout).text(), new Response(p.stderr).text()]);
    return { code: await p.exited, stdout: stdout.trim(), stderr: stderr.trim() };
  };
  const json = async <T = any>(...args: string[]): Promise<T> => {
    const r = await shepherd(...args, "--json");
    if (r.code !== 0) throw new Error(`shepherd ${args.join(" ")} exited ${r.code}: ${r.stderr}`);
    return JSON.parse(r.stdout);
  };
  const until = async (what: string, ok: () => unknown, ms = 10_000) => {
    for (const end = Date.now() + ms; Date.now() < end; await Bun.sleep(100)) if (await ok()) return;
    throw new Error(`timed out waiting for ${what}`);
  };
  return { plugin, session, data, shepherd, json, until };
}
