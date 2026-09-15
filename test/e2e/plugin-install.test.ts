// `shepherd plugin install <git-url>`, against local bare repositories (no network): it clones, records the source,
// ref and commit, links and starts the plugin with no restart; --ref and --subdir pick a branch and a directory;
// escapes, bad refs and bad manifests leave nothing behind; a name collision leaves the existing install alone; a
// plugin that installs but can't start says so; setup it doesn't do is pointed out; unlinking an install stops it in
// every running session before deleting its checkout, and keeps it (saying why) if a session can't be reached; a
// directory you linked is never deleted. Remote-helper URLs, and git settings or variables inherited from the caller,
// can't make the install run a program or touch another repository. Every outcome is checked in its --json form.
// Each test makes its own repositories and plugin names, so each passes alone and in any order.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { sandbox, startServer } from "../support/harness";
import { cliResults } from "../../src/protocol/schema";

const sb = sandbox("plugin-install");
const S = "inst";
const MANAGED = `${sb.root}/state/plugins-src`;
const LINKS = `${sb.root}/config/plugins`;
const git = (cwd: string, ...args: string[]) => Bun.$`git -c user.name=test -c user.email=test@example.com -c init.defaultBranch=main ${args}`.cwd(cwd).quiet();
const list = async (session = S) => JSON.parse((await sb.run(session, ["plugin", "list", "--json"])).stdout) as any[];
const exists = async (path: string) => (await Bun.$`test -e ${path}`.quiet().nothrow()).exitCode === 0;
const staging = async () => (await Bun.$`ls -A ${MANAGED}`.quiet().nothrow().text()).split("\n").filter((e) => e.startsWith(".staging"));
const until = async (what: string, ok: () => Promise<boolean>, ms = 15000) => {
  for (const end = Date.now() + ms; Date.now() < end; await Bun.sleep(100)) if (await ok()) return;
  throw new Error(`timed out waiting for ${what}`);
};

async function installJson(args: string[], env: Record<string, string> = {}) {
  const r = await sb.run(S, ["plugin", "install", ...args, "--json"], env);
  const result = JSON.parse(r.stdout);
  const parsed = cliResults["plugin install"].safeParse(result);
  expect(parsed.success, parsed.error?.message).toBe(true);
  return { code: r.code, result };
}
async function unlinkJson(name: string) {
  const r = await sb.run(S, ["plugin", "unlink", name, "--json"]);
  const result = JSON.parse(r.stdout);
  const parsed = cliResults["plugin unlink"].safeParse(result);
  expect(parsed.success, parsed.error?.message).toBe(true);
  return { code: r.code, result };
}

// a bare repository with a scaffolded plugin (at `subdir`, or the root), one commit on main
async function repo(id: string, options: { plugin?: string; subdir?: string; change?: (dir: string) => Promise<unknown> } = {}) {
  const work = `${sb.root}/work-${id}`;
  const dir = options.subdir ? `${work}/${options.subdir}` : work;
  expect((await sb.run(S, ["plugin", "new", options.plugin ?? id, "--dir", dir])).code).toBe(0);
  await options.change?.(dir);
  await git(work, "init", "--quiet");
  await git(work, "add", "-A");
  await git(work, "commit", "--quiet", "-m", "plugin");
  const bare = `${sb.root}/${id}.git`;
  await Bun.$`git clone --quiet --bare ${work} ${bare}`.quiet();
  return { work, bare, url: `file://${bare}`, head: (await Bun.$`git -C ${bare} rev-parse HEAD`.text()).trim() };
}

beforeAll(async () => {
  await startServer(sb, S);
}, 60000);

afterAll(async () => {
  for (const s of [S, "second", "ghost"]) await sb.run(s, ["kill", s]);
  await sb.cleanup();
});

test("install clones, records, links and starts the plugin, and its action works with no restart", async () => {
  const demo = await repo("demo");
  const { code, result } = await installJson([demo.url]);
  expect(code).toBe(0);
  expect(result).toMatchObject({ installed: true, name: "demo", source: demo.url, ref: null, commit: demo.head, start: { session: S, state: "started" }, hints: [] });
  expect(result.checkout).toEndWith("/plugins-src/demo/checkout");
  expect(result.start.log).toContain("demo");
  expect((await sb.run(S, ["plugin", "run", "demo", "status"])).code).toBe(0);
  expect((await list()).find((p) => p.name === "demo").install).toEqual({ source: demo.url, ref: null, commit: demo.head });
  expect((await sb.run(S, ["plugin", "list"])).stdout).toContain(`@${demo.head.slice(0, 7)}`);
}, 30000);

