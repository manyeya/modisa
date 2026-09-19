// A remote client (ssh running `modisa proxy` on the far side) gets plugins' UI like a local one, bound and drawn by
// its own config: its theme, and its own [plugin_keys]. The plugin, its processes and its files stay with the server.
// When the connection drops, what plugins showed is hidden until the attach snapshot brings it back, without replaying
// a toast shown before and without moving focus.
// ponytail: the "ssh" is a script on this machine, with separate state and config roots for the server, the remote
// client and a second local client. It checks each side keeps to its own config and files; it can't prove real ssh
// or another platform.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { MAIN, Screen, sandbox, startServer } from "../support/harness";
import { connectUnix } from "../../src/protocol/transport";

const sb = sandbox("remote-plugin-ui"); // sb.env is the server's side: its state, config and plugin data
const S = "rui";
const REPO = `${import.meta.dir}/../..`;
const dir = `${sb.root}/rui-demo`;
const CLIENT = `${sb.root}/remote-client`; // the remote client's own state and config
const LOCAL = `${sb.root}/local-client`; // a second client, attached directly, with its own config
const MARKS = `${sb.root}/state/plugins/rui-demo/marks.log`; // in the server's data root
const run = (...args: string[]) => sb.run(S, args);
let remote: Screen;
let local: Screen | undefined;

// have the plugin make these ui.* calls from its bound connection; each gives "ok" or the error code
const apply = async (...calls: [string, object][]) => (await sb.json<string[]>(S, ["plugin", "run", "rui-demo", "apply", JSON.stringify({ calls })]));
const marks = async () => (await Bun.file(MARKS).text().catch(() => "")).split("\n").filter(Boolean).map((l) => JSON.parse(l) as { target: { pane: string; instance: string } | null });
const focused = async () => (await sb.json<any[]>(S, ["pane", "list"], { retry: "startup" })).find((p) => p.focused);
const untilMarks = async (n: number) => {
  for (let i = 0; i < 100 && (await marks()).length < n; i++) await Bun.sleep(100);
  expect((await marks()).length).toBe(n);
};

beforeAll(async () => {
  await Bun.write(
    `${dir}/plugin.json`,
    JSON.stringify({
      name: "rui-demo", protocol: 1, run: ["bun", "plugin.ts"],
      actions: [{ id: "hello", title: "Say hello" }, { id: "apply", title: "Apply UI calls" }, { id: "mark", title: "Mark the pane" }],
      keys: [{ key: "G", action: "mark", description: "mark the focused pane" }],
    }),
  );
  await Bun.write(`${dir}/modisa-plugin.ts`, await Bun.file(`${REPO}/src/plugins/modisa-plugin.ts`).text());
  await Bun.write(
    `${dir}/plugin.ts`,
    `import { runPlugin } from "./modisa-plugin";
runPlugin(async (modisa) => {
  await modisa.hello({
    hello: () => "hi from the far side",
    apply: async (p) => {
      const out: string[] = [];
      for (const [method, params] of p.calls as [string, any][]) out.push(await modisa.request(method, params).then(() => "ok", (e) => e.code));
      return out;
    },
    mark: async (_p, call) => {
      const file = Bun.env.MODISA_PLUGIN_DATA + "/marks.log";
      await Bun.write(file, (await Bun.file(file).text().catch(() => "")) + JSON.stringify({ target: call.target ?? null }) + "\\n");
      return "marked " + (call.target?.pane ?? "nothing");
    },
  });
});
`,
  );
  // the server's config: another theme, and plugin.json's key G as it is
  await Bun.write(`${sb.root}/config/config.toml`, `theme = "gruvbox"\n`);
  await startServer(sb, S);
  expect((await run("plugin", "link", dir)).code).toBe(0);
  for (let i = 0; i < 100 && !(await sb.json(S, ["plugin", "list"], { retry: "startup" })).find((p: any) => p.name === "rui-demo")?.connected; i++) await Bun.sleep(100);
  expect(await apply(["ui.status.set", { id: "count", text: "1 waiting", tone: "warn" }], ["ui.sidebar.set", { title: "Remote queue", rows: [{ text: "@far blocked", tone: "warn" }] }])).toEqual(["ok", "ok"]);

  // fake ssh: drop "-T" and the host, and run the rest as the far side would, with the server's roots
  await Bun.write(`${sb.root}/bin/fakessh`, `#!/bin/sh\nshift; shift\nexport MODISA_DIR='${sb.root}/state' MODISA_CONFIG_DIR='${sb.root}/config'\neval "$@"\n`);
  await Bun.$`chmod +x ${sb.root}/bin/fakessh`;
  await Bun.write(`${CLIENT}/config/config.toml`, `remote_command = "bun ${MAIN}"\ntheme = "nord"\n\n[plugin_keys]\n"rui-demo.mark" = "Y"\n`);
  remote = new Screen(["-s", S, "--remote", "ssh://devbox"], { ...sb.env, MODISA_DIR: `${CLIENT}/state`, MODISA_CONFIG_DIR: `${CLIENT}/config`, MODISA_SSH: `${sb.root}/bin/fakessh` }, sb.root);
  await remote.until("the remote client", (s) => s.includes("AGENTS") && s.includes("+ agent"), 20000);
}, 60000);

afterAll(async () => {
  remote?.close();
  local?.close();
  await run("kill", S);
  await sb.cleanup();
});

const ready = (screen: Screen) => screen.until("the plugin's UI", (s) => s.includes("▾ rui-demo"), 20000);

