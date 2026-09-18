// Agents' logos: the font and each terminal's settings in (in a HOME of its own), the TUI drawing them, and everything
// out again exactly, never to come back unasked.
import { test, expect, afterAll } from "bun:test";
import { Screen, sandbox } from "../support/harness";
import { logo } from "../../src/config/agents/brands";

const sb = sandbox("logos");
const home = `${sb.root}/home`;
const mac = (await Bun.$`uname -s`.text()).trim() === "Darwin";
const fontAt = mac ? `${home}/Library/Fonts/ModisaMarks.ttf` : `${home}/.local/share/fonts/ModisaMarks.ttf`;
const vscodeAt = mac ? `${home}/Library/Application Support/Code/User/settings.json` : `${home}/.config/Code/User/settings.json`;
const env = { HOME: home, MODISA_LOGOS: "" }; // its own HOME: this is the one test that installs them
const cli = (...args: string[]) => sb.run("logos", args, env);
const vscode = `{\n    // mine\n    "files.autoSave": "afterDelay",\n}\n`;

afterAll(async () => {
  await sb.run("logos", ["kill", "logos"], env);
  await sb.cleanup();
});

test("install puts the font in and tells each terminal; the TUI draws the logos; uninstall takes exactly that out", async () => {
  await Bun.write(vscodeAt, vscode);
  await Bun.$`mkdir -p ${home}/.config/kitty`.quiet(); // kitty's there
  const installed = await cli("logos", "install");
  expect(installed.code).toBe(0);
  expect(installed.out).toContain("kitty");
  expect(installed.out).toContain("VS Code");
  expect((await Bun.file(fontAt).bytes()).length).toBeGreaterThan(1000);
  expect(await Bun.file(`${home}/.config/kitty/kitty.conf`).text()).toContain("symbol_map U+F5A00-U+F5AFF Modisa Marks");
  expect(await Bun.file(vscodeAt).text()).toContain(`"terminal.integrated.fontFamily": "`);
  expect(await Bun.file(vscodeAt).text()).toContain("// mine");
  expect((await cli("logos", "status")).out).toMatch(/kitty\s+set up/);

  // told to (this test's terminal is no terminal modisa knows), the sidebar draws an agent's logo
  await Bun.write(`${sb.root}/config/config.toml`, `[sidebar]\nlogos = "on"\n`);
  const ui = new Screen(["-s", "logos"], { ...sb.env, ...env }, sb.root);
  await ui.until("dashboard", (s) => s.includes("SPACES"), 20000);
  const id = (await cli("pane", "split", "--name", "review")).out.trim();
  await cli("pane", "run", id, "sleep 600");
  for (let i = 0; i < 10; i++) {
    await cli("report", id, "--source", "logos-test", "--agent", "codex", "--state", "working");
    if ((await cli("wait", id, "--state", "working", "--timeout", "1")).code === 0) break;
  }
  await ui.until("codex's logo", (s) => s.includes(`${logo("codex")}  @review`), 15000);
  ui.close();

  expect((await cli("logos", "uninstall")).code).toBe(0);
  expect(await Bun.file(fontAt).exists()).toBe(false);
  expect(await Bun.file(vscodeAt).text()).toBe(vscode); // byte for byte
  expect(await Bun.file(`${home}/.config/kitty/kitty.conf`).text()).not.toContain("Modisa Marks");
}, 90000);

test("the TUI installs them the first time, but never again once they've been taken out", async () => {
  await Bun.write(`${sb.root}/config/config.toml`, "");
  const ui = new Screen(["-s", "logos"], { ...sb.env, ...env }, sb.root);
  await ui.until("dashboard", (s) => s.includes("SPACES"), 20000);
  await Bun.sleep(2000);
  ui.close();
  expect(await Bun.file(fontAt).exists()).toBe(false); // uninstalled above: stays out
  await Bun.$`rm -f ${sb.root}/state/logos.json`.quiet(); // a first start
  const fresh = new Screen(["-s", "logos"], { ...sb.env, ...env }, sb.root);
  await fresh.until("installed, and said so", (s) => s.includes("agent logos installed"), 20000);
  fresh.close();
  expect(await Bun.file(fontAt).exists()).toBe(true);
}, 60000);
