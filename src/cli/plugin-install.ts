// `shepherd plugin install <git-url> [--ref r] [--subdir d]`, and unlinking what it installed.
//
// An install clones into a staging directory, resolves the ref to a commit, checks the directory and plugin.json, and
// only then takes the name: the checkout moves to <state>/plugins-src/<name>, beside shepherd's install record, and is
// linked and started exactly like `plugin link`. No dependencies are installed and no build scripts run; the plugin's
// own entrypoint starts in the running session reached. A failure before the name is taken leaves nothing behind.
import { socketPath } from "../core/paths";
import { MANAGED_DIR, PLUGINS_DIR, readInstall, readManifest, withoutCredentials, type InstallRecord } from "../config/plugins";
import { connectExisting, connectUnix, runningPid } from "../protocol/transport";
import { DIR } from "../core/paths";
import { describeStart, sessionName, startIn, type StartOutcome } from "./plugin";

type Stage = "git" | "clone" | "ref" | "subdir" | "manifest" | "collision";
type InstallResult = {
  installed: boolean;
  alreadyInstalled?: boolean;
  name?: string;
  source: string;
  ref: string | null;
  commit?: string;
  checkout?: string;
  dir?: string;
  start?: StartOutcome;
  hints?: string[];
  stage?: Stage;
  reason?: string;
};

// git with argv values only (never a shell), and never prompting for credentials
async function git(args: string[], cwd?: string) {
  const p = Bun.spawn(["git", ...args], { cwd, env: { ...Bun.env, GIT_TERMINAL_PROMPT: "0" }, stdout: "pipe", stderr: "pipe" });
  const [out, err] = await Promise.all([new Response(p.stdout).text(), new Response(p.stderr).text()]);
  return { code: await p.exited, out: out.trim(), err: err.trim() };
}
const real = async (path: string) => (await Bun.$`realpath ${path}`.quiet().nothrow().text()).trim();
const inside = (path: string, root: string) => path === root || path.startsWith(`${root}/`);
const isDir = async (path: string) => (await Bun.$`test -d ${path}`.quiet().nothrow()).exitCode === 0;

// setup shepherd doesn't do for you; its own wording, never commands from the plugin
async function setupHints(dir: string, name: string) {
  const pkg = await Bun.file(`${dir}/package.json`).json().catch(() => undefined);
  const deps = pkg ? Object.keys({ ...pkg.dependencies, ...pkg.devDependencies }).length : 0;
  return deps && !(await isDir(`${dir}/node_modules`)) ? [`it has package.json dependencies, which install doesn't fetch: run bun install in ${dir}, then shepherd plugin start ${name}`] : [];
}

function report(result: InstallResult, json: boolean) {
  if (json) console.log(JSON.stringify(result, null, 2));
  else if (!result.installed && !result.alreadyInstalled) console.error(`shepherd: not installed (${result.stage}): ${result.reason}`);
  else {
    const at = `${result.ref ? `ref ${result.ref}, ` : ""}commit ${result.commit}`;
    console.log(`${result.alreadyInstalled ? "already installed" : "installed"} ${result.name} from ${result.source} (${at})\n  ${result.dir}`);
    if (result.start) console.log(describeStart(result.start, result.alreadyInstalled ? "already installed" : "installed"));
    for (const hint of result.hints ?? []) console.log(`note: ${hint}`);
  }
  const started = !result.start || ["started", "already-running", "not-started"].includes(result.start.state);
  return (result.installed || result.alreadyInstalled) && started ? 0 : 1;
}

