// Spaces: named groups of tabs and panes — create, rename in place, delete, from the keyboard, mouse and CLI.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { Screen, sandbox, borders } from "../../support/harness";
import { click, hover } from "../../support/mouse";

const sb = sandbox("spaces");
const S = "spaces";
const cli = (...args: string[]) => sb.cli(S, args);
let ui: Screen;

beforeAll(async () => {
  await Bun.$`mkdir -p ${sb.root}`.quiet();
  ui = new Screen(["-s", S], sb.env, sb.root);
  await ui.until("dashboard", (s) => s.includes("SPACES") && borders(s) === 1);
  await Bun.sleep(300);
}, 20000);

afterAll(async () => {
  await cli("kill", S);
  ui?.close();
  await sb.cleanup();
});

test("new spaces are named tab groups in the same directory", async () => {
  const before = JSON.parse(await cli("pane", "list", "--json"));
  ui.write("\x02W");
  await ui.until("space name prompt", (s) => s.includes("new space — name"));
  ui.write("\x01\x0bresearch\r");
  await ui.until("new named space", (s) => s.includes("research") && borders(s) === 1);
  const after = JSON.parse(await cli("pane", "list", "--json"));
  const added = after.find((p: any) => !before.some((old: any) => old.id === p.id));
  expect(added.cwd).toBe(before[0].cwd);
  expect(await cli("workspace", "list")).toContain("research");
  expect(await Bun.file(`${sb.root}/research`).exists()).toBe(false);
}, 10000);

test("spaces with the mouse: double-click or ✎ renames in place, ✕ deletes, right-click menu; the last space stays", async () => {
  expect(ui.lines().find((l) => l.includes("SPACES"))).not.toContain("hide");
  const row = (name: string) => ui.lines().findIndex((l, i) => i > 0 && l.slice(0, 26).includes(name)); // sidebar rows, not the tab bar
  const line = (name: string) => ui.lines()[row(name)]!.slice(0, 26);
  const editing = (name: string) => row(name) > 0 && line(name).includes("↵"); // the input, with its Enter hint
  // ✎ and ✕ only show on the row under the pointer
  expect(line("research")).not.toContain("✕");
  const col = async (glyph: string, name: string) => {
    hover(ui, 4, row(name));
    await ui.until(`${glyph} on hover`, () => line(name).includes(glyph));
    return line(name).indexOf(glyph);
  };
  // double-click the name: edit in place; Esc cancels
  click(ui, 0, 4, row("research"));
  click(ui, 0, 4, row("research"));
  await ui.until("inline editor", () => editing("research"));
  ui.write("\x1b");
  await ui.until("edit cancelled", () => row("research") > 0 && !editing("research"));
  // ✎ edits in place; Enter saves
  click(ui, 0, await col("✎", "research"), row("research"));
  await ui.until("inline editor via ✎", () => editing("research"));
  ui.write("\x01\x0blab\r");
  await ui.until("renamed", () => row("lab") > 0 && row("research") < 0 && !editing("lab"));
  expect(await cli("workspace", "list")).toContain("lab");
  // the right-click menu still works
  click(ui, 2, 4, row("lab"));
  await ui.until("space menu", (s) => s.includes("SPACE / lab") && s.includes("Delete space"));
  ui.write("\x1b");
  await ui.until("menu closed", (s) => !s.includes("SPACE / lab"));
  // ✕ asks first: n keeps it, y deletes it and its panes
  const panes = JSON.parse(await cli("pane", "list", "--json")).length;
  click(ui, 0, await col("✕", "lab"), row("lab"));
  await ui.until("confirmation", (s) => s.includes('Delete space "lab"?') && s.includes("closes its 1 pane"));
  ui.write("n");
  await ui.until("kept", (s) => !s.includes("Delete space") && row("lab") > 0);
  click(ui, 0, await col("✕", "lab"), row("lab"));
  await ui.until("confirmation again", (s) => s.includes('Delete space "lab"?'));
  ui.write("y");
  await ui.until("space deleted", () => row("lab") < 0);
  expect(JSON.parse(await cli("pane", "list", "--json"))).toHaveLength(panes - 1);
  // the CLI does the same; the only remaining space can't be deleted
  const spaces = () => cli("workspace", "list", "--json").then((s) => JSON.parse(s));
  await cli("workspace", "create", "tmp");
  expect(await cli("workspace", "rename", "tmp", "scratch")).toBe("");
  expect((await spaces()).map((w: any) => w.name)).toContain("scratch");
  expect(await cli("workspace", "close", "scratch")).toBe("");
  expect(await spaces()).toHaveLength(1);
  expect(await cli("workspace", "close", (await spaces())[0].name)).toContain("can't delete the only space");
  ui.write("\x02&");
  await ui.until("refused in the UI too", (s) => s.includes("can't delete the only space"));
}, 30000);
