// Each space's git state, for the sidebar: the repository its focused pane is in (following the pane's cd), the branch,
// how far it's ahead of and behind its upstream, and how many files have changes. Looked at every few seconds while a
// client is attached, and asked of git only when the space's directory or its panes changed since (else every RECHECK).
// Modisa never fetches, so "behind" is as of your last fetch; a repository too slow to answer in 2s shows nothing; and
// GIT_OPTIONAL_LOCKS=0 keeps `git status` from taking the index lock under your own git commands.
use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Duration;

use crate::core::layout::panes;
use crate::platform::procs;
use crate::protocol::types::GitView;
use crate::server::session::pane::now_ms;
use crate::server::Shared;

const EVERY: Duration = Duration::from_secs(5);

async fn git(cwd: &str, args: &[&str]) -> Option<String> {
    let mut cmd = tokio::process::Command::new("git");
    cmd.arg("-C").arg(cwd).args(args).env("GIT_OPTIONAL_LOCKS", "0").stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).kill_on_drop(true);
    let out = tokio::time::timeout(Duration::from_secs(2), cmd.output()).await.ok()?.ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

// `git status --porcelain=v2 --branch`, read: the branch (a detached HEAD by its short commit), ahead/behind when there's
// an upstream, and a count of changed, staged and untracked files.
pub fn parse_status(top: &str, status: &str) -> GitView {
    let (mut branch, mut oid, mut changes) = (String::new(), String::new(), 0);
    let mut ab = None;
    for line in status.split('\n') {
        if let Some(b) = line.strip_prefix("# branch.head ") {
            branch = b.into();
        } else if let Some(o) = line.strip_prefix("# branch.oid ") {
            oid = o.into();
        } else if let Some(x) = line.strip_prefix("# branch.ab ") {
            let mut it = x.split(' ').map(|n| n.parse::<i64>().unwrap_or(0).unsigned_abs() as u32);
            ab = Some((it.next().unwrap_or(0), it.next().unwrap_or(0)));
        } else if !line.is_empty() && !line.starts_with('#') {
            changes += 1;
        }
    }
    let trimmed = top.trim_end_matches('/');
    let repo = trimmed.rsplit('/').next().filter(|r| !r.is_empty()).unwrap_or(top).to_string();
    let branch = if branch == "(detached)" { oid.chars().take(7).collect() } else { branch };
    GitView { repo, branch, ahead: ab.map(|a| a.0), behind: ab.map(|a| a.1), changes }
}

pub async fn git_status(cwd: &str) -> Option<GitView> {
    let top = git(cwd, &["rev-parse", "--show-toplevel"]).await?.trim().to_string();
    if top.is_empty() {
        return None;
    }
    let status = git(cwd, &["status", "--porcelain=v2", "--branch"]).await?;
    Some(parse_status(&top, &status))
}

// What each space was last checked at: its directory, its panes' generations (what's on their screens) and when. A
// space whose directory and panes haven't changed isn't asked again until RECHECK has passed: a change made from outside
// (another terminal, an editor) shows within that.
#[derive(PartialEq)]
struct Checked {
    dir: String,
    generations: u64,
    at: u64,
}

thread_local! {
    static CHECKED: RefCell<HashMap<String, Checked>> = RefCell::new(HashMap::new());
}

const RECHECK: u64 = 30_000;

async fn tick(shared: &Shared) {
    let now = now_ms();
    // the spaces to ask about now: (space id, the directory its focused pane is in)
    let due: Vec<(String, String, u64)> = {
        let srv = shared.borrow();
        if srv.down || srv.attached().is_empty() {
            return;
        }
        let s = &srv.s;
        let due = s
            .workspaces
            .iter()
            .filter_map(|ws| {
                let focused = ws.tabs.get(ws.active).and_then(|t| s.panes.get(&t.focused));
                let dir = match focused {
                    Some(p) if p.info.running() => procs::cwd(p.pid).unwrap_or_else(|| p.info.cwd.clone()),
                    Some(p) => p.info.cwd.clone(),
                    None => ws.cwd.clone(),
                };
                let generations = ws.tabs.iter().flat_map(|t| panes(&t.tree)).filter_map(|id| s.panes.get(&id)).map(|p| p.generation).sum();
                let fresh = CHECKED.with_borrow(|c| c.get(&ws.id).is_some_and(|k| k.dir == dir && k.generations == generations && now - k.at < RECHECK));
                (!fresh).then(|| (ws.id.clone(), dir, generations))
            })
            .collect();
        let live: Vec<&String> = s.workspaces.iter().map(|w| &w.id).collect();
        CHECKED.with_borrow_mut(|c| c.retain(|id, _| live.contains(&id)));
        due
    };
    let mut found: HashMap<String, Option<GitView>> = HashMap::new();
    for (_, dir, _) in &due {
        if !found.contains_key(dir) {
            found.insert(dir.clone(), git_status(dir).await);
        }
    }
    let mut srv = shared.borrow_mut();
    let mut changed = false;
    for (id, dir, generations) in due {
        let Some(ws) = srv.s.workspaces.iter_mut().find(|w| w.id == id) else { continue };
        let next = found.get(&dir).cloned().flatten();
        if next != ws.git {
            ws.git = next;
            changed = true;
        }
        CHECKED.with_borrow_mut(|c| c.insert(id, Checked { dir, generations, at: now }));
    }
    if changed {
        srv.changed();
    }
}

pub fn start(shared: Shared) {
    let me = std::rc::Rc::downgrade(&shared);
    drop(shared);
    tokio::task::spawn_local(async move {
        loop {
            let Some(s) = me.upgrade() else { return };
            tick(&s).await;
            drop(s);
            tokio::time::sleep(EVERY).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_porcelain() {
        let g = parse_status("/x/modisa/", "# branch.oid abcdef1234\n# branch.head main\n# branch.ab +2 -1\n1 .M N... a\n? b\n");
        assert_eq!(g, GitView { repo: "modisa".into(), branch: "main".into(), ahead: Some(2), behind: Some(1), changes: 2 });
        let d = parse_status("/r", "# branch.oid abcdef1234\n# branch.head (detached)\n");
        assert_eq!((d.branch.as_str(), d.ahead, d.changes), ("abcdef1", None, 0));
    }
}
