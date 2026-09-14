// One JSON-RPC connection over any byte transport (unix socket or ssh stdio).
import type { Msg } from "./schema";
import { ERROR_CODES, type ErrorCode } from "./types";

// An error carrying a stable code, on either side of the wire.
export const fail = (code: ErrorCode, message: string) => Object.assign(new Error(message), { code });

// Only shepherd's own codes count; anything else (an OS error's ECONNRESET, a library's code) is plain "error".
export const errorCode = (x: unknown): ErrorCode => ((ERROR_CODES as readonly unknown[]).includes(x) ? (x as ErrorCode) : "error");

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
        m.error ? p.reject(fail(errorCode(m.error.data?.code), m.error.message)) : p.resolve(m.result);
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

// A peer that stops reading can't make this side buffer without limit: past this many queued bytes the connection is
// closed. For a subscriber that's a gap in its history; it reconnects and takes a new snapshot.
export const WRITE_QUEUE_LIMIT = Number(Bun.env.SHEPHERD_WRITE_QUEUE_LIMIT) || 16 * 1024 * 1024;

// Bun socket → Conn, with a bounded write queue for backpressure. Used on both ends.
export function socketConn(s: { write(d: Uint8Array): number; end(): void; terminate?(): void }) {
  const enc = new TextEncoder();
  const queue: Uint8Array[] = [];
  let queued = 0;
  const flush = () => {
    while (queue.length) {
      const chunk = queue[0]!;
      const n = Math.max(0, s.write(chunk));
      queued -= n;
      if (n < chunk.length) {
        queue[0] = chunk.subarray(n);
        return;
      }
      queue.shift();
    }
  };
  const conn: Conn = new Conn((str) => {
    const bytes = enc.encode(str);
    if (queued + bytes.length > WRITE_QUEUE_LIMIT) {
      const message = `more than ${WRITE_QUEUE_LIMIT} bytes waiting to be written: the peer isn't reading`;
      console.error(`shepherd: closing a connection: ${message}`); // in the server's log
      queue.length = 0;
      queued = 0;
      // abruptly: a graceful end() waits on a peer that isn't reading, and it would never see the close
      s.terminate?.();
      conn.close();
      throw new Error(message);
    }
    queue.push(bytes);
    queued += bytes.length;
    flush();
  }, () => s.end());
  return { conn, flush };
}
