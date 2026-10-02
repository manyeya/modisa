// pane attach: one pane full-screen in another terminal (here a PTY), which is its emulator. A takeover holds the pane at
// that terminal's size and takes only its typing, follows its resizes, and gives everything back on detach (the prefix
// then d), leaving the terminal as it found it. Observing types nothing and resizes nothing. One takeover at a time; the
// pane's exit status comes through; and it works over ssh like attach does.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { z } from "zod";
import { MAIN, Screen, sandbox, startServer } from "../support/harness";
import { connectUnix } from "../../src/protocol/transport";
import { b64 } from "../../src/protocol/conn";
import { attachNotifications, results } from "../../src/protocol/schema";

const sb = sandbox("pane-attach");
const S = "att";
const sock = () => `${sb.root}/state/${S}.sock`;
const cli = (...args: string[]) => sb.cli(S, args);
const info = async (id: string) => (await sb.json<any[]>(S, ["pane", "list"])).find((p) => p.id === id);
const read = async (id: string) => (await sb.json(S, ["pane", "read", id, "--lines", "200"])).content as string;
const split = async (...args: string[]) => (await cli("pane", "split", ...args)).trim();
const bytes = (s: string) => b64(new TextEncoder().encode(s));
const screens: Screen[] = [];
// `modisa pane attach <args…>` in a terminal of its own
const attach = (args: string[], cols = 100, rows = 30, env = sb.env) => {
  const s = new Screen(["-s", S, "pane", "attach", ...args], env, sb.root, cols, rows);
  screens.push(s);
  return s;
};
const exited = (s: Screen) => Promise.race([s.proc.exited, Bun.sleep(10000).then(() => "still running")]);
// A PTY can deliver what the CLI printed just after the process is reported gone (Linux does), so wait for it.
const said = (s: Screen, text: string) => s.until(text, (t) => t.includes(text));
const until = async (what: string, ok: () => boolean | Promise<boolean>, ms = 10000) => {
  for (const end = Date.now() + ms; Date.now() < end; await Bun.sleep(100)) if (await ok()) return;
  throw new Error(`timed out waiting for ${what}`);
};
const sized = (id: string, cols: number, rows: number, takeover?: true) => until(`${id} at ${cols}×${rows}`, async () => {
  const p = await info(id);
  return p.cols === cols && p.rows === rows && p.takeover === takeover;
});
const matches = (schema: z.ZodType, value: unknown, what: string) => {
  const r = schema.safeParse(value);
  expect(r.success, `${what} doesn't match its schema: ${r.error?.message}\n${JSON.stringify(value)}`).toBe(true);
};

beforeAll(async () => {
  await startServer(sb, S);
}, 20000);

afterAll(async () => {
  for (const s of screens) s.close();
  await cli("kill", S);
  await sb.cleanup();
});

test("a takeover gets this terminal's size and typing, and no one else's; detaching gives both back", async () => {
  const id = await split("--name", "driven");
  const box = await info(id); // the size its place in the tab gives it
  const t = attach([id], 100, 30);
  await sized(id, 100, 30, true);
  t.write("echo typed-$((40+2))\r");
  await t.until("what was typed, run", (s) => s.includes("typed-42"));

  // another connection's typing is dropped; a pane not taken over still gets it, and pane run still reaches this one
  const other = await connectUnix(sock());
  other.notify("input", { pane: id, data: bytes("echo dropped-$((1+1))\r") });
  other.notify("input", { pane: "p1", data: bytes("echo reached-$((1+1))\r") });
  await until("p1 to take the other connection's typing", async () => (await read("p1")).includes("reached-2"));
  await cli("pane", "run", id, "echo via-run-$((2+2))");
  await t.until("pane run's output", (s) => s.includes("via-run-4"));
  expect(await read(id)).not.toContain("dropped");
  other.close();

  // resizing this terminal resizes the pane
  t.resize(90, 25);
  await sized(id, 90, 25, true);

  // what the pane's program turns on is turned off again on the way out, and this terminal's own screen is back
  t.write("printf '\\033[?1000h\\033[?1006h\\033[?2004h\\033[?1004h\\033[?25l'; echo modes-$((0+1))\r");
  await t.until("the modes on", (s) => s.includes("modes-1"));
  expect([t.vt.mode("alt_screen_save"), t.vt.mode("normal_mouse"), t.vt.mode("bracketed_paste"), t.vt.mode("cursor_visible")]).toEqual([true, true, true, false]);
  t.write("\x02d");
  expect(await exited(t)).toBe(0);
  await said(t, `[detached from ${id}]`);
  expect(t.text()).not.toContain("typed-42"); // that was the alternate screen
  for (const mode of ["alt_screen_save", "normal_mouse", "sgr_mouse", "bracketed_paste", "focus_event"] as const) expect(t.vt.mode(mode), mode).toBe(false);
  expect(t.vt.mode("cursor_visible")).toBe(true);
  await sized(id, box.cols, box.rows); // back in its box, and no longer taken over
}, 60000);

