// `modisa plugin marketplace add|list|update|remove`, and installing `<plugin>@<marketplace>`, against local bare
// repositories (no network): a marketplace is cloned and recorded under the state directory; search lists its plugins
// beside the index's; a plugin inside it installs from its checkout, from a directory that must really be inside it,
// and one it points elsewhere installs from there at the ref it names; `plugin update` brings the marketplace up to
// date first and restarts the plugin; a bad marketplace leaves nothing behind; removing one keeps what was installed
// from it. Every result is checked in its --json form against its published schema.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { sandbox, startServer } from "../support/harness";
import { commitBare, git, pluginRepo, push, type Repo } from "../support/plugin-repos";
import { cliResults } from "../../src/protocol/schema";

const sb = sandbox("plugin-marketplace");
const S = "market";
const STATE = `${sb.root}/state`;
const OFF = { MODISA_PLUGIN_INDEX: "off" }; // the index is never reached
let market: Repo;
let ext: Repo;
let v1 = "";

// `modisa plugin <args> --json`, parsed and checked against cliResults[schema]
async function json(schema: keyof typeof cliResults, args: string[], env: Record<string, string> = OFF) {
  const r = await sb.run(S, ["plugin", ...args, "--json"], env);
  const result = JSON.parse(r.stdout);
  const parsed = cliResults[schema].safeParse(result);
  expect(parsed.success, `${args.join(" ")}: ${parsed.error?.message}\n${r.stdout}`).toBe(true);
  return { code: r.code, result };
}
const list = async () => sb.json<any[]>(S, ["plugin", "list"], { retry: "startup" });
const exists = async (path: string) => (await Bun.$`test -e ${path}`.quiet().nothrow()).exitCode === 0;
const staging = async () => (await Bun.$`ls -A ${STATE}/marketplaces`.quiet().nothrow().text()).split("\n").filter((e) => e.startsWith(".staging"));

beforeAll(async () => {
  await startServer(sb, S, OFF);
  // a plugin elsewhere, at tag v1 (main moves on past it)
  ext = await pluginRepo(sb, "ext");
  await git(ext.work, "tag", "v1");
  await git(ext.work, "push", "--quiet", ext.bare, "v1");
  v1 = ext.head;
  await push(ext, (w) => Bun.write(`${w}/AFTER_V1`, "not in v1"));
  // the marketplace: a plugin inside it, one elsewhere, and one whose source leaves the repository through a symlink
  const work = `${sb.root}/work-acme`;
  expect((await sb.run(S, ["plugin", "new", "hello", "--dir", `${work}/plugins/hello`])).code).toBe(0);
  expect((await sb.run(S, ["plugin", "new", "outside", "--dir", `${sb.root}/outside`])).code).toBe(0);
  await Bun.$`ln -s ${sb.root}/outside ${work}/away`.quiet();
  await Bun.write(`${work}/modisa-marketplace.json`, JSON.stringify({
    name: "acme", description: "Acme's plugins\x1b[31m", owner: "acme",
    plugins: [
      { name: "hello", description: "says hello", source: "./plugins/hello" },
      { name: "ext", description: "from elsewhere", source: { git: ext.url, ref: "v1" } },
      { name: "outside", source: "./away" },
    ],
  }));
  market = await commitBare(sb, "acme", work);
}, 60000);

afterAll(async () => {
  await sb.run(S, ["kill", S]);
  await sb.cleanup();
});

test("add clones and records a marketplace; adding it again is a no-op; list shows it", async () => {
  const added = await json("plugin marketplace add", ["marketplace", "add", market.url]);
  expect(added.code).toBe(0);
  expect(added.result).toMatchObject({ added: true, name: "acme", source: market.url, ref: null, commit: market.head, description: "Acme's plugins", plugins: ["hello", "ext", "outside"] });
  expect(await exists(`${STATE}/marketplaces/acme/modisa-marketplace.json`)).toBe(true);
  expect((await Bun.file(`${STATE}/marketplaces.json`).json())[0]).toMatchObject({ name: "acme", source: market.url, commit: market.head });

  const again = await json("plugin marketplace add", ["marketplace", "add", market.url]);
  expect(again.result).toMatchObject({ added: false, alreadyAdded: true, name: "acme" });
  const { result } = await json("plugin marketplace list", ["marketplace", "list"]);
  expect(result).toMatchObject([{ name: "acme", plugins: 3, owner: "acme", commit: market.head }]);
  expect((await sb.run(S, ["plugin", "marketplace", "list"])).stdout).toMatch(/acme\s+3\s+file:/);
}, 30000);

