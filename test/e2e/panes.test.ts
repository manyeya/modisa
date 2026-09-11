// Panes: the shell owns its terminal (^C, job control), and splitting, focus, zoom and tabs work
// from the prefix keys.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { Screen, sandbox, startServer, borders } from "../support/harness";
import { installFakeAgent } from "../support/fake-agent";

const sb = sandbox("panes");
const S = "panes";
let ui: Screen;

beforeAll(async () => {
  await installFakeAgent(sb.root);
  ui = new Screen(["-s", S], sb.env, sb.root);
  await ui.until("first pane", (s) => s.includes("SPACES") && borders(s) === 1);
}, 20000);

afterAll(async () => {
  ui?.close();
  await sb.cli(S, ["kill", S]);
  await sb.cleanup();
});

test("^C reaches the foreground job", async () => {
  ui.write("sleep 30\r");
  await Bun.sleep(300);
  ui.write("\x03");
  ui.write("echo after-int\r");
  await ui.until("interrupted", (s) => s.includes("after-int"));
}, 10000);

test("prefix keys: split, focus, zoom, tabs", async () => {
  ui.write("\x02v");
  await ui.until("two panes", (s) => borders(s) === 2);
  ui.write("\x02h");
  await Bun.sleep(200);
  ui.write("echo left-pane\r");
  await ui.until("typed into left pane", (s) => s.split("\n").some((l) => l.includes("left-pane") && l.indexOf("left-pane") < 80));
  ui.write("\x02z");
  await ui.until("zoomed", (s) => borders(s) === 1 && s.includes("[Z]"));
  ui.write("\x02z\x02c");
  await ui.until("second tab", (s) => s.includes(" 2:"));
  ui.write("\x02p");
  await ui.until("back on tab 1", (s) => borders(s) === 2);
}, 15000);

test.skipIf(!Bun.which("zsh"))("zsh panes own their tty, and an agent typed into a plain pane is detected", async () => {
  // started from inside another agent's session: its markers must not reach the panes
  // an empty ZDOTDIR: with no .zshrc, zsh on some systems (Ubuntu) opens its new-user setup menu instead
  await Bun.write(`${sb.root}/zdot/.zshrc`, "");
  const server = await startServer(sb, "zsh", { SHELL: Bun.which("zsh")!, ZDOTDIR: `${sb.root}/zdot`, CLAUDECODE: "1", CLAUDE_CODE_CHILD_SESSION: "1", CLAUDE_CODE_USE_BEDROCK: "keep" });
  const z = (...a: string[]) => sb.cli("zsh", a);
  // the shell has a controlling terminal (bash takes one itself; zsh only gets one via __pty-exec)
  const probe = await z("pane", "split", "--name", "probe", "ps -o tty= -p $$");
  expect(await z("wait", probe, "--exited", "--timeout", "10")).toBe("exited 0");
  expect(await z("pane", "read", probe)).toMatch(/tty|pts/);
  const envProbe = await z("pane", "split", "--name", "envprobe", "echo marker=${CLAUDE_CODE_CHILD_SESSION:-none} setting=$CLAUDE_CODE_USE_BEDROCK");
  await z("wait", envProbe, "--exited", "--timeout", "10");
  expect(await z("pane", "read", envProbe)).toContain("marker=none setting=keep");
  // launching an agent by hand in an ordinary shell pane
  await z("pane", "run", "p1", "fakeagent");
  const state = await z("wait", "p1", "--state", "working", "--timeout", "10");
  if (state !== "working") console.log("zsh pane diagnostics:\n", await z("pane", "read", "p1"), "\n", await z("debug", "detect", "p1"), "\n", await Bun.$`ps -A -o pid=,ppid=,tpgid=,tty=,args=`.text());
  expect(state).toBe("working");
  expect(await z("agent", "list")).toMatch(/p1\s+fakeagent/);
  await z("kill", "zsh");
  await server.exited;
}, 30000);
