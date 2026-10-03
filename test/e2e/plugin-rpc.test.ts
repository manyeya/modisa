// The plugin manager's requests, as a TUI (local or --remote) makes them over the socket, against local bare
// repositories and a stand-in for the plugin index (no network): resolve shows the exact source and commit before
// anything is fetched; install refuses a ref that has moved since it was shown and otherwise installs, links and
// starts the plugin in the server's own session; the catalog lists the index's plugins and the marketplaces', saying
// which are here; logs, update and unlink act on the server's machine; marketplaces are added, listed, updated and
// removed. Only the user can: every one of these is refused with a caller (an agent in a pane). Each reply is checked
// against its published result schema.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { z } from "zod";
import { sandbox, startServer } from "../support/harness";
import { commitBare, git, pluginRepo, push, type Repo } from "../support/plugin-repos";
import { connectUnix } from "../../src/protocol/transport";
import { results } from "../../src/protocol/schema";
import type { Conn } from "../../src/protocol/conn";

const sb = sandbox("plugin-rpc");
const S = "prpc";
const MANAGED = `${sb.root}/state/plugins-src`;
let index: ReturnType<typeof Bun.serve>;
let conn: Conn;
let demo: Repo;

// a request on the socket, its reply checked against results[method]
async function rpc<M extends keyof typeof results>(method: M, params: object = {}): Promise<z.infer<(typeof results)[M]>> {
  const reply = await conn.request(method, params);
  const parsed = results[method].safeParse(reply);
  expect(parsed.success, `${method}: ${parsed.error?.message}\n${JSON.stringify(reply)}`).toBe(true);
  return reply;
}
const failure = (method: string, params: object) => conn.request(method, params).then(() => undefined, (e: Error & { code?: string }) => e);
const exists = async (path: string) => (await Bun.$`test -e ${path}`.quiet().nothrow()).exitCode === 0;

beforeAll(async () => {
  index = Bun.serve({
    port: 0,
    fetch: () => Response.json({ total_count: 1, items: [{ name: "starry", full_name: "someone/starry", html_url: "https://github.com/someone/starry", clone_url: "https://github.com/someone/starry.git", description: "from the index\x1b[31m", stargazers_count: 9, archived: true }] }),
  });
  await startServer(sb, S, { MODISA_PLUGIN_INDEX: `http://localhost:${index.port}` });
  conn = await connectUnix(`${sb.root}/state/${S}.sock`);
  demo = await pluginRepo(sb, "demo");
  await git(demo.work, "tag", "v1");
  await git(demo.work, "push", "--quiet", demo.bare, "v1");
}, 60000);

afterAll(async () => {
  conn?.close();
  index?.stop(true);
  await sb.run(S, ["kill", S]);
  await sb.cleanup();
});

test("resolve says where an install would come from and the commit it would get, fetching nothing", async () => {
  expect(await rpc("plugin.resolve", { source: demo.url })).toEqual({ source: demo.url, from: demo.url, ref: null, subdir: null, commit: demo.head });
  expect(await rpc("plugin.resolve", { source: demo.url, ref: "v1", subdir: "x" })).toMatchObject({ ref: "v1", subdir: "x", commit: demo.head });
  expect((await failure("plugin.resolve", { source: demo.url, ref: "no-such-branch" }))?.message).toContain("no branch or tag no-such-branch");
  expect((await failure("plugin.resolve", { source: "ext::sh" }))?.message).toContain("remote-helper");
  expect((await failure("plugin.resolve", { source: demo.url, marketplacePlugin: "a@b" }))?.code).toBe("invalid_params");
  expect(await exists(MANAGED)).toBe(false);
}, 20000);

test("install refuses a ref that moved since it was shown, else installs, links and starts it in this session", async () => {
  const shown = (await rpc("plugin.resolve", { source: demo.url })).commit!;
  const moved = await push(demo, (w) => Bun.write(`${w}/MOVED`, "after it was shown"));
  const refused = await rpc("plugin.install", { source: demo.url, commit: shown });
  expect(refused).toMatchObject({ installed: false, stage: "ref", reason: expect.stringContaining("as shown before installing") });
  expect(await exists(`${MANAGED}/demo`)).toBe(false);

  const r = await rpc("plugin.install", { source: demo.url, commit: moved });
  expect(r).toMatchObject({ installed: true, name: "demo", commit: moved, start: { session: S, state: "started" }, hints: [] });
  expect((await rpc("plugin.list")).find((p) => p.name === "demo")).toMatchObject({ connected: true, install: { source: demo.url, commit: moved } });
  demo.head = moved;
}, 30000);

