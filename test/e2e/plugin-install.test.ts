// `shepherd plugin install <git-url>`, against local bare repositories (no network): it clones, records the source,
// ref and commit, links and starts the plugin with no restart; --ref and --subdir pick a branch and a directory;
// escapes, bad refs and bad manifests leave nothing behind; a name collision leaves the existing install alone; a
// plugin that installs but can't start says so; unlinking an install stops it in every running session before
// deleting its checkout, and keeps it (saying why) if a session can't be reached; a directory you linked is never
// deleted. Every outcome is checked in its --json form.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { sandbox, startServer } from "../support/harness";
import { cliResults } from "../../src/protocol/schema";

const sb = sandbox("plugin-install");
const S = "inst";
const run = (...args: string[]) => sb.run(S, args);
const MANAGED = `${sb.root}/state/plugins-src`;
const git = (cwd: string, ...args: string[]) => Bun.$`git -c user.name=test -c user.email=test@example.com -c init.defaultBranch=main ${args}`.cwd(cwd).quiet();
const list = async (session = S) => JSON.parse((await sb.run(session, ["plugin", "list", "--json"])).stdout) as any[];
const exists = async (path: string) => (await Bun.$`test -e ${path}`.quiet().nothrow()).exitCode === 0;
const managedEntries = async () => (await Bun.$`ls -A ${MANAGED}`.quiet().nothrow().text()).split("\n").filter(Boolean).sort();
const until = async (what: string, ok: () => Promise<boolean>, ms = 15000) => {
  for (const end = Date.now() + ms; Date.now() < end; await Bun.sleep(100)) if (await ok()) return;
  throw new Error(`timed out waiting for ${what}`);
};

async function installJson(...args: string[]) {
  const r = await run("plugin", "install", ...args, "--json");
  const result = JSON.parse(r.stdout);
  const parsed = cliResults["plugin install"].safeParse(result);
  expect(parsed.success, parsed.error?.message).toBe(true);
  return { code: r.code, result };
}
async function unlinkJson(name: string) {
  const r = await run("plugin", "unlink", name, "--json");
  const result = JSON.parse(r.stdout);
  const parsed = cliResults["plugin unlink"].safeParse(result);
  expect(parsed.success, parsed.error?.message).toBe(true);
  return { code: r.code, result };
}

// a bare repository with a scaffolded plugin (at `subdir`, or the root), one commit on main
async function repo(id: string, options: { plugin?: string; subdir?: string; change?: (dir: string) => Promise<unknown> } = {}) {
  const work = `${sb.root}/work-${id}`;
  const dir = options.subdir ? `${work}/${options.subdir}` : work;
  expect((await run("plugin", "new", options.plugin ?? id, "--dir", dir)).code).toBe(0);
  await options.change?.(dir);
  await git(work, "init", "--quiet");
  await git(work, "add", "-A");
  await git(work, "commit", "--quiet", "-m", "plugin");
  const bare = `${sb.root}/${id}.git`;
  await Bun.$`git clone --quiet --bare ${work} ${bare}`.quiet();
  return { work, bare, url: `file://${bare}`, head: (await Bun.$`git -C ${bare} rev-parse HEAD`.text()).trim() };
}

let demo: Awaited<ReturnType<typeof repo>>;

beforeAll(async () => {
  await startServer(sb, S);
  demo = await repo("demo");
}, 60000);

afterAll(async () => {
  for (const s of [S, "second", "ghost"]) await sb.run(s, ["kill", s]);
  await sb.cleanup();
});

test("install clones, records, links and starts the plugin, and its action works with no restart", async () => {
  const { code, result } = await installJson(demo.url);
  expect(code).toBe(0);
  expect(result).toMatchObject({ installed: true, name: "demo", source: demo.url, ref: null, commit: demo.head, start: { session: S, state: "started" }, hints: [] });
  expect(result.checkout).toEndWith("/plugins-src/demo/checkout");
  expect(result.start.log).toContain("demo");
  expect((await run("plugin", "run", "demo", "status")).code).toBe(0);
  expect((await list()).find((p) => p.name === "demo").install).toEqual({ source: demo.url, ref: null, commit: demo.head });
  expect((await run("plugin", "list")).stdout).toContain(`@${demo.head.slice(0, 7)}`);
}, 30000);

test("the same source again is already installed, one run; another source under that name is refused, the install intact", async () => {
  const pid = (await list()).find((p) => p.name === "demo").pid;
  const again = await installJson(demo.url);
  expect(again.code).toBe(0);
  expect(again.result).toMatchObject({ installed: false, alreadyInstalled: true, commit: demo.head, start: { state: "already-running", pid } });

  const other = await repo("other", { plugin: "demo" });
  const clash = await installJson(other.url);
  expect(clash.code).toBe(1);
  expect(clash.result).toMatchObject({ installed: false, stage: "collision" });
  expect((await list()).filter((p) => p.name === "demo")).toMatchObject([{ pid, install: { commit: demo.head } }]);
  expect(await exists(`${MANAGED}/demo/checkout/plugin.json`)).toBe(true);
  expect((await managedEntries()).filter((e) => e.startsWith(".staging"))).toEqual([]);
}, 30000);

