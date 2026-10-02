// `modisa pane attach [pane] [--takeover | --observe] [--remote ssh://host]`: one pane full-screen in this terminal,
// which is its emulator: the server's replay redraws the pane's screen (its modes too), then its output streams in.
// --takeover (the default) drives it, at this terminal's size, and only this terminal's typing reaches it; --observe
// watches it at its own size and types nothing. The prefix then d detaches (the prefix twice types it once); observing,
// q or Ctrl-C does too. Exit status: 0 detached (or the pane was closed), the pane's own when its process exits, 3 when
// the connection is lost.
import { connectExisting, connectStdio } from "../protocol/transport";
import { ConnectionClosedError, b64, errorCode, unb64, type Conn } from "../protocol/conn";
import type { Msg } from "../protocol/schema";
import { loadConfig, parsePrefix } from "../config/config";
import { socketPath } from "../core/paths";
import { sshProxy } from "./sessions";
import { failed } from "./commands";
import { str, type Args } from "./args";

// Everything a program in the pane may have turned on, off again: style, a hidden cursor and its shape, mouse reporting
// (1000/1002/1003 and SGR 1006), bracketed paste, focus events, application cursor keys and keypad, the kitty keyboard
// flags it pushed, the scroll region. Kitty's flags are kept per screen, so popping them on the alternate one leaves the
// shell's alone.
const MODES_OFF = "\x1b[0m\x1b[?25h\x1b[0 q\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?2004l\x1b[?1004l\x1b[?1l\x1b>\x1b[<99u\x1b[r";
const ENTER = `\x1b[?1049h${MODES_OFF}\x1b[H\x1b[2J`; // the alternate screen, as a program's would start
const LEAVE = `${MODES_OFF}\x1b[?1049l`; // and this terminal's own screen back as it was
// What the pane sends can't reach this terminal's own screen: its screen switches stay out (the server redraws the
// pane instead, attach.ts), and so does erasing the scrollback, which is the user's.
const OWN_SCREEN = /\x1b\[(?:\?(?:1049|1047|47)[hl]|3J)/g;
const kept = (b: Uint8Array) => (b.includes(0x1b) ? Buffer.from(Buffer.from(b).toString("latin1").replace(OWN_SCREEN, ""), "latin1") : b);

// One key at the start of `s` (bytes as latin1): a kitty keyboard report (CSI code[:alternates] [;mods[:event]] [;text] u,
// sent once the program in the pane asks for them) or one byte. ctrl: with Ctrl and nothing else; plain: no modifier;
// up: a key's release (reported when asked for).
const CSI_U = /^\x1b\[(\d+)(?::[\d:]*)?(?:;(\d*)(?::(\d+))?)?(?:;[\d:]*)?u/;
function key(s: string) {
  const m = CSI_U.exec(s);
  if (m) {
    const mods = (Math.max(1, Number(m[2] || 1)) - 1) & ~(64 | 128); // caps and num lock don't count
    return { len: m[0].length, ch: String.fromCodePoint(Math.min(Number(m[1]), 0x10ffff)), ctrl: mods === 4, plain: mods === 0, up: m[3] === "3" };
  }
  const b = s.charCodeAt(0);
  return b < 0x20 ? { len: 1, ch: String.fromCharCode(b | 0x60), ctrl: true, plain: false, up: false } : { len: 1, ch: s[0]!, ctrl: false, plain: true, up: false };
}

export async function paneAttach(a: Args, target?: string): Promise<number> {
  const f = a.flags;
  if (f.takeover && f.observe) return failed("usage", "modisa: use one of --takeover and --observe", false);
  if (!process.stdin.isTTY || !process.stdout.isTTY) return failed("usage", "modisa: pane attach needs a terminal: its input and output can't be redirected", false);
  const observe = !!f.observe;
  const remote = str(f.remote), session = str(f.session);
  const cfg = await loadConfig();
  // $MODISA_PANE_ID is a pane of this pane's own server: over ssh, or in another session, the caller is no pane at all
  const caller = !remote && (!session || socketPath(session) === Bun.env.MODISA_SOCKET) ? Bun.env.MODISA_PANE_ID : undefined;
  let conn: Conn;
  try {
    conn = remote ? connectStdio(sshProxy(remote, cfg.remote_command, session ?? "default")) : await connectExisting(session);
  } catch (e: any) {
    return failed("unreachable", e.message, false);
  }
  // What arrives with the reply, or before this side is ready for it, waits its turn: the replay is drawn first.
  const queue: (Msg | "closed")[] = [];
  let handle: (m: Msg | "closed") => void = (m) => queue.push(m);
  conn.onMessage = (m) => handle(m);
  conn.onClose = () => handle("closed");

  const size = () => ({ cols: Math.max(2, process.stdout.columns || 80), rows: Math.max(1, process.stdout.rows || 24) });
  let r: { pane: string; cols: number; rows: number; data: string };
  try {
    r = await conn.request("pane.attach", { caller, target, mode: observe ? "observe" : "takeover", ...size() });
  } catch (e: any) {
    conn.onClose = () => {};
    conn.close();
    return failed(e instanceof ConnectionClosedError ? "unreachable" : errorCode(e.code), `modisa: ${e.message}`, false);
  }

  const write = (b: string | Uint8Array) => {
    try { process.stdout.write(b); } catch {} // a terminal that's gone (SIGHUP)
  };
  const prefix = parsePrefix(cfg.prefix).name; // the letter: C-b → b
  return new Promise<number>((resolve) => {
    let lost = "connection lost";
    let done = false;
    const finish = (code: number, note: string, error = false) => {
      if (done) return;
      done = true;
      process.stdin.off("data", typed);
      process.off("SIGWINCH", resized);
      for (const [sig, h] of signals) process.off(sig, h);
      write(LEAVE);
      try { process.stdin.setRawMode(false); } catch {}
      process.stdin.pause();
      conn.onClose = () => {};
      conn.close();
      if (note) (error ? console.error : console.log)(note);
      resolve(code);
    };

    // Observing, the pane keeps its own size: one bigger than this terminal is cut off at its edge, so say so (over the
    // bottom row, where the pane's next redraw of it goes over it).
    const warn = () => {
      const { cols, rows } = size();
      if (!observe || (cols >= r.cols && rows >= r.rows)) return;
      const text = ` ${r.pane} is ${r.cols}×${r.rows}, bigger than this terminal (${cols}×${rows}): past its edge is cut off `;
      write(`\x1b7\x1b[${rows};1H\x1b[0;7m${text.slice(0, cols)}\x1b[0m\x1b8`);
    };
    const resized = () => (observe ? warn() : conn.request("pane.attach.resize", size()).catch(() => {}));

    let armed = false; // the prefix was pressed: the next key is for modisa
    const typed = (chunk: Buffer) => {
      const s = chunk.toString("latin1");
      let send = "";
      const flush = () => {
        if (send && !observe) conn.notify("input", { pane: r.pane, data: b64(Buffer.from(send, "latin1")) });
        send = "";
      };
      for (let i = 0; i < s.length; ) {
        const k = key(s.slice(i));
        const raw = s.slice(i, (i += k.len));
        if (armed) {
          if (k.up) continue; // the prefix's own release
          armed = false;
          if (k.plain && k.ch === "d") {
            flush(); // what came before it in this chunk
            return finish(0, `[detached from ${r.pane}]`);
          }
          // the prefix twice types it once; after any other key the prefix is dropped and the key goes on
        } else if (k.ctrl && k.ch === prefix && !k.up) {
          armed = true;
          continue;
        } else if (observe && !k.up && ((k.plain && k.ch === "q") || (k.ctrl && k.ch === "c"))) return finish(0, `[stopped watching ${r.pane}]`);
        send += raw;
      }
      flush();
    };

    const signals = (["SIGINT", "SIGTERM", "SIGHUP"] as const).map((sig, i) => [sig, () => finish(128 + [2, 15, 1][i]!, "")] as const);
    for (const [sig, h] of signals) process.on(sig, h);
    process.on("SIGWINCH", resized);

    write(ENTER);
    write(kept(unb64(r.data)));
    warn();
    handle = (m) => {
      if (m === "closed") return finish(3, `[${lost}]`, true);
      const d = m.params;
      if (m.method === "output") write(kept(unb64(d.data)));
      else if (m.method === "attach.end") d.reason === "exited" ? finish(d.exitCode ?? 1, `[${r.pane} exited ${d.exitCode ?? "?"}]`) : finish(0, `[${r.pane} closed]`);
      else if (m.method === "restart") lost = "the session's server is restarting: attach again once it's back";
      else if (m.method === "exit") lost = "the session ended";
    };
    for (const m of queue.splice(0)) handle(m);
    if (done) return;
    process.stdin.setRawMode(true);
    process.stdin.on("data", typed);
    process.stdin.resume();
  });
}
