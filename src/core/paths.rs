// Where modisa keeps things, and how it re-runs itself.
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

pub static HOME: LazyLock<String> = LazyLock::new(|| std::env::var("HOME").unwrap_or_else(|_| "/tmp".into()));
pub static DIR: LazyLock<String> = LazyLock::new(|| std::env::var("MODISA_DIR").unwrap_or_else(|_| format!("{}/.local/state/modisa", *HOME)));

pub fn socket_path(session: &str) -> String {
    format!("{}/{session}.sock", *DIR)
}

pub fn cwd() -> String {
    std::env::var("PWD").unwrap_or_else(|_| HOME.clone())
}

// Lexically resolved, like node's path.resolve: `..` and `.` collapse without touching the disk.
pub fn resolve(base: &str, p: &str) -> String {
    let joined = if p.starts_with('/') { PathBuf::from(p) } else { Path::new(base).join(p) };
    let mut out = PathBuf::from("/");
    for c in joined.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::Normal(s) => out.push(s),
            _ => {}
        }
    }
    out.to_string_lossy().into_owned()
}

// A directory as the user typed it (~, ~/x, relative), made absolute where they typed it: the server it's sent to has a
// working directory of its own.
pub fn abs_path(p: &str) -> String {
    let base = std::env::var("PWD").ok().or_else(|| std::env::current_dir().ok().map(|d| d.to_string_lossy().into_owned())).unwrap_or_default();
    let p = if p == "~" { HOME.clone() } else if let Some(rest) = p.strip_prefix("~/") { format!("{}/{rest}", *HOME) } else { p.to_string() };
    resolve(&base, &p)
}

// The command that re-runs this program (server spawn, hooks, plugins).
pub fn self_exe() -> String {
    std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| "modisa".into())
}

// What agents' hooks and plugins run. A release binary prefers the `modisa` on PATH: Homebrew and mise install into a
// directory per version, and a hook pointing there breaks at the next upgrade, while the PATH entry stays put. A build
// from a cargo target directory is a developer's: it runs itself.
// ponytail: trusts that whatever is called modisa on PATH is this program.
pub fn stable_self() -> String {
    let me = self_exe();
    if crate::core::version::FROM_SOURCE {
        return me;
    }
    which("modisa").unwrap_or(me)
}

pub fn which(name: &str) -> Option<String> {
    std::env::var("PATH").ok()?.split(':').map(|d| format!("{d}/{name}")).find(|p| is_executable(p))
}

fn is_executable(p: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
}

// Identifies the code a process runs, so a client can tell its server is out of date.
pub fn code_version() -> String {
    version_of(&self_exe())
}

// What code_version says of the binary at `exe` now: a replaced file (an update, a build) reads differently.
pub fn version_of(exe: &str) -> String {
    let meta = std::fs::metadata(exe).ok();
    let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let modified = meta.and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis()).unwrap_or(0);
    format!("bin-{size}-{modified}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves() {
        assert_eq!(resolve("/a/b", "../c"), "/a/c");
        assert_eq!(resolve("/a/b", "/x/./y"), "/x/y");
        assert_eq!(resolve("/a/b", "."), "/a/b");
    }
}