test("the same source again is already installed, one run; another source under that name is refused, the install intact", async () => {
  const again = await repo("again");
  expect((await installJson([again.url])).code).toBe(0);
  const pid = (await list()).find((p) => p.name === "again").pid;
  const second = await installJson([again.url]);
  expect(second.code).toBe(0);
  expect(second.result).toMatchObject({ installed: false, alreadyInstalled: true, commit: again.head, start: { state: "already-running", pid } });

  const other = await repo("other-again", { plugin: "again" });
  const clash = await installJson([other.url]);
  expect(clash.code).toBe(1);
  expect(clash.result).toMatchObject({ installed: false, stage: "collision" });
  expect((await list()).filter((p) => p.name === "again")).toMatchObject([{ pid, install: { commit: again.head } }]);
  expect(await exists(`${MANAGED}/again/checkout/plugin.json`)).toBe(true);
  expect(await staging()).toEqual([]);
}, 40000);

test("--ref and --subdir install a branch's plugin from a directory in the repository", async () => {
  const mono = await repo("mono", { plugin: "inner", subdir: "plugins/inner" });
  await git(mono.work, "checkout", "--quiet", "-b", "feature");
  await Bun.write(`${mono.work}/plugins/inner/FEATURE`, "on the branch");
  await git(mono.work, "add", "-A");
  await git(mono.work, "commit", "--quiet", "-m", "feature");
  await Bun.$`git -C ${mono.work} push --quiet ${mono.bare} feature`.quiet();
  const feature = (await Bun.$`git -C ${mono.bare} rev-parse feature`.text()).trim();

  const { code, result } = await installJson([mono.url, "--ref", "feature", "--subdir", "plugins/inner"]);
  expect(code).toBe(0);
  expect(result).toMatchObject({ installed: true, name: "inner", ref: "feature", commit: feature, start: { state: "started" } });
  expect(result.dir).toEndWith("/checkout/plugins/inner");
  expect(await exists(`${result.dir}/FEATURE`)).toBe(true);
  expect((await list()).find((p) => p.name === "inner").install).toEqual({ source: mono.url, ref: "feature", commit: feature });
}, 60000);

test("a bad ref, a --subdir that escapes the repository, or a bad manifest installs nothing and leaves nothing behind", async () => {
  await Bun.$`mkdir -p ${sb.root}/outside`.quiet();
  if (!(await exists(`${sb.root}/outside/p/plugin.json`))) expect((await sb.run(S, ["plugin", "new", "outside", "--dir", `${sb.root}/outside/p`])).code).toBe(0);
  const plain = await repo("plain");
  const escape = await repo("escape", { change: (dir) => Bun.$`ln -s ${sb.root}/outside/p ${dir}/away`.quiet() });
  const bad = await repo("badmanifest", { change: (dir) => Bun.write(`${dir}/plugin.json`, JSON.stringify({ name: "badmanifest", protocol: 1 })) });
  const cases: [string[], string][] = [
    [[plain.url, "--ref", "no-such-branch"], "ref"],
    [[escape.url, "--subdir", "../outside/p"], "subdir"],
    [[escape.url, "--subdir", "/etc"], "subdir"],
    [[escape.url, "--subdir", "away"], "subdir"], // a symlink out of the checkout
    [[bad.url], "manifest"],
  ];
  for (const [args, stage] of cases) {
    const { code, result } = await installJson(args);
    expect(code).toBe(1);
    expect(result).toMatchObject({ installed: false, stage });
  }
  for (const name of ["plain", "escape", "badmanifest", "outside"]) {
    expect(await exists(`${MANAGED}/${name}`)).toBe(false);
    expect(await exists(`${LINKS}/${name}`)).toBe(false);
  }
  expect(await staging()).toEqual([]);
}, 60000);

