// How `modisa plugin install` (and update, and marketplaces) runs git: which URLs it takes, which transports git may
// use, and the environment and settings every installer git call gets. No modisa imports, so tests can check the
// policy without loading the CLI.
use std::ffi::OsString;
use std::process::Stdio;
use std::sync::LazyLock;

use regex::Regex;

// The git transports install uses. Anything else, a remote helper above all (ext::, fd::, <helper>::), could run a
// program while cloning, before the plugin is checked.
pub const TRANSPORTS: &str = "https:ssh:git:file";

// JavaScript's \s, as a regex class body: Rust's \s has U+0085 and lacks U+FEFF.
pub const JS_SPACE: &str = r"\t\n\x0B\x0C\r \x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}";

static HELPER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9+.-]*::").unwrap());
static SCHEME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([A-Za-z][A-Za-z0-9+.-]*)://").unwrap());
static SCP: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"^[A-Za-z0-9._-]+@[A-Za-z0-9.-]+:[^{JS_SPACE}]+$")).unwrap());

// Why install won't fetch from `url`, if it won't: https://, ssh://, git:// and file:// URLs, and scp-style
// user@host:path, and nothing else — no <helper>:: URLs, no plain http, no bare local paths.
pub fn source_problem(url: &str) -> Option<String> {
    if url.starts_with('-') {
        return Some(format!("not a git URL: {url}"));
    }
    if HELPER.is_match(url) {
        return Some("a remote-helper URL (<helper>::…) can run programs while cloning: use an https, ssh, git or file URL".into());
    }
    if let Some(c) = SCHEME.captures(url) {
        let scheme = c[1].to_lowercase();
        return match scheme.as_str() {
            "https" | "ssh" | "git" | "file" => None,
            "http" => Some("plain http isn't used: use https".into()),
            _ => Some(format!("{scheme}:// isn't a supported git transport: use https, ssh, git or file")),
        };
    }
    if SCP.is_match(url) {
        return None; // user@host:path
    }
    Some(format!("not a git URL install supports: {url} (use https://, ssh://, git://, file://, or user@host:path)"))
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
const ROUTING: &[&str] = &["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_OBJECT_DIRECTORY", "GIT_ALTERNATE_OBJECT_DIRECTORIES", "GIT_COMMON_DIR", "GIT_NAMESPACE", "GIT_CEILING_DIRECTORIES", "GIT_DISCOVERY_ACROSS_FILESYSTEM"];
static CONFIG_N: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^GIT_CONFIG_(COUNT|KEY_\d+|VALUE_\d+)$").unwrap());

pub fn git_env(env: impl IntoIterator<Item = (OsString, OsString)>) -> Vec<(OsString, OsString)> {
    let dropped = |k: &str| ROUTING.contains(&k) || k == "GIT_CONFIG_PARAMETERS" || k == "GIT_ALLOW_PROTOCOL" || k == "GIT_TERMINAL_PROMPT" || CONFIG_N.is_match(k);
    let mut out: Vec<(OsString, OsString)> = env.into_iter().filter(|(k, _)| !k.to_str().is_some_and(dropped)).collect();
    out.push(("GIT_ALLOW_PROTOCOL".into(), TRANSPORTS.into()));
    out.push(("GIT_TERMINAL_PROMPT".into(), "0".into()));
    out
}

// Settings on every installer git call: ext refused outright, and no hooks, fsmonitor or submodules. It isn't a
// sandbox: ssh and credential helpers still run as the user's own git would run them.
pub const SAFE_GIT: &[&str] = &["-c", "protocol.ext.allow=never", "-c", "core.hooksPath=/dev/null", "-c", "core.fsmonitor=false", "-c", "submodule.recurse=false"];

pub struct GitOut {
    pub code: i32,
    pub out: String,
    pub err: String,
}

// Every installer git call (plugins and marketplaces): argv values only (never a shell), with the environment and
// settings above. Async, so a slow clone in the server never holds up its other requests.
pub async fn git(args: &[&str], cwd: Option<&str>) -> GitOut {
    let mut cmd = tokio::process::Command::new("git");
    cmd.args(SAFE_GIT).args(args).env_clear().envs(git_env(std::env::vars_os())).stdin(Stdio::null());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    match cmd.output().await {
        Ok(o) => GitOut { code: o.status.code().unwrap_or(-1), out: String::from_utf8_lossy(&o.stdout).trim().into(), err: String::from_utf8_lossy(&o.stderr).trim().into() },
        Err(e) => GitOut { code: -1, out: String::new(), err: e.to_string() },
    }
}

// a ref as JavaScript tests one: "" is none
fn given(r#ref: Option<&str>) -> Option<&str> {
    r#ref.filter(|r| !r.is_empty())
}

// A fresh clone at `cwd` moved to `ref` (a tag, a branch or a commit), detached; no ref: the default branch it cloned.
// The commit, or why not.
pub async fn checkout_ref(cwd: &str, r#ref: Option<&str>, source: &str) -> Result<String, String> {
    let Some(r) = given(r#ref) else {
        let head = git(&["rev-parse", "HEAD"], Some(cwd)).await;
        // ponytail: the original went on with an empty commit here (git itself failing); this says so instead
        return if head.out.is_empty() { Err(if head.err.is_empty() { "git rev-parse HEAD failed".into() } else { head.err }) } else { Ok(head.out) };
    };
    if r.starts_with('-') {
        return Err(format!("not a ref: {r}"));
    }
    let resolve = |name: String| async move { git(&["rev-parse", "--verify", "--quiet", &format!("{name}^{{commit}}")], Some(cwd)).await.out };
    let mut resolved = resolve(r.to_string()).await;
    if resolved.is_empty() {
        resolved = resolve(format!("origin/{r}")).await;
    }
    if resolved.is_empty() {
        return Err(format!("no branch, tag or commit {r} in {source}"));
    }
    let switched = git(&["checkout", "--quiet", "--detach", &resolved], Some(cwd)).await;
    if switched.code != 0 {
        return Err(if switched.err.is_empty() { format!("couldn't check out {r}") } else { switched.err });
    }
    Ok(resolved)
}

// The commit `ref` is at now in `source` (no ref: the source's HEAD), fetched into the checkout at `cwd`: a branch's
// latest, a tag (moved or not), or a commit. The URL is given, never read from the checkout's own config.
pub async fn fetch_latest(cwd: &str, source: &str, r#ref: Option<&str>) -> Result<String, String> {
    if let Some(problem) = source_problem(source) {
        return Err(problem);
    }
    let r = given(r#ref);
    let fetched = match r {
        Some(_) => git(&["fetch", "--quiet", "--force", "--tags", "--", source, "+refs/heads/*:refs/remotes/origin/*"], Some(cwd)).await,
        None => git(&["fetch", "--quiet", "--", source, "HEAD"], Some(cwd)).await,
    };
    if fetched.code != 0 {
        return Err(if fetched.err.is_empty() { "git fetch failed".into() } else { fetched.err });
    }
    let names = match r {
        Some(r) => vec![format!("refs/remotes/origin/{r}"), format!("refs/tags/{r}"), r.to_string()],
        None => vec!["FETCH_HEAD".to_string()],
    };
    for name in names {
        let commit = git(&["rev-parse", "--verify", "--quiet", &format!("{name}^{{commit}}")], Some(cwd)).await.out;
        if !commit.is_empty() {
            return Ok(commit);
        }
    }
    Err(format!("no branch, tag or commit {} in {source}", r#ref.unwrap_or("null")))
}

static FULL_COMMIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9a-f]{40}$").unwrap());
static SHORT_COMMIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9a-f]{4,39}$").unwrap());

// The commit `ref` (no ref: HEAD) is at in `source` now, read with ls-remote: nothing is fetched or written. None: an
// abbreviated commit, which only a fetch can tell.
pub async fn remote_commit(source: &str, r#ref: Option<&str>) -> Result<Option<String>, String> {
    if let Some(problem) = source_problem(source) {
        return Err(problem);
    }
    let r = given(r#ref);
    if let Some(r) = r.filter(|r| FULL_COMMIT.is_match(r)) {
        return Ok(Some(r.to_string()));
    }
    let names = match r {
        Some(r) => vec![format!("refs/tags/{r}^{{}}"), format!("refs/tags/{r}"), format!("refs/heads/{r}")],
        None => vec!["HEAD".to_string()],
    };
    let mut args = vec!["ls-remote", "--", source];
    args.extend(names.iter().map(String::as_str));
    let listed = git(&args, None).await;
    if listed.code != 0 {
        return Err(if listed.err.is_empty() { "git ls-remote failed".into() } else { listed.err });
    }
    // name → commit; a later line for a name wins, as a Map built from them would have it
    let mut refs = std::collections::HashMap::new();
    for line in listed.out.split('\n') {
        let mut cells = line.split('\t');
        if let (Some(sha), Some(name)) = (cells.next(), cells.next()) {
            refs.insert(name.to_string(), sha.to_string());
        }
    }
    if let Some(commit) = names.iter().filter_map(|n| refs.get(n)).find(|c| !c.is_empty()) {
        return Ok(Some(commit.clone()));
    }
    match r {
        Some(r) if SHORT_COMMIT.is_match(r) => Ok(None),
        _ => Err(format!("no branch or tag {} in {source}", r.unwrap_or("HEAD"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_fetches_only_https_ssh_git_file_and_scp_urls() {
        for ok in ["https://github.com/a/b.git", "HTTPS://github.com/a/b", "ssh://git@example.com:2222/a/b.git", "git://example.com/a/b.git", "file:///tmp/repo.git", "git@github.com:a/b.git", "me@build-01.internal:plugins/x.git"] {
            assert_eq!(source_problem(ok), None, "{ok}");
        }
        let refused = |url: &str| source_problem(url).unwrap_or_default();
        assert!(refused("ext::sh -c touch% /tmp/pwned").contains("remote-helper"));
        assert!(refused("fd::3").contains("remote-helper"));
        assert!(refused("foo::bar").contains("remote-helper"));
        assert!(refused("::bar").contains("remote-helper"));
        assert!(refused("http://github.com/a/b").contains("use https"));
        assert!(refused("ftp://example.com/a.git").contains("isn't a supported git transport"));
        assert!(refused("/tmp/repo.git").contains("not a git URL"));
        assert!(refused("./repo").contains("not a git URL"));
        assert!(refused("--upload-pack=touch /tmp/x").contains("not a git URL"));
        assert!(refused("git@github.com:a/b.git with spaces").contains("not a git URL"));
    }

    #[test]
    fn installer_git_never_inherits_routing_injected_config_or_an_allowlist() {
        let given = [
            ("GIT_DIR", "/elsewhere/.git"), ("GIT_WORK_TREE", "/elsewhere"), ("GIT_INDEX_FILE", "x"), ("GIT_OBJECT_DIRECTORY", "x"), ("GIT_ALTERNATE_OBJECT_DIRECTORIES", "x"),
            ("GIT_COMMON_DIR", "x"), ("GIT_NAMESPACE", "x"), ("GIT_CEILING_DIRECTORIES", "x"), ("GIT_DISCOVERY_ACROSS_FILESYSTEM", "1"),
            ("GIT_CONFIG_PARAMETERS", "'protocol.ext.allow'='always'"), ("GIT_CONFIG_COUNT", "1"), ("GIT_CONFIG_KEY_0", "protocol.ext.allow"), ("GIT_CONFIG_VALUE_0", "always"),
            ("GIT_ALLOW_PROTOCOL", "ext"), ("GIT_TERMINAL_PROMPT", "1"),
            ("HOME", "/home/me"), ("PATH", "/usr/bin"), ("SSH_AUTH_SOCK", "/tmp/agent.sock"), ("GIT_SSH_COMMAND", "ssh -i key"), ("GIT_SSH", "/usr/bin/ssh"),
            ("GIT_CONFIG_GLOBAL", "/home/me/.gitconfig"), ("HTTPS_PROXY", "http://proxy:3128"), ("NO_PROXY", "localhost"),
        ];
        let env: std::collections::HashMap<String, String> = git_env(given.iter().map(|(k, v)| (OsString::from(k), OsString::from(v)))).into_iter().map(|(k, v)| (k.into_string().unwrap(), v.into_string().unwrap())).collect();
        for k in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_OBJECT_DIRECTORY", "GIT_ALTERNATE_OBJECT_DIRECTORIES", "GIT_COMMON_DIR", "GIT_NAMESPACE", "GIT_CEILING_DIRECTORIES", "GIT_DISCOVERY_ACROSS_FILESYSTEM", "GIT_CONFIG_PARAMETERS", "GIT_CONFIG_COUNT", "GIT_CONFIG_KEY_0", "GIT_CONFIG_VALUE_0"] {
            assert!(!env.contains_key(k), "{k}");
        }
        for (k, v) in [("GIT_ALLOW_PROTOCOL", TRANSPORTS), ("GIT_TERMINAL_PROMPT", "0"), ("HOME", "/home/me"), ("PATH", "/usr/bin"), ("SSH_AUTH_SOCK", "/tmp/agent.sock"), ("GIT_SSH_COMMAND", "ssh -i key"), ("GIT_SSH", "/usr/bin/ssh"), ("GIT_CONFIG_GLOBAL", "/home/me/.gitconfig"), ("HTTPS_PROXY", "http://proxy:3128"), ("NO_PROXY", "localhost")] {
            assert_eq!(env.get(k).map(String::as_str), Some(v), "{k}");
        }
        assert!(!TRANSPORTS.split(':').any(|t| t == "ext"));
    }
}
