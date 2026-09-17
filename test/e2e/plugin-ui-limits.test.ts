// Limits across a session's plugins: several plugins, each within its own limits, still can't together crowd the
// status row, stack badges on one pane, fill the sidebar or the menu, flood toasts, or exceed the session's update rate.
import { test, expect, beforeAll, beforeEach, afterAll } from "bun:test";
import { sandbox, startServer } from "../support/harness";

const sb = sandbox("plugin-ui-limits");
const S = "limits";
const REPO = `${import.meta.dir}/../..`;
const NAMES = ["pa", "pb", "pc", "pd", "pe", "pf", "pg"];
const run = (...args: string[]) => sb.run(S, args);
// have one plugin make these ui.* calls from its bound connection; each gives "ok" or the error code
const apply = async (name: string, ...calls: [string, object][]) => (await sb.json<string[]>(S, ["plugin", "run", name, "apply", JSON.stringify({ calls })]));
const connected = async (names: string[]) => {
  for (let i = 0; i < 150; i++, await Bun.sleep(100)) {
    const list = (await sb.json<any[]>(S, ["plugin", "list"], { retry: "startup" }));
    if (names.every((n) => list.find((p) => p.name === n)?.connected)) return;
  }
  throw new Error(`not all of ${names.join(", ")} connected`);
};
let p1 = { instance: "" };

beforeAll(async () => {
  await startServer(sb, S);
  for (const name of NAMES) {
    const dir = `${sb.root}/${name}`;
    await Bun.write(`${dir}/plugin.json`, JSON.stringify({ name, protocol: 1, run: ["bun", "plugin.ts"], actions: [{ id: "hello", title: "Hello" }, { id: "apply", title: "Apply" }] }));
    await Bun.write(`${dir}/modisa-plugin.ts`, await Bun.file(`${REPO}/src/plugins/modisa-plugin.ts`).text());
    await Bun.write(
      `${dir}/plugin.ts`,
      `import { runPlugin } from "./modisa-plugin";
runPlugin(async (modisa) => {
  await modisa.hello({
    hello: () => "hi",
    apply: async (p) => {
      const out: string[] = [];
      for (const [method, params] of p.calls as [string, any][]) out.push(await modisa.request(method, params).then(() => "ok", (e) => e.code));
      return out;
    },
  });
});
`,
    );
    expect((await run("plugin", "link", dir)).code).toBe(0);
  }
  p1 = await sb.json(S, ["pane", "read", "p1"]);
}, 60000);

// every test starts from fresh runs of every plugin (nothing shown), and the session's budgets refilled
beforeEach(async () => {
  await Promise.all(NAMES.map((n) => run("plugin", "stop", n)));
  await Promise.all(NAMES.map((n) => run("plugin", "start", n)));
  await connected(NAMES);
  await Bun.sleep(2500);
}, 60000);

afterAll(async () => {
  await run("kill", S);
  await sb.cleanup();
});

const segments = (n: number, prefix: string): [string, object][] => Array.from({ length: n }, (_, i) => ["ui.status.set", { id: `${prefix}${i}`, text: `${prefix}${i}` }]);

test("status segments: 12 across the session's plugins, though each may have 4", async () => {
  for (const name of ["pa", "pb", "pc"]) expect(await apply(name, ...segments(4, name))).toEqual(["ok", "ok", "ok", "ok"]);
  expect(await apply("pd", ...segments(1, "pd"))).toEqual(["error"]);
  expect(await apply("pa", ["ui.status.set", { id: "pa0", text: "replaced" }])).toEqual(["ok"]); // replacing its own isn't more
  expect(await apply("pa", ["ui.status.clear", { id: "pa1" }])).toEqual(["ok"]);
  expect(await apply("pd", ...segments(1, "pd"))).toEqual(["ok"]); // room again
}, 30000);

test("badges: at most 4 plugins' on one pane", async () => {
  const badge: [string, object] = ["ui.badge.set", { pane: "p1", instance: p1.instance, text: "b" }];
  for (const name of ["pa", "pb", "pc", "pd"]) expect(await apply(name, badge)).toEqual(["ok"]);
  expect(await apply("pe", badge)).toEqual(["error"]);
  expect(await apply("pa", badge)).toEqual(["ok"]); // its own again
}, 30000);

test("sidebar sections: at most 6 plugins'", async () => {
  const section: [string, object] = ["ui.sidebar.set", { title: "T", rows: [{ text: "row" }] }];
  for (const name of NAMES.slice(0, 6)) expect(await apply(name, section)).toEqual(["ok"]);
  expect(await apply("pg", section)).toEqual(["error"]);
  expect(await apply("pa", section)).toEqual(["ok"]); // replacing its own
}, 30000);

test("menu entries: 24 across the session's plugins, though each may have 8", async () => {
  const menu = (name: string, n: number): [string, object] => ["ui.menu.set", { items: Array.from({ length: n }, (_, i) => ({ id: `${name}${i}`, title: `${name} ${i}`, action: "hello" })) }];
  for (const name of ["pa", "pb", "pc"]) expect(await apply(name, menu(name, 8))).toEqual(["ok"]);
  expect(await apply("pd", menu("pd", 1))).toEqual(["error"]);
  expect(await apply("pa", menu("pa", 7))).toEqual(["ok"]); // shrinking its own makes room
  expect(await apply("pd", menu("pd", 1))).toEqual(["ok"]);
}, 30000);

test("toasts: 6 every 10s across the session's plugins, though each may send 3", async () => {
  const toasts = (n: number): [string, object][] => Array.from({ length: n }, (_, i) => ["ui.toast", { text: `t${i}` }]);
  expect(await apply("pa", ...toasts(3))).toEqual(["ok", "ok", "ok"]);
  expect(await apply("pb", ...toasts(3))).toEqual(["ok", "ok", "ok"]);
  expect(await apply("pc", ...toasts(1))).toEqual(["rate_limited"]);
}, 30000);

test("updates: the session's rate holds even when each plugin keeps to its own", async () => {
  const burst = (name: string) => apply(name, ...Array.from({ length: 25 }, (): [string, object] => ["ui.status.set", { id: "x", text: "x" }]));
  const each = await Promise.all(["pa", "pb", "pc", "pd"].map(burst)); // 100 updates at once, 25 from each
  expect(each.flat()).toContain("rate_limited");
  await Bun.sleep(2500);
  expect(await burst("pe")).not.toContain("rate_limited"); // one plugin's 25 alone are fine
}, 30000);
