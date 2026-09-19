// The settings page (Ctrl+B s): sections down the side, a search over all of them; every change applies at once
// and is saved to config.toml with comments kept.
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
const SECTIONS = ["theme", "general", "layout", "git", "indicators", "sound", "alerts", "agents", "integrations"];
let ui: Screen;

async function openSection(name: string) {
  ui.write("\x02s");
  await ui.until("settings page", (s) => SECTIONS.every((x) => s.includes(x[0]!.toUpperCase() + x.slice(1))));
  ui.write("\t".repeat(SECTIONS.indexOf(name)));
}
// the selected row: the one whose first cell after the side list's │ (the page's own border is the first) has a
// background no other row has, its highlight
const selectedRow = () => {
  const rows = ui.lines().flatMap((l, y) => {
    const m = /^[^│]*│[^│]*│ /.exec(l);
    return m ? [{ l, bg: JSON.stringify(ui.vt.cellAt({ x: Bun.stringWidth(m[0]), y })?.style?.bg ?? null) }] : [];
  });
  return rows.find((r) => r.bg !== "null" && rows.filter((o) => o.bg === r.bg).length === 1)?.l ?? "";
};
const close = async () => {
  ui.write("\x1b");
  await ui.until("settings closed", (s) => !s.includes("Integrations"));
};

beforeAll(async () => {
  await installFakeAgent(sb.root);
  await Bun.$`mkdir -p ${home}`.quiet();
  await Bun.write(`${sb.root}/config/config.toml`, '# my settings\ntheme = "ion"\n\n[notify]   # keep\nblocked = ["toast", "system", "sound"]\ndone = ["toast"]\n');
  ui = new Screen(["-s", S], env, sb.root);
  await ui.until("dashboard", (s) => s.includes("AGENTS"));
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
  await ui.until("dots applied", (s) => /@asker\s+●/.test(s) && s.split("\n")[0]!.includes("●"));
  ui.write("\x1b[B\x1b[B"); // letters, then (past the heading) tab bar badge
  await ui.until("tab badge row", () => selectedRow().includes("tab bar badge"));
  ui.write("\r");
  await ui.until("badge off", (s) => selectedRow().includes("○ off") && !s.split("\n")[0]!.includes("●"));
  expect((await config()).indicators).toMatchObject({ style: "dots", tab: false });
  await close();
  await cli("pane", "close", "@asker");
}, 30000);

test("alerts: each alert kind toggles per event", async () => {
  await openSection("alerts");
  await ui.until("toasts rows", (s) => s.includes("WHEN AN AGENT NEEDS YOU") && selectedRow().includes("toast"));
  ui.write("\x1b[B"); // system notification
  await ui.until("system row", () => selectedRow().includes("system notification"));
  ui.write(" ");
  await ui.until("system off", () => selectedRow().includes("system notification") && selectedRow().includes("○ off"));
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

test("the mouse: the side list switches sections, the pointer moves the selection, a click toggles", async () => {
  await openSection("theme");
  await ui.until("theme rows", () => selectedRow().includes("ion"));
  const y0 = ui.lines().findIndex((l) => l.includes(" Indicators"));
  click(ui, 0, ui.lines()[y0]!.indexOf("Indicators"), y0);
  await ui.until("indicators section", (s) => s.includes("pane border title"));
  const y = ui.lines().findIndex((l) => /│.*\bsidebar\b.*● on/.test(l));
  const x = ui.lines()[y]!.lastIndexOf("sidebar");
  hover(ui, x, y);
  await ui.until("hover selects", () => selectedRow().includes("sidebar"));
  click(ui, 0, x, y);
  await ui.until("toggled by click", () => selectedRow().includes("sidebar") && selectedRow().includes("○ off"));
  expect((await config()).indicators.sidebar).toBe(false);
  await close();
}, 20000);

test("search: typing finds settings in every section, under where each lives", async () => {
  await openSection("theme");
  ui.write("changed");
  await ui.until("git's changed files found", (s) => s.includes("GIT · IN THE STATUS ROW") && selectedRow().includes("changed files"));
  ui.write("\r"); // space would be part of the search
  await ui.until("toggled from the results", () => selectedRow().includes("○ off"));
  expect((await config()).git.changes).toBe(false);
  ui.write("\x15zzzz"); // ^u clears; nothing matches this
  await ui.until("no match", (s) => s.includes("No settings match “zzzz”"));
  await close();
}, 20000);

test("general: select on hover can be turned off; clicks still work", async () => {
  await openSection("general");
  ui.write("hover");
  await ui.until("select on hover", () => selectedRow().includes("select on hover") && selectedRow().includes("● on"));
  ui.write("\r");
  await ui.until("off", () => selectedRow().includes("○ off"));
  expect((await config()).mouse.hover).toBe(false);
  // the pointer resting on another section's row no longer selects it; a click on the side list still switches
  const y0 = ui.lines().findIndex((l) => l.includes(" Layout"));
  click(ui, 0, ui.lines()[y0]!.indexOf("Layout"), y0);
  await ui.until("layout section", () => selectedRow().includes("show the sidebar"));
  const y = ui.lines().findIndex((l) => l.includes("pane count"));
  hover(ui, ui.lines()[y]!.indexOf("pane count"), y);
  await Bun.sleep(400);
  expect(selectedRow()).toContain("show the sidebar");
  click(ui, 0, ui.lines()[y]!.indexOf("pane count"), y);
  await ui.until("a click selects and toggles", () => selectedRow().includes("pane count") && selectedRow().includes("○ off"));
  expect((await config()).status.panes).toBe(false);
  await close();
}, 20000);