test("--ref and --subdir install a branch's plugin from a directory in the repository", async () => {
  const mono = await repo("mono", { plugin: "inner", subdir: "plugins/inner" });
  await git(mono.work, "checkout", "--quiet", "-b", "feature");
  await Bun.write(`${mono.work}/plugins/inner/FEATURE`, "on the branch");
  await git(mono.work, "add", "-A");
  await git(mono.work, "commit", "--quiet", "-m", "feature");
  await Bun.$`git -C ${mono.work} push --quiet ${mono.bare} feature`.quiet();
  const feature = (await Bun.$`git -C ${mono.bare} rev-parse feature`.text()).trim();

  const { code, result } = await installJson(mono.url, "--ref", "feature", "--subdir", "plugins/inner");
  expect(code).toBe(0);
  expect(result).toMatchObject({ installed: true, name: "inner", ref: "feature", commit: feature, start: { state: "started" } });
  expect(result.dir).toEndWith("/checkout/plugins/inner");
  expect(await exists(`${result.dir}/FEATURE`)).toBe(true);
  expect((await list()).find((p) => p.name === "inner").install).toEqual({ source: mono.url, ref: "feature", commit: feature });
}, 60000);

test("a bad ref, a --subdir that escapes the repository, or a bad manifest installs nothing and leaves nothing behind", async () => {
  await Bun.$`mkdir -p ${sb.root}/outside`.quiet();
  expect((await run("plugin", "new", "outside", "--dir", `${sb.root}/outside/p`)).code).toBe(0);
  const escape = await repo("escape", { change: (dir) => Bun.$`ln -s ${sb.root}/outside/p ${dir}/away`.quiet() });
  const bad = await repo("badmanifest", { change: (dir) => Bun.write(`${dir}/plugin.json`, JSON.stringify({ name: "badmanifest", protocol: 1 })) });
  const before = await managedEntries();
  const cases: [string[], string][] = [
    [[demo.url, "--ref", "no-such-branch"], "ref"],
    [[escape.url, "--subdir", "../outside/p"], "subdir"],
    [[escape.url, "--subdir", "/etc"], "subdir"],
    [[escape.url, "--subdir", "away"], "subdir"], // a symlink out of the checkout
    [[bad.url], "manifest"],
  ];
  for (const [args, stage] of cases) {
    const { code, result } = await installJson(...args);
    expect(code).toBe(1);
    expect(result).toMatchObject({ installed: false, stage });
  }
  expect(await managedEntries()).toEqual(before);
  expect(await exists(`${sb.root}/config/plugins/badmanifest`)).toBe(false);
  expect(await exists(`${sb.root}/config/plugins/outside`)).toBe(false);
}, 60000);

test("a plugin that installs but can't start says so, and keeps what was installed", async () => {
  const broken = await repo("broken", { change: (dir) => Bun.write(`${dir}/plugin.json`, JSON.stringify({ name: "broken", protocol: 1, run: ["bun", "missing.ts"] })) });
  const { code, result } = await installJson(broken.url);
  expect(code).toBe(1);
  expect(result).toMatchObject({ installed: true, name: "broken", commit: broken.head, start: { state: "failed" } });
  expect(result.start.log).toContain("broken");
  expect(await exists(`${result.checkout}/plugin.json`)).toBe(true);
  expect(await exists(`${sb.root}/config/plugins/broken`)).toBe(true);
  expect((await run("plugin", "install", broken.url)).stdout).toContain("already installed, but failed to start");
}, 30000);

test("unlinking an install stops it in every running session, then deletes its checkout but not its data", async () => {
  await startServer(sb, "second");
  await until("demo in the second session, which starts linked plugins itself",async () => (await list("second")).find((p) => p.name === "demo")?.connected === true);
  const { code, result } = await unlinkJson("demo");
  expect(code).toBe(0);
  expect(result).toMatchObject({ managed: true, stillUsing: [], unreachable: [], checkout: { deleted: true } });
  expect(result.stoppedIn.sort()).toEqual([S, "second"].sort());
  expect(await exists(`${MANAGED}/demo`)).toBe(false);
  expect(await exists(`${sb.root}/state/plugins/demo`)).toBe(true); // its data directory
  expect((await list("second")).find((p) => p.name === "demo")).toMatchObject({ status: "stopped" });
}, 40000);

test("if a session can't be reached, unlinking an install keeps its checkout and says which session and where", async () => {
  await startServer(sb, "ghost");
  const sock = `${sb.root}/state/ghost.sock`;
  await Bun.$`mv ${sock} ${sock}.real && touch ${sock}`; // as if a sandbox blocked it
  try {
    const { result } = await unlinkJson("inner");
    expect(result).toMatchObject({ managed: true, unreachable: ["ghost"], checkout: { deleted: false } });
    expect(result.stoppedIn).toContain(S);
    expect(await exists(`${result.checkout.path}/plugins/inner/plugin.json`)).toBe(true);
    expect(await exists(`${sb.root}/config/plugins/inner`)).toBe(false); // the link is gone either way
  } finally {
    await Bun.$`rm ${sock} && mv ${sock}.real ${sock}`;
  }
}, 30000);

test("unlinking a directory you linked yourself stops it here and never deletes it", async () => {
  const mine = `${sb.root}/mine`;
  expect((await run("plugin", "new", "mine", "--dir", mine)).code).toBe(0);
  expect((await run("plugin", "link", mine)).code).toBe(0);
  const { result } = await unlinkJson("mine");
  expect(result).toMatchObject({ managed: false, stoppedIn: [S] });
  expect(result.checkout).toBeUndefined();
  expect(await exists(`${mine}/plugin.json`)).toBe(true);
  expect((await run("plugin", "link", mine)).code).toBe(0);
  expect((await run("plugin", "unlink", "mine")).stdout).toContain("Its directory is untouched");
}, 30000);
