// A remote client (ssh running `shepherd proxy` on the far side) gets plugins' UI like a local one: status segments,
// its sidebar section, palette actions and toasts, drawn with the local theme. When the connection drops and the client
// reconnects, what plugins showed is hidden until the attach snapshot brings it back, without replaying a toast shown
// before and without moving focus.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { MAIN, Screen, sandbox, startServer } from "../support/harness";

const sb = sandbox("remote-plugin-ui");
const S = "rui";
const REPO = `${import.meta.dir}/../..`;
const dir = `${sb.root}/rui-demo`;
const run = (...args: string[]) => sb.run(S, args);
let remote: Screen;

// have the plugin make these ui.* calls from its bound connection; each gives "ok" or the error code
const apply = async (...calls: [string, object][]) => JSON.parse((await run("plugin", "run", "rui-demo", "apply", JSON.stringify({ calls }))).stdout) as string[];

beforeAll(async () => {
  await Bun.write(`${dir}/plugin.json`, JSON.stringify({ name: "rui-demo", protocol: 1, run: ["bun", "plugin.ts"], actions: [{ id: "hello", title: "Say hello" }, { id: "apply", title: "Apply UI calls" }] }));
  await Bun.write(`${dir}/shepherd-plugin.ts`, await Bun.file(`${REPO}/src/plugins/shepherd-plugin.ts`).text());
  await Bun.write(
    `${dir}/plugin.ts`,
    `import { runPlugin } from "./shepherd-plugin";
runPlugin(async (shepherd) => {
  await shepherd.hello({
    hello: () => "hi from the far side",
    apply: async (p) => {
      const out: string[] = [];
      for (const [method, params] of p.calls as [string, any][]) out.push(await shepherd.request(method, params).then(() => "ok", (e) => e.code));
      return out;
    },
  });
});
`,
  );
  await startServer(sb, S);
  expect((await run("plugin", "link", dir)).code).toBe(0);
  for (let i = 0; i < 100 && !JSON.parse((await run("plugin", "list", "--json")).stdout).find((p: any) => p.name === "rui-demo")?.connected; i++) await Bun.sleep(100);
  expect(await apply(["ui.status.set", { id: "count", text: "1 waiting", tone: "warn" }], ["ui.sidebar.set", { title: "Remote queue", rows: [{ text: "@far blocked", tone: "warn" }] }])).toEqual(["ok", "ok"]);

  // fake ssh: drop "-T" and the host, run the rest locally like a remote shell would
  await Bun.write(`${sb.root}/bin/fakessh`, `#!/bin/sh\nshift; shift; eval "$@"\n`);
  await Bun.$`chmod +x ${sb.root}/bin/fakessh`;
  await Bun.write(`${sb.root}/config/config.toml`, `remote_command = "bun ${MAIN}"\n`);
  remote = new Screen(["-s", S, "--remote", "ssh://devbox"], { ...sb.env, SHEPHERD_SSH: `${sb.root}/bin/fakessh` }, sb.root);
  await remote.until("the remote client", (s) => s.includes("SPACES") && s.includes("+ agent"), 20000);
}, 60000);

afterAll(async () => {
  remote?.close();
  await run("kill", S);
  await sb.cleanup();
});

test("a remote client draws plugins' status and sidebar, runs a palette action, and shows a plugin toast, all attributed", async () => {
  await remote.until("the plugin's status and sidebar section", (s) => s.includes("rui-demo: 1 waiting") && s.includes("▾ rui-demo") && s.includes("Remote queue"), 10000);
  remote.write("\x02:");
  await remote.until("the palette", (s) => s.includes("commands"));
  remote.write("Say hello");
  await remote.until("the plugin's action", (s) => s.includes("rui-demo: Say hello"));
  remote.write("\r");
  await remote.until("the action's result", (s) => s.includes("rui-demo: Say hello → hi from the far side"));
  expect(await apply(["ui.toast", { text: "from the far side" }])).toEqual(["ok"]);
  await remote.until("the toast", (s) => s.includes("rui-demo: from the far side"));
}, 40000);

test("after the connection drops, the reconnect snapshot brings the UI back, without replaying a toast or moving focus", async () => {
  const focused = async () => (JSON.parse((await run("pane", "list", "--json")).stdout) as any[]).find((p) => p.focused)?.id;
  const before = await focused();
  expect(await apply(["ui.toast", { text: "shown before the drop" }])).toEqual(["ok"]);
  await remote.until("the toast", (s) => s.includes("shown before the drop"));
  await remote.until("the toast gone", (s) => !s.includes("shown before the drop"), 15000);

  // the ssh connection dies: kill the proxy and the fake ssh running it
  const procs = (await Bun.$`ps -eo pid=,args=`.text()).split("\n").filter((l) => l.includes(MAIN) && /\bproxy\b/.test(l));
  expect(procs.length).toBeGreaterThan(0);
  for (const line of procs) process.kill(Number(line.trim().split(/\s+/)[0]), "SIGKILL");
  // while it's disconnected, nothing a plugin showed can be clicked: it's hidden
  await remote.until("the plugin's UI hidden while reconnecting", (s) => !s.includes("rui-demo: 1 waiting") && !s.includes("▾ rui-demo"), 5000);
  await remote.until("reconnected", (s) => s.includes("reconnected"), 20000);

  await remote.until("the plugin's UI, from the attach snapshot", (s) => s.includes("rui-demo: 1 waiting") && s.includes("▾ rui-demo"), 10000);
  await Bun.sleep(1500);
  expect(remote.text()).not.toContain("shown before the drop");
  expect(await focused()).toBe(before);
  expect(await apply(["ui.status.set", { id: "count", text: "2 waiting", tone: "warn" }])).toEqual(["ok"]); // and it's live again
  await remote.until("a new update", (s) => s.includes("rui-demo: 2 waiting"));
}, 60000);
