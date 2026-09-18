// Plugin link handlers: Ctrl+click on an http(s) URL in a pane runs the action whose manifest link (a URL glob, or an
// RE2 regex) matches it, with the URL as the invocation's link (not params); several matching plugins ask which,
// listed by plugin name; none says so; other schemes never run anything; the server refuses a link its action's links
// don't match, and a stale run or target; a pattern that backtracks exponentially elsewhere never holds up the TUI or
// the server. Each test dismisses whatever an earlier one left open, so each passes alone and in any order.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { Screen, sandbox, startServer } from "../support/harness";
import { connectUnix } from "../../src/protocol/transport";

const sb = sandbox("plugin-links");
const S = "links";
const REPO = `${import.meta.dir}/../..`;
const run = (...args: string[]) => sb.run(S, args);
let ui: Screen;

async function plugin(name: string, links: object[]) {
  const dir = `${sb.root}/${name}`;
  await Bun.write(`${dir}/plugin.json`, JSON.stringify({ name, protocol: 1, run: ["bun", "plugin.ts"], actions: [{ id: "open", title: "Open" }], links: links.map((l) => ({ ...l, action: "open" })) }));
  await Bun.write(`${dir}/modisa-plugin.ts`, await Bun.file(`${REPO}/src/plugins/modisa-plugin.ts`).text());
  await Bun.write(`${dir}/plugin.ts`, `import { runPlugin } from "./modisa-plugin";\nrunPlugin(async (modisa) => { await modisa.hello({ open: (p, call) => "${name} got " + call.link + " params " + JSON.stringify(p) }); });\n`);
  expect((await run("plugin", "link", dir)).code).toBe(0);
  for (let i = 0; i < 100 && !(await sb.json(S, ["plugin", "list"], { retry: "startup" })).find((p: any) => p.name === name)?.connected; i++) await Bun.sleep(100);
}

// Ctrl+click (SGR mouse, left button with ctrl) `offset` cells into text on screen; columns are cells, not characters
async function ctrlClick(text: string, offset = 2) {
  const lines = ui.lines();
  const row = lines.findIndex((l) => l.includes(text));
  expect(row).toBeGreaterThanOrEqual(0);
  const col = Bun.stringWidth(lines[row]!.slice(0, lines[row]!.indexOf(text))) + offset;
  ui.write(`\x1b[<16;${col + 1};${row + 1}M\x1b[<16;${col + 1};${row + 1}m`);
}
// a clean start: no chooser an earlier test left open, no toast still showing
async function fresh() {
  if (ui.text().includes("esc close")) ui.write("\x1b");
  await ui.until("a clean screen", (s) => !s.includes("esc close") && !s.includes("No plugin handles") && !s.includes(" got "), 15000);
}

// a near-match for slow's regexes, failing only at its last character (a letter: trailing punctuation would be trimmed)
const PATHOLOGICAL = `https://example.test/${"a".repeat(80)}X`;
const BOTH_REGEXES = `https://example.test/${"a".repeat(12)}`;

beforeAll(async () => {
  await startServer(sb, S);
  await plugin("beta", [{ pattern: "https://example.com/shared*" }]);
  await plugin("alpha", [{ pattern: "https://example.com/*" }]);
  await plugin("pr", [{ regex: "^https://example\\.com/pr/\\d+$" }]);
  await plugin("slow", [{ pattern: `https://${"a*".repeat(20)}b` }, { regex: "^https://example\\.test/(a|aa)+$" }, { regex: "^https://example\\.test/(a+)+$" }]);
  const lines = [
    "https://example.com/only-alpha",
    "https://example.com/shared",
    "https://nobody.dev/x",
    "https://example.com/$(touch%20pwned)x",
    `https://${"a".repeat(40)}c`,
    "ftp://example.com/x",
    "javascript:alert(1)",
    "https://Example.COM/Path_A/%7Euser?Q=Mixed",
    "日本語 https://example.com/wide",
    "https://example.com/pr/42",
    PATHOLOGICAL,
    BOTH_REGEXES,
  ];
  const urls = (await run("pane", "split", "--name", "urls", `printf '%s\\n\\n' ${lines.map((l) => `'${l}'`).join(" ")}; sleep 300`)).stdout; // as arguments: % in a URL isn't a format
  await run("pane", "close", "p1"); // the URLs get the full width
  await run("pane", "focus", urls);
  ui = new Screen(["-s", S], sb.env, sb.root);
  await ui.until("the URLs", (s) => s.includes("https://example.com/only-alpha") && s.includes(PATHOLOGICAL), 20000);
}, 90000);

afterAll(async () => {
  ui?.close();
  await run("kill", S);
  await sb.cleanup();
});

test("one matching plugin: Ctrl+click runs its action with the URL as call.link, not params", async () => {
  await fresh();
  await ctrlClick("https://example.com/only-alpha");
  await ui.until("alpha's answer", (s) => s.includes("alpha: Open → alpha got https://example.com/only-alpha params {}"));
}, 30000);

test("two matching plugins: a chooser, in plugin-name order", async () => {
  await fresh();
  await ctrlClick("https://example.com/shared");
  await ui.until("the chooser, titled with the link", (s) => s.includes("Open https://example.c… with") && s.includes("alpha: Open") && s.includes("beta: Open"));
  const lines = ui.lines();
  expect(lines.findIndex((l) => l.includes("alpha: Open"))).toBeLessThan(lines.findIndex((l) => l.includes("beta: Open")));
  ui.write("\x1b[B\r"); // the second: beta
  await ui.until("beta's answer", (s) => s.includes("beta: Open → beta got https://example.com/shared"));
}, 30000);

