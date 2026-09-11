// The chrome around the panes: the sidebar toggle, and themes on the settings page (live preview, saved to config).
import { test, expect, beforeAll, afterAll } from "bun:test";
import { Screen, sandbox } from "../../support/harness";
import { click } from "../../support/mouse";

const sb = sandbox("chrome");
const S = "chrome";
let ui: Screen;

beforeAll(async () => {
  await Bun.write(`${sb.root}/config/config.toml`, '# keep my config\ntheme = "ion"\n[sidebar]\nvisible = true\n');
  ui = new Screen(["-s", S], sb.env, sb.root);
  await ui.until("dashboard", (s) => s.includes("SPACES") && s.includes("+ agent") && s.includes("sidebar"));
  await Bun.sleep(300); // let the renderer finish capability negotiation and the first hit grid
}, 20000);

afterAll(async () => {
  await sb.cli(S, ["kill", S]);
  ui?.close();
  await sb.cleanup();
});

test("the sidebar hides and returns from its button in the status row", async () => {
  const bottom = () => ui.lines().length - 1;
  click(ui, 0, ui.lines().at(-1)!.indexOf("sidebar"), bottom());
  await ui.until("sidebar hidden", (s) => !s.includes("SPACES") && s.includes("sidebar"));
  click(ui, 0, ui.lines().at(-1)!.indexOf("sidebar"), bottom());
  await ui.until("sidebar shown", (s) => s.includes("SPACES"));
}, 15000);

test("theme previews cancel, save, preserve config and survive reattach", async () => {
  ui.write("\x02t");
  await ui.until("settings on themes", (s) => s.includes("integrations") && s.includes("dracula"));
  ui.write("\x1b[B");
  await ui.until("theme preview", (s) => s.split("\n").at(-1)!.includes("tokyonight"));
  ui.write("\x1b");
  await ui.until("theme preview cancelled", (s) => !s.includes("integrations") && s.split("\n").at(-1)!.includes("ion"));
  ui.write("\x02t\x1b[B\r");
  await ui.until("theme saved", (s) => s.includes("Theme saved: tokyonight"));
  const config = await Bun.file(`${sb.root}/config/config.toml`).text();
  expect(config).toContain("# keep my config");
  expect((Bun.TOML.parse(config) as any).theme).toBe("tokyonight");
  ui.write("\x1b"); // ↵ applies and keeps the page open; esc keeps what was applied
  await ui.until("settings closed on the saved theme", (s) => !s.includes("integrations") && s.split("\n").at(-1)!.includes("tokyonight"));
  ui.write("\x02d");
  await ui.proc.exited;
  ui.pty.close();
  ui = new Screen(["-s", S], sb.env, sb.root);
  await ui.until("persisted theme", (s) => s.includes("tokyonight") && s.includes("SPACES"));
}, 15000);
