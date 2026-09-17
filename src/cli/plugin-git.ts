// How `modisa plugin install` runs git: which URLs it takes, which transports git may use, and the environment and
// settings every installer git call gets. No imports, so tests can check the policy without loading the CLI.

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
