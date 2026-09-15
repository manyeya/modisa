// Plugin link handlers: Ctrl+click on an http(s) URL in a pane runs the action whose manifest URL glob matches it, with
// the URL as the invocation's link (not params); two matching plugins ask which, listed by plugin name; none says so;
// other schemes never run anything; the server refuses a link its action's globs don't match, and a stale run or
// target; a worst-case glob never holds up the session.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { Screen, sandbox, startServer } from "../support/harness";
import { connectUnix } from "../../src/protocol/transport";

const sb = sandbox("plugin-links");
const S = "links";
const REPO = `${import.meta.dir}/../..`;
const run = (...args: string[]) => sb.run(S, args);
let ui: Screen;

async function plugin(name: string, pattern: string) {
  const dir = `${sb.root}/${name}`;
  await Bun.write(`${dir}/plugin.json`, JSON.stringify({ name, protocol: 1, run: ["bun", "plugin.ts"], actions: [{ id: "open", title: "Open" }], links: [{ pattern, action: "open" }] }));
  await Bun.write(`${dir}/shepherd-plugin.ts`, await Bun.file(`${REPO}/src/plugins/shepherd-plugin.ts`).text());
  await Bun.write(`${dir}/plugin.ts`, `import { runPlugin } from "./shepherd-plugin";\nrunPlugin(async (shepherd) => { await shepherd.hello({ open: (p, call) => "${name} got " + call.link + " params " + JSON.stringify(p) }); });\n`);
  expect((await run("plugin", "link", dir)).code).toBe(0);
  for (let i = 0; i < 100 && !JSON.parse((await run("plugin", "list", "--json")).stdout).find((p: any) => p.name === name)?.connected; i++) await Bun.sleep(100);
}

// Ctrl+click (SGR mouse, left button with ctrl) a few cells into text on screen
async function ctrlClick(text: string) {
  const lines = ui.lines();
  const row = lines.findIndex((l) => l.includes(text));
  expect(row).toBeGreaterThanOrEqual(0);
  const col = lines[row]!.indexOf(text) + 2;
  ui.write(`\x1b[<16;${col + 1};${row + 1}M\x1b[<16;${col + 1};${row + 1}m`);
}
const noToast = () => ui.until("earlier toasts gone", (s) => !s.includes("No plugin handles") && !s.includes(" got "), 15000);

beforeAll(async () => {
  await startServer(sb, S);
  await plugin("beta", "https://example.com/shared*");
  await plugin("alpha", "https://example.com/*");
  await plugin("slow", `https://${"a*".repeat(20)}b`);
  const lines = [
    "https://example.com/only-alpha",
    "https://example.com/shared",
    "https://nobody.dev/x",
    "https://example.com/$(touch%20pwned)x",
    `https://${"a".repeat(40)}c`,
    "ftp://example.com/x",
    "javascript:alert(1)",
  ];
  await run("pane", "split", "--name", "urls", `printf '%s\\n\\n' ${lines.map((l) => `'${l}'`).join(" ")}; sleep 120`); // as arguments: % in a URL isn't a format
  ui = new Screen(["-s", S], sb.env, sb.root);
  await ui.until("the URLs", (s) => s.includes("https://example.com/only-alpha") && s.includes("javascript:alert(1)"), 20000);
}, 60000);

afterAll(async () => {
  ui?.close();
  await run("kill", S);
  await sb.cleanup();
});

test("one matching plugin: Ctrl+click runs its action with the URL as call.link, not params", async () => {
  await noToast();
  await ctrlClick("https://example.com/only-alpha");
  await ui.until("alpha's answer", (s) => s.includes("alpha: Open → alpha got https://example.com/only-alpha params {}"));
}, 30000);

test("two matching plugins: a chooser, in plugin-name order", async () => {
  await noToast();
  await ctrlClick("https://example.com/shared");
  await ui.until("the chooser", (s) => s.includes("alpha: Open") && s.includes("beta: Open") && s.includes("esc dismiss"));
  const lines = ui.lines();
  expect(lines.findIndex((l) => l.includes("alpha: Open"))).toBeLessThan(lines.findIndex((l) => l.includes("beta: Open")));
  ui.write("\x1b[B\r"); // the second: beta
  await ui.until("beta's answer", (s) => s.includes("beta: Open → beta got https://example.com/shared"));
}, 30000);

test("no matching plugin says so", async () => {
  await noToast();
  await ctrlClick("https://nobody.dev/x");
  await ui.until("told", (s) => s.includes("No plugin handles https://nobody.dev/x"));
}, 30000);

test("the URL arrives exactly as data: shell syntax in it is never run", async () => {
  await noToast();
  await ctrlClick("https://example.com/$(touch");
  await ui.until("alpha's answer", (s) => s.includes("alpha got https://example.com/$(touch%20pwned)x params {}"));
  expect(await Bun.file(`${sb.root}/pwned`).exists()).toBe(false);
  expect(await Bun.file(`${sb.root}/alpha/pwned`).exists()).toBe(false);
}, 30000);

test("other schemes never run anything", async () => {
  for (const text of ["ftp://example.com/x", "javascript:alert(1)"]) {
    await noToast();
    await ctrlClick(text);
    await Bun.sleep(1000);
    expect(ui.text()).not.toContain("No plugin handles");
    expect(ui.text()).not.toContain(" got ");
  }
}, 40000);

test("a worst-case glob neither stalls the TUI nor the server", async () => {
  await noToast();
  const start = performance.now();
  await ctrlClick(`https://${"a".repeat(40)}c`);
  const list = await run("pane", "list", "--json"); // the server answers meanwhile
  expect(list.code).toBe(0);
  await ui.until("the click's outcome", (s) => s.includes("No plugin handles https://aaaa"), 5000);
  expect(performance.now() - start).toBeLessThan(5000);
}, 30000);

test("the server refuses a link its action doesn't match, a stale run and a stale target", async () => {
  const conn = await connectUnix(`${sb.root}/state/${S}.sock`);
  const invoke = (params: object) => conn.request("plugin.invoke", { action: "open", ...params }).then((r) => r, (e) => e.code);
  expect(await invoke({ plugin: "beta", link: "https://example.com/only-alpha" })).toBe("invalid_params");
  expect(await invoke({ plugin: "alpha", link: "https://example.com/only-alpha" })).toBe("alpha got https://example.com/only-alpha params {}");
  expect(await invoke({ plugin: "alpha", link: "ftp://example.com/x" })).toBe("invalid_params");
  expect(await invoke({ plugin: "alpha", link: `https://example.com/${"x".repeat(2049)}` })).toBe("invalid_params");
  expect(await invoke({ plugin: "alpha", link: "https://example.com/a", run: "an-ended-run" })).toBe("plugin_unavailable");
  expect(await invoke({ plugin: "alpha", link: "https://example.com/a", target: { pane: "p1", instance: "not-this-one" } })).toBe("pane_gone");
  conn.close();
});
