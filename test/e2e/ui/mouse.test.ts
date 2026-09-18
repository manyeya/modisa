// The mouse: right-click menus, small terminals, and resizing panes by dragging their borders — in every
// encoding terminals use, with lost releases, and in short terminals like VS Code's panel.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { Screen, sandbox, borders } from "../../support/harness";
import { click, resizeTerminal, sgr, x10 } from "../../support/mouse";

const sb = sandbox("mouse");
const S = "mouse";
const cli = (...args: string[]) => sb.cli(S, args);
let ui: Screen;

// cols × rows of every pane in the current space, oldest first
async function sizes(): Promise<Record<string, [number, number]>> {
  const space = JSON.parse(await cli("workspace", "list", "--json")).find((w: any) => w.active).name;
  return Object.fromEntries(JSON.parse(await cli("pane", "list", "--json")).filter((p: any) => p.workspace === space).map((p: any) => [p.id, [p.cols, p.rows]]));
}

// a fresh tab split into `split` (v = side by side, - = stacked)
async function freshTab(split: "v" | "-") {
  ui.write("\x02c");
  await ui.until("new tab", (s) => borders(s) === 1);
  ui.write(`\x02${split}`);
  await ui.until("split", (s) => borders(s) === 2);
  await Bun.sleep(400);
}

beforeAll(async () => {
  await Bun.$`mkdir -p ${sb.root}`.quiet();
  ui = new Screen(["-s", S], sb.env, sb.root);
  await ui.until("dashboard", (s) => s.includes("SPACES") && s.includes("+ agent"));
  await Bun.sleep(300); // let the renderer finish capability negotiation and the first hit grid
}, 20000);

afterAll(async () => {
  await cli("kill", S);
  ui?.close();
  await sb.cleanup();
});

// Skip the sidebar's new divider and the outer edge of the first pane.
function verticalDivider(y: number) {
  const paneLeft = ui.lines().find(line => line.includes("┌"))!.indexOf("┌");
  return ui.lines()[y]!.indexOf("││", paneLeft + 1);
}

test("right-click opens actions; keyboard activates split; outside click dismisses", async () => {
  click(ui, 2, 80, 15);
  await ui.until("context menu", (s) => s.includes("Copy visible output") && s.includes("Change theme"));
  ui.write("\x1b[B\r");
  await ui.until("menu split", (s) => borders(s) === 2 && !s.includes("Copy visible output"));
  expect(JSON.parse(await cli("pane", "list", "--json"))).toHaveLength(2);
  click(ui, 2, 138, 36);
  await ui.until("menu clamped at edge", (s) => s.includes("Close pane") && s.includes("esc dismiss"));
  click(ui, 0, 2, 1);
  await ui.until("outside dismissal", (s) => !s.includes("Copy visible output"));
  ui.write("echo menu-input-ok\r");
  await ui.until("shell focus restored", (s) => s.includes("menu-input-ok"));
}, 15000);

test("small terminals focus one pane, resize its PTY, and restore splits", async () => {
  resizeTerminal(ui, 44, 14);
  await ui.until("compact focus", (s) => !s.includes("SPACES") && borders(s) === 1);
  const panes = JSON.parse(await cli("pane", "list", "--json"));
  expect(panes.some((p: any) => p.cols === 42 && p.rows === 10)).toBe(true);
  ui.write("\x02e");
  await ui.until("compact context menu", (s) => s.includes("Focus pane"));
  resizeTerminal(ui, 30, 9);
  await ui.until("resized menu", (s) => s.includes("esc dismiss"));
  ui.write("\x1b");
  resizeTerminal(ui, 140, 40);
  await ui.until("restored split tree", (s) => s.includes("SPACES") && borders(s) === 2);
}, 15000);

test("drag the border between panes to resize them, side by side and stacked, even with a fast drag", async () => {
  await freshTab("v");
  const [left, right] = Object.keys(await sizes()).slice(-2); // this tab's panes are the newest
  const y = 10;
  const divider = verticalDivider(y);
  const before = await sizes();
  // slow drag to the left
  ui.write(sgr(0, divider, y));
  for (let x = divider - 1; x >= divider - 16; x--) { ui.write(sgr(32, x, y)); await Bun.sleep(15); }
  ui.write(sgr(0, divider - 16, y, true));
  await Bun.sleep(700);
  const after = await sizes();
  expect(after[left!]![0]).toBeLessThan(before[left!]![0] - 10);
  expect(after[right!]![0]).toBeGreaterThan(before[right!]![0] + 10);
  // fast drag back: press, moves and release in one burst
  const d2 = verticalDivider(y);
  ui.write(sgr(0, d2, y) + sgr(32, d2 + 4, y) + sgr(32, d2 + 12, y) + sgr(0, d2 + 12, y, true));
  await Bun.sleep(700);
  expect((await sizes())[left!]![0]).toBeGreaterThan(after[left!]![0] + 8);
  // stacked: split the right pane down and drag the horizontal border up
  ui.write("\x02-");
  await ui.until("stacked", (s) => borders(s) === 3);
  await Bun.sleep(300);
  const panesNow = await sizes();
  const bottom = Object.keys(panesNow).at(-1)!; // the pane the split just made
  const x = 130; // inside the right column
  const hy = ui.lines().findIndex((l, i) => i > 1 && l[x] === "─"); // bottom border of the upper pane
  ui.write(sgr(0, x, hy));
  for (let yy = hy - 1; yy >= hy - 6; yy--) { ui.write(sgr(32, x, yy)); await Bun.sleep(15); }
  ui.write(sgr(0, x, hy - 6, true));
  await Bun.sleep(700);
  expect((await sizes())[bottom]![1]).toBeGreaterThan(panesNow[bottom]![1] + 4);
  // a release we never hear about (let go outside the window, say) must not leave the resize stuck:
  // the next move with no button held ends it, and later moves change nothing
  const hy2 = ui.lines().findIndex((l, i) => i > 1 && l[x] === "─");
  ui.write(sgr(0, x, hy2));
  for (let yy = hy2 + 1; yy <= hy2 + 3; yy++) { ui.write(sgr(32, x, yy)); await Bun.sleep(15); }
  ui.write(sgr(35, x, hy2 + 3)); // plain move: the button is up
  await Bun.sleep(500);
  const settled = await sizes();
  for (let yy = hy2 + 3; yy >= hy2 - 8; yy--) { ui.write(sgr(35, x, yy)); await Bun.sleep(15); }
  await Bun.sleep(500);
  expect(await sizes()).toEqual(settled);
  // and the screen takes typing again
  ui.write("echo after-resize\r");
  await ui.until("typing works after the resize", (s) => s.includes("after-resize"));
}, 30000);