test("a glob and a regex that match the same URL share the chooser", async () => {
  await fresh();
  await ctrlClick("https://example.com/pr/42");
  await ui.until("the chooser", (s) => s.includes("alpha: Open") && s.includes("pr: Open"));
  const lines = ui.lines();
  expect(lines.findIndex((l) => l.includes("alpha: Open"))).toBeLessThan(lines.findIndex((l) => l.includes("pr: Open")));
  ui.write("\x1b[B\r"); // the regex's plugin
  await ui.until("pr's answer", (s) => s.includes("pr got https://example.com/pr/42 params {}"));
}, 30000);

test("an action counts once however many of its plugin's links match: it runs, no chooser", async () => {
  await fresh();
  await ctrlClick(`${BOTH_REGEXES} `); // its own line: the pathological one starts with the same text
  await ui.until("slow's answer", (s) => s.includes(`slow got ${BOTH_REGEXES} params {}`));
  expect(ui.text()).not.toContain("esc close");
}, 30000);

test("no matching plugin says so", async () => {
  await fresh();
  await ctrlClick("https://nobody.dev/x");
  await ui.until("told", (s) => s.includes("No plugin handles https://nobody.dev/x"));
}, 30000);

test("the URL arrives exactly as data: shell syntax in it is never run", async () => {
  await fresh();
  await ctrlClick("https://example.com/$(touch");
  await ui.until("alpha's answer", (s) => s.includes("alpha got https://example.com/$(touch%20pwned)x params {}"));
  expect(await Bun.file(`${sb.root}/pwned`).exists()).toBe(false);
  expect(await Bun.file(`${sb.root}/alpha/pwned`).exists()).toBe(false);
}, 30000);

test("the host matches in any case, and the action gets the URL byte for byte: case, escapes and query kept", async () => {
  await fresh();
  await ctrlClick("https://Example.COM/Path_A");
  await ui.until("alpha's answer", (s) => s.includes("alpha got https://Example.COM/Path_A/%7Euser?Q=Mixed params {}"));
}, 30000);

test("after wide characters, a click on the URL's last cell still reaches it", async () => {
  await fresh();
  await ctrlClick("https://example.com/wide", "https://example.com/wide".length - 1);
  await ui.until("alpha's answer", (s) => s.includes("alpha got https://example.com/wide params {}"));
}, 30000);

test("other schemes never run anything", async () => {
  for (const text of ["ftp://example.com/x", "javascript:alert(1)"]) {
    await fresh();
    await ctrlClick(text);
    await Bun.sleep(1000);
    expect(ui.text()).not.toContain("No plugin handles");
    expect(ui.text()).not.toContain(" got ");
  }
}, 40000);

// the TUI and the server are processes of their own; this test holds the deadlines
const within = <T>(ms: number, work: Promise<T>) => Promise.race([work, Bun.sleep(ms).then(() => "missed the deadline" as const)]);

test("a worst-case glob or backtracking-prone regex, clicked, neither stalls the TUI nor the server", async () => {
  for (const text of [`https://${"a".repeat(40)}c`, PATHOLOGICAL]) {
    await fresh();
    const start = performance.now();
    await ctrlClick(text);
    const list = await within(3000, run("pane", "list", "--json")); // another process's round trip, meanwhile
    expect(list).not.toBe("missed the deadline");
    await ui.until("the click's outcome", (s) => s.includes(`No plugin handles ${text.slice(0, 30)}`), 3000);
    expect(performance.now() - start).toBeLessThan(3000);
  }
}, 40000);

test("the server checks a backtracking-prone regex against a full-length near-match fast, and keeps answering", async () => {
  const conn = await connectUnix(`${sb.root}/state/${S}.sock`);
  const prefix = "https://example.test/";
  const link = prefix + "a".repeat(2048 - prefix.length - 1) + "!";
  expect(link).toHaveLength(2048);
  const start = performance.now();
  const [outcome, list] = await Promise.all([
    within(3000, conn.request("plugin.invoke", { plugin: "slow", action: "open", link }).then((r) => r, (e) => e.code)),
    within(3000, run("pane", "list", "--json")),
  ]);
  expect(outcome).toBe("invalid_params"); // no link of slow's matches it
  expect(list).not.toBe("missed the deadline");
  expect(performance.now() - start).toBeLessThan(3000);
  conn.close();
}, 20000);

test("the server refuses a link its action doesn't match, a stale run and a stale target", async () => {
  const conn = await connectUnix(`${sb.root}/state/${S}.sock`);
  const invoke = (params: object) => conn.request("plugin.invoke", { action: "open", ...params }).then((r) => r, (e) => e.code);
  expect(await invoke({ plugin: "beta", link: "https://example.com/only-alpha" })).toBe("invalid_params");
  expect(await invoke({ plugin: "alpha", link: "https://example.com/only-alpha" })).toBe("alpha got https://example.com/only-alpha params {}");
  expect(await invoke({ plugin: "pr", link: "https://example.com/pr/42" })).toBe("pr got https://example.com/pr/42 params {}");
  expect(await invoke({ plugin: "pr", link: "https://example.com/pr/x" })).toBe("invalid_params");
  expect(await invoke({ plugin: "alpha", link: "ftp://example.com/x" })).toBe("invalid_params");
  expect(await invoke({ plugin: "alpha", link: `https://example.com/${"x".repeat(2049)}` })).toBe("invalid_params");
  expect(await invoke({ plugin: "alpha", link: "https://example.com/a", run: "an-ended-run" })).toBe("plugin_unavailable");
  expect(await invoke({ plugin: "alpha", link: "https://example.com/a", target: { pane: "p9", instance: "not-this-one" } })).toBe("pane_gone");
  conn.close();
});
