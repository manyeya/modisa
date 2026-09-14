// One JSON-RPC connection over any byte transport (unix socket or ssh stdio).
import type { Msg } from "./schema";
import type { ErrorCode } from "./types";

// An error carrying a stable code, on either side of the wire.
export const fail = (code: ErrorCode, message: string) => Object.assign(new Error(message), { code });

export const b64 = (bytes: Uint8Array) => bytes.toBase64();
export const unb64 = (s: string) => Uint8Array.fromBase64(s);

export class ConnectionClosedError extends Error {
  constructor(cause?: Error) {
    super(cause ? `connection closed: ${cause.message}` : "connection closed", { cause });
    this.name = "ConnectionClosedError";
  }
}

// One connection, either transport. Lines out via `write`, lines in via `feed`.
export class Conn {
  private buf = "";
  private seq = 0;
  private ended = false;
  private pending = new Map<number, { resolve: (v: any) => void; reject: (e: Error) => void }>();
  onMessage: (m: Msg) => void = () => {};
  onClose: () => void = () => {};
  closed = false;
  closeReason: ConnectionClosedError | undefined;

  constructor(private write: (s: string) => void, private end: () => void) {}

  feed(chunk: string) {
    this.buf += chunk;
    let nl;
    while ((nl = this.buf.indexOf("\n")) >= 0) {
      const line = this.buf.slice(0, nl);
      this.buf = this.buf.slice(nl + 1);
      if (!line) continue;
      let m: Msg;
      try {
        m = JSON.parse(line);
      } catch {
        this.send({ jsonrpc: "2.0", error: { code: -32700, message: "parse error" } });
        continue;
      }
      if (m.id !== undefined && !m.method && this.pending.has(m.id)) {
        const p = this.pending.get(m.id)!;
        this.pending.delete(m.id);
        m.error ? p.reject(fail(m.error.data?.code ?? "error", m.error.message)) : p.resolve(m.result);
      } else this.onMessage(m);
    }
  }

  send(m: Msg) {
    if (this.closed) return;
    try { this.write(JSON.stringify(m) + "\n"); }
    catch (error) { this.closedByPeer(error instanceof Error ? error : new Error(String(error))); }
  }

  notify(method: string, params: any) {
    this.send({ jsonrpc: "2.0", method, params });
  }

  request<T = any>(method: string, params: any = {}): Promise<T> {
    if (this.closed) return Promise.reject(this.closeReason ?? new ConnectionClosedError());
    const id = ++this.seq;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.send({ jsonrpc: "2.0", id, method, params });
    });
  }

  closedByPeer(cause?: Error) {
    if (this.closed) return;
    this.closed = true;
    this.closeReason = new ConnectionClosedError(cause);
    for (const p of this.pending.values()) p.reject(this.closeReason);
    this.pending.clear();
    this.onClose();
  }

  close() {
    if (this.ended) return;
    this.ended = true;
    this.closedByPeer();
    try { this.end(); } catch { /* The peer may have already closed the transport. */ }
  }
}

// Bun socket → Conn, with a write queue for backpressure. Used on both ends.
export function socketConn(s: { write(d: Uint8Array): number; end(): void }) {
  const enc = new TextEncoder();
  const queue: Uint8Array[] = [];
  const flush = () => {
    while (queue.length) {
      const n = s.write(queue[0]!);
      if (n < queue[0]!.length) {
        queue[0] = queue[0]!.subarray(n);
        return;
      }
      queue.shift();
    }
  };
  const conn = new Conn((str) => { queue.push(enc.encode(str)); flush(); }, () => s.end());
  return { conn, flush };
}
