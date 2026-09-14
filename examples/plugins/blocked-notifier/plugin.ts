#!/usr/bin/env bun
// A shepherd plugin: shout when an agent gets blocked, with the last few lines of its screen.
//
// Run it from ~/.config/shepherd/config.toml:
//
//   [[plugin]]
//   run = "bun ~/code/shepherd/examples/plugins/blocked-notifier/plugin.ts"
//
// The server starts this with $SHEPHERD_SOCKET set and restarts nothing if it dies, so it keeps its
// own reconnect loop. It talks the same JSON-RPC the CLI does — no shepherd import, no dependencies.

const SOCKET = Bun.env.SHEPHERD_SOCKET;
if (!SOCKET) {
  console.error("blocked-notifier: no $SHEPHERD_SOCKET — run me from [[plugin]], not by hand");
  process.exit(1);
}

// What to do about a blocked agent. Swap in `notify-send`, a Slack webhook, anything.
async function announce(name: string, harness: string | undefined, screen: string) {
  const tail = screen.trimEnd().split("\n").slice(-3).join("\n  ");
  console.log(`[blocked-notifier] @${name}${harness ? ` (${harness})` : ""} needs you\n  ${tail}`);
}

// ---- the plumbing: newline-delimited JSON-RPC 2.0 over the unix socket ----

type Msg = { id?: number; method?: string; params?: any; result?: any; error?: { message: string } };

class Session {
  private socket!: Awaited<ReturnType<typeof Bun.connect>>;
  private pending = new Map<number, (r: any) => void>();
  private failed = new Map<number, (e: Error) => void>();
  private buf = "";
  private seq = 0;
  onEvent: (e: any) => void = () => {};

  async open() {
    const dec = new TextDecoder();
    this.socket = await Bun.connect({
      unix: SOCKET!,
      socket: {
        data: (_s, d) => this.feed(dec.decode(d, { stream: true })),
        close: () => this.drop(new Error("server closed the connection")),
        error: (_s, e) => this.drop(e),
      },
    });
  }

  // The server answers requests by id and pushes events as a bare `event` notification.
  private feed(chunk: string) {
    this.buf += chunk;
    for (let nl; (nl = this.buf.indexOf("\n")) >= 0; ) {
      const line = this.buf.slice(0, nl);
      this.buf = this.buf.slice(nl + 1);
      if (!line.trim()) continue;
      let m: Msg;
      try {
        m = JSON.parse(line);
      } catch {
        continue; // never let a bad line kill the plugin
      }
      if (m.id !== undefined && !m.method) {
        const ok = this.pending.get(m.id);
        const no = this.failed.get(m.id);
        this.pending.delete(m.id);
        this.failed.delete(m.id);
        m.error ? no?.(new Error(m.error.message)) : ok?.(m.result);
      } else if (m.method === "event") this.onEvent(m.params);
    }
  }

  private drop(cause: Error) {
    for (const reject of this.failed.values()) reject(cause);
    this.pending.clear();
    this.failed.clear();
    this.onClose(cause);
  }
  onClose: (cause: Error) => void = () => {};

  request<T>(method: string, params: Record<string, unknown> = {}): Promise<T> {
    const id = ++this.seq;
    return new Promise<T>((resolve, reject) => {
      this.pending.set(id, resolve);
      this.failed.set(id, reject);
      this.socket.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n");
    });
  }
}

// ---- the loop ----

async function run() {
  const s = new Session();
  await s.open();

  s.onEvent = async (e) => {
    if (e.type !== "agent.state" || e.to !== "blocked") return;
    // Read the pane that just blocked, so the notification carries the question being asked.
    // pane.read answers with the pane's info plus `screen` (what's visible) and `recentOutput`
    // (the tail of scrollback) — not a bare string.
    const pane = await s.request<{ screen: string }>("pane.read", { target: e.pane, lines: 20 }).catch(() => undefined);
    await announce(e.name ?? e.pane, e.harness, pane?.screen ?? "");
  };

  // Ask for events. Without `output: true` the firehose of pane.output is left out — take it only if
  // you actually read terminal bytes, it is every keystroke of every pane.
  await s.request("events.subscribe", { output: false });
  console.log("[blocked-notifier] watching");

  await new Promise<void>((resolve) => { s.onClose = () => resolve(); });
}

// Exit when the socket goes: the server starts plugins itself, so a restart spawns a fresh copy of
// this. Looping forever here would leave the old process reconnecting alongside the new one, and
// every restart would add another.
try {
  await run();
  console.log("[blocked-notifier] session ended");
} catch (e) {
  console.error(`[blocked-notifier] ${(e as Error).message}`);
  process.exit(1);
}
