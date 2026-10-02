// Pane layout from the CLI: a pane moves, process and all, beside another, into a tab, or to a new tab or space; what it
// leaves empty closes, and the view stays put unless --focus. Two panes swap places in a tab or across tabs, keeping
// the tree's shape; resize, zoom and focus by direction take a target. Popups and plugins' overlays stay where they
// are, moves that go nowhere are refused, and a rearranged layout survives a restart.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { sandbox, startServer } from "../support/harness";
import { connectUnix } from "../../src/protocol/transport";
import { PLUGIN_UI } from "../../src/protocol/types";

const sb = sandbox("pane-layout");
const sessions = new Set<string>();
const sock = (session: string) => `${sb.root}/state/${session}.sock`;

// a session of its own, starting with one pane (p1) on a 120×38 area (no client has set another)
async function fresh(session: string) {
  sessions.add(session);
  await startServer(sb, session);
  const run = (...args: string[]) => sb.run(session, args);
  const json = (...args: string[]): Promise<any> => sb.json(session, args);
  const panes = (): Promise<any[]> => json("pane", "list");
  const tabOf = async (id: string) => (await panes()).find((p) => p.id === id)!.tabId as string;
  // what clients draw (attaching with no area changes nothing): every space's tabs with their trees, focus and zoom
  const view = async () => {
    const conn = await connectUnix(sock(session));
    const v = await conn.request<any>("attach", {});
    conn.close();
    return { ...v, tab: (id: string) => v.workspaces.flatMap((w: any) => w.tabs).find((t: any) => t.id === id) };
  };
  return { run, json, panes, tabOf, view };
}

beforeAll(async () => {
  // a plugin with an overlay and a popup; linked before any session runs, so each one starts it
  const dir = `${sb.root}/layout-plugin`;
  const pane = (id: string, placement: string) => ({ id, title: id, placement, run: ["sleep", "600"] });
  await Bun.write(`${dir}/plugin.json`, JSON.stringify({ name: "layout", protocol: 1, run: ["sleep", "600"], panes: [pane("peek", "overlay"), pane("pop", "popup")] }));
  expect((await sb.run("none", ["plugin", "link", dir])).code).toBe(0);
});

afterAll(async () => {
  for (const s of sessions) await sb.run(s, ["kill", s]);
  await sb.cleanup();
});

test("a pane moves into a tab, beside a pane, and out of a tab or space, which closes", async () => {
  const { run, json, panes, tabOf, view } = await fresh("move");
  await run("pane", "split", "--name", "two"); // p2, beside p1
  const first = await tabOf("p1");
  const { instance } = (await panes()).find((p) => p.id === "p2")!;
  await run("tab", "create", "second"); // p3, in a tab of its own
  const second = await tabOf("p3");

  // into a tab: beside its focused pane, the same process
  const moved = await json("pane", "move", "p2", "--tab", "second");
  expect(moved).toEqual({ pane: "p2", instance, workspaceId: (await panes())[0].workspaceId, tabId: second });
  let v = await view();
  expect(v.tab(second).tree).toEqual({ dir: "row", ratio: 0.5, a: { pane: "p3" }, b: { pane: "p2" } });
  expect(v.tab(first).tree).toEqual({ pane: "p1" });

  // beside a pane, below it with 30% of the space
  expect(await run("pane", "move", "p2", "--target", "p1", "--split", "down", "--ratio", "0.3")).toMatchObject({ code: 0, out: "" });
  v = await view();
  expect(v.tab(first).tree).toEqual({ dir: "col", ratio: 0.7, a: { pane: "p1" }, b: { pane: "p2" } });
  expect(v.tab(second).tree).toEqual({ pane: "p3" });

  // the last pane out of a tab closes it
  await run("pane", "move", "p3", "--target", "p2");
  v = await view();
  expect(v.workspaces[0].tabs.map((t: any) => t.id)).toEqual([first]);
  expect(v.tab(first).tree).toEqual({ dir: "col", ratio: 0.7, a: { pane: "p1" }, b: { dir: "row", ratio: 0.5, a: { pane: "p2" }, b: { pane: "p3" } } });

  // and out of a space, closes that
  await run("workspace", "create", "other"); // p4
  expect(await json("workspace", "list")).toHaveLength(2);
  await run("pane", "move", "p4", "--target", "p3");
  expect((await json("workspace", "list")).map((w: any) => w.name)).not.toContain("other");
  expect((await panes()).map((p) => [p.id, p.tabId])).toEqual([["p1", first], ["p2", first], ["p3", first], ["p4", first]]);
}, 30000);

