// Sessions: attaching starts a server, detach/reattach replays the screen, the layout survives a
// crash and a restart, and closing every pane ends the session.
import { test, expect, afterAll } from "bun:test";
import { Screen, sandbox, startServer, borders } from "../support/harness";

const sb = sandbox("sessions");
const S = "sess";
const cli = (...args: string[]) => sb.cli(S, args);
let ui: Screen;

afterAll(async () => {
  ui?.close();
  await cli("kill", S);
  await sb.cleanup();
});

test("attach starts a server and renders a shell with the sidebar", async () => {
  await Bun.$`mkdir -p ${sb.root}`.quiet();
  ui = new Screen(["-s", S], sb.env, sb.root);
  await ui.until("first pane + sidebar", (s) => s.includes("SPACES") && s.includes("AGENTS") && borders(s) === 1);
  ui.write("echo shepherd-ok\r");
  await ui.until("shell output", (s) => s.includes("shepherd-ok"));
}, 20000);

test("detach keeps the server; reattach replays the screen", async () => {
  await cli("pane", "split", "--name", "keep", "echo kept-output; sleep 600");
  await ui.until("second pane", (s) => s.includes("kept-output"));
  ui.write("\x02d");
  expect(await Promise.race([ui.proc.exited, Bun.sleep(5000).then(() => "timeout")])).toBe(0);
  expect(await cli("ls")).toContain(`${S}\t`);
  ui = new Screen(["-s", S], sb.env, sb.root);
  await ui.until("screen restored on reattach", (s) => s.includes("shepherd-ok") && s.includes("kept-output") && s.includes("@keep"));
}, 20000);

test("server restart restores the layout from sqlite", async () => {
  await cli("tab", "create", "second");
  await Bun.sleep(1500); // debounced save
  const pid = (await Bun.$`pgrep -f ${"server -s " + S}`.nothrow().text()).trim().split("\n")[0];
  await Bun.$`kill -9 ${pid}`.nothrow();
  await Promise.race([ui.proc.exited, Bun.sleep(5000)]);
  expect(await cli("ls")).toContain("attach to restore");
  ui = new Screen(["-s", S], sb.env, sb.root);
  await ui.until("restored panes", (s) => s.includes(" 2:") && s.includes("1:"), 15000);
  expect(await cli("pane", "list")).toContain("@keep");
}, 30000);

test("restart loads new code, keeps the layout, and the attached client follows", async () => {
  const before = JSON.parse(await cli("pane", "list", "--json")).length;
  expect(await cli("restart")).toBe(`restarted ${S}`);
  await ui.until("client reconnected to the new server", (s) => s.includes("server restarted"), 15000);
  expect(JSON.parse(await cli("pane", "list", "--json"))).toHaveLength(before);
  ui.write("echo after-restart\r");
  await ui.until("typing reaches the new server", (s) => s.includes("after-restart"));
}, 30000);

test("a restart restores every space, tab and pane name", async () => {
  const c = (...a: string[]) => sb.cli("restore", a);
  let srv = await startServer(sb, "restore");
  await c("pane", "rename", "p1", "maria");
  await c("pane", "split", "--target", "p1", "--name", "angel");
  await c("tab", "create", "second");
  await c("workspace", "create", "space2");
  await c("pane", "split", "--name", "color");
  await c("workspace", "create", "space3");
  const shape = async () => JSON.parse(await c("pane", "list", "--json")).map((p: any) => `${p.workspace}:${p.name ?? p.title}`).sort();
  const spaces = async () => (await c("workspace", "list")).split("\n").slice(1).map((l) => l.split(/\s+/).slice(1, 3).join(" "));
  const before = await shape();
  const spacesBefore = await spaces();
  await Bun.sleep(1500); // saves are debounced by a second
  srv.kill(9); // no chance to save on the way out
  await srv.exited;
  srv = await startServer(sb, "restore");
  expect(await shape()).toEqual(before);
  expect(await spaces()).toEqual(spacesBefore);
  await c("kill", "restore");
  await srv.exited;
}, 30000);

test("closing every pane ends the session", async () => {
  await cli("kill", S);
  expect(await Promise.race([ui.proc.exited, Bun.sleep(5000).then(() => "timeout")])).toBe(0);
  expect(await cli("ls")).toBe("no sessions");
}, 15000);