test("search lists the marketplaces' plugins, labelled, beside the index's (or alone when the index can't be read)", async () => {
  const { code, result } = await json("plugin search", ["search", "hello"]);
  expect(code).toBe(0);
  expect(result.indexError).toContain("turned off");
  expect(result.marketplacePlugins).toEqual([expect.objectContaining({ name: "hello", marketplace: "acme", subdir: "plugins/hello", from: market.url, install: "modisa plugin install hello@acme", installed: false })]);
  const text = await sb.run(S, ["plugin", "search"], OFF);
  expect(text.stdout).toContain("hello@acme  marketplace acme");
  expect(text.stdout).toContain("modisa plugin install ext@acme");

  const index = Bun.serve({ port: 0, fetch: () => Response.json({ total_count: 1, items: [{ name: "indexed", full_name: "someone/indexed", html_url: "https://github.com/someone/indexed", clone_url: "https://github.com/someone/indexed.git", description: "from the index", stargazers_count: 5 }] }) });
  try {
    const both = await json("plugin search", ["search"], { MODISA_PLUGIN_INDEX: `http://localhost:${index.port}` });
    expect(both.result.indexError).toBeUndefined();
    expect(both.result.results.map((r: any) => r.name)).toEqual(["indexed"]);
    expect(both.result.marketplacePlugins.map((p: any) => p.name)).toEqual(["hello", "ext", "outside"]);
  } finally {
    index.stop(true);
  }
}, 30000);

test("a plugin inside the marketplace installs from its checkout, starts, and remembers where it came from", async () => {
  const { code, result } = await json("plugin install", ["install", "hello@acme"]);
  expect(code).toBe(0);
  expect(result).toMatchObject({ installed: true, name: "hello", marketplace: "acme", ref: null, commit: market.head, start: { session: S, state: "started" } });
  expect(result.source).toBe(`file://${(await Bun.$`realpath ${STATE}/marketplaces/acme`.text()).trim()}`);
  expect(result.dir).toEndWith("/plugins-src/hello/checkout/plugins/hello");
  expect((await list()).find((p) => p.name === "hello")).toMatchObject({ connected: true, install: { marketplace: "acme", commit: market.head } });
  expect((await json("plugin search", ["search", "hello"])).result.marketplacePlugins[0].installed).toBe(true);
}, 30000);

test("a plugin the marketplace points elsewhere installs from there, at the ref it names", async () => {
  const { code, result } = await json("plugin install", ["install", "ext@acme"]);
  expect(code).toBe(0);
  expect(result).toMatchObject({ installed: true, name: "ext", source: ext.url, ref: "v1", commit: v1, marketplace: "acme" });
  expect(await exists(`${result.dir}/AFTER_V1`)).toBe(false);
}, 30000);

test("a source that leaves the marketplace's repository, an unknown plugin or marketplace, or --ref installs nothing", async () => {
  const away = await json("plugin install", ["install", "outside@acme"]);
  expect(away.code).toBe(1);
  expect(away.result).toMatchObject({ installed: false, stage: "marketplace", reason: expect.stringContaining("isn't a directory inside the marketplace's repository") });
  expect((await json("plugin install", ["install", "nope@acme"])).result).toMatchObject({ stage: "marketplace", reason: "marketplace acme lists no plugin named nope" });
  expect((await json("plugin install", ["install", "hello@nowhere"])).result).toMatchObject({ stage: "marketplace", reason: expect.stringContaining("no marketplace named nowhere") });
  expect((await sb.run(S, ["plugin", "install", "hello@acme", "--ref", "x"])).code).toBe(2);
  expect(await exists(`${sb.root}/config/plugins/outside`)).toBe(false);
  expect(await exists(`${STATE}/plugins-src/outside`)).toBe(false);
}, 30000);

