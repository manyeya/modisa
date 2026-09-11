// The settings page (Ctrl+B s): sections for theme, indicators, sound, toasts, pane labels and
// integrations; every change applies at once and is saved to config.toml with comments kept.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { Screen, sandbox } from "../../support/harness";
import { installFakeAgent } from "../../support/fake-agent";
import { click, hover } from "../../support/mouse";

const sb = sandbox("settings");
const S = "settings";
const home = `${sb.root}/home`;
// its own HOME for integrations, and no real claude/codex on PATH
const env = { ...sb.env, HOME: home, PATH: `${sb.root}/bin:${Bun.which("bun")!.replace(/\/bun$/, "")}:/usr/bin:/bin` };
const cli = (...args: string[]) => sb.cli(S, args, { HOME: home });
const config = async () => Bun.TOML.parse(await Bun.file(`${sb.root}/config/config.toml`).text()) as any;
const SECTIONS = ["theme", "indicators", "sound", "toasts", "pane labels", "integrations"];
let ui: Screen;

async function openSection(name: string) {
  ui.write("\x02s");
  await ui.until("settings page", (s) => SECTIONS.every((x) => s.includes(x)));
  ui.write("\t".repeat(SECTIONS.indexOf(name)));
}
const selectedRow = () => ui.lines().find((l) => l.includes(" ▸ ")) ?? "";
const close = async () => {
  ui.write("\x1b");
  await ui.until("settings closed", (s) => !s.includes("integrations"));
};

beforeAll(async () => {
  await installFakeAgent(sb.root);
  await Bun.$`mkdir -p ${home}`.quiet();
  await Bun.write(`${sb.root}/config/config.toml`, '# my settings\ntheme = "ion"\n\n[notify]   # keep\nblocked = ["toast", "system", "sound"]\ndone = ["toast"]\n');
  ui = new Screen(["-s", S], env, sb.root);
  await ui.until("dashboard", (s) => s.includes("SPACES"));
  await Bun.sleep(300);
}, 20000);

afterAll(async () => {
  await cli("kill", S);
  ui?.close();
  await sb.cleanup();
});

test("sound: ←→ picks what 'needs you' plays and saves it; off takes sound out of [notify]", async () => {
  await openSection("sound");
  await ui.until("sound rows", (s) => s.includes("needs you") && s.includes("volume") && selectedRow().includes("chime"));
  ui.write("\x1b[C"); // →
  await ui.until("next sound", () => selectedRow().includes("sparkle"));
  await Bun.sleep(200);
  expect((await config()).sound.blocked).toBe("sparkle");
  ui.write("\x1b[D\x1b[D"); // ← ← : chime, then off
  await ui.until("off", () => selectedRow().includes("off"));
  await Bun.sleep(200);
  const saved = await config();
  expect(saved.notify.blocked).toEqual(["toast", "system"]);
  expect(await Bun.file(`${sb.root}/config/config.toml`).text()).toStartWith("# my settings\n");
  expect(await Bun.file(`${sb.root}/config/config.toml`).text()).toContain("[notify]   # keep");
  await close();
}, 20000);

test("indicators: a glyph style shows everywhere; the tab badge can be turned off", async () => {
  await cli("pane", "split", "--name", "asker", "fakeagent --ask");
  expect(await cli("wait", "@asker", "--state", "blocked", "--timeout", "10")).toBe("blocked");
  await ui.until("tab badge", (s) => s.split("\n")[0]!.includes(" !"));
  await openSection("indicators");
  await ui.until("styles", () => selectedRow().includes("symbols"));
  ui.write("\x1b[B\r"); // ↓ dots, ↵
  await ui.until("dots applied", (s) => s.includes("● @asker") && s.split("\n")[0]!.includes("●"));
  ui.write("\x1b[B\x1b[B"); // letters, then (past the heading) tab bar badge
  await ui.until("tab badge row", () => selectedRow().includes("tab bar badge"));
  ui.write("\r");
  await ui.until("badge off", (s) => selectedRow().includes("[ ]") && !s.split("\n")[0]!.includes("●"));
  expect((await config()).indicators).toMatchObject({ style: "dots", tab: false });
  await close();
  await cli("pane", "close", "@asker");
}, 30000);

test("toasts: each alert kind toggles per event", async () => {
  await openSection("toasts");
  await ui.until("toasts rows", (s) => s.includes("when an agent needs you") && selectedRow().includes("toast"));
  ui.write("\x1b[B"); // system notification
  await ui.until("system row", () => selectedRow().includes("system notification"));
  ui.write(" ");
  await ui.until("system off", () => selectedRow().includes("[ ] system notification"));
  await Bun.sleep(200);
  expect((await config()).notify.blocked).toEqual(["toast"]);
  await close();
}, 20000);

test("integrations: every agent is listed; the recommended ones install from the page", async () => {
  await Bun.$`mkdir -p ${`${home}/.claude`}`; // Claude Code is set up here; no other agent is
  await openSection("integrations");
  await ui.until("every agent listed", (s) => ["Claude Code", "Codex", "OpenCode", "Pi", "Droid"].every((n) => s.includes(n)) && s.includes("not found"));
  await ui.until("recommended first", () => selectedRow().includes("Install all (1)"));
  ui.write("\r");
  await ui.until("installed", (s) => /Claude Code\s+✓ installed/.test(s) && !s.includes("Install all"), 10000);
  expect(await Bun.file(`${home}/.claude/settings.json`).text()).toContain("hook claude-code session");
  expect(await cli("integration", "status")).toMatch(/Claude Code\s+✓ installed/);
  await close();
}, 20000);

test("the mouse: tabs switch sections, the pointer moves the selection", async () => {
  await openSection("theme");
  await ui.until("theme rows", () => selectedRow().includes("ion"));
  const tabs = ui.lines().findIndex((l) => l.includes("indicators") && l.includes("integrations"));
  click(ui, 0, ui.lines()[tabs]!.indexOf("pane labels") + 1, tabs);
  await ui.until("pane labels section", (s) => s.includes("agent and state in the border title"));
  const y = ui.lines().findIndex((l) => l.includes("id and status on the bottom border"));
  hover(ui, ui.lines()[y]!.indexOf("id and status"), y);
  await ui.until("hover selects", () => selectedRow().includes("id and status"));
  click(ui, 0, ui.lines()[y]!.indexOf("id and status"), y);
  await ui.until("toggled by click", () => selectedRow().includes("[ ] id and status"));
  expect((await config()).pane_labels.status).toBe(false);
  await close();
  expect(ui.text()).not.toContain("p1 / running");
}, 20000);
