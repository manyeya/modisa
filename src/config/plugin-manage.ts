// Installing, updating and unlinking plugins, as results: the CLI prints them (cli/plugin-install.ts) and the server
// answers the plugin manager's requests with them (server/plugin-manager.ts). Each acts in the running sessions it
// reaches: over their sockets, or, in a session's own server, straight through its plugin host (`Here`).
//
// An install clones into a staging directory, resolves the ref to a commit, checks the directory and plugin.json, and
// only then takes the name: the checkout moves to <state>/plugins-src/<name>, beside modisa's install record, and is
// linked exactly like `plugin link`, for the caller to start. No dependencies are installed and no build scripts run.
// A failure before the name is taken leaves nothing behind.
import type { z } from "zod";
import { DIR, socketPath } from "../core/paths";
import { checkoutRef, fetchLatest, git, remoteCommit, sourceProblem } from "../cli/plugin-git";
import { fail } from "../protocol/conn";
import { connectExisting, connectUnix, runningPid } from "../protocol/transport";
import type { pluginInstall, pluginStart, pluginUnlink, pluginUpdate } from "../protocol/schema";
import { MANAGED_DIR, PLUGINS_DIR, inside, isDir, readInstall, readManifest, real, withoutCredentials, writeInstall, type InstallRecord } from "./plugins";
import { MARKETPLACES_DIR, resolveEntry, updateMarketplaces } from "./marketplaces";

export type StartOutcome = z.infer<typeof pluginStart>;
export type InstallResult = z.infer<typeof pluginInstall>;
export type UnlinkResult = z.infer<typeof pluginUnlink>;
export type UpdateResult = z.infer<typeof pluginUpdate>;

// Requests to a session, and the session they reach
export type Ask = <T = any>(method: string, params?: object) => Promise<T>;
export type Here = { session: string; ask: Ask };

export const sessionName = (session?: string) => session ?? Bun.env.MODISA_SESSION ?? "default";
// plugin.json names are ids, so a name that isn't one never makes it into a path
const PLUGIN_NAME = /^[a-z0-9][a-z0-9-]*$/;
const named = (name: string) => {
  if (!PLUGIN_NAME.test(name)) throw fail("no_such_plugin", `no plugin named ${JSON.stringify(name.slice(0, 64))}: a plugin's name is lowercase letters, digits and dashes`);
};

// ---------- starting ----------
const HELLO_MS = 10_000;
// "<key>: <why>" for each of a plugin's keys that's off in that session
const offKeys = (status: any): string[] => (status?.keys ?? []).filter((k: any) => k.state === "disabled").map((k: any) => `${k.key || "(none)"}: ${k.reason}`);

// Start a linked plugin in a session, and wait for it to connect: a process that started isn't a plugin that's ready.
export async function startWith(ask: Ask, where: string, name: string): Promise<StartOutcome> {
  const listed = async () => (await ask<any[]>("plugin.list")).find((p) => p.name === name);
  let status: any;
  try {
    status = await ask("plugin.start", { name });
  } catch (e) {
    const code = (e as { code?: string }).code;
    if (code === "already_running") {
      const s = await listed();
      return { session: where, state: "already-running", pid: s?.pid, log: s?.log, disabledKeys: offKeys(s) };
    }
    return { session: where, state: "failed", reason: (e as Error).message };
  }
  for (const end = Date.now() + HELLO_MS; ; await Bun.sleep(200)) {
    const s = (await listed()) ?? status;
    if (s.connected) return { session: where, state: "started", pid: s.pid, log: s.log, disabledKeys: offKeys(s) };
    if (s.status !== "running" && s.status !== "starting") return { session: where, state: "failed", reason: s.error ?? `it ${s.status}`, pid: s.pid, log: s.log };
    if (Date.now() > end) return { session: where, state: "no-hello", reason: `it started but didn't connect within ${HELLO_MS / 1000}s`, pid: s.pid, log: s.log };
  }
}

// stopped and started again where it runs; nothing where it doesn't
async function restartWith(ask: Ask, where: string, name: string) {
  const running = (await ask<any[]>("plugin.list").catch(() => [])).some((p) => p.name === name && (p.status === "running" || p.status === "starting"));
  if (!running) return undefined;
  await ask("plugin.stop", { name }).catch(() => {});
  return startWith(ask, where, name);
}

// running sessions modisa can see: each socket in the state directory (as `modisa ls` finds them)
async function sessions() {
  return (await Bun.$`ls -1 ${DIR}`.quiet().nothrow().text()).split("\n").filter((f) => f.endsWith(".sock")).map((f) => f.slice(0, -5)).sort();
}