test("a remote client draws plugins' status and sidebar, runs a palette action, and shows a plugin toast, all attributed", async () => {
  await ready(remote);
  await remote.until("the plugin's status and sidebar section", (s) => s.includes("rui-demo: ") && s.includes("Remote queue"), 10000);
  remote.write("\x02:");
  await remote.until("the palette", (s) => s.includes("Commands") && s.includes("Type to search"));
  remote.write("Say hello");
  await remote.until("the plugin's action", (s) => s.includes("rui-demo: Say hello"));
  remote.write("\r");
  await remote.until("the action's result", (s) => s.includes("rui-demo: Say hello → hi from the far side"));
  expect(await apply(["ui.toast", { text: "from the far side" }])).toEqual(["ok"]);
  await remote.until("the toast, titled with its plugin", (s) => /rui-demo[^\n]*\n[^\n]*from the far side/.test(s));
}, 40000);

test("the remote client draws in its own theme and binds plugin keys with its own [plugin_keys]; the action runs on the server's side", async () => {
  await ready(remote);
  await currentTheme(remote, "nord"); // its theme, not the server's gruvbox
  const before = (await marks()).length;
  remote.write("\x02Y"); // its remap
  await untilMarks(before + 1);
  const pane = await focused();
  expect((await marks()).at(-1)!.target).toEqual({ pane: pane.id, instance: pane.instance }); // the whole target
  remote.write("\x02G"); // plugin.json's key, which this client moved
  await Bun.sleep(1500);
  expect((await marks()).length).toBe(before + 1);
  // the plugin's files are the server's: nothing under the client's roots
  expect((await Bun.$`find ${CLIENT} -name marks.log`.text()).trim()).toBe("");
  // and the server's own table, which only its CLI reports, still binds G
  expect((await sb.json(S, ["plugin", "list"])).find((p: any) => p.name === "rui-demo").keys).toContainEqual(expect.objectContaining({ key: "G", state: "active" }));
}, 40000);

test("a second client with its own remap uses its own binding, without changing the remote client's", async () => {
  await Bun.write(`${LOCAL}/config/config.toml`, `theme = "tokyonight"\n\n[plugin_keys]\n"rui-demo.mark" = "Q"\n`);
  local ??= new Screen(["-s", S], { ...sb.env, MODISA_CONFIG_DIR: `${LOCAL}/config` }, sb.root);
  await ready(local);
  await currentTheme(local, "tokyonight");
  const before = (await marks()).length;
  local.write("\x02Q");
  await untilMarks(before + 1);
  local.write("\x02Y"); // the remote client's key isn't this one's
  await Bun.sleep(1500);
  expect((await marks()).length).toBe(before + 1);
  await ready(remote);
  remote.write("\x02Y"); // and the remote client's still works
  await untilMarks(before + 2);
  expect((await Bun.$`find ${LOCAL} -name marks.log`.text()).trim()).toBe("");
}, 60000);

test("a key's target is the focused pane's process; once that pane closes, the target is refused", async () => {
  await ready(remote);
  const stale = (await run("pane", "split", "--name", "stale", "sleep 300")).stdout;
  await run("pane", "focus", stale);
  const before = (await marks()).length;
  await Bun.sleep(300);
  remote.write("\x02Y");
  await untilMarks(before + 1);
  const target = (await marks()).at(-1)!.target!;
  expect(target.pane).toBe(stale);
  expect(target.instance).toBeTruthy();
  expect((await run("pane", "close", stale)).code).toBe(0);
  const conn = await connectUnix(`${sb.root}/state/${S}.sock`);
  const outcome = await conn.request("plugin.invoke", { plugin: "rui-demo", action: "mark", target }).then(() => "ok", (e) => e.code);
  conn.close();
  expect(outcome).toBe("pane_gone");
  expect((await marks()).length).toBe(before + 1);
}, 40000);

test("after the connection drops, the reconnect snapshot brings the UI back, without replaying a toast or moving focus", async () => {
  await ready(remote);
  const before = (await focused())?.id;
  expect(await apply(["ui.toast", { text: "shown before the drop" }])).toEqual(["ok"]);
  await remote.until("the toast", (s) => s.includes("shown before the drop"));
  await remote.until("the toast gone", (s) => !s.includes("shown before the drop"), 15000);

  // the ssh connection dies: kill the proxy and the fake ssh running it
  const procs = (await Bun.$`ps -eo pid=,args=`.text()).split("\n").filter((l) => l.includes(MAIN) && /\bproxy\b/.test(l));
  expect(procs.length).toBeGreaterThan(0);
  for (const line of procs) process.kill(Number(line.trim().split(/\s+/)[0]), "SIGKILL");
  // while it's disconnected, nothing a plugin showed can be clicked: it's hidden
  await remote.until("the plugin's UI hidden while reconnecting", (s) => !s.includes("▾ rui-demo"), 5000);
  await remote.until("reconnected", (s) => s.includes("reconnected"), 20000);

  await ready(remote);
  await Bun.sleep(1500);
  expect(remote.text()).not.toContain("shown before the drop");
  expect((await focused())?.id).toBe(before);
  expect(await apply(["ui.status.set", { id: "count", text: "2 waiting", tone: "warn" }])).toEqual(["ok"]); // and it's live again
  await remote.until("a new update", (s) => s.includes("rui-demo: 2 waiting"));
}, 60000);

// the theme picker opens on the theme in use
async function currentTheme(screen: Screen, name: string) {
  screen.write("\x02t");
  await screen.until(`the theme picker on ${name}`, (s) => s.includes(`◉ ${name}`));
  screen.write("\x1b");
  await screen.until("the picker closed", (s) => !s.includes(`◉ ${name}`));
}