test("a plugin that installs but can't start says so, and keeps what was installed", async () => {
  const broken = await repo("broken", { change: (dir) => Bun.write(`${dir}/plugin.json`, JSON.stringify({ name: "broken", protocol: 1, run: ["bun", "missing.ts"] })) });
  const { code, result } = await installJson([broken.url]);
  expect(code).toBe(1);
  expect(result).toMatchObject({ installed: true, name: "broken", commit: broken.head, start: { state: "failed" } });
  expect(result.start.log).toContain("broken");
  expect(await exists(`${result.checkout}/plugin.json`)).toBe(true);
  expect(await exists(`${LINKS}/broken`)).toBe(true);
  expect((await sb.run(S, ["plugin", "install", broken.url])).stdout).toContain("already installed, but failed to start");
}, 30000);

test("a plugin whose package.json has dependencies gets a hint, since install fetches none", async () => {
  const deps = await repo("deps", { change: (dir) => Bun.write(`${dir}/package.json`, JSON.stringify({ name: "deps", dependencies: { "left-pad": "1.3.0" } })) });
  const { result } = await installJson([deps.url]);
  expect(result).toMatchObject({ installed: true, name: "deps" });
  expect(result.hints).toEqual([expect.stringContaining(`run bun install in ${result.dir}`)]);
  expect(await exists(`${result.dir}/node_modules`)).toBe(false); // and nothing was fetched
}, 30000);

test("unlinking an install stops it in every running session, then deletes its checkout but not its data", async () => {
  const shared = await repo("shared");
  expect((await installJson([shared.url])).code).toBe(0);
  await startServer(sb, "second"); // starts linked plugins itself
  await until("shared in the second session", async () => (await list("second")).find((p) => p.name === "shared")?.connected === true);
  const { code, result } = await unlinkJson("shared");
  expect(code).toBe(0);
  expect(result).toMatchObject({ managed: true, stillUsing: [], unreachable: [], checkout: { deleted: true } });
  expect(result.stoppedIn).toEqual(expect.arrayContaining([S, "second"]));
  expect(await exists(`${MANAGED}/shared`)).toBe(false);
  expect(await exists(`${sb.root}/state/plugins/shared`)).toBe(true); // its data directory
  expect((await list("second")).find((p) => p.name === "shared")).toMatchObject({ status: "stopped" });
}, 60000);

test("if a session can't be reached, unlinking an install keeps its checkout and says which session and where", async () => {
  const kept = await repo("kept");
  expect((await installJson([kept.url])).code).toBe(0);
  await startServer(sb, "ghost");
  const sock = `${sb.root}/state/ghost.sock`;
  await Bun.$`mv ${sock} ${sock}.real && touch ${sock}`; // as if a sandbox blocked it
  try {
    const { result } = await unlinkJson("kept");
    expect(result).toMatchObject({ managed: true, unreachable: ["ghost"], checkout: { deleted: false } });
    expect(result.stoppedIn).toContain(S);
    expect(await exists(`${result.checkout.path}/plugin.json`)).toBe(true);
    expect(await exists(`${LINKS}/kept`)).toBe(false); // the link is gone either way
  } finally {
    await Bun.$`rm ${sock} && mv ${sock}.real ${sock}`;
  }
}, 40000);

test("unlinking a directory you linked yourself stops it here and never deletes it", async () => {
  const mine = `${sb.root}/mine`;
  if (!(await exists(`${mine}/plugin.json`))) expect((await sb.run(S, ["plugin", "new", "mine", "--dir", mine])).code).toBe(0);
  expect((await sb.run(S, ["plugin", "link", mine])).code).toBe(0);
  const { result } = await unlinkJson("mine");
  expect(result).toMatchObject({ managed: false, stoppedIn: [S] });
  expect(result.checkout).toBeUndefined();
  expect(await exists(`${mine}/plugin.json`)).toBe(true);
  expect((await sb.run(S, ["plugin", "link", mine])).code).toBe(0);
  expect((await sb.run(S, ["plugin", "unlink", "mine"])).stdout).toContain("Its directory is untouched");
}, 30000);

// ---------- a git URL, or the caller's git setup, can't make install run a program or touch another repository ----------

