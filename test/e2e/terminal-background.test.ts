// The terminal's own default background follows the theme (OSC 11), so the window padding a terminal draws around its
// cells matches the TUI; a theme change repaints it; and detaching gives the terminal its own colour back (OSC 111).
// Read from the client's raw output: the harness's Screen hands it to a terminal emulator, which consumes OSC.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { MAIN, sandbox, startServer } from "../support/harness";

const sb = sandbox("terminal-background");
const S = "termbg";

beforeAll(async () => {
  await startServer(sb, S);
}, 30000);

afterAll(async () => {
  await sb.run(S, ["kill", S]);
  await sb.cleanup();
});

test("the terminal's default background is the theme's while attached, follows a theme change, and is reset on detach", async () => {
  let raw = "";
  const pty = new Bun.Terminal({ cols: 120, rows: 35, data: (_t, bytes) => (raw += Buffer.from(bytes).toString("latin1")) });
  const client = Bun.spawn(["bun", MAIN, "-s", S], { terminal: pty, env: { ...sb.env, TERM: "xterm-256color", COLORTERM: "truecolor" }, cwd: sb.root });
  const until = async (what: string, ok: () => boolean, ms = 15000) => {
    for (const end = Date.now() + ms; Date.now() < end; await Bun.sleep(50)) if (ok()) return;
    throw new Error(`timed out waiting for ${what}; OSC 11/111 seen: ${JSON.stringify(raw.match(/\x1b\]11{1,2}[^\x07]*\x07/g))}`);
  };
  try {
    const ion = "\x1b]11;rgb:09/0f/1b\x07"; // ion, the default theme: bg #090f1b
    await until("the ion background", () => raw.includes(ion));

    await Bun.write(`${sb.root}/config/config.toml`, `theme = "nord"\n`); // bg #2e3440, applied live
    const nord = "\x1b]11;rgb:2e/34/40\x07";
    await until("the nord background", () => raw.includes(nord));

    pty.write("\x02d"); // detach
    await Promise.race([client.exited, Bun.sleep(5000)]);
    const reset = raw.lastIndexOf("\x1b]111\x07");
    expect(reset).toBeGreaterThan(raw.lastIndexOf(nord)); // the terminal's own colour, back after the last set
  } finally {
    client.kill();
    pty.close();
  }
}, 40000);