// `act` in every running session: `here` through its own host, the rest over their sockets. Returns those running
// but unreachable (a sandbox?).
async function everySession(here: Here | undefined, act: (session: string, ask: Ask) => Promise<void>) {
  const unreachable: string[] = [];
  for (const s of new Set([...(here ? [here.session] : []), ...(await sessions())])) {
    if (s === here?.session) {
      await act(s, here.ask);
      continue;
    }
    const sock = socketPath(s);
    const conn = await connectUnix(sock).catch(() => undefined);
    if (!conn) {
      if (await runningPid(sock)) unreachable.push(s);
      continue;
    }
    try {
      await act(s, (method, params) => conn.request(method, params));
    } finally {
      conn.close();
    }
  }
  return unreachable;
}

// ---------- installing ----------
type Stage = NonNullable<InstallResult["stage"]>;
// a git URL with a ref and a subdir, or a marketplace's entry (name@marketplace), which has its own
export type InstallFrom = { source?: string; marketplacePlugin?: string; ref?: string; subdir?: string };

// setup modisa doesn't do for you; its own wording, never commands from the plugin
async function setupHints(dir: string, name: string) {
  const pkg = await Bun.file(`${dir}/package.json`).json().catch(() => undefined);
  const deps = pkg ? Object.keys({ ...pkg.dependencies, ...pkg.devDependencies }).length : 0;
  return deps && !(await isDir(`${dir}/node_modules`)) ? [`it has package.json dependencies, which install doesn't fetch: run bun install in ${dir}, then modisa plugin start ${name}`] : [];
}

// The plugin's directory, and its plugin.json, must really be inside the checkout (no .., no symlink out)
async function checked(checkout: string, subdir: string | null): Promise<{ stage: "subdir" | "manifest"; reason: string } | { dir: string; name: string }> {
  const root = await real(checkout);
  const dir = await real(subdir ? `${checkout}/${subdir}` : checkout);
  if (!dir || !inside(dir, root) || !(await isDir(dir))) return { stage: "subdir", reason: `${subdir ?? "."} isn't a directory inside the repository` };
  const manifestFile = await real(`${dir}/plugin.json`);
  if (manifestFile && !inside(manifestFile, root)) return { stage: "manifest", reason: "plugin.json points outside the repository" };
  const { manifest, error } = await readManifest(dir);
  if (!manifest) return { stage: "manifest", reason: error! };
  return { dir, name: manifest.name };
}

const target = async (o: InstallFrom) => (o.marketplacePlugin ? resolveEntry(o.marketplacePlugin) : { url: o.source!, ref: o.ref ?? null, subdir: o.subdir ?? null, from: withoutCredentials(o.source!), name: undefined, marketplace: undefined });

// Where an install would fetch from, and the commit its ref is at now: what the user is shown before installing.
// Nothing is fetched or written.
export async function resolvePlugin(o: InstallFrom) {
  const t = await target(o);
  const at = await remoteCommit(t.url, t.ref);
  if (at.reason) throw new Error(withoutCredentials(at.reason));
  return { ...(t.name && { name: t.name }), ...(t.marketplace && { marketplace: t.marketplace }), source: withoutCredentials(t.url), from: t.from, ref: t.ref, subdir: t.subdir, commit: at.commit ?? null };
}

// Install it and link it (not start it). commit: what the user was shown; the install fails if the ref has moved.
// cloning: told the source once it's been checked, before git runs.
export async function installPlugin(o: InstallFrom & { commit?: string; cloning?: (source: string) => void }): Promise<InstallResult> {
  if (!o.marketplacePlugin) return fetchPlugin(o.source!, { ...o, ref: o.ref ?? null, subdir: o.subdir ?? null });
  try {
    const t = await resolveEntry(o.marketplacePlugin);
    return fetchPlugin(t.url, { ...o, ref: t.ref, subdir: t.subdir, name: t.name, marketplace: t.marketplace });
  } catch (e) {
    return { installed: false, source: o.marketplacePlugin, ref: null, stage: "marketplace", reason: (e as Error).message };
  }
}

type FetchOptions = { ref: string | null; subdir: string | null; name?: string; marketplace?: string; commit?: string; cloning?: (source: string) => void };