test("resizing works whichever way the terminal encodes mouse motion", async () => {
  await freshTab("v");
  const y = 10;
  const divider = () => verticalDivider(y);
  // X10 (legacy) encoding: every motion is "move"; held = 32, nothing held = 35
  let d = divider();
  let before = await sizes();
  ui.write(x10(0, d, y));
  for (let x = d + 1; x <= d + 10; x++) { ui.write(x10(32, x, y)); await Bun.sleep(15); }
  ui.write(x10(3, d + 10, y));
  await Bun.sleep(600);
  let after = await sizes();
  expect(after).not.toEqual(before);
  for (let x = d + 10; x >= d - 10; x--) { ui.write(x10(35, x, y)); await Bun.sleep(10); } // released: moving changes nothing
  await Bun.sleep(400);
  expect(await sizes()).toEqual(after);
  // VS Code's real held-drag reports can use code 35; keep resizing until release.
  d = divider();
  before = await sizes();
  ui.write(sgr(0, d, y));
  for (let x = d - 1; x >= d - 10; x--) { ui.write(sgr(35, x, y)); await Bun.sleep(15); }
  await Bun.sleep(300);
  expect(await sizes()).not.toEqual(before); // movement must work while still held
  ui.write(sgr(0, d - 10, y, true));
  await Bun.sleep(600);
  after = await sizes();
  expect(after).not.toEqual(before);
  for (let x = d - 10; x <= d + 5; x++) { ui.write(sgr(35, x, y)); await Bun.sleep(10); }
  await Bun.sleep(400);
  expect(await sizes()).toEqual(after);
}, 30000);

test("pane resize releases on a fresh click, focus loss, and release outside the screen", async () => {
  await freshTab("v");
  const all = async () => JSON.parse(await cli("pane", "list", "--json")).map((p: any) => [p.id, p.cols, p.rows]);
  for (const cancel of ["click", "blur", "outside", "escape"] as const) {
    const d = verticalDivider(10);
    expect(d).toBeGreaterThan(0);
    const before = await all();
    ui.write(sgr(0, d, 10));
    await Bun.sleep(100);
    if (cancel === "click") ui.write(sgr(0, d + 5, 10));
    if (cancel === "blur") ui.write("\x1b[O\x1b[I");
    if (cancel === "outside") ui.write(sgr(0, 180, 50, true));
    if (cancel === "escape") { ui.write("\x1b"); await Bun.sleep(100); }
    // A stray held-motion report must not resurrect the cancelled resize.
    ui.write(sgr(32, d + 12, 10) + sgr(0, d + 12, 10, true));
    await Bun.sleep(300);
    expect(await all()).toEqual(before);
    // Clicks must reach the pane again.
    click(ui, 2, d + 10, 12);
    await ui.until(`context menu after ${cancel}`, (s) => s.includes("Copy visible output"));
    ui.write("\x1b");
    await ui.until("menu dismissed", (s) => !s.includes("Copy visible output"));
  }
}, 30000);

test("in a short terminal (VS Code's panel), dragging a bottom pane's border never collapses the split", async () => {
  ui.write("\x02c");
  await ui.until("new tab", (s) => borders(s) === 1);
  resizeTerminal(ui, 140, 20);
  await Bun.sleep(400);
  ui.write("\x02-");
  await ui.until("stacked", (s) => borders(s) === 2);
  await Bun.sleep(400);
  const x = 100;
  const d = ui.lines().findIndex((l, i) => i > 1 && l[x] === "─");
  ui.write(sgr(0, x, d));
  await Bun.sleep(120);
  for (let y = d + 1; y <= 19; y++) { ui.write(sgr(32, x, y)); await Bun.sleep(30); expect(borders(ui.text())).toBe(2); }
  ui.write(sgr(0, x, 19, true));
  await Bun.sleep(500);
  expect(borders(ui.text())).toBe(2); // still two panes, the border stopped at the smallest usable size
  // and back up the other way
  const d2 = ui.lines().findIndex((l, i) => i > 1 && l[x] === "─");
  ui.write(sgr(0, x, d2));
  await Bun.sleep(120);
  for (let y = d2 - 1; y >= 1; y--) { ui.write(sgr(32, x, y)); await Bun.sleep(30); expect(borders(ui.text())).toBe(2); }
  ui.write(sgr(0, x, 1, true));
  await Bun.sleep(500);
  expect(borders(ui.text())).toBe(2);
  resizeTerminal(ui, 140, 40);
  await ui.until("full size again", (s) => s.includes("SPACES"));
}, 30000);