export async function install(url: string | undefined, options: { ref?: string; subdir?: string; session?: string; json: boolean }): Promise<number> {
  if (!url) {
    console.error("usage: shepherd plugin install <git-url> [--ref branch|tag|commit] [--subdir path] [--json]");
    return 2;
  }
  const source = withoutCredentials(url);
  const ref = options.ref ?? null;
  const { json } = options;
  let staging: string | undefined;
  const failed = async (stage: Stage, reason: string) => {
    if (staging) await Bun.$`rm -rf ${staging}`.quiet().nothrow(); // this attempt's files only
    return report({ installed: false, source, ref, stage, reason: withoutCredentials(reason) }, json);
  };

  if (!json) console.log(`installing from ${source}. A plugin runs as you, with your files and network; it isn't sandboxed.`);
  if (!Bun.which("git")) return failed("git", "git isn't installed");
  if (ref?.startsWith("-")) return failed("ref", `not a ref: ${ref}`);
  const subdir = options.subdir?.replace(/\/+$/, "") || null;
  if (subdir && (subdir.startsWith("/") || subdir.split("/").includes(".."))) return failed("subdir", `--subdir must be a relative path inside the repository: ${subdir}`);

  await Bun.$`mkdir -p ${MANAGED_DIR}`.quiet();
  staging = (await Bun.$`mktemp -d ${`${MANAGED_DIR}/.staging-XXXXXX`}`.text()).trim();
  const checkout = `${staging}/checkout`;
  const cloned = await git(["clone", "--quiet", "--", url, checkout]);
  if (cloned.code !== 0) return failed("clone", cloned.err || "git clone failed");
  await git(["remote", "set-url", "origin", source], checkout); // no credentials left in the checkout's own config

  let commit = (await git(["rev-parse", "HEAD"], checkout)).out;
  if (ref) {
    const resolve = async (name: string) => (await git(["rev-parse", "--verify", "--quiet", `${name}^{commit}`], checkout)).out;
    const resolved = (await resolve(ref)) || (await resolve(`origin/${ref}`));
    if (!resolved) return failed("ref", `no branch, tag or commit ${ref} in ${source}`);
    const switched = await git(["checkout", "--quiet", "--detach", resolved], checkout);
    if (switched.code !== 0) return failed("ref", switched.err || `couldn't check out ${ref}`);
    commit = resolved;
  }

  // the plugin's directory, and its plugin.json, must really be inside the checkout (no .., no symlink out)
  const root = await real(checkout);
  const dir = await real(subdir ? `${checkout}/${subdir}` : checkout);
  if (!dir || !inside(dir, root) || !(await isDir(dir))) return failed("subdir", `${subdir ?? "."} isn't a directory inside the repository`);
  const manifestFile = await real(`${dir}/plugin.json`);
  if (manifestFile && !inside(manifestFile, root)) return failed("manifest", "plugin.json points outside the repository");
  const { manifest, error } = await readManifest(dir);
  if (!manifest) return failed("manifest", error!);
  const name = manifest.name;

  const existing = await readInstall(name);
  const link = `${PLUGINS_DIR}/${name}`;
  if (existing && existing.source === source && existing.subdir === subdir) {
    await Bun.$`rm -rf ${staging}`.quiet().nothrow();
    const start = await startIn(options.session, name);
    return report({ installed: false, alreadyInstalled: true, name, source, ref: existing.ref, commit: existing.commit, checkout: existing.checkout, dir: existing.dir, start, hints: [] }, json);
  }
  const linkedTo = (await Bun.$`readlink ${link}`.quiet().nothrow().text()).trim();
  if (existing) return failed("collision", `${name} is already installed from ${existing.source}; shepherd plugin unlink ${name} first`);
  if (linkedTo) return failed("collision", `${name} is already linked to ${linkedTo}; shepherd plugin unlink ${name} first`);

  // Take the name. mkdir and ln -s both refuse to overwrite, so two installs racing for it can't both get it.
  const home = `${MANAGED_DIR}/${name}`;
  if ((await Bun.$`mkdir ${home}`.quiet().nothrow()).exitCode !== 0) return failed("collision", `${name} is already being installed, or left behind at ${home}`);
  await Bun.$`mv ${checkout} ${home}/checkout`.quiet();
  await Bun.$`rm -rf ${staging}`.quiet().nothrow();
  staging = undefined;
  const finalCheckout = await real(`${home}/checkout`);
  const finalDir = await real(subdir ? `${finalCheckout}/${subdir}` : finalCheckout);
  const record: InstallRecord = { name, source, ref, commit, checkout: finalCheckout, dir: finalDir, subdir, installedAt: new Date().toISOString() };
  await Bun.write(`${home}/install.json`, JSON.stringify(record, null, 2) + "\n");
  await Bun.$`mkdir -p ${PLUGINS_DIR}`.quiet();
  if ((await Bun.$`ln -s ${finalDir} ${link}`.quiet().nothrow()).exitCode !== 0) {
    await Bun.$`rm -rf ${home}`.quiet().nothrow(); // ours: taken above, nothing else uses it
    return failed("collision", `${name} was linked by something else during the install`);
  }

  const hints = await setupHints(finalDir, name);
  const start = await startIn(options.session, name);
  return report({ installed: true, name, source, ref, commit, checkout: finalCheckout, dir: finalDir, start, hints }, json);
}