test("plugin update brings the marketplace up to date first, then the plugin, and restarts it", async () => {
  const before = (await list()).find((p) => p.name === "hello").pid;
  const next = await push(market, (w) => Bun.write(`${w}/plugins/hello/NEW`, "a new commit"));
  const { code, result } = await json("plugin update", ["update", "hello"]);
  expect(code).toBe(0);
  expect(result).toMatchObject({ name: "hello", updated: true, from: market.head, commit: next, marketplace: "acme", restarted: [{ session: S, state: "started" }] });
  expect(result.restarted[0].pid).not.toBe(before);
  expect(await exists(`${STATE}/plugins-src/hello/checkout/plugins/hello/NEW`)).toBe(true);
  expect((await json("plugin marketplace list", ["marketplace", "list"])).result[0].commit).toBe(next);
  expect((await json("plugin marketplace update", ["marketplace", "update"])).result).toEqual([{ name: "acme", updated: false, from: next, commit: next, plugins: 3 }]);
  expect((await sb.run(S, ["plugin", "update", "hello"], OFF)).stdout).toContain("hello is up to date");
  market.head = next;
}, 40000);

test("a marketplace update whose file no longer checks out is undone", async () => {
  await push(market, (w) => Bun.write(`${w}/modisa-marketplace.json`, JSON.stringify({ name: "acme", plugins: [{ name: "Bad Name", source: "./x" }] })));
  const { code, result } = await json("plugin marketplace update", ["marketplace", "update", "acme"]);
  expect(code).toBe(1);
  expect(result).toMatchObject([{ name: "acme", updated: false, commit: market.head, reason: expect.stringContaining("lowercase letters") }]);
  expect((await Bun.$`git -C ${STATE}/marketplaces/acme rev-parse HEAD`.text()).trim()).toBe(market.head);
  expect((await json("plugin marketplace list", ["marketplace", "list"])).result[0]).toMatchObject({ commit: market.head, plugins: 3 });
}, 30000);

test("a repository that isn't a marketplace, or a URL install won't fetch, adds nothing and leaves nothing behind", async () => {
  const plain = await pluginRepo(sb, "not-a-market");
  const none = await json("plugin marketplace add", ["marketplace", "add", plain.url]);
  expect(none.code).toBe(1);
  expect(none.result).toMatchObject({ added: false, stage: "manifest", reason: expect.stringContaining("no modisa-marketplace.json") });
  expect((await json("plugin marketplace add", ["marketplace", "add", "ext::sh"])).result).toMatchObject({ stage: "source" });
  expect((await json("plugin marketplace add", ["marketplace", "add", market.url, "--ref", "no-such-ref"])).result).toMatchObject({ stage: "ref" });
  expect(await staging()).toEqual([]);
  expect((await json("plugin marketplace list", ["marketplace", "list"])).result.map((m: any) => m.name)).toEqual(["acme"]);
}, 30000);

test("remove deletes the marketplace's checkout and record, and what was installed from it stays", async () => {
  const { code, result } = await json("plugin marketplace remove", ["marketplace", "remove", "acme"]);
  expect(code).toBe(0);
  expect(result).toMatchObject({ name: "acme", removed: true });
  expect(result.installed.sort()).toEqual(["ext", "hello"]);
  expect(await exists(`${STATE}/marketplaces/acme`)).toBe(false);
  expect((await json("plugin marketplace list", ["marketplace", "list"])).result).toEqual([]);
  expect((await list()).find((p) => p.name === "hello")).toMatchObject({ connected: true });
  expect((await sb.run(S, ["plugin", "marketplace", "remove", "acme"], OFF)).stderr).toContain("no marketplace named acme");
}, 30000);