test("a program switching screens is redrawn on this terminal's alternate screen, never on its own", async () => {
  const id = await split("--name", "screens");
  const t = attach([id], 80, 24);
  await sized(id, 80, 24, true);
  t.write("echo shell-$((1+1)); printf '\\033[?1049halt-%s' screen; sleep 1; printf '\\033[3J\\033[?1049l'; echo back-$((2+2))\r");
  await t.until("the pane's alternate screen", (s) => s.includes("alt-screen") && !s.includes("shell-2"));
  await t.until("the pane's own screen again", (s) => s.includes("shell-2") && s.includes("back-4") && !s.includes("alt-screen"));
  expect(t.vt.mode("alt_screen_save")).toBe(true); // this terminal never left its alternate screen
  t.write("\x02d");
  expect(await exited(t)).toBe(0);
  await said(t, `[detached from ${id}]`);
  expect(t.text()).not.toContain("shell-2"); // its own screen is as it was
}, 30000);

test("the TUI marks a pane taken over while it is, and the prefix twice types it once", async () => {
  const id = (await cli("tab", "create", "literal", "--pane-name", "literal", "--command", "cat -v")).trim(); // on screen
  const ui = new Screen(["-s", S], sb.env, sb.root);
  screens.push(ui);
  await ui.until("the TUI", (s) => s.includes("@literal"), 15000);
  const t = attach([id], 80, 24);
  await ui.until("the border's note", (s) => s.includes("@literal [attached elsewhere]"));
  t.write("a\x02\x02b\r");
  await t.until("one ^B through", (s) => s.includes("a^Bb"));
  expect(await read(id)).not.toContain("^B^B");
  t.write("\x02d");
  expect(await exited(t)).toBe(0);
  await ui.until("the note gone", (s) => !s.includes("[attached elsewhere]"));
  ui.write("\x02d");
  expect(await exited(ui)).toBe(0);
}, 40000);

test("observing types nothing and resizes nothing; q, or Ctrl-C, stops it", async () => {
  const id = await split("--name", "watched");
  const box = await info(id);
  const o = attach(["--observe", id], 140, 40);
  await cli("pane", "run", id, "echo seen-$((1+1))");
  await o.until("the pane's output", (s) => s.includes("seen-2"));
  o.write("echo nope\r");
  await Bun.sleep(500);
  expect(await read(id)).not.toContain("nope");
  expect(await info(id)).toMatchObject({ cols: box.cols, rows: box.rows });
  expect((await info(id)).takeover).toBeUndefined();
  o.write("q");
  expect(await exited(o)).toBe(0);
  await said(o, `[stopped watching ${id}]`);

  // in a terminal smaller than the pane, it says what's cut off
  const small = attach(["--observe", id], 50, 10);
  await small.until("the warning", (s) => s.includes("bigger than this terminal"));
  small.write("\x03");
  expect(await exited(small)).toBe(0);
}, 30000);

test("one takeover at a time; your own pane, two at once and no terminal are refused", async () => {
  const id = await split("--name", "busy");
  const first = attach([id], 80, 24);
  await sized(id, 80, 24, true);
  const second = attach([id], 80, 24);
  expect(await exited(second)).toBe(1);
  await said(second, "already taken over");

  const conn = await connectUnix(sock());
  await expect(conn.request("pane.attach", { target: id, cols: 80, rows: 24 })).rejects.toMatchObject({ code: "ui_busy" });
  // watching it is fine, but only once per connection, and only the one that took it over sizes it
  matches(results["pane.attach"], await conn.request("pane.attach", { target: id, mode: "observe", cols: 80, rows: 24 }), "pane.attach");
  await expect(conn.request("pane.attach", { target: "p1", mode: "observe", cols: 80, rows: 24 })).rejects.toMatchObject({ code: "usage" });
  await expect(conn.request("pane.attach.resize", { cols: 70, rows: 20 })).rejects.toMatchObject({ code: "usage" });
  conn.close();
  const own = await connectUnix(sock());
  await expect(own.request("pane.attach", { caller: id, mode: "observe", cols: 80, rows: 24 })).rejects.toMatchObject({ code: "usage" });
  own.close();
  expect((await sb.run(S, ["pane", "attach", id])).code).toBe(2); // its input and output aren't a terminal

  // once the pane's program asks for kitty keyboard reports, keys come that way: Ctrl+B, its release, then d
  first.write("\x1b[98;5u\x1b[98;5:3u\x1b[100u");
  expect(await exited(first)).toBe(0);
}, 30000);

