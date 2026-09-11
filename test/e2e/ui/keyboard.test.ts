// Keyboard-driven UI: the command palette, renaming tabs and panes, copy mode and search, live config.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { Screen, sandbox, borders } from "../../support/harness";

const sb = sandbox("keyboard");
const S = "keys";
const cli = (...args: string[]) => sb.cli(S, args);
let ui: Screen;

beforeAll(async () => {
  await Bun.$`mkdir -p ${sb.root}`.quiet();
  ui = new Screen(["-s", S], sb.env, sb.root);
  await ui.until("first pane", (s) => s.includes("SPACES") && borders(s) === 1);
}, 20000);

afterAll(async () => {
  ui?.close();
  await cli("kill", S);
  await sb.cleanup();
});

test("command palette opens, filters and runs", async () => {
  ui.write("\x02:");
  await ui.until("palette", (s) => s.includes("commands") && s.includes("Split right"));
  ui.write("new tab");
  await ui.until("filtered", (s) => s.includes("commands: new tab"));
  ui.write("\r");
  await ui.until("new tab opened", (s) => s.includes(" 2:"));
}, 15000);

test("rename tab and pane from the keyboard", async () => {
  ui.write("\x02,");
  await ui.until("rename prompt", (s) => s.includes("rename tab"));
  ui.write("work\r");
  await ui.until("tab renamed", (s) => s.includes("2:work"));
  ui.write("\x02.");
  await ui.until("pane rename prompt", (s) => s.includes("rename pane"));
  ui.write("scratch\r");
  expect(await cli("wait", "@scratch", "--match", ".", "--timeout", "5")).toBeTruthy();
}, 15000);

test("copy mode and search", async () => {
  await cli("pane", "run", "@scratch", "for i in $(seq 1 200); do echo line-$i; done; echo needle-here");
  await Bun.sleep(500);
  ui.write("\x02[");
  await Bun.sleep(200);
  ui.write("g");
  await ui.until("scrolled to top", (s) => !s.includes("line-199") && /line-1\b/.test(s));
  ui.write("q");
  await ui.until("back to normal", (s) => s.includes("line-199"));
  ui.write("\x02/");
  await ui.until("search prompt", (s) => s.includes("search"));
  ui.write("line-5\r");
  await ui.until("match found", (s) => /match \d+\/\d+/.test(s));
  ui.write("q");
}, 20000);

test("config changes apply live", async () => {
  await Bun.write(`${sb.root}/config/config.toml`, `theme = "gruvbox"\n`);
  await ui.until("reload toast", (s) => s.includes("config reloaded"), 6000);
}, 10000);
