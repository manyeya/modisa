// Survive server restarts: each session's layout and pane metadata, saved in SQLite (the same file and shape as the
// TypeScript build's bun:sqlite store, so either can restore what the other saved).
use std::cell::RefCell;
use std::collections::HashMap;

use indexmap::IndexMap;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::core::layout::Node;
use crate::core::paths::DIR;
use crate::server::Shared;

// env: what it was started with (pane split --env…), so it comes back with it
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedPane {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SavedSession>,
    #[serde(default = "user")]
    pub created_by: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<IndexMap<String, String>>,
}

fn user() -> String {
    "user".into()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedSession {
    pub agent: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedTab {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub zoomed: bool,
    pub focused: String,
    pub tree: Node,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedWorkspace {
    pub name: String,
    pub cwd: String,
    pub active: usize,
    pub tabs: Vec<SavedTab>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Saved {
    pub active: usize,
    pub workspaces: Vec<SavedWorkspace>,
    pub panes: IndexMap<String, SavedPane>,
}

// Live cwd of each shell (follows `cd`), from the kernel (platform/procs.rs): no lsof.
pub fn cwds(pids: &[i32]) -> HashMap<i32, String> {
    pids.iter().filter_map(|&p| Some((p, crate::platform::procs::cwd(p)?))).collect()
}

thread_local! {
    static DB: RefCell<Option<Connection>> = const { RefCell::new(None) };
}

// One row per session in ~/.local/state/modisa/modisa.db.
fn with_db<T>(f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Option<T> {
    DB.with(|db| {
        let mut db = db.borrow_mut();
        if db.is_none() {
            let _ = std::fs::create_dir_all(&*DIR);
            let path = format!("{}/modisa.db", *DIR);
            let conn = Connection::open(&path).ok()?;
            conn.execute("CREATE TABLE IF NOT EXISTS sessions (name TEXT PRIMARY KEY, data TEXT NOT NULL, saved_at INTEGER NOT NULL)", []).ok()?;
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)); // it holds panes' --env values: yours alone to read
            *db = Some(conn);
        }
        f(db.as_ref()?).map_err(|e| eprintln!("modisa: session store: {e}")).ok()
    })
}

// The variables a pane was given, less modisa's own (a plugin pane's MODISA_PLUGIN_*): those are for the run that
// started it, and whatever starts a pane sets them afresh.
fn chosen(env: &Option<IndexMap<String, String>>) -> Option<IndexMap<String, String>> {
    let kept: IndexMap<String, String> = env.iter().flatten().filter(|(k, _)| !k.starts_with("MODISA_")).map(|(k, v)| (k.clone(), v.clone())).collect();
    (!kept.is_empty()).then_some(kept)
}

// `debounced`: a save scheduled after a change, which a shutdown makes moot. It's checked after the async cwd lookup so
// a save racing a shutdown never resurrects a killed session.
pub async fn save_shared(shared: &Shared, debounced: bool) {
    let pids: Vec<i32> = {
        let srv = shared.borrow();
        if srv.s.workspaces.is_empty() {
            return;
        }
        srv.s.panes.values().filter(|p| p.info.running()).map(|p| p.pid).collect()
    };
    let live = cwds(&pids);
    let srv = shared.borrow();
    if debounced && srv.down {
        return;
    }
    let s = &srv.s;
    let data = Saved {
        active: s.active,
        workspaces: s
            .workspaces
            .iter()
            .map(|ws| SavedWorkspace { name: ws.name.clone(), cwd: ws.cwd.clone(), active: ws.active, tabs: ws.tabs.iter().map(|t| SavedTab { name: t.name.clone(), zoomed: t.zoomed, focused: t.focused.clone(), tree: t.tree.clone() }).collect() })
            .collect(),
        panes: s
            .panes
            .values()
            .map(|p| {
                let agent = p.info.agent.as_ref().map(|a| a.harness.clone());
                // only while that agent runs
                let session = p.info.session.as_ref().filter(|ses| Some(&ses.agent) == agent.as_ref()).map(|ses| SavedSession { agent: ses.agent.clone(), id: ses.id.clone(), source: Some(ses.source.clone()) });
                let pane = SavedPane { name: p.info.name.clone(), cwd: live.get(&p.pid).cloned().unwrap_or_else(|| p.info.cwd.clone()), command: p.info.command.clone(), harness: p.info.harness.clone(), agent, session, created_by: p.info.created_by.clone(), env: chosen(&p.env) };
                (p.info.id.clone(), pane)
            })
            .collect(),
    };
    let session = srv.session.clone();
    drop(srv);
    let json = serde_json::to_string(&data).unwrap();
    with_db(|db| db.execute("INSERT OR REPLACE INTO sessions (name, data, saved_at) VALUES (?, ?, ?)", params![session, json, crate::server::session::pane::now_ms() as i64]));
}

pub fn load(session: &str) -> Option<Saved> {
    let data: String = with_db(|db| db.query_row("SELECT data FROM sessions WHERE name = ?", [session], |r| r.get(0)).optional()).flatten()?;
    serde_json::from_str(&data).map_err(|e| eprintln!("modisa: saved session {session}: {e}")).ok()
}

pub fn forget(session: &str) {
    with_db(|db| db.execute("DELETE FROM sessions WHERE name = ?", [session]));
}

pub fn saved() -> Vec<(String, i64)> {
    with_db(|db| {
        let mut q = db.prepare("SELECT name, saved_at FROM sessions ORDER BY name")?;
        let rows = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    })
    .unwrap_or_default()
}
