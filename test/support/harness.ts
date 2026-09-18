// End-to-end harness: an isolated sandbox per suite, the TUI in a real PTY with its screen read back
// through libghostty, the CLI, and a bare server for suites that don't need a UI.
import { Terminal, Formatter } from "libghostty-vt";

export const MAIN = `${import.meta.dir}/../../src/main.ts`;

export type Sandbox = ReturnType<typeof sandbox>;

// Its own state, config and PATH, so suites never touch the real ~/.config or ~/.local/state.
export function sandbox(name: string) {
  const root = `${Bun.env.TMPDIR ?? "/tmp"}/modisa-${name}-${Date.now()}`;
  const env: Record<string, string> = {
    ...(Bun.env as Record<string, string>),
    MODISA_DIR: `${root}/state`,
    MODISA_CONFIG_DIR: `${root}/config`,
    SHELL: "/bin/sh",
    MODISA_SOUND: "off", // the suite stays silent
    MODISA_UPDATE_URL: "off", // and offline
    MODISA_LOGOS: "off", // and never installs a font or edits a terminal's settings in the real home
    PWD: root,
    PATH: `${root}/bin:${Bun.env.PATH}`,
  };
  delete env.MODISA_SOCKET;
  delete env.MODISA_PANE_ID;
  // the developer's own terminal: shells in panes would load its startup scripts (Terminal.app's prints errors and OSC 7)
  for (const k of ["TERM_PROGRAM", "TERM_PROGRAM_VERSION", "TERM_SESSION_ID", "ITERM_SESSION_ID", "LC_TERMINAL", "LC_TERMINAL_VERSION", "WT_SESSION", "KITTY_WINDOW_ID", "GHOSTTY_RESOURCES_DIR", "VTE_VERSION"]) delete env[k];
  // agents' config-directory overrides would point integration tests at the real ones
  for (const k of ["CLAUDE_CONFIG_DIR", "CODEX_HOME", "COPILOT_HOME", "CURSOR_CONFIG_DIR", "XDG_CONFIG_HOME", "QODER_CONFIG_DIR", "QWEN_HOME", "GROK_CONFIG_DIR", "GROK_HOME", "ANTIGRAVITY_CLI_CONFIG_DIR", "HERMES_HOME", "KIMI_CODE_HOME", "PI_CODING_AGENT_DIR", "PI_CONFIG_DIR"]) delete env[k];
  // `modisa -s <session> <args…>`: stdout and stderr together (out) and apart, trimmed, and the exit status
  const run = async (session: string, args: string[], extra: Record<string, string> = {}) => {
    const p = Bun.spawn(["bun", MAIN, "-s", session, ...args], { env: { ...env, ...extra }, cwd: root, stdout: "pipe", stderr: "pipe" });
    const [stdout, stderr] = await Promise.all([new Response(p.stdout).text(), new Response(p.stderr).text()]);
    return { out: (stdout + stderr).trim(), stdout: stdout.trim(), stderr: stderr.trim(), code: await p.exited };
  };
  const cli = async (session: string, args: string[], extra: Record<string, string> = {}) => (await run(session, args, extra)).out;
  // `modisa -s <session> <args…> --json`, parsed. With retry "startup", a session that isn't answering yet (exit 3,
  // unreachable: no socket yet, or a connection refused or closed while it starts) is tried again until `ms` runs out.
  // Anything else fails at once, as the regression it is: another exit status, a zero exit with nothing on stdout,
  // or stdout that isn't JSON. Every failure names the command, its exit status, stdout and stderr.
  const json = async <T = any>(session: string, args: string[], options: { retry?: "startup" | "none"; ms?: number; env?: Record<string, string> } = {}): Promise<T> => {
    const argv = args.includes("--json") ? args : [...args, "--json"];
    const limit = options.ms ?? 15000;
    const end = Date.now() + limit;
    for (;;) {
      const r = await run(session, argv, options.env);
      const failure = (what: string) => new Error(`modisa -s ${session} ${argv.join(" ")}: ${what}\n  exit status: ${r.code}\n  stdout: ${r.stdout.slice(0, 2000) || "(empty)"}\n  stderr: ${r.stderr.slice(0, 2000) || "(empty)"}`);
      if (r.code === 3) {
        if (options.retry === "startup" && Date.now() < end) {
          await Bun.sleep(100);
          continue;
        }
        throw failure(options.retry === "startup" ? `still unreachable after ${limit}ms` : "unreachable");
      }
      if (r.code !== 0) throw failure("failed");
      if (!r.stdout) throw failure("exit 0 but nothing on stdout");
      try {
        return JSON.parse(r.stdout) as T;
      } catch {
        throw failure("exit 0 but stdout isn't JSON");
      }
    }
  };
  const cleanup = async () => {
    await Bun.$`rm -rf ${root}`.nothrow().quiet();
  };
  return { root, env, cli, run, json, cleanup };
}

// A session server with no client attached; resolves once its first pane exists.
export async function startServer(sb: Sandbox, session: string, extra: Record<string, string> = {}) {
  await Bun.$`mkdir -p ${sb.root}`.quiet();
  const proc = Bun.spawn(["bun", MAIN, "server", "-s", session], { env: { ...sb.env, ...extra }, cwd: sb.root, stdout: "ignore", stderr: "ignore" });
  for (let i = 0; i < 150 && !(await sb.cli(session, ["pane", "list"])).includes("p1"); i++) await Bun.sleep(100); // up to 15s on a loaded machine
  return proc;
}

// The TUI in a PTY. text() is the screen as plain text; until() waits for a condition on it.
export class Screen {
  vt: Terminal;
  fmt = new Formatter({ format: "plain" });
  pty: Bun.Terminal;
  proc: Bun.Subprocess;
  constructor(args: string[], env: Record<string, string>, cwd: string, cols = 140, rows = 40) {
    this.vt = new Terminal({ cols, rows, onWritePty: (b) => queueMicrotask(() => this.pty.write(b)) });
    this.pty = new Bun.Terminal({ cols, rows, data: (_t, b) => this.vt.vtWrite(b) });
    this.proc = Bun.spawn(["bun", MAIN, ...args], { terminal: this.pty, env: { ...env, TERM: "xterm-256color", COLORTERM: "truecolor" }, cwd, detached: true });
  }
  text = () => this.fmt.formatString(this.vt);
  lines = () => this.text().split("\n");
  write = (s: string) => this.pty.write(s);
  resize(cols: number, rows: number) {
    this.vt.resize(cols, rows);
    this.pty.resize(cols, rows);
    process.kill(this.proc.pid, "SIGWINCH"); // the PTY isn't the detached client's controlling terminal, so say it
  }
  async until(what: string, ok: (s: string) => boolean, ms = 8000) {
    for (const end = Date.now() + ms; Date.now() < end; await Bun.sleep(50)) if (ok(this.text())) return;
    throw new Error(`timed out waiting for: ${what}\n----\n${this.text()}\n----`);
  }
  close() {
    this.proc.kill();
    this.pty.close();
  }
}

// How many pane boxes are on screen.
export const borders = (s: string) => s.split("┌").length - 1;
