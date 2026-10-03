// `modisa plugin update <name>`, against local bare repositories (no network): an installed plugin moves to what its
// ref is at now (the default branch's new HEAD when it has none), its record says so, and it restarts in every
// running session that ran it and in none that didn't; a new commit whose plugin.json doesn't check out is undone,
// leaving the running plugin alone; a plugin you linked is never updated. Every result is checked in its --json form.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { sandbox, startServer } from "../support/harness";
import { git, pluginRepo, push } from "../support/plugin-repos";
import { cliResults } from "../../src/protocol/schema";

const sb = sandbox("plugin-update");
const S = "upd";
const MANAGED = `${sb.root}/state/plugins-src`;
const list = async (session = S) => sb.json<any[]>(session, ["plugin", "list"], { retry: "startup" });
const find = async (name: string, session = S) => (await list(session)).find((p) => p.name === name);
const record = (name: string) => Bun.file(`${MANAGED}/${name}/install.json`).json();
const until = async (what: string, ok: () => Promise<boolean>, ms = 15000) => {
  for (const end = Date.now() + ms; Date.now() < end; await Bun.sleep(100)) if (await ok()) return;
  throw new Error(`timed out waiting for ${what}`);
};

async function update(name: string) {
  const r = await sb.run(S, ["plugin", "update", name, "--json"]);
  const result = JSON.parse(r.stdout);
  const parsed = cliResults["plugin update"].safeParse(result);
  expect(parsed.success, parsed.error?.message).toBe(true);
  return { code: r.code, result };
}
const install = async (...args: string[]) => {
  const r = await sb.run(S, ["plugin", "install", ...args, "--json"]);
  expect(r.code, r.stdout + r.stderr).toBe(0);
  return JSON.parse(r.stdout);
};

beforeAll(async () => {
  await startServer(sb, S);
}, 60000);

afterAll(async () => {
  for (const s of [S, "other"]) await sb.run(s, ["kill", s]);
  await sb.cleanup();
});

test("a new commit on the default branch is picked up, recorded, and the plugin restarts with it", async () => {
  const repo = await pluginRepo(sb, "fresh");
  await install(repo.url);
  const before = await find("fresh");
  expect(before).toMatchObject({ connected: true, install: { commit: repo.head } });

  const head = await push(repo, async (w) => Bun.write(`${w}/plugin.ts`, (await Bun.file(`${w}/plugin.ts`).text()).replace(': watching"', ': watching, updated"')));
  const { code, result } = await update("fresh");
  expect(code).toBe(0);
  expect(result).toMatchObject({ name: "fresh", updated: true, source: repo.url, ref: null, from: repo.head, commit: head, restarted: [{ session: S, state: "started" }] });
  expect(result.restarted[0].pid).not.toBe(before.pid);
  expect(await record("fresh")).toMatchObject({ commit: head, updatedAt: expect.any(String), installedAt: expect.any(String) });
  expect(await find("fresh")).toMatchObject({ connected: true, pid: result.restarted[0].pid, install: { commit: head } });
  await until("the new version's output", async () => (await sb.run(S, ["plugin", "logs", "fresh"])).stdout.includes("watching, updated"));

  const again = await update("fresh");
  expect(again).toMatchObject({ code: 0, result: { updated: false, upToDate: true, commit: head, restarted: [] } });
  expect((await sb.run(S, ["plugin", "update", "fresh"])).stdout).toContain(`fresh is up to date: ${repo.url} (commit ${head})`);
}, 60000);

test("with a ref, update follows that branch, and restarts it only where it runs", async () => {
  const repo = await pluginRepo(sb, "branchy");
  await git(repo.work, "checkout", "--quiet", "-b", "feature");
  const feature = await push(repo, (w) => Bun.write(`${w}/FEATURE`, "1"), "feature");
  await install(repo.url, "--ref", "feature");
  await sb.run(S, ["plugin", "stop", "branchy"]);
  await startServer(sb, "other"); // starts linked plugins itself
  await until("branchy in the other session", async () => (await find("branchy", "other"))?.connected === true);

  await git(repo.work, "checkout", "--quiet", "main");
  await push(repo, (w) => Bun.write(`${w}/ON_MAIN`, "not on feature"));
  await git(repo.work, "checkout", "--quiet", "feature");
  const next = await push(repo, (w) => Bun.write(`${w}/FEATURE`, "2"), "feature");
  const { result } = await update("branchy");
  expect(result).toMatchObject({ updated: true, ref: "feature", from: feature, commit: next, restarted: [{ session: "other", state: "started" }] });
  expect(await Bun.file(`${MANAGED}/branchy/checkout/FEATURE`).text()).toBe("2");
  expect(await Bun.file(`${MANAGED}/branchy/checkout/ON_MAIN`).exists()).toBe(false);
  expect(await find("branchy")).toMatchObject({ status: "stopped" }); // not restarted where it was stopped
  await sb.run("other", ["kill", "other"]);
}, 60000);

test("a new commit whose plugin.json doesn't check out is undone, and the running plugin is left alone", async () => {
  const repo = await pluginRepo(sb, "breaks");
  await install(repo.url);
  const pid = (await find("breaks")).pid;
  await push(repo, (w) => Bun.write(`${w}/plugin.json`, JSON.stringify({ name: "breaks", protocol: 1 })));
  const { code, result } = await update("breaks");
  expect(code).toBe(1);
  expect(result).toMatchObject({ updated: false, stage: "manifest", commit: repo.head, restarted: [] });
  expect((await Bun.$`git -C ${MANAGED}/breaks/checkout rev-parse HEAD`.text()).trim()).toBe(repo.head);
  expect((await record("breaks")).commit).toBe(repo.head);
  expect(await find("breaks")).toMatchObject({ connected: true, pid });

  await push(repo, (w) => Bun.write(`${w}/plugin.json`, JSON.stringify({ name: "renamed", protocol: 1, run: ["bun", "plugin.ts"] })));
  expect((await update("breaks")).result).toMatchObject({ stage: "manifest", reason: expect.stringContaining("names it renamed") });
  expect((await sb.run(S, ["plugin", "update", "breaks"])).stderr).toContain("modisa: not updated (manifest)");
}, 60000);

test("a plugin you linked is updated where it lives, never by modisa; an unknown one is an error", async () => {
  const mine = `${sb.root}/mine`;
  expect((await sb.run(S, ["plugin", "new", "mine", "--dir", mine])).code).toBe(0);
  expect((await sb.run(S, ["plugin", "link", mine])).code).toBe(0);
  const { code, result } = await update("mine");
  expect(code).toBe(1);
  expect(result).toMatchObject({ updated: false, stage: "linked", reason: `linked from ${(await Bun.$`realpath ${mine}`.text()).trim()}: update it there` });
  const unknown = await sb.run(S, ["plugin", "update", "nothing-here"]);
  expect(unknown.code).toBe(1);
  expect(unknown.stderr).toContain("no linked plugin named nothing-here");
  expect((await sb.run(S, ["plugin", "update"])).code).toBe(2);
}, 30000);