test("logs reads its log on the server, cleaned to show", async () => {
  const log = await rpc("plugin.logs", { name: "demo" });
  expect(log).toMatchObject({ name: "demo", log: expect.stringContaining(`${S}.demo.log`) });
  expect(log.text).toContain("demo: watching");
  expect((await failure("plugin.logs", { name: "nothing" }))?.code).toBe("no_such_plugin");
}, 20000);

test("the catalog lists the index's plugins and the marketplaces', each saying whether it's here", async () => {
  const work = `${sb.root}/work-shelf`;
  expect((await sb.run(S, ["plugin", "new", "shelved", "--dir", `${work}/shelved`])).code).toBe(0);
  await Bun.write(`${work}/.modisa/marketplace.json`, JSON.stringify({ name: "shelf", plugins: [{ name: "shelved", description: "on the shelf", source: "./shelved" }, { name: "demo", source: { git: demo.url } }] }));
  const shelf = await commitBare(sb, "shelf", work);
  expect(await rpc("marketplace.add", { source: shelf.url })).toMatchObject({ added: true, name: "shelf", plugins: ["shelved", "demo"] });

  const cat = await rpc("plugin.catalog", {});
  expect(cat.index).toEqual({ total: 1, results: [expect.objectContaining({ name: "starry", source: "https://github.com/someone/starry.git", description: "from the index", archived: true, installed: false })] });
  expect(cat.marketplaces).toEqual([
    expect.objectContaining({ name: "shelved", marketplace: "shelf", subdir: "shelved", installed: false }),
    expect.objectContaining({ name: "demo", marketplace: "shelf", from: demo.url, installed: true }),
  ]);
  expect((await rpc("plugin.catalog", { query: "shelf" })).marketplaces.map((p) => p.name)).toEqual(["shelved", "demo"]);
  expect((await rpc("plugin.resolve", { marketplacePlugin: "shelved@shelf" })).commit).toBe(shelf.head);
  expect(await rpc("plugin.install", { marketplacePlugin: "shelved@shelf", commit: shelf.head })).toMatchObject({ installed: true, marketplace: "shelf", start: { state: "started" } });

  expect(await rpc("marketplace.list")).toMatchObject([{ name: "shelf", plugins: 2 }]);
  expect(await rpc("marketplace.update", {})).toEqual([{ name: "shelf", updated: false, from: shelf.head, commit: shelf.head, plugins: 2 }]);
  expect(await rpc("marketplace.remove", { name: "shelf" })).toMatchObject({ name: "shelf", removed: true, installed: ["shelved"] });
  expect(await rpc("marketplace.list")).toEqual([]);
}, 40000);

test("update fetches the latest and restarts it here; unlink stops it and deletes what was installed", async () => {
  const before = (await rpc("plugin.list")).find((p) => p.name === "demo")!.pid;
  const next = await push(demo, (w) => Bun.write(`${w}/NEXT`, "1"));
  const u = await rpc("plugin.update", { name: "demo" });
  expect(u).toMatchObject({ updated: true, from: demo.head, commit: next, restarted: [{ session: S, state: "started" }] });
  expect(u.restarted[0]!.pid).not.toBe(before);

  const gone = await rpc("plugin.unlink", { name: "demo" });
  expect(gone).toMatchObject({ name: "demo", managed: true, stoppedIn: [S], checkout: { deleted: true } });
  expect(await exists(`${MANAGED}/demo`)).toBe(false);
  expect((await rpc("plugin.list")).find((p) => p.name === "demo")).toBeUndefined(); // out of the TUI's list too
  expect((await failure("plugin.unlink", { name: "../demo" }))?.code).toBe("no_such_plugin");
}, 40000);

test("only the user: every plugin manager request is refused with a caller, and changes nothing", async () => {
  const asAgent = { caller: "p1" };
  const requests: [string, object][] = [
    ["plugin.resolve", { source: demo.url }], ["plugin.install", { source: demo.url }], ["plugin.update", { name: "shelved" }], ["plugin.unlink", { name: "shelved" }],
    ["plugin.logs", { name: "shelved" }], ["plugin.catalog", {}], ["marketplace.list", {}], ["marketplace.add", { source: demo.url }],
    ["marketplace.update", {}], ["marketplace.remove", { name: "shelf" }],
  ];
  for (const [method, params] of requests) {
    const e = await failure(method, { ...params, ...asAgent });
    expect(e?.message, method).toBe("only the user can manage plugins and marketplaces");
  }
  expect(await exists(`${MANAGED}/demo`)).toBe(false);
  expect((await rpc("plugin.list")).find((p) => p.name === "shelved")).toMatchObject({ connected: true });
}, 30000);
