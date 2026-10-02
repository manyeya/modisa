// `modisa notify`: a toast in every attached TUI, titled with who sent it (the calling pane, else "notify"), asking for
// a system notification or a sound only as flags the client weighs against its own config; at most 3 every 10s from one
// sender and 6 from all of them together; with nobody attached it says so and still succeeds.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { Screen, sandbox, startServer } from "../support/harness";
import { connectUnix } from "../../src/protocol/transport";
import { PLUGIN_UI } from "../../src/protocol/types";

const sb = sandbox("notify");
const S = "notify";
let ui: Screen | undefined;
let lastToast = 0; // when the last toast went: the limits count the 10s before
const notify = async (args: string[], pane?: string) => {
  const r = await sb.run(S, ["notify", ...args], pane ? { MODISA_PANE_ID: pane } : {});
  if (r.code === 0) lastToast = Date.now();
  return r;
};

beforeAll(async () => {
  // no system notifications from this suite's TUI: they'd reach the real desktop
  await Bun.write(`${sb.root}/config/config.toml`, '[notify]\nblocked = ["toast"]\ndone = ["toast"]\nworking = []\n');
  await startServer(sb, S);
}, 30000);

afterAll(async () => {
  ui?.close();
  await sb.run(S, ["kill", S]);
  await sb.cleanup();
});

test("with no client attached it says so on stderr and exits 0", async () => {
  expect(await notify(["hello"])).toMatchObject({ code: 0, stdout: "", stderr: "no client attached" });
  const r = await notify(["hello", "again", "--json"]);
  expect(r.code).toBe(0);
  expect(JSON.parse(r.stdout)).toEqual({ clients: 0 });
  expect((await notify([])).code).toBe(2); // a title is needed
  expect((await notify(["x", "--tone", "loud"])).code).toBe(2);
}, 20000);

test("an attached client gets it as a plugin toast, titled with the pane that sent it, with what it asked for", async () => {
  const sock = `${sb.root}/state/${S}.sock`;
  const [older, current] = await Promise.all([connectUnix(sock), connectUnix(sock)]);
  const seen: Record<"older" | "current", any[]> = { older: [], current: [] };
  older.onMessage = (m) => seen.older.push(m);
  current.onMessage = (m) => seen.current.push(m);
  await older.request("attach", {});
  await current.request("attach", { ui: PLUGIN_UI });
  await sb.run(S, ["pane", "rename", "p1", "worker"]);
  const r = await notify(["build", "--body", "all\ngreen", "--tone", "done", "--system", "--sound", "--json"], "p1");
  expect(JSON.parse(r.stdout)).toEqual({ clients: 1 }); // the older client can't draw one
  await Bun.sleep(300);
  expect(seen.current.find((m) => m.method === "plugin.toast")?.params).toEqual({ plugin: "@worker", text: "build: all green", tone: "done", system: true, sound: true });
  expect(seen.older.some((m) => m.method === "plugin.toast")).toBe(false);
  older.close();
  current.close();
}, 20000);

test("the TUI shows it, titled with its sender", async () => {
  ui = new Screen(["-s", S], sb.env, sb.root);
  await ui.until("attached", (s) => s.includes("AGENTS"), 20000);
  expect((await notify(["deploy finished", "--tone", "accent"])).code).toBe(0);
  await ui.until("the toast, titled notify", (s) => /notify[^\n]*\n[^\n]*deploy finished/.test(s));
}, 30000);

test("at most 3 every 10s from one sender, and 6 from all senders together", async () => {
  await Bun.sleep(Math.max(0, lastToast + 10_500 - Date.now())); // what's gone before is out of the window
  const ids: string[] = [];
  for (const name of ["a", "b", "c"]) ids.push((await sb.cli(S, ["pane", "split", "--name", name, "sleep 120"])).trim());
  const [a, b, c] = ids as [string, string, string];
  for (let i = 0; i < 3; i++) expect((await notify([`a${i}`], a)).code).toBe(0);
  const limited = await notify(["a3", "--json"], a);
  expect(limited.code).toBe(1);
  expect(JSON.parse(limited.stderr).error).toMatchObject({ code: "rate_limited", message: expect.stringContaining("@a") });
  for (let i = 0; i < 3; i++) expect((await notify([`b${i}`], b)).code).toBe(0);
  // six in the window: anyone else waits too, though they've sent none
  for (const [who, pane] of [["c", c], ["the user", undefined]] as const) {
    const r = await notify(["one more", "--json"], pane);
    expect(r.code, who).toBe(1);
    expect(JSON.parse(r.stderr).error.message).toContain("at most 6");
  }
}, 40000);
