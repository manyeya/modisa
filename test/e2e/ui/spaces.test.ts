// Spaces: named groups of tabs and panes — create, rename in place, delete, from the keyboard, mouse and CLI.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { Screen, sandbox, borders } from "../../support/harness";
import { click } from "../../support/mouse";

const sb = sandbox("spaces");
const S = "spaces";
const cli = (...args: string[]) => sb.cli(S, args);
let ui: Screen;

beforeAll(async () => {
  await Bun.$`mkdir -p ${sb.root}`.quiet();
  ui = new Screen(["-s", S], sb.env, sb.root);
  await ui.until("dashboard", (s) => s.includes("AGENTS") && borders(s) === 1);
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
  await ui.until("space name prompt", (s) => s.includes("New space"));
  ui.write("\x01\x0bresearch\r");
  await ui.until("new named space", (s) => s.includes("research") && borders(s) === 1);
  const after = JSON.parse(await cli("pane", "list", "--json"));
  const added = after.find((p: any) => !before.some((old: any) => old.id === p.id));
  expect(added.cwd).toBe(before[0].cwd);
  expect(await cli("workspace", "list")).toContain("research");
  expect(await Bun.file(`${sb.root}/research`).exists()).toBe(false);
}, 10000);

test("spaces from the picker: right-click one to rename or delete it; the last space stays", async () => {
  const row = (name: string) => ui.lines().findIndex((l) => l.includes(name) && l.includes("tab")); // a picker row: its name, then its tabs
  const open = async () => {
    ui.write("\x02w");
    await ui.until("space picker", (s) => s.includes("Spaces") && row("research") > 0);
  };
  // rename from the right-click menu
  await open();
  click(ui, 2, ui.lines()[row("research")]!.indexOf("research"), row("research"));
  await ui.until("space menu", (s) => s.includes("Space · research") && s.includes("Delete space"));
  ui.write("r");
  await ui.until("rename prompt", (s) => s.includes("Rename space") && s.includes("Cancel"));
  ui.write("\x01\x0blab\r");
  await ui.until("renamed", (s) => s.includes("◈ lab")); // the active space, named in the tab bar
  expect(await cli("workspace", "list")).toContain("lab");
  // delete asks first: n keeps it, y deletes it and its panes
  const panes = JSON.parse(await cli("pane", "list", "--json")).length;
  const menu = async () => {
    ui.write("\x02w");
    await ui.until("space picker", (s) => s.includes("Spaces") && row("lab") > 0);
    click(ui, 2, ui.lines()[row("lab")]!.indexOf("lab"), row("lab"));
    await ui.until("space menu", (s) => s.includes("Space · lab"));
    ui.write("d");
    await ui.until("confirmation", (s) => s.includes('Delete space "lab"?') && s.includes("closes its 1 pane"));
  };
  await menu();
  ui.write("n");
  await ui.until("kept", (s) => !s.includes("Delete space"));
  await menu();
  ui.write("y");
  await ui.until("space deleted", (s) => !s.includes("Delete space") && !s.includes("◈ lab"));
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

test("each space in the picker shows ✎ and ✕: a click or ^r renames, ^d deletes; the last space has no ✕", async () => {
  const row = (name: string) => ui.lines().findIndex((l) => l.includes(name) && l.includes("tab"));
  const open = async (name: string) => {
    ui.write("\x02w");
    await ui.until("space picker", (s) => s.includes("Spaces") && row(name) > 0);
  };
  const listed = async (name: string, is: boolean) => {
    for (let i = 0; i < 100 && (await cli("workspace", "list")).includes(name) !== is; i++) await Bun.sleep(50);
    expect(await cli("workspace", "list")).toEqual(is ? expect.stringContaining(name) : expect.not.stringContaining(name));
  };
  await cli("workspace", "create", "alpha");
  await open("alpha");
  expect(ui.lines()[row("alpha")]).toMatch(/✎\s+✕/);
  click(ui, 0, ui.lines()[row("alpha")]!.indexOf("✎"), row("alpha"));
  await ui.until("rename prompt", (s) => s.includes("Rename space") && s.includes("Cancel"));
  ui.write("\x01\x0bbeta\r");
  await listed("beta", true);

  await open("beta");
  // select beta, its name bold: ↓ until it is (the pointer, left on a row by the click above, may have selected it)
  const bold = () => !!ui.vt.cellAt({ x: ui.lines()[row("beta")]!.indexOf("beta"), y: row("beta") })?.style?.bold;
  for (let i = 0; i < 3 && !bold(); i++) (ui.write("\x1b[B"), await Bun.sleep(200));
  await ui.until("beta selected, its keys in the footer", (s) => bold() && s.includes("^r rename") && s.includes("^d delete"));
  ui.write("\x04"); // ^d
  await ui.until("confirmation", (s) => s.includes('Delete space "beta"?'));
  ui.write("y");
  await listed("beta", false);

  const only = (await cli("workspace", "list", "--json").then((s) => JSON.parse(s)))[0].name;
  await open(only);
  expect(ui.lines()[row(only)]).toContain("✎");
  expect(ui.lines()[row(only)]).not.toContain("✕");
  ui.write("\x1b");
}, 30000);
