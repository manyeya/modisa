// The plugin manager (prefix P): add a marketplace, find its plugin, see exactly where it comes from (source and
// commit) before it installs, install it and see it running, read its log, and remove it. Against a local bare
// repository, with the plugin index off (no network); everything goes through the server, as with --remote.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { Screen, sandbox } from "../../support/harness";
import { commitBare, type Repo } from "../../support/plugin-repos";

const sb = sandbox("plugins-ui");
const S = "plugins-ui";
const env = { ...sb.env, MODISA_PLUGIN_INDEX: "off" };
let ui: Screen;
let market: Repo;
const list = async () => sb.json<any[]>(S, ["plugin", "list"], { retry: "startup" });

beforeAll(async () => {
  const work = `${sb.root}/work-acme`;
  await Bun.$`mkdir -p ${sb.root}`.quiet();
  expect((await sb.run(S, ["plugin", "new", "hello", "--dir", `${work}/plugins/hello`])).code).toBe(0);
  await Bun.write(`${work}/modisa-marketplace.json`, JSON.stringify({ name: "acme", plugins: [{ name: "hello", description: "says hello", source: "./plugins/hello" }] }));
  market = await commitBare(sb, "acme", work);
  ui = new Screen(["-s", S], env, sb.root);
  await ui.until("dashboard", (s) => s.includes("AGENTS"), 20000);
  await Bun.sleep(300);
}, 40000);

afterAll(async () => {
  await sb.run(S, ["kill", S]);
  ui?.close();
  await sb.cleanup();
});

test("add a marketplace, install its plugin after seeing its source and commit, see it running, read its log, remove it", async () => {
  ui.write("\x02P");
  await ui.until("the manager", (s) => s.includes("Discover") && s.includes("Marketplaces") && s.includes("Add from URL…"));
  ui.write("marketplaces\r");
  await ui.until("marketplaces", (s) => s.includes("+ Add marketplace…"));
  ui.write("add\r");
  await ui.until("the prompt", (s) => s.includes("Add a marketplace"));
  ui.write(`${market.url}\r`);
  await ui.until("added", (s) => s.includes("added marketplace acme: 1 plugins") && /acme\s+1 plugin ·/.test(s), 15000);

  ui.write("acme\r"); // its plugins
  await ui.until("its plugins", (s) => s.includes("Marketplace acme") && s.includes("@acme · says hello"), 15000);
  ui.write("\r");
  await ui.until("the warning, with the exact source and commit", (s) => s.includes("Install hello?") && s.includes("unsandboxed") && s.includes(market.head) && s.includes("plugins/hello"), 15000);
  ui.write("y");
  await ui.until("installed and running", (s) => s.includes("Installed plugins") && /hello\s+running · pid \d+ · @acme/.test(s) && s.includes(`installed hello @${market.head.slice(0, 7)}, running`), 20000);
  expect((await list()).find((p) => p.name === "hello")).toMatchObject({ connected: true, install: { marketplace: "acme", commit: market.head } });

  ui.write("hello\x0c"); // ^l: its log
  await ui.until("its log", (s) => s.includes("hello: its log") && s.includes("hello: watching"));
  ui.write("\x1b");
  await ui.until("back to the list", (s) => s.includes("Installed plugins"));

  ui.write("hello\x04"); // ^d: remove
  await ui.until("asked first", (s) => s.includes("Stop hello in every session and delete its checkout?"));
  ui.write("y");
  await ui.until("removed", (s) => s.includes("removed hello") && s.includes("Installed plugins") && !/hello\s+(running|stopped)/.test(s), 15000);
  expect((await list()).some((p) => p.name === "hello")).toBe(false);
  expect(await Bun.file(`${sb.root}/state/plugins-src/hello/install.json`).exists()).toBe(false);
  ui.write("\x1b"); // back to the manager's menu
  await ui.until("the menu", (s) => s.includes("Add from URL…") && !s.includes("Installed plugins"));
  ui.write("\x1b");
  await ui.until("closed", (s) => !s.includes("Add from URL…"));
}, 90000);

test("the settings page lists installed plugins and opens the manager; so does the palette", async () => {
  ui.write("\x02:");
  await ui.until("palette", (s) => s.includes("Type to search"));
  ui.write("plugins");
  await ui.until("the palette's entry", (s) => /Plugins…\s+P/.test(s));
  ui.write("\x1b");
  await ui.until("palette closed", (s) => !s.includes("⌕ plugins"));
  ui.write("\x02s");
  await ui.until("settings", (s) => s.includes("Integrations") && s.includes("Plugins"));
  ui.write("add from url");
  await ui.until("its row", (s) => s.includes("add from URL") && s.includes("↵ open"));
  ui.write("\r");
  await ui.until("the URL prompt", (s) => s.includes("Install from a git URL"));
  ui.write("\x1b");
  await ui.until("closed", (s) => !s.includes("Install from a git URL"));
}, 30000);