test("a pane moves to a new tab or space, and the view stays put unless --focus", async () => {
  const { run, json, panes, tabOf, view } = await fresh("new");
  await run("pane", "split", "--name", "two"); // p2; p1 keeps the focus
  const first = await tabOf("p1");
  const focused = async () => (await panes()).find((p) => p.focused)?.id;

  const solo = await json("pane", "move", "p2", "--new-tab", "--name", "solo");
  let v = await view();
  expect(v.workspaces[0].tabs.map((t: any) => [t.id, t.name])).toEqual([[first, undefined], [solo.tabId, "solo"]]);
  expect(v.tab(solo.tabId).tree).toEqual({ pane: "p2" });
  expect(v.workspaces[0].active).toBe(0); // still showing the first tab
  expect(await focused()).toBe("p1");

  // a new space starts where the pane's shell is now; --focus takes the view with it
  await Bun.$`mkdir -p ${sb.root}/elsewhere`.quiet();
  await run("pane", "run", "p2", `cd ${sb.root}/elsewhere && echo there-$((1+1))`);
  expect((await run("wait", "p2", "--match", "there-2", "--timeout", "10")).code).toBe(0);
  const away = await json("pane", "move", "p2", "--new-workspace", "--name", "away", "--focus");
  v = await view();
  expect(v.workspaces.map((w: any) => w.name)).toEqual([v.workspaces[0].name, "away"]);
  expect(v.workspaces[1]).toMatchObject({ id: away.workspaceId, tabs: [{ id: away.tabId, tree: { pane: "p2" } }] });
  expect(v.workspaces[1].cwd).toEndWith("/elsewhere");
  expect(v.workspaces[0].tabs.map((t: any) => t.id)).toEqual([first]); // "solo" closed with nothing in it
  expect(v.active).toBe(1);
  expect(await focused()).toBe("p2");

  // a moved pane that had the focus leaves it to its neighbour; the view stays on that tab
  await run("pane", "split", "--target", "p1"); // p3
  await run("pane", "focus", "p3");
  await run("pane", "move", "p3", "--target", "p2");
  v = await view();
  expect(v.active).toBe(0);
  expect(await focused()).toBe("p1");
}, 30000);

test("popups, overlays, moves beside itself or to where it already is alone, and unclear destinations are refused", async () => {
  const { run, json, view } = await fresh("refuse");
  for (let i = 0; i < 100 && (await json("plugin", "list"))[0]?.status !== "running"; i++) await Bun.sleep(100);

  // a popup has no place in the layout (it opens only in a TUI client)
  const conn = await connectUnix(sock("refuse"));
  await conn.request("attach", { ui: PLUGIN_UI });
  const pop = await conn.request<any>("plugin.pane.open", { plugin: "layout", pane: "pop" });
  for (const args of [["pane", "move", pop.pane, "--new-tab"], ["pane", "move", "p1", "--target", pop.pane], ["pane", "swap", pop.pane, "p1"], ["pane", "zoom", pop.pane], ["pane", "resize", pop.pane, "--direction", "left"]])
    expect(await run(...args)).toMatchObject({ code: 1, out: `modisa: ${pop.pane} is a popup: it has no place in a tab` });
  conn.close();

  // an overlay stays over the pane it opened on
  const peek = await json("plugin", "pane", "layout", "peek");
  for (const args of [["pane", "move", peek.pane, "--new-tab"], ["pane", "swap", "p1", peek.pane]])
    expect(await run(...args)).toMatchObject({ code: 1, out: `modisa: ${peek.pane} is a plugin's overlay: it stays over the pane it opened on` });
  await run("pane", "close", peek.pane);

  expect(await run("pane", "move", "p1", "--target", "p1")).toMatchObject({ code: 1, out: "modisa: can't move p1 beside itself" });
  expect(await run("pane", "move", "p1", "--new-tab")).toMatchObject({ code: 1, out: "modisa: p1 is already alone in its tab" });
  expect(await run("pane", "move", "p1", "--new-workspace")).toMatchObject({ code: 1, out: "modisa: p1 is already alone in its space" });
  expect(await run("pane", "swap", "p1", "p1")).toMatchObject({ code: 1, out: "modisa: can't swap p1 with itself" });
  const other = (await run("tab", "create", "other")).out;
  expect(await run("pane", "move", "p1", "--tab", "other", "--target", "p1")).toMatchObject({ code: 1, out: "modisa: p1 isn't in tab other" });
  // exactly one destination
  expect((await run("pane", "move", "p1")).code).toBe(2);
  expect((await run("pane", "move", "p1", "--new-tab", "--new-workspace")).code).toBe(2);
  expect((await run("pane", "move", "p1", "--tab", "other", "--new-tab")).code).toBe(2);
  expect((await run("pane", "swap")).code).toBe(2);
  // nothing moved
  const v = await view();
  expect(v.workspaces.map((w: any) => w.tabs.map((t: any) => t.tree))).toEqual([[{ pane: "p1" }, { pane: other }]]);
}, 40000);

