// Creating panes with options: split --ratio is the new pane's share; --env (once per variable) reaches the new pane
// on every creating command, survives a restart, and can't set a MODISA_ variable or a bad name; --cwd is made absolute
// where the command runs (~ and relative paths, modisa new included); tab create takes --cwd and --pane-name, workspace
// create --command; and every creating command's --json says where the pane is, while its plain output stays the id.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { resolve } from "node:path";
import { stat } from "node:fs/promises";
import { Screen, sandbox, startServer, borders } from "../support/harness";
import { connectUnix } from "../../src/protocol/transport";

const sb = sandbox("create-options");
const sessions = new Set<string>();

async function fresh(session: string) {
  sessions.add(session);
  await startServer(sb, session);
  const run = (...args: string[]) => sb.run(session, args);
  const json = (...args: string[]): Promise<any> => sb.json(session, args);
  const panes = (): Promise<any[]> => json("pane", "list");
  const cols = async () => Object.fromEntries((await panes()).map((p) => [p.id, [p.cols, p.rows]]));
  const waitFor = async (target: string, re: string) => expect((await run("wait", target, "--match", re, "--timeout", "10")).code).toBe(0);
  return { run, json, panes, cols, waitFor };
}

beforeAll(async () => {
  await Bun.$`mkdir -p ${sb.root}/sub ${sb.root}/home/proj`.quiet();
});

afterAll(async () => {
  for (const s of sessions) await sb.run(s, ["kill", s]);
  await sb.cleanup();
});

test("split --ratio is the new pane's share of the room", async () => {
  const { run, cols } = await fresh("ratio");
  await run("pane", "split", "--ratio", "0.25"); // p2, right of the focused p1 on the 120×38 area
  expect(await cols()).toEqual({ p1: [88, 36], p2: [28, 36] });
  await run("pane", "split", "--target", "p1", "--down", "--ratio", "0.7"); // p3, below p1
  expect(await cols()).toEqual({ p1: [88, 9], p2: [28, 36], p3: [88, 25] });
  for (const ratio of ["0.05", "0.95", "half"]) expect((await run("pane", "split", "--ratio", ratio)).code).toBe(2);
  expect(Object.keys(await cols())).toHaveLength(3);
}, 30000);

test("--env reaches the new pane on every creating command; MODISA_ variables and bad names are refused", async () => {
  const { run, json, panes, waitFor } = await fresh("env");
  // the last of a repeated name wins, and the pane's own id can't be changed
  const split = await json("pane", "split", "--env", "GREETING=hello there", "--env", "N=1", "--env", "N=2=two", "echo split:$GREETING/$N/$MODISA_PANE_ID; sleep 600");
  await waitFor(split.id, `split:hello there/2=two/${split.id}$`);
  const tab = await json("tab", "create", "--env", "WHERE=tab", "--command", "echo in-$WHERE; sleep 600");
  await waitFor(tab.id, "in-tab");
  const space = await json("workspace", "create", "--env", "WHERE=space", "--command", "echo in-$WHERE; sleep 600");
  await waitFor(space.id, "in-space");
  const agent = await json("agent", "spawn", "echo in-$WHERE; sleep 600", "--env", "WHERE=agent");
  await waitFor(agent.id, "in-agent");

  const before = (await panes()).length;
  for (const cmd of [["pane", "split"], ["tab", "create"], ["workspace", "create"], ["agent", "spawn", "true"]]) {
    const r = await run(...cmd, "--env", "MODISA_PANE_ID=p9", "--json");
    expect(r.code).toBe(2);
    expect(JSON.parse(r.stderr).error).toEqual({ code: "invalid_params", message: "modisa: invalid params: env.MODISA_PANE_ID is modisa's: MODISA_ variables can't be set" });
  }
  expect(await run("pane", "split", "--env", "1UP=x")).toMatchObject({ code: 2, out: "modisa: invalid params: env.1UP isn't a variable name (letters, digits and _)" });
  expect(await run("pane", "split", "--env", "BARE")).toMatchObject({ code: 2, out: 'modisa: --env takes NAME=value, not "BARE"' });
  expect(await run("pane", "split", "--env", "=x")).toMatchObject({ code: 2 });
  expect((await panes()).length).toBe(before); // nothing was made
}, 40000);

