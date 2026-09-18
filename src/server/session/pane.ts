// Server-side pane: PTY + headless libghostty screen. Lives as long as the server, clients come and go.
import { Terminal, Formatter, RenderState } from "libghostty-vt";
import type { PaneInfo } from "../../protocol/types";
import { self } from "../../core/paths";

const plain = new Formatter({ format: "plain" });
const replay = new Formatter({ format: "vt", modes: true, cursor: true, style: true });

// The payload of the last complete ESC ] 9 ; 4 … (BEL or ESC \) in a chunk of output.
const OSC_PROGRESS = /\x1b\]9;(4;[^\x07\x1b]*)(?:\x07|\x1b\\)/g;
const latin1 = new TextDecoder("latin1");
function progress(bytes: Uint8Array): string | undefined {
  let found = false; // most output has no OSC 9 at all: skip decoding it
  for (let i = bytes.indexOf(0x1b); i >= 0 && !found; i = bytes.indexOf(0x1b, i + 1)) found = bytes[i + 1] === 0x5d && bytes[i + 2] === 0x39 && bytes[i + 3] === 0x3b;
  if (!found) return;
  let last: string | undefined;
  for (const m of latin1.decode(bytes).matchAll(OSC_PROGRESS)) last = m[1];
  return last;
}

export class PtyPane {
  info: PaneInfo;
  vt: Terminal;
  pty: Bun.Terminal;
  proc: Bun.Subprocess;
  lastOutput = 0;
  oscTitle = ""; // the terminal title the program last set (OSC 0/2), e.g. an agent's spinner
  oscProgress = ""; // its last OSC 9;4 progress report, e.g. "4;3" busy, "4;0" cleared
  disposed = false; // its libghostty screen is freed; async work that held on to it must skip it
  closedWhileRunning = false; // closed before its process exited, so the exit that follows was caused by the close
  private rs = new RenderState();

  constructor(
    opts: { id: string; cwd: string; command?: string; harness?: string; name?: string; createdBy: string; cols: number; rows: number; env?: Record<string, string> },
    private hooks: { output: (p: PtyPane, bytes: Uint8Array) => void; exit: (p: PtyPane) => void; title: (p: PtyPane) => void },
  ) {
    const shell = Bun.env.SHELL || "/bin/sh";
    const cols = Math.max(opts.cols, 2), rows = Math.max(opts.rows, 1);
    this.info = {
      id: opts.id,
      instance: crypto.randomUUID().slice(0, 8),
      name: opts.name,
      title: opts.name ?? (opts.command ? opts.command.split(" ")[0]! : shell.split("/").pop()!),
      cwd: opts.cwd,
      command: opts.command,
      harness: opts.harness,
      createdBy: opts.createdBy,
      status: "running",
      cols,
      rows,
    };
    this.vt = new Terminal({
      cols,
      rows,
      maxScrollback: 10_000,
      // query replies (DA, cursor position…) come from here, not from clients, so they work while detached
      onWritePty: (b) => queueMicrotask(() => this.write(b)),
      onTitleChanged: (t) => {
        this.oscTitle = t;
        // read on request (list, a snapshot): not pushed to clients, which a spinning title would do many times a second
        if (t) this.info.terminalTitle = t;
        else delete this.info.terminalTitle;
        if (!this.info.name && t) {
          this.info.title = t;
          queueMicrotask(() => this.hooks.title(this));
        }
      },
    });
    this.pty = new Bun.Terminal({
      cols,
      rows,
      data: (_t, bytes) => {
        this.lastOutput = Date.now();
        this.oscProgress = progress(bytes) ?? this.oscProgress;
        this.vt.vtWrite(bytes);
        this.hooks.output(this, bytes);
      },
    });
    // via __pty-exec so the shell gets the PTY as its controlling terminal (job control, ^C, detection)
    const argv = [...self(), "__pty-exec", ...(opts.command ? [shell, "-lc", opts.command] : [shell, "-l"])];
    try { this.proc = Bun.spawn(argv, {
      terminal: this.pty,
      detached: true, // setsid, so the PTY becomes the controlling tty (^C, job control)
      cwd: opts.cwd,
      env: {
        ...Bun.env,
        TERM: "xterm-256color",
        COLORTERM: "truecolor",
        MODISA_PANE_ID: opts.id,
        PWD: opts.cwd,
        ...opts.env,
      },
    }); } catch (error) {
      this.pty.close();
      this.rs.close();
      this.vt.close();
      throw error;
    }
    this.proc.exited.then((code) => {
      this.info.status = "exited";
      this.info.exitCode = code ?? undefined;
      this.hooks.exit(this);
    });
  }

  get id() {
    return this.info.id;
  }

  write(data: string | Uint8Array) {
    if (this.info.status === "running" && !this.pty.closed) this.pty.write(data);
  }

  // Text as typed input; bracketed when the app asked for it so newlines don't submit early.
  paste(text: string) {
    this.write(this.vt.mode("bracketed_paste") ? `\x1b[200~${text}\x1b[201~` : text);
  }

  resize(cols: number, rows: number) {
    cols = Math.max(cols, 2);
    rows = Math.max(rows, 1);
    if (cols === this.info.cols && rows === this.info.rows) return;
    this.info.cols = cols;
    this.info.rows = rows;
    this.vt.resize(cols, rows);
    if (!this.pty.closed) this.pty.resize(cols, rows);
  }

  // VT stream that reproduces the current screen on a fresh client terminal.
  replay(): string {
    return replay.formatString(this.vt);
  }

  // Visible screen only (no scrollback) — what detection rules look at. Soft-wrapped rows are
  // joined so a rule still matches when a narrow pane wraps it.
  screen(): string {
    this.rs.update(this.vt);
    let out = "";
    for (const row of this.rs.rows()) {
      let s = "";
      for (const c of row.cells()) if (!c.isWideContinuation) s += c.text || " ";
      out += row.wrapped ? s : s.trimEnd() + "\n";
    }
    return out.trimEnd();
  }

  // Scrollback + screen as plain text.
  text(): string {
    return plain.formatString(this.vt);
  }

  kill() {
    Bun.$`kill -HUP -- -${this.proc.pid}`.quiet().nothrow(); // whole process group, like closing a terminal
    if (!this.pty.closed) this.pty.close();
  }

  dispose() {
    this.closedWhileRunning = this.info.status !== "exited"; // any exit from here on is the kill below, not its own
    this.disposed = true;
    this.kill();
    this.rs.close();
    this.vt.close();
  }
}