test("two panes swap places in a tab and across tabs, keeping the tree's shape; each tab's focus follows its pane", async () => {
  const { run, tabOf, view } = await fresh("swap");
  await run("pane", "split"); // p2; p1 keeps the focus
  await run("pane", "resize", "p1", "--direction", "right", "--amount", "10");
  const first = await tabOf("p1");
  const before = (await view()).tab(first).tree;
  expect(before.ratio).not.toBe(0.5);

  expect(await run("pane", "swap", "p1", "p2")).toMatchObject({ code: 0, out: "" });
  let v = await view();
  expect(v.tab(first)).toMatchObject({ tree: { ...before, a: { pane: "p2" }, b: { pane: "p1" } }, focused: "p1" });

  await run("tab", "create"); // p3, its tab on screen
  const second = await tabOf("p3");
  await run("pane", "swap", "p2", "p3");
  v = await view();
  expect(v.tab(first)).toMatchObject({ tree: { ratio: before.ratio, a: { pane: "p3" }, b: { pane: "p1" } }, focused: "p1" });
  expect(v.tab(second)).toMatchObject({ tree: { pane: "p2" }, focused: "p2" }); // p3 had the focus: p2 took its place
  expect(v.workspaces[0].active).toBe(1); // the view didn't move

  // with the neighbour on one side; one positional is the pane to swap with
  await run("pane", "swap", "p3", "--direction", "right");
  expect((await view()).tab(first).tree).toMatchObject({ a: { pane: "p1" }, b: { pane: "p3" } });
  await run("pane", "focus", "p1");
  await run("pane", "swap", "p3"); // the focused pane (p1) with p3
  expect((await view()).tab(first).tree).toMatchObject({ a: { pane: "p3" }, b: { pane: "p1" } });
  const none = await run("pane", "swap", "p3", "--direction", "left", "--json");
  expect(none.code).toBe(1);
  expect(JSON.parse(none.stderr).error).toEqual({ code: "no_such_pane", message: "modisa: no pane left of p3" });
}, 30000);

test("resize moves the border on one side and changes the panes' columns", async () => {
  const { run, json, panes } = await fresh("resize");
  await run("pane", "split"); // p2
  const cols = async () => Object.fromEntries((await panes()).map((p) => [p.id, p.cols]));
  expect(await cols()).toEqual({ p1: 58, p2: 58 });
  expect(await json("pane", "resize", "p1", "--direction", "right", "--amount", "10")).toEqual({ changed: true });
  expect(await cols()).toEqual({ p1: 68, p2: 48 });
  expect(await run("pane", "resize", "p2", "--direction", "left")).toMatchObject({ code: 0, out: "changed" }); // 2 cells by default
  expect(await cols()).toEqual({ p1: 66, p2: 50 });
  expect(await run("pane", "resize", "p1", "--direction", "left")).toMatchObject({ code: 0, out: "unchanged" }); // no border there
  await run("pane", "resize", "p1", "--direction", "right", "--amount", "500");
  expect(await json("pane", "resize", "p1", "--direction", "right")).toEqual({ changed: false }); // as far as it goes
  expect((await cols()).p2).toBe(22); // still usable
  expect((await run("pane", "resize", "p1")).code).toBe(2); // which side?
}, 30000);