test("a pane's --env is saved, in a database only you can read, and a restart keeps it", async () => {
  const { run, panes, waitFor } = await fresh("keep");
  await run("pane", "split", "--name", "kept", "--env", "KEEP=kept value"); // a shell
  expect((await run("restart")).out).toBe("restarted keep");
  for (let i = 0; i < 100 && (await panes()).length < 2; i++) await Bun.sleep(100); // it restores after it listens
  await run("pane", "run", "kept", 'echo "keep=$KEEP"');
  await waitFor("kept", "^keep=kept value$");
  expect((await stat(`${sb.root}/state/modisa.db`)).mode & 0o777).toBe(0o600);
}, 40000);

test("--cwd is made absolute where modisa runs: relative paths and ~", async () => {
  const { json } = await fresh("cwd");
  const home = { HOME: `${sb.root}/home` };
  expect((await json("pane", "split", "--cwd", "sub")).cwd).toBe(resolve(sb.root, "sub"));
  expect((await json("pane", "split", "--cwd", "./sub/..")).cwd).toBe(resolve(sb.root));
  expect((await sb.json("cwd", ["tab", "create", "--cwd", "~/proj"], { env: home })).cwd).toBe(resolve(sb.root, "home/proj"));
  expect((await sb.json("cwd", ["workspace", "create", "--cwd", "~"], { env: home })).cwd).toBe(resolve(sb.root, "home"));
  expect((await sb.json("cwd", ["workspace", "list"])).at(-1).cwd).toBe(resolve(sb.root, "home")); // the space's too
}, 30000);

test("modisa new --cwd takes a relative path from where it runs", async () => {
  sessions.add("rel");
  const ui = new Screen(["new", "rel", "--cwd", "sub"], sb.env, sb.root);
  try {
    await ui.until("first pane", (s) => borders(s) === 1);
    const [p1] = await sb.json("rel", ["pane", "list"], { retry: "startup" });
    expect(p1.cwd).toBe(resolve(sb.root, "sub"));
  } finally {
    ui.close();
  }
}, 30000);

test("tab create takes --cwd and --pane-name, and workspace create --command", async () => {
  const { json, waitFor } = await fresh("flags");
  const tab = await json("tab", "create", "named", "--cwd", "sub", "--pane-name", "tp", "--command", "pwd; sleep 600");
  expect(tab).toMatchObject({ name: "tp", title: "tp", cwd: resolve(sb.root, "sub"), command: "pwd; sleep 600" });
  await waitFor("@tp", "/sub$");
  const space = await json("workspace", "create", "made", "--cwd", "sub", "--command", "echo made-$((6*7)); sleep 600");
  await waitFor(space.id, "made-42");
  const conn = await connectUnix(`${sb.root}/state/flags.sock`);
  const snap = await conn.request<any>("session.info", { snapshot: true });
  conn.close();
  const tabOf = (id: string) => snap.workspaces.flatMap((w: any) => w.tabs).find((t: any) => t.id === id);
  expect(tabOf(tab.tabId)).toMatchObject({ name: "named", tree: { pane: tab.id } });
  expect(snap.workspaces.find((w: any) => w.id === space.workspaceId)).toMatchObject({ name: "made", cwd: resolve(sb.root, "sub"), tabs: [{ id: space.tabId, tree: { pane: space.id } }] });
}, 30000);

test("creating commands print the new pane's id, and with --json the whole pane and where it is", async () => {
  const { run, json, panes } = await fresh("ids");
  for (const cmd of [["pane", "split"], ["agent", "spawn", "true"], ["tab", "create"], ["workspace", "create"]]) {
    expect((await run(...cmd)).out).toMatch(/^p\d+$/);
    const made = await json(...cmd);
    const listed = (await panes()).find((p) => p.id === made.id);
    expect(made).toMatchObject({ id: listed.id, instance: listed.instance, workspaceId: listed.workspaceId, tabId: listed.tabId });
    expect(made.workspaceId).toMatch(/^w\d+$/);
    expect(made.tabId).toMatch(/^t\d+$/);
  }
}, 40000);