// running sessions shepherd can see: each socket in the state directory (as `shepherd ls` finds them)
async function sessions() {
  return (await Bun.$`ls -1 ${DIR}`.quiet().nothrow().text()).split("\n").filter((f) => f.endsWith(".sock")).map((f) => f.slice(0, -5)).sort();
}

export async function unlinkPlugin(name: string | undefined, session: string | undefined, json: boolean): Promise<number> {
  if (!name) {
    console.error("usage: shepherd plugin unlink <name> [--json]");
    return 2;
  }
  const link = `${PLUGINS_DIR}/${name}`;
  if ((await Bun.$`test -L ${link}`.quiet().nothrow()).exitCode !== 0) {
    console.error(`shepherd: no linked plugin named ${name} in ${PLUGINS_DIR}`);
    return 1;
  }
  const record = await readInstall(name);
  const managed = !!record && (await real(link)) === record.dir;
  await Bun.$`rm ${link}`.quiet(); // the link only
  const result = { name, unlinked: true as const, managed, stoppedIn: [] as string[], stillUsing: [] as string[], unreachable: [] as string[], checkout: undefined as { path: string; deleted: boolean } | undefined };

  if (!managed) {
    // a directory you linked: stopped in the session reached, and never deleted
    const where = sessionName(session);
    const conn = await connectExisting(session).catch(() => undefined);
    if (conn && (await conn.request("plugin.stop", { name }).then(() => true, () => false))) result.stoppedIn.push(where);
    conn?.close();
    if (json) console.log(JSON.stringify(result, null, 2));
    else console.log(`unlinked ${name}\n${result.stoppedIn.length ? `stopped it in session ${where}` : `no running copy in session ${where}`}; other running sessions keep theirs until they restart. Its directory is untouched.`);
    return 0;
  }

  // installed by shepherd: stop it everywhere, and delete the checkout only once nothing runs it
  for (const s of await sessions()) {
    const sock = socketPath(s);
    const conn = await connectUnix(sock).catch(() => undefined);
    if (!conn) {
      if (await runningPid(sock)) result.unreachable.push(s);
      continue;
    }
    try {
      if (await conn.request("plugin.stop", { name }).then(() => true, () => false)) result.stoppedIn.push(s);
      const still = (await conn.request<any[]>("plugin.list").catch(() => [])).find((p) => p.name === name && p.group === "running");
      if (still) result.stillUsing.push(s);
    } finally {
      conn.close();
    }
  }
  const keep = result.unreachable.length > 0 || result.stillUsing.length > 0;
  if (!keep) await Bun.$`rm -rf ${MANAGED_DIR}/${name}`.quiet(); // the checkout and install record; never its data or logs
  result.checkout = { path: record!.checkout, deleted: !keep };

  if (json) console.log(JSON.stringify(result, null, 2));
  else {
    console.log(`unlinked ${name}${result.stoppedIn.length ? `; stopped it in session ${result.stoppedIn.join(", ")}` : ""}`);
    if (!keep) console.log(`deleted its checkout (${record!.checkout}); its data and logs are kept`);
    else {
      const why = [...result.stillUsing.map((s) => `session ${s} still runs it`), ...result.unreachable.map((s) => `session ${s} can't be reached (a sandbox?)`)].join("; ");
      console.log(`kept its checkout at ${record!.checkout}: ${why}. Once that's stopped, remove ${MANAGED_DIR}/${name}.`);
    }
  }
  return 0;
}