async function fetchPlugin(url: string, o: FetchOptions): Promise<InstallResult> {
  const source = withoutCredentials(url);
  const { ref } = o;
  const from = o.marketplace ? { marketplace: o.marketplace } : {};
  let staging: string | undefined;
  const failed = async (stage: Stage, reason: string): Promise<InstallResult> => {
    if (staging) await Bun.$`rm -rf ${staging}`.quiet().nothrow(); // this attempt's files only
    return { installed: false, source, ref, stage, reason: withoutCredentials(reason), ...from };
  };

  const unsupported = sourceProblem(url);
  if (unsupported) return failed("source", unsupported);
  o.cloning?.(source);
  if (!Bun.which("git")) return failed("git", "git isn't installed");
  if (ref?.startsWith("-")) return failed("ref", `not a ref: ${ref}`);
  const subdir = o.subdir?.replace(/\/+$/, "") || null;
  if (subdir && (subdir.startsWith("/") || subdir.split("/").includes(".."))) return failed("subdir", `--subdir must be a relative path inside the repository: ${subdir}`);

  await Bun.$`mkdir -p ${MANAGED_DIR}`.quiet();
  staging = (await Bun.$`mktemp -d ${`${MANAGED_DIR}/.staging-XXXXXX`}`.text()).trim();
  const checkout = `${staging}/checkout`;
  const cloned = await git(["clone", "--quiet", "--", url, checkout]);
  if (cloned.code !== 0) return failed("clone", cloned.err || "git clone failed");
  await git(["remote", "set-url", "origin", source], checkout); // no credentials left in the checkout's own config
  const at = await checkoutRef(checkout, ref, source);
  if (!at.commit) return failed("ref", at.reason!);
  const commit = at.commit;
  if (o.commit && commit !== o.commit) return failed("ref", `${ref ?? "the default branch"} of ${source} is at ${commit.slice(0, 7)} now, not ${o.commit.slice(0, 7)} as shown before installing: look at it again`);

  const plugin = await checked(checkout, subdir);
  if ("reason" in plugin) return failed(plugin.stage, plugin.reason);
  const { name } = plugin;
  if (o.name && name !== o.name) return failed("manifest", `marketplace ${o.marketplace} lists it as ${o.name}, but its plugin.json names it ${name}`);

  const existing = await readInstall(name);
  const link = `${PLUGINS_DIR}/${name}`;
  if (existing && existing.source === source && existing.subdir === subdir) {
    await Bun.$`rm -rf ${staging}`.quiet().nothrow();
    return { installed: false, alreadyInstalled: true, name, source, ref: existing.ref, commit: existing.commit, checkout: existing.checkout, dir: existing.dir, hints: [], ...from };
  }
  const linkedTo = (await Bun.$`readlink ${link}`.quiet().nothrow().text()).trim();
  if (existing) return failed("collision", `${name} is already installed from ${existing.source}; modisa plugin unlink ${name} first`);
  if (linkedTo) return failed("collision", `${name} is already linked to ${linkedTo}; modisa plugin unlink ${name} first`);

  // Take the name. mkdir and ln -s both refuse to overwrite, so two installs racing for it can't both get it.
  const home = `${MANAGED_DIR}/${name}`;
  if ((await Bun.$`mkdir ${home}`.quiet().nothrow()).exitCode !== 0) return failed("collision", `${name} is already being installed, or left behind at ${home}`);
  await Bun.$`mv ${checkout} ${home}/checkout`.quiet();
  await Bun.$`rm -rf ${staging}`.quiet().nothrow();
  staging = undefined;
  const finalCheckout = await real(`${home}/checkout`);
  const finalDir = await real(subdir ? `${finalCheckout}/${subdir}` : finalCheckout);
  const record: InstallRecord = { name, source, ref, commit, checkout: finalCheckout, dir: finalDir, subdir, installedAt: new Date().toISOString(), ...from };
  await writeInstall(record);
  await Bun.$`mkdir -p ${PLUGINS_DIR}`.quiet();
  if ((await Bun.$`ln -s ${finalDir} ${link}`.quiet().nothrow()).exitCode !== 0) {
    await Bun.$`rm -rf ${home}`.quiet().nothrow(); // ours: taken above, nothing else uses it
    return failed("collision", `${name} was linked by something else during the install`);
  }
  return { installed: true, name, source, ref, commit, checkout: finalCheckout, dir: finalDir, hints: await setupHints(finalDir, name), ...from };
}