// nothing of `name` left behind: no marker from a program, no link, no checkout, no staging directory
async function nothingFrom(name: string, marker: string) {
  expect(await exists(marker)).toBe(false);
  expect(await exists(`${LINKS}/${name}`)).toBe(false);
  expect(await exists(`${MANAGED}/${name}`)).toBe(false);
  expect(await staging()).toEqual([]);
}
const enableExt = { GIT_CONFIG_COUNT: "1", GIT_CONFIG_KEY_0: "protocol.ext.allow", GIT_CONFIG_VALUE_0: "always" };

test("(a) an ext:: URL is refused before git runs, even when the caller's git config allows ext", async () => {
  const marker = `${sb.root}/marker-a`;
  const { code, result } = await installJson([`ext::sh -c touch% ${marker}`], enableExt);
  expect(code).toBe(1);
  expect(result).toMatchObject({ installed: false, stage: "source" });
  await Bun.sleep(300);
  await nothingFrom("marker-a", marker);
}, 20000);

// a global git config (which install keeps, for credential helpers) that allows ext and rewrites an https host to it
async function rewritingConfig(id: string, marker: string) {
  const file = `${sb.root}/gitconfig-${id}`;
  await Bun.write(file, `[protocol "ext"]\n\tallow = always\n[url "ext::sh -c touch% ${marker}% #"]\n\tinsteadOf = https://evil.example/\n`);
  return file;
}

test("(b) an https URL that the caller's git config rewrites to ext:: fails without running anything", async () => {
  const marker = `${sb.root}/marker-b`;
  const { code, result } = await installJson(["https://evil.example/plugin.git"], { GIT_CONFIG_GLOBAL: await rewritingConfig("b", marker) });
  expect(code).toBe(1);
  expect(result).toMatchObject({ installed: false, stage: "clone" });
  await Bun.sleep(300);
  await nothingFrom("plugin", marker);
}, 30000);

test("(c) an inherited GIT_ALLOW_PROTOCOL=ext, with ext allowed in config, still can't enable it", async () => {
  const marker = `${sb.root}/marker-c`;
  const { code, result } = await installJson(["https://evil.example/plugin.git"], { GIT_CONFIG_GLOBAL: await rewritingConfig("c", marker), GIT_ALLOW_PROTOCOL: "ext:https", ...enableExt });
  expect(code).toBe(1);
  expect(result).toMatchObject({ installed: false, stage: "clone" });
  await Bun.sleep(300);
  await nothingFrom("plugin", marker);
}, 30000);

test("(d) an unknown remote helper URL is refused before git runs", async () => {
  for (const url of ["foo::bar", "fd::3"]) {
    const { code, result } = await installJson([url]);
    expect(code).toBe(1);
    expect(result).toMatchObject({ installed: false, stage: "source" });
  }
  expect(await staging()).toEqual([]);
}, 20000);

test("(e) GIT_DIR and GIT_WORK_TREE pointing at another repository don't redirect the install, and leave that repository alone", async () => {
  const bystander = `${sb.root}/bystander`;
  await Bun.$`mkdir -p ${bystander}`.quiet();
  await git(bystander, "init", "--quiet");
  await Bun.write(`${bystander}/README`, "mine");
  await git(bystander, "add", "-A");
  await git(bystander, "commit", "--quiet", "-m", "mine");
  const before = { head: (await Bun.$`git -C ${bystander} rev-parse HEAD`.text()).trim(), refs: await Bun.$`git -C ${bystander} for-each-ref`.text(), files: await Bun.$`ls -A ${bystander}`.text() };

  const routed = await repo("routed");
  const { code, result } = await installJson([routed.url], { GIT_DIR: `${bystander}/.git`, GIT_WORK_TREE: bystander });
  expect(code).toBe(0);
  expect(result).toMatchObject({ installed: true, name: "routed", commit: routed.head });
  expect(await exists(`${result.checkout}/.git`)).toBe(true);
  expect(await exists(`${result.checkout}/plugin.json`)).toBe(true);
  expect((await Bun.$`git -C ${bystander} rev-parse HEAD`.text()).trim()).toBe(before.head);
  expect(await Bun.$`git -C ${bystander} for-each-ref`.text()).toBe(before.refs);
  expect(await Bun.$`ls -A ${bystander}`.text()).toBe(before.files);
  expect((await Bun.$`git -C ${bystander} status --porcelain`.text()).trim()).toBe("");
}, 30000);
