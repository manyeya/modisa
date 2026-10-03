// How `modisa plugin install` (and update, and marketplaces) runs git: which URLs it takes, which transports git may
// use, and the environment and settings every installer git call gets. No imports, so tests can check the policy
// without loading the CLI.

// The git transports install uses. Anything else, a remote helper above all (ext::, fd::, <helper>::), could run a
// program while cloning, before the plugin is checked.
export const TRANSPORTS = "https:ssh:git:file";

// Why install won't fetch from `url`, if it won't: https://, ssh://, git:// and file:// URLs, and scp-style
// user@host:path, and nothing else — no <helper>:: URLs, no plain http, no bare local paths.
export function sourceProblem(url: string) {
  if (url.startsWith("-")) return `not a git URL: ${url}`;
  if (/^[A-Za-z0-9+.-]*::/.test(url)) return "a remote-helper URL (<helper>::…) can run programs while cloning: use an https, ssh, git or file URL";
  const scheme = /^([A-Za-z][A-Za-z0-9+.-]*):\/\//.exec(url)?.[1]?.toLowerCase();
  if (scheme) return ["https", "ssh", "git", "file"].includes(scheme) ? undefined : scheme === "http" ? "plain http isn't used: use https" : `${scheme}:// isn't a supported git transport: use https, ssh, git or file`;
  if (/^[A-Za-z0-9._-]+@[A-Za-z0-9.-]+:[^\s]+$/.test(url)) return undefined; // user@host:path
  return `not a git URL install supports: ${url} (use https://, ssh://, git://, file://, or user@host:path)`;
}

// The environment git runs in for an install:
// - stripped, so nothing points installer git at another repository: GIT_DIR, GIT_WORK_TREE, GIT_INDEX_FILE,
//   GIT_OBJECT_DIRECTORY, GIT_ALTERNATE_OBJECT_DIRECTORIES, GIT_COMMON_DIR, GIT_NAMESPACE, GIT_CEILING_DIRECTORIES,
//   GIT_DISCOVERY_ACROSS_FILESYSTEM;
// - stripped, so no inherited setting re-enables a transport or a hook: GIT_CONFIG_PARAMETERS, GIT_CONFIG_COUNT and
//   GIT_CONFIG_KEY_n / GIT_CONFIG_VALUE_n, and GIT_ALLOW_PROTOCOL, which is set to TRANSPORTS instead (it overrides any
//   protocol.*.allow and applies after url.*.insteadOf rewrites);
// - kept, so authenticated transports work as in the user's own git: HOME, PATH, the global and system config (and
//   GIT_CONFIG_GLOBAL / GIT_CONFIG_SYSTEM / GIT_CONFIG_NOSYSTEM) with its credential helpers, SSH_AUTH_SOCK,
//   GIT_SSH / GIT_SSH_COMMAND, and proxy variables. GIT_TERMINAL_PROMPT=0: never prompt.
const ROUTING = ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_OBJECT_DIRECTORY", "GIT_ALTERNATE_OBJECT_DIRECTORIES", "GIT_COMMON_DIR", "GIT_NAMESPACE", "GIT_CEILING_DIRECTORIES", "GIT_DISCOVERY_ACROSS_FILESYSTEM"];
export function gitEnv(env: Record<string, string | undefined>) {
  const out: Record<string, string> = {};
  for (const [k, v] of Object.entries(env)) {
    if (v === undefined || ROUTING.includes(k) || k === "GIT_CONFIG_PARAMETERS" || k === "GIT_ALLOW_PROTOCOL" || /^GIT_CONFIG_(COUNT|KEY_\d+|VALUE_\d+)$/.test(k)) continue;
    out[k] = v;
  }
  return { ...out, GIT_ALLOW_PROTOCOL: TRANSPORTS, GIT_TERMINAL_PROMPT: "0" };
}