// ---------- updating ----------
// An installed plugin moved to what its ref (none: its source's HEAD) is at now, checked like an install, and
// restarted wherever it was running. A plugin inside a marketplace's repository comes from its checkout here, so that's
// brought up to date first. Anything wrong at the new commit puts the old one back.
export async function updatePlugin(name: string, o: { here?: Here } = {}): Promise<UpdateResult> {
  named(name);
  const linked = await real(`${PLUGINS_DIR}/${name}`);
  if (!linked) throw fail("no_such_plugin", `no linked plugin named ${name} in ${PLUGINS_DIR}`);
  const record = await readInstall(name);
  if (!record || record.dir !== linked) return { name, updated: false, restarted: [], stage: "linked", reason: `linked from ${linked}: update it there` };
  const base = { name, source: record.source, ref: record.ref, from: record.commit, ...(record.marketplace && { marketplace: record.marketplace }) };
  const failed = (stage: NonNullable<UpdateResult["stage"]>, reason: string): UpdateResult => ({ ...base, updated: false, commit: record.commit, restarted: [], stage, reason: withoutCredentials(reason) });

  if (record.marketplace && record.source === `file://${await real(`${MARKETPLACES_DIR}/${record.marketplace}`)}`) {
    const [m] = await updateMarketplaces(record.marketplace).catch((e: Error) => [{ reason: e.message }]);
    if (m && "reason" in m && m.reason) return failed("marketplace", `marketplace ${record.marketplace}: ${m.reason}`);
  }
  const latest = await fetchLatest(record.checkout, record.source, record.ref);
  if (!latest.commit) return failed("fetch", latest.reason!);
  if (latest.commit === record.commit) return { ...base, updated: false, upToDate: true, commit: record.commit, restarted: [] };
  const moved = await git(["checkout", "--quiet", "--detach", latest.commit], record.checkout);
  if (moved.code !== 0) return failed("checkout", moved.err || `couldn't check out ${latest.commit}`);
  const plugin = await checked(record.checkout, record.subdir);
  const problem = "reason" in plugin ? plugin
    : plugin.dir !== record.dir ? { stage: "subdir" as const, reason: `${record.subdir ?? "."} is somewhere else at ${latest.commit.slice(0, 7)}` }
    : plugin.name !== name ? { stage: "manifest" as const, reason: `at ${latest.commit.slice(0, 7)} its plugin.json names it ${plugin.name}: unlink it and install that` }
    : undefined;
  if (problem) {
    await git(["checkout", "--quiet", "--detach", record.commit], record.checkout);
    return failed(problem.stage, problem.reason);
  }
  await writeInstall({ ...record, commit: latest.commit, updatedAt: new Date().toISOString() });
  const restarted: StartOutcome[] = [];
  await everySession(o.here, async (s, ask) => {
    const start = await restartWith(ask, s, name);
    if (start) restarted.push(start);
  });
  return { ...base, updated: true, commit: latest.commit, restarted, hints: await setupHints(record.dir, name) };
}

// ---------- unlinking ----------
// The link goes, so no session starts it again. Installed by modisa: stopped in every running session, then its
// checkout and install record deleted, unless a session still runs it or can't be reached. Linked by you: stopped in
// the session reached (here, else the default or `session`), and never deleted.
export async function unlinkPlugin(name: string, o: { session?: string; here?: Here } = {}): Promise<UnlinkResult> {
  named(name);
  const link = `${PLUGINS_DIR}/${name}`;
  if ((await Bun.$`test -L ${link}`.quiet().nothrow()).exitCode !== 0) throw fail("no_such_plugin", `no linked plugin named ${name} in ${PLUGINS_DIR}`);
  const record = await readInstall(name);
  const managed = !!record && (await real(link)) === record.dir;
  await Bun.$`rm ${link}`.quiet(); // the link only
  const result: UnlinkResult = { name, unlinked: true, managed, stoppedIn: [], stillUsing: [], unreachable: [] };
  const stop = (ask: Ask) => ask("plugin.stop", { name }).then(() => true, () => false);

  if (!managed) {
    const where = o.here?.session ?? sessionName(o.session);
    const conn = o.here ? undefined : await connectExisting(o.session).catch(() => undefined);
    const ask: Ask | undefined = o.here?.ask ?? (conn && ((method, params) => conn.request(method, params)));
    if (ask && (await stop(ask))) result.stoppedIn.push(where);
    conn?.close();
    return result;
  }
  result.unreachable = await everySession(o.here, async (s, ask) => {
    if (await stop(ask)) result.stoppedIn.push(s);
    if ((await ask<any[]>("plugin.list").catch(() => [])).some((p) => p.name === name && p.group === "running")) result.stillUsing.push(s);
  });
  const keep = result.unreachable.length > 0 || result.stillUsing.length > 0;
  if (!keep) await Bun.$`rm -rf ${MANAGED_DIR}/${name}`.quiet(); // the checkout and install record; never its data or logs
  result.checkout = { path: record!.checkout, deleted: !keep };
  return result;
}