test("a signal or the pane closing ends it too, with the terminal and the pane put back", async () => {
  const id = await split("--name", "signalled");
  const box = await info(id);
  const t = attach([id], 80, 24);
  await sized(id, 80, 24, true);
  process.kill(t.proc.pid, "SIGTERM");
  expect(await exited(t)).toBe(143);
  expect(t.vt.mode("alt_screen_save")).toBe(false);
  await sized(id, box.cols, box.rows);

  const c = attach([id], 80, 24);
  await sized(id, 80, 24, true);
  await cli("pane", "close", id);
  expect(await exited(c)).toBe(0);
  await said(c, `[${id} closed]`);
}, 30000);

test("the pane's exit status comes through when its process exits, and the connection is told why", async () => {
  // a command pane stays once its process exits
  const job = await split("--name", "job", "read x; exit $x");
  const j = attach([job], 80, 24);
  await sized(job, 80, 24, true);
  j.write("5\r");
  expect(await exited(j)).toBe(5);
  await said(j, `[${job} exited 5]`);
  expect((await info(job)).takeover).toBeUndefined();

  // a shell pane closes itself when it exits: still its code
  const shell = await split("--name", "shell");
  const s = attach([shell], 80, 24);
  await sized(shell, 80, 24, true);
  s.write("exit 4\r");
  expect(await exited(s)).toBe(4);

  // on the wire: the owner's typing gets through, output follows, then attach.end
  const other = await split("--name", "wire", "read x; echo got-$x; exit 6");
  const conn = await connectUnix(sock());
  const seen: any[] = [];
  conn.onMessage = (m) => seen.push(m);
  const r = await conn.request("pane.attach", { target: other, cols: 70, rows: 20 });
  matches(results["pane.attach"], r, "pane.attach");
  expect(r).toMatchObject({ pane: other, mode: "takeover", cols: 70, rows: 20 });
  matches(results["pane.attach.resize"], await conn.request("pane.attach.resize", { cols: 72, rows: 21 }), "pane.attach.resize");
  expect(await info(other)).toMatchObject({ cols: 72, rows: 21, takeover: true });
  conn.notify("input", { pane: other, data: bytes("ok\r") });
  await until("attach.end", () => seen.some((m) => m.method === "attach.end"));
  for (const m of seen) matches((attachNotifications as Record<string, z.ZodType>)[m.method]!, m.params, m.method);
  expect(seen.filter((m) => m.method === "output").map((m) => atob(m.params.data)).join("")).toContain("got-ok");
  expect(seen.at(-1).params).toEqual({ pane: other, reason: "exited", exitCode: 6 });
  conn.close();
}, 60000);

test("--remote attaches through ssh running `modisa proxy` on the far side", async () => {
  // fake ssh: drop "-T" and the host, run the rest locally like a remote shell would
  await Bun.write(`${sb.root}/bin/fakessh`, `#!/bin/sh\nshift; shift; eval "$@"\n`);
  await Bun.$`chmod +x ${sb.root}/bin/fakessh`;
  await Bun.write(`${sb.root}/config/config.toml`, `remote_command = "bun ${MAIN}"\n`);
  const id = await split("--name", "far");
  const box = await info(id);
  const r = attach([id, "--remote", "ssh://devbox"], 96, 28, { ...sb.env, MODISA_SSH: `${sb.root}/bin/fakessh` });
  await sized(id, 96, 28, true);
  r.write("echo over-ssh-$((1+2))\r");
  await r.until("the remote pane's output", (s) => s.includes("over-ssh-3"));
  r.write("\x02d");
  expect(await exited(r)).toBe(0);
  await sized(id, box.cols, box.rows);
}, 60000);