// Settings on every installer git call: ext refused outright, and no hooks, fsmonitor or submodules. It isn't a
// sandbox: ssh and credential helpers still run as the user's own git would run them.
export const SAFE_GIT = ["-c", "protocol.ext.allow=never", "-c", "core.hooksPath=/dev/null", "-c", "core.fsmonitor=false", "-c", "submodule.recurse=false"];

// Every installer git call (plugins and marketplaces): argv values only (never a shell), with the environment and
// settings above. Async, so a slow clone in the server never holds up its other requests.
export async function git(args: string[], cwd?: string) {
  const p = Bun.spawn(["git", ...SAFE_GIT, ...args], { cwd, env: gitEnv(Bun.env), stdout: "pipe", stderr: "pipe" });
  const [out, err] = await Promise.all([new Response(p.stdout).text(), new Response(p.stderr).text()]);
  return { code: await p.exited, out: out.trim(), err: err.trim() };
}

// A fresh clone at `cwd` moved to `ref` (a tag, a branch or a commit), detached; no ref: the default branch it cloned.
export async function checkoutRef(cwd: string, ref: string | null, source: string): Promise<{ commit?: string; reason?: string }> {
  if (!ref) return { commit: (await git(["rev-parse", "HEAD"], cwd)).out };
  if (ref.startsWith("-")) return { reason: `not a ref: ${ref}` };
  const resolve = async (name: string) => (await git(["rev-parse", "--verify", "--quiet", `${name}^{commit}`], cwd)).out;
  const resolved = (await resolve(ref)) || (await resolve(`origin/${ref}`));
  if (!resolved) return { reason: `no branch, tag or commit ${ref} in ${source}` };
  const switched = await git(["checkout", "--quiet", "--detach", resolved], cwd);
  if (switched.code !== 0) return { reason: switched.err || `couldn't check out ${ref}` };
  return { commit: resolved };
}

// The commit `ref` is at now in `source` (no ref: the source's HEAD), fetched into the checkout at `cwd`: a branch's
// latest, a tag (moved or not), or a commit. The URL is given, never read from the checkout's own config.
export async function fetchLatest(cwd: string, source: string, ref: string | null): Promise<{ commit?: string; reason?: string }> {
  const problem = sourceProblem(source);
  if (problem) return { reason: problem };
  const fetched = ref
    ? await git(["fetch", "--quiet", "--force", "--tags", "--", source, "+refs/heads/*:refs/remotes/origin/*"], cwd)
    : await git(["fetch", "--quiet", "--", source, "HEAD"], cwd);
  if (fetched.code !== 0) return { reason: fetched.err || "git fetch failed" };
  for (const name of ref ? [`refs/remotes/origin/${ref}`, `refs/tags/${ref}`, ref] : ["FETCH_HEAD"]) {
    const commit = (await git(["rev-parse", "--verify", "--quiet", `${name}^{commit}`], cwd)).out;
    if (commit) return { commit };
  }
  return { reason: `no branch, tag or commit ${ref} in ${source}` };
}

// The commit `ref` (no ref: HEAD) is at in `source` now, read with ls-remote: nothing is fetched or written. null:
// an abbreviated commit, which only a fetch can tell.
export async function remoteCommit(source: string, ref: string | null): Promise<{ commit?: string | null; reason?: string }> {
  const problem = sourceProblem(source);
  if (problem) return { reason: problem };
  if (ref && /^[0-9a-f]{40}$/.test(ref)) return { commit: ref };
  const names = ref ? [`refs/tags/${ref}^{}`, `refs/tags/${ref}`, `refs/heads/${ref}`] : ["HEAD"];
  const listed = await git(["ls-remote", "--", source, ...names]);
  if (listed.code !== 0) return { reason: listed.err || "git ls-remote failed" };
  const refs = new Map(listed.out.split("\n").map((l) => l.split("\t")).map(([sha, name]) => [name, sha]));
  const commit = names.map((n) => refs.get(n)).find(Boolean);
  if (commit) return { commit };
  return ref && /^[0-9a-f]{4,39}$/.test(ref) ? { commit: null } : { reason: `no branch or tag ${ref ?? "HEAD"} in ${source}` };
}