test("zoom turns on, off and toggles for a pane, which it focuses in its tab", async () => {
  const { run, json, panes, view } = await fresh("zoom");
  await run("pane", "split"); // p2; p1 keeps the focus
  const zoom = async () => {
    const t = (await view()).workspaces[0].tabs[0];
    return [t.zoomed, t.focused];
  };
  expect(await json("pane", "zoom", "p2")).toEqual({ zoomed: true });
  expect(await zoom()).toEqual([true, "p2"]);
  expect((await panes()).find((p) => p.id === "p2").cols).toBe(118); // the whole tab
  expect(await run("pane", "zoom", "p2")).toMatchObject({ code: 0, out: "unzoomed" });
  expect(await zoom()).toEqual([false, "p2"]);
  for (let i = 0; i < 2; i++) expect((await run("pane", "zoom", "p1", "--on")).out).toBe("zoomed");
  expect(await zoom()).toEqual([true, "p1"]);
  expect((await run("pane", "zoom", "p2", "--toggle")).out).toBe("zoomed"); // another pane: it's zoomed instead
  expect(await zoom()).toEqual([true, "p2"]);
  for (let i = 0; i < 2; i++) expect((await run("pane", "zoom", "p2", "--off")).out).toBe("unzoomed");
  expect(await zoom()).toEqual([false, "p2"]);
  expect((await run("pane", "zoom", "p1", "--on", "--off")).code).toBe(2);
  // a tab that isn't on screen is zoomed where it is
  await run("tab", "create"); // p3, its tab on screen
  await run("pane", "zoom", "p1", "--on");
  const v = await view();
  expect(v.workspaces[0]).toMatchObject({ active: 1, tabs: [{ zoomed: true, focused: "p1" }, { zoomed: false }] });
}, 30000);

test("focus by direction goes to the neighbour on that side, from a pane or the focused one", async () => {
  const { run, panes } = await fresh("focus");
  await run("pane", "split"); // p2, right of p1
  await run("pane", "split", "--target", "p2", "--down"); // p3, below p2
  const focused = async () => (await panes()).find((p) => p.focused)?.id;
  expect(await focused()).toBe("p1");
  await run("pane", "focus", "p3", "--direction", "up");
  expect(await focused()).toBe("p2");
  await run("pane", "focus", "--direction", "down");
  expect(await focused()).toBe("p3");
  await run("pane", "focus", "p3", "--direction", "left");
  expect(await focused()).toBe("p1");
  const none = await run("pane", "focus", "p1", "--direction", "left", "--json");
  expect(none.code).toBe(1);
  expect(JSON.parse(none.stderr).error).toEqual({ code: "no_such_pane", message: "modisa: no pane left of p1" });
  expect(await focused()).toBe("p1");
  expect((await run("pane", "focus", "p1", "--direction", "sideways")).code).toBe(2);
}, 30000);

test("a moved and swapped layout survives a restart", async () => {
  const { run, panes, view } = await fresh("survive");
  await run("pane", "rename", "p1", "one");
  await run("pane", "split", "--name", "two");
  await run("tab", "create", "second");
  await run("pane", "rename", "p3", "three");
  await run("pane", "move", "@two", "--tab", "second", "--split", "down", "--ratio", "0.3");
  await run("pane", "swap", "@one", "@three");
  // ids are new after a restart: compare by name
  const shape = async () => {
    const [v, ps] = [await view(), await panes()];
    const name = (id: string) => ps.find((p) => p.id === id)?.name;
    const named = (n: any): any => ("pane" in n ? name(n.pane) : { dir: n.dir, ratio: n.ratio, a: named(n.a), b: named(n.b) });
    return v.workspaces.map((w: any) => w.tabs.map((t: any) => ({ name: t.name, tree: named(t.tree), focused: name(t.focused) })));
  };
  const before = await shape();
  expect(before).toEqual([[{ name: undefined, tree: "three", focused: "three" }, { name: "second", tree: { dir: "col", ratio: 0.7, a: "one", b: "two" }, focused: "one" }]]);
  expect((await run("restart")).out).toBe("restarted survive");
  for (let i = 0; i < 100 && (await panes()).length < 3; i++) await Bun.sleep(100); // the new server restores after it listens
  expect(await shape()).toEqual(before);
}, 40000);
