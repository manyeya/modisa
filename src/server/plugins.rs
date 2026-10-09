// Plugins: programs started with the session server. Linked plugins (a plugin.json in a directory linked under
// ~/.config/modisa/plugins) start from their argv in their own directory; [[plugin]] run lines from config.toml start
// through a login shell. Each gets $MODISA_PLUGIN_DATA, a directory of its own, and a log file.
//
// Every start is a run: its own token, its own process group. Stopping a run (plugin stop, unlink, session stop) or its
// process exiting first revokes it: the token stops binding, its connection is closed, and everything it showed in the
// TUI is cleared. Then its group gets TERM, and KILL after STOP_MS, but only while the group is provably still that
// run's (see OwnedGroup).
//
// A run's connection binds with its token (plugin.hello) and can offer actions, which plugin.invoke (`modisa plugin
// run`, the palette, a status segment, a sidebar row, a menu entry) calls. An action that doesn't answer in time has an
// unknown outcome: the plugin is told to cancel (advisory), a late reply is logged, and nothing retries. The bound
// connection can also put data into the TUI (ui.*): status segments, a sidebar section, pane badges, menu entries and
// toasts, which modisa draws itself. Nothing restarts a plugin; plugin.start does, when asked.
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Write;
use std::process::Stdio;
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::Duration;

use indexmap::IndexMap;
use serde_json::{json, Value};
use tokio::io::AsyncReadExt;

use super::rpc::dispatch::Handler;
use super::session::SpawnOpts;
use super::views::{check_cells, check_view, BLITS, VIEW_LIMIT};
use super::{understands_plugins, understands_views, Server, Shared};
use crate::async_handler;
use crate::config::agents::brands::brand;
use crate::config::keys::{bind_plugin_keys, bindings, DeclaredKey};
use crate::config::plugins::{linked_plugins, read_install, read_manifest};
use crate::config::CONFIG_DIR;
use crate::core::layout::Axis;
use crate::core::paths::DIR;
use crate::core::text::{clean_text, width};
use crate::protocol::conn::{fail, RpcError, RpcResult};
use crate::protocol::links::link_matches;
use crate::protocol::plugin::PluginManifest;
use crate::protocol::schema::{Params, PROTOCOL};
use crate::server::persist::template::quote;
use crate::server::session::pane::{now_ms, random_hex};

const STOP: Duration = Duration::from_millis(2000);
static INVOKE_MS: LazyLock<u64> = LazyLock::new(|| std::env::var("MODISA_PLUGIN_INVOKE_MS").ok().and_then(|v| v.parse().ok()).filter(|&n| n > 0).unwrap_or(30_000));
const LOG_LIMIT: usize = 5 * 1024 * 1024; // per run; past it the rest is read and dropped, so the plugin never blocks on output
const DRAIN: Duration = Duration::from_millis(1000); // after a run exits, how long its log waits for output still in the pipes

// A run's process group, signalled only while it's provably still ours. Once it's seen gone (or owned by someone else)
// it's retired for good: the id can be reused by an unrelated group, which must never be signalled.
// ponytail: a small window remains. If the group's last process exits and its id is reused before the next probe (the
// 1s watch after the leader exits, or stop's own probe), that probe can't tell. Closing it needs pidfds.
pub struct OwnedGroup {
    pub pgid: i32,
    retired: std::cell::Cell<bool>,
}

impl OwnedGroup {
    pub fn new(pgid: i32) -> OwnedGroup {
        OwnedGroup { pgid, retired: std::cell::Cell::new(false) }
    }
    pub fn alive(&self) -> bool {
        if self.retired.get() {
            return false;
        }
        if unsafe { libc::kill(-self.pgid, 0) } == 0 {
            return true;
        }
        self.retired.set(true); // gone, or (EPERM) not ours
        false
    }
    pub fn signal(&self, sig: libc::c_int) {
        if self.alive() {
            unsafe { libc::kill(-self.pgid, sig) };
        }
    }
}

// ---------- what a run shows in the TUI ----------
// Limits keep a plugin from crowding out modisa's own chrome or flooding clients with redraws.
const STATUS_SEGMENTS: usize = 4;
const STATUS_TEXT: usize = 32;
const SIDEBAR_TITLE: usize = 30;
const SIDEBAR_ROWS: usize = 40;
const ROW_TEXT: usize = 60;
const BADGE_TEXT: usize = 12;
const BADGES: usize = 50;
const MENU_ITEMS: usize = 8;
const MENU_TITLE: usize = 40;
const UPDATES_BURST: f64 = 30.0; // ui.* calls per run
const UPDATES_PER_SECOND: f64 = 10.0;
// and across all the session's plugins together, so many plugins, each within its own limits, can't do it either: first
// come, first served (toasts' limits are the server's)
const SESSION_STATUS_SEGMENTS: usize = 12;
const SESSION_BADGES_PER_PANE: usize = 4;
const SESSION_SIDEBAR_SECTIONS: usize = 6;
const SESSION_MENU_ITEMS: usize = 24;
const SESSION_BURST: f64 = 60.0;
const SESSION_PER_SECOND: f64 = 30.0;

// A token bucket: `tokens` refilled at `rate` a second, up to `burst`.
#[derive(Clone)]
struct Bucket {
    tokens: f64,
    refilled: u64,
}

impl Bucket {
    fn full(burst: f64) -> Bucket {
        Bucket { tokens: burst, refilled: now_ms() }
    }
    fn refill(&mut self, burst: f64, rate: f64) {
        let now = now_ms();
        self.tokens = burst.min(self.tokens + (now.saturating_sub(self.refilled)) as f64 / 1000.0 * rate);
        self.refilled = now;
    }
}

// a view, and its Rasters' sizes by id: what a blit must match
pub struct OpenView {
    pub state: Value,
    pub rasters: HashMap<String, (u64, u64)>,
}

#[derive(Default)]
struct UiState {
    status: IndexMap<String, Value>,
    sidebar: Option<Value>,
    badges: IndexMap<String, Value>,
    menu: Vec<Value>,
    views: IndexMap<String, OpenView>,
    tokens: Option<Bucket>,
    blits: Option<Bucket>,
}

impl UiState {
    fn new() -> UiState {
        UiState { tokens: Some(Bucket::full(UPDATES_BURST)), blits: Some(Bucket::full(BLITS.0)), ..Default::default() }
    }
}

// A row's spans, cleaned like any plugin text and cut to the row's budget of cells: an icon takes one, and text is cut
// (with …) where the budget runs out; what's past it is dropped.
fn clean_spans(spans: &[Value]) -> Vec<Value> {
    let mut out = vec![];
    let mut left = ROW_TEXT as i64;
    for x in spans {
        if left <= 0 {
            break;
        }
        if let Some(icon) = x.get("icon").and_then(Value::as_str) {
            let icon = clean_text(icon, 40);
            if !icon.is_empty() {
                out.push(json!({ "icon": icon }));
                left -= 1;
            }
            continue;
        }
        let text = clean_text(x["text"].as_str().unwrap_or(""), left as usize);
        if text.is_empty() {
            continue;
        }
        left -= width(&text) as i64;
        let mut s = json!({ "text": text });
        if let Some(t) = x.get("tone").filter(|t| t.is_string()) {
            s["tone"] = t.clone();
        }
        if x.get("bold").and_then(Value::as_bool) == Some(true) {
            s["bold"] = json!(true);
        }
        out.push(s);
    }
    out
}

// id: public, shown with the run's UI so an action taken from it can be refused once the run has ended
pub struct Run {
    pub id: String,
    group: OwnedGroup,
    token: String,
    revoked: bool,
    log: Rc<RefCell<Log>>,
}

// A run's log: both streams, and modisa's own notes, into one file in the order they happen, up to LOG_LIMIT.
struct Log {
    file: Option<std::fs::File>,
    written: usize,
}

impl Log {
    fn write(&mut self, chunk: &[u8]) {
        if self.written > LOG_LIMIT {
            return;
        }
        self.written += chunk.len();
        let Some(f) = self.file.as_mut() else { return };
        let _ = if self.written > LOG_LIMIT { f.write_all(format!("\n[modisa: log truncated at {} MB]\n", LOG_LIMIT / 1024 / 1024).as_bytes()) } else { f.write_all(chunk) };
    }
    fn note(&mut self, line: &str) {
        self.write(format!("{line}\n").as_bytes());
    }
}

pub struct Plugin {
    pub name: String,
    pub source: &'static str, // "linked" (a plugin.json) or "config" (a [[plugin]] run line)
    pub dir: Option<String>,
    pub status: &'static str, // starting | running | exited | failed | stopped
    pub pid: Option<i32>,
    pub exit_code: Option<i32>,
    pub signal: Option<String>,
    pub error: Option<String>,
    pub log: String,
    pub install: Option<Value>,
    pub run: Option<Run>,
    pub argv: Option<Vec<String>>,
    pub manifest: Option<PluginManifest>,
    pub client: Option<u64>,
    pub actions: Vec<String>,
    pub stopping: bool,
    pub starting: bool,
    ui: UiState,
}

impl Plugin {
    fn live(&self) -> bool {
        self.run.as_ref().is_some_and(|r| !r.revoked)
    }
}

// Placements. split, tab and zoomed are ordinary panes in the layout (zoomed: split, then the tab zoomed); they belong to
// the session and outlive the plugin. overlay is a temporary zoomed pane over its origin, closed when its process exits
// or its plugin stops; closing puts focus (and zoom) back on the origin if it still exists and the user hasn't gone to
// another tab, and otherwise leaves focus where closing put it. popup is a terminal with no place in the layout, owned
// by the one TUI client that opened it (others never see it), one per session, closed when its process exits, its client
// closes it (prefix x), the plugin closes it, or the plugin stops.
// had_focus: the overlay was the focused pane of the tab on screen when it closed. Only then does focus go back.
struct Overlay {
    plugin: String,
    origin: Option<String>,
    instance: Option<String>,
    was_zoomed: bool,
    had_focus: bool,
}

struct Popup {
    plugin: String,
    client: u64,
}

#[derive(Default)]
pub struct Host {
    pub plugins: IndexMap<String, Plugin>,
    overlays: HashMap<String, Overlay>,
    popups: HashMap<String, Popup>,
    invocations: u64,
    session_ui: Option<Bucket>,
}

// A plugin's own directories: DATA for its state (under modisa's state directory), CONFIG for settings the user edits
// (under ~/.config/modisa/plugin-config).
fn dirs_of(name: &str) -> Vec<(String, String)> {
    vec![
        ("MODISA_PLUGIN".into(), name.into()),
        ("MODISA_PLUGIN_DATA".into(), format!("{}/plugins/{name}", *DIR)),
        ("MODISA_PLUGIN_CONFIG".into(), format!("{}/plugin-config/{name}", *CONFIG_DIR)),
    ]
}

// where an installed plugin came from, if this link is the one `plugin install` made
fn install_of(name: &str, dir: &str) -> Option<Value> {
    let r = read_install(name).filter(|r| r.dir == dir)?;
    let mut v = json!({ "source": r.source, "ref": r.r#ref, "commit": r.commit });
    if let Some(m) = r.marketplace {
        v["marketplace"] = json!(m);
    }
    Some(v)
}

fn add<'a>(srv: &'a mut Server, name: &str, source: &'static str, dir: Option<String>) -> &'a mut Plugin {
    // starting until it's launched or fails to: never reported as failed before it has had a chance to run
    let log = format!("{}/plugins/{}.{name}.log", *DIR, srv.session);
    srv.host.plugins.insert(name.into(), Plugin { name: name.into(), source, dir, status: "starting", pid: None, exit_code: None, signal: None, error: None, log, install: None, run: None, argv: None, manifest: None, client: None, actions: vec![], stopping: false, starting: false, ui: UiState::new() });
    srv.host.plugins.get_mut(name).unwrap()
}

fn need<'a>(srv: &'a mut Server, name: &str) -> RpcResult<&'a mut Plugin> {
    srv.host.plugins.get_mut(name).ok_or_else(|| fail("no_such_plugin", format!("no plugin named {name} (see modisa plugin list)")))
}

// The run's token stops binding, its connection is closed and its TUI contributions go, whether or not its group is
// still alive.
fn revoke(srv: &mut Server, name: &str) {
    let Some(pl) = srv.host.plugins.get_mut(name) else { return };
    if let Some(r) = pl.run.as_mut() {
        r.revoked = true;
    }
    let client = pl.client.take();
    pl.actions.clear();
    let views: Vec<String> = pl.ui.views.keys().cloned().collect();
    pl.ui = UiState::new();
    for id in views {
        view_closed(srv, name, &id);
    }
    if let Some(c) = client.and_then(|c| srv.clients.get(&c)) {
        c.conn.close();
    }
    // its transient panes go with it; split, tab and zoomed panes are the session's and stay
    let overlays: Vec<String> = srv.host.overlays.iter().filter(|(_, o)| o.plugin == name).map(|(id, _)| id.clone()).collect();
    for id in overlays {
        srv.s.close(Some(&id));
    }
    let popups: Vec<String> = srv.host.popups.iter().filter(|(_, p)| p.plugin == name).map(|(id, _)| id.clone()).collect();
    for id in popups {
        srv.s.drop_hidden(&id);
    }
    srv.changed();
}

fn failed(pl: &mut Plugin, error: String) {
    pl.status = "failed";
    pl.pid = None;
    let _ = std::fs::create_dir_all(format!("{}/plugins", *DIR));
    let _ = std::fs::write(&pl.log, format!("modisa: {error}\n"));
    pl.error = Some(error);
}

// A linked plugin's manifest is read again on every start, so edits to plugin.json apply.
fn prepare(pl: &mut Plugin) -> bool {
    if pl.source != "linked" {
        return true;
    }
    let why = match read_manifest(pl.dir.as_deref().unwrap_or("")) {
        Err(e) => Some(e),
        Ok(m) if m.name != pl.name => Some(format!("linked as {}, but plugin.json names it {}", pl.name, m.name)),
        Ok(m) if m.protocol != PROTOCOL as u64 => Some(format!("plugin.json says protocol {}; this modisa speaks protocol {PROTOCOL}", m.protocol)),
        Ok(m) => {
            pl.argv = Some(m.run.clone());
            pl.manifest = Some(m);
            None
        }
    };
    match why {
        Some(e) => {
            failed(pl, e);
            false
        }
        None => true,
    }
}

// Start a run of the plugin: its own token and process group, its output into its log, and the exit watched.
fn launch(shared: &Shared, name: &str) {
    let mut srv = shared.borrow_mut();
    let Some(pl) = srv.host.plugins.get_mut(name) else { return };
    let dirs = dirs_of(name);
    for (_, d) in &dirs[1..] {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::create_dir_all(format!("{}/plugins", *DIR));
    let file = std::fs::File::create(&pl.log).ok();
    pl.stopping = false;
    pl.pid = None;
    pl.exit_code = None;
    pl.signal = None;
    pl.error = None;
    let token = format!("{}-{}-{}-{}", random_hex(4), random_hex(2), random_hex(2), random_hex(8));
    let argv = pl.argv.clone().unwrap_or_default();
    let Some(program) = argv.first() else { return failed(pl, "nothing to run".into()) };
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(&argv[1..]).envs(dirs.iter().cloned()).env("MODISA_PLUGIN_TOKEN", &token).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(d) = &pl.dir {
        cmd.current_dir(d);
    }
    // detached: a new session and process group, led by the plugin, so the whole group can be signalled
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return failed(pl, format!("couldn't start {program}: {e}")),
    };
    let pid = child.id().unwrap_or(0) as i32;
    let log = Rc::new(RefCell::new(Log { file, written: 0 }));
    let run_id = random_hex(4);
    pl.run = Some(Run { id: run_id.clone(), group: OwnedGroup::new(pid), token, revoked: false, log: log.clone() });
    pl.status = "running";
    pl.pid = Some(pid);
    pl.ui = UiState::new();
    drop(srv);
    // both streams into the log as they come
    let pump = |stream: Option<Box<dyn tokio::io::AsyncRead + Unpin>>, log: Rc<RefCell<Log>>| async move {
        let Some(mut s) = stream else { return };
        let mut buf = vec![0u8; 16 * 1024];
        while let Ok(n @ 1..) = s.read(&mut buf).await {
            log.borrow_mut().write(&buf[..n]);
        }
    };
    let out = tokio::task::spawn_local(pump(child.stdout.take().map(|s| Box::new(s) as _), log.clone()));
    let err = tokio::task::spawn_local(pump(child.stderr.take().map(|s| Box::new(s) as _), log.clone()));
    let (me, name) = (Rc::downgrade(shared), name.to_string());
    tokio::task::spawn_local(async move {
        let status = child.wait().await;
        let Some(shared) = me.upgrade() else { return };
        let outcome = {
            let mut srv = shared.borrow_mut();
            let Some(pl) = srv.host.plugins.get_mut(&name).filter(|p| p.run.as_ref().is_some_and(|r| r.id == run_id)) else { return }; // started again since
            use std::os::unix::process::ExitStatusExt;
            let (code, signal) = match &status {
                Ok(s) => (s.code(), s.signal().map(signal_name)),
                Err(_) => (None, None),
            };
            pl.exit_code = code;
            pl.signal = signal;
            let outcome = if pl.stopping { "stopped" } else if code == Some(0) { "exited" } else { "failed" };
            // At once: the exit's facts, and the run loses its token, connection and UI; the group is watched from now on.
            revoke(&mut srv, &name);
            srv.settle();
            outcome
        };
        // retire the group's id as soon as it's gone (its children can outlive the leader)
        let watch = me.clone();
        let (wname, wrun) = (name.clone(), run_id.clone());
        tokio::task::spawn_local(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let Some(s) = watch.upgrade() else { return };
                let srv = s.borrow();
                let Some(r) = srv.host.plugins.get(&wname).and_then(|p| p.run.as_ref()).filter(|r| r.id == wrun) else { return };
                if !r.group.alive() {
                    return;
                }
            }
        });
        // Then the log gets a moment to take the last of its output, so a crash's final stderr is there when the failure
        // shows; a child holding the pipes open only delays that by DRAIN, and the log says it may be incomplete.
        let complete = tokio::time::timeout(DRAIN, async {
            let _ = out.await;
            let _ = err.await;
        })
        .await
        .is_ok();
        if !complete {
            log.borrow_mut().write(b"[modisa: output still open after exit; log may be incomplete]\n");
        }
        let mut srv = shared.borrow_mut();
        let Some(pl) = srv.host.plugins.get_mut(&name).filter(|p| p.run.as_ref().is_some_and(|r| r.id == run_id)) else { return };
        pl.status = outcome;
        if outcome == "failed" {
            let how = pl.signal.clone().unwrap_or_else(|| pl.exit_code.map(|c| c.to_string()).unwrap_or_default());
            pl.error = Some(format!("exited with {how}{}; see {}", if complete { "" } else { " (output still open after exit; log may be incomplete)" }, pl.log));
        }
    });
}

fn signal_name(n: i32) -> String {
    match n {
        libc::SIGHUP => "SIGHUP",
        libc::SIGINT => "SIGINT",
        libc::SIGQUIT => "SIGQUIT",
        libc::SIGKILL => "SIGKILL",
        libc::SIGTERM => "SIGTERM",
        libc::SIGSEGV => "SIGSEGV",
        libc::SIGABRT => "SIGABRT",
        libc::SIGPIPE => "SIGPIPE",
        _ => return format!("signal {n}"),
    }
    .into()
}

// Tests only: hold each linked plugin, before its manifest is read, until this file exists, so the starting state can be
// seen deterministically. Unset, it does nothing.
async fn hold() {
    let Ok(file) = std::env::var("MODISA_TEST_PLUGIN_HOLD") else { return };
    while !std::path::Path::new(&file).exists() {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

// At server start: every linked plugin, then every [[plugin]] run line.
pub async fn start(shared: Shared) {
    let linked = linked_plugins();
    {
        // all listed as starting at once, and flagged so plugin.start answers already_running until each is launched or not
        let mut srv = shared.borrow_mut();
        for l in &linked {
            add(&mut srv, &l.name, "linked", Some(l.dir.clone())).starting = true;
        }
    }
    for l in linked {
        {
            let mut srv = shared.borrow_mut();
            if let Some(pl) = srv.host.plugins.get_mut(&l.name) {
                pl.install = install_of(&l.name, &l.dir);
            }
        }
        hold().await;
        let go = {
            let mut srv = shared.borrow_mut();
            let Some(pl) = srv.host.plugins.get_mut(&l.name) else { continue };
            let go = if pl.stopping {
                pl.status = "stopped"; // stopped before it was launched
                false
            } else if let Some(e) = l.error.clone() {
                failed(pl, e);
                false
            } else {
                prepare(pl) && !pl.stopping
            };
            if !go && pl.stopping {
                pl.status = "stopped";
            }
            go
        };
        if go {
            launch(&shared, &l.name);
        }
        if let Some(pl) = shared.borrow_mut().host.plugins.get_mut(&l.name) {
            pl.starting = false;
        }
    }
    let runs: Vec<String> = shared.borrow().cfg.plugin.iter().map(|p| p.run.clone()).collect();
    for (i, run) in runs.into_iter().enumerate() {
        let name = format!("config-{}", i + 1);
        {
            let mut srv = shared.borrow_mut();
            let pl = add(&mut srv, &name, "config", None);
            pl.argv = Some(vec![std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()), "-lc".into(), run]);
        }
        launch(&shared, &name);
    }
    shared.borrow_mut().settle();
}

// Stop one plugin, or every one: revoke, TERM each group still there, KILL what's left after STOP.
pub async fn stop(shared: &Shared, only: Option<&str>) {
    let live: Vec<String> = {
        let mut srv = shared.borrow_mut();
        let names: Vec<String> = match only {
            Some(n) => vec![n.to_string()],
            None => srv.host.plugins.keys().cloned().collect(),
        };
        for n in &names {
            if let Some(pl) = srv.host.plugins.get_mut(n) {
                pl.stopping = true;
                if pl.status == "starting" && pl.run.is_none() {
                    pl.status = "stopped"; // not launched yet: the server's start won't now
                }
            }
            revoke(&mut srv, n);
        }
        let live: Vec<String> = names.into_iter().filter(|n| srv.host.plugins.get(n).and_then(|p| p.run.as_ref()).is_some_and(|r| r.group.alive())).collect();
        for n in &live {
            srv.host.plugins[n].run.as_ref().unwrap().group.signal(libc::SIGTERM);
        }
        srv.settle();
        live
    };
    let end = tokio::time::Instant::now() + STOP;
    while tokio::time::Instant::now() < end {
        let any = {
            let srv = shared.borrow();
            live.iter().any(|n| srv.host.plugins.get(n).and_then(|p| p.run.as_ref()).is_some_and(|r| r.group.alive()))
        };
        if !any {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let srv = shared.borrow();
    for n in &live {
        if let Some(r) = srv.host.plugins.get(n).and_then(|p| p.run.as_ref()) {
            r.group.signal(libc::SIGKILL);
        }
    }
}

// Keys for this session's running plugins as the SERVER's config binds them ([plugin_keys], and the prefix keys its
// [keys] makes): what `plugin list` reports. Clients get plugin.json's keys in the view and bind them with their own
// config, so these aren't theirs.
fn declared_keys(pl: &Plugin) -> Vec<DeclaredKey> {
    pl.manifest.iter().flat_map(|m| m.keys.iter().flatten()).map(|k| DeclaredKey { plugin: pl.name.clone(), key: k.key.clone(), action: k.action.clone(), pane: k.pane.clone(), description: k.description.clone() }).collect()
}

fn keys_of(srv: &Server, name: &str) -> Vec<Value> {
    let declared: Vec<DeclaredKey> = srv.host.plugins.values().filter(|p| p.live()).flat_map(declared_keys).collect();
    bind_plugin_keys(&declared, &srv.cfg.plugin_keys, &bindings(&srv.cfg))
        .into_iter()
        .filter(|k| k.declared.plugin == name)
        .map(|k| {
            let mut v = serde_json::to_value(&k).unwrap();
            v.as_object_mut().unwrap().remove("plugin");
            v
        })
        .collect()
}

// A plugin as `plugin list` shows it.
fn status_of(srv: &Server, pl: &Plugin) -> Value {
    let mut v = json!({ "name": pl.name, "source": pl.source });
    let o = v.as_object_mut().unwrap();
    if let Some(d) = &pl.dir {
        o.insert("dir".into(), json!(d));
    }
    o.insert("status".into(), json!(pl.status));
    o.insert("log".into(), json!(pl.log));
    let client = pl.client.and_then(|c| srv.clients.get(&c));
    o.insert("connected".into(), json!(client.is_some()));
    o.insert("actions".into(), json!(pl.actions));
    if let Some(i) = &pl.install {
        o.insert("install".into(), i.clone());
    }
    if let Some(p) = pl.pid {
        o.insert("pid".into(), json!(p));
    }
    if let Some(c) = pl.exit_code {
        o.insert("exitCode".into(), json!(c));
    }
    if let Some(s) = &pl.signal {
        o.insert("signal".into(), json!(s));
    }
    if let Some(e) = &pl.error {
        o.insert("error".into(), json!(e));
    }
    if let Some(r) = &pl.run {
        o.insert("group".into(), json!(if r.group.alive() { "running" } else { "gone" }));
    }
    if let Some(c) = client {
        o.insert("invocations".into(), json!(c.conn.in_flight()));
    }
    if pl.live() {
        o.insert("keys".into(), json!(keys_of(srv, &pl.name)));
    }
    v
}

// what one plugin shows in the TUI now
fn ui_of(pl: &Plugin) -> Value {
    let titles: HashMap<&String, &crate::protocol::plugin::ManifestAction> = pl.manifest.iter().flat_map(|m| m.actions.iter().flatten()).map(|a| (&a.id, a)).collect();
    let actions: Vec<Value> = if pl.client.is_some() {
        pl.actions
            .iter()
            .map(|id| {
                let a = titles.get(id);
                let mut v = json!({ "id": id, "title": a.map(|a| a.title.clone()).unwrap_or(id.clone()) });
                if let Some(d) = a.and_then(|a| a.description.clone()) {
                    v["description"] = json!(d);
                }
                v
            })
            .collect()
    } else {
        vec![]
    };
    let mut v = json!({ "plugin": pl.name, "run": pl.run.as_ref().map(|r| r.id.clone()).unwrap_or_default(), "actions": actions, "status": pl.ui.status.values().collect::<Vec<_>>() });
    if let Some(s) = &pl.ui.sidebar {
        v["sidebar"] = s.clone();
    }
    let live = pl.live();
    let manifest = pl.manifest.as_ref();
    v["badges"] = json!(pl.ui.badges.values().collect::<Vec<_>>());
    v["menu"] = json!(pl.ui.menu);
    // plugin.json's defaults: each client binds them with its own config
    v["keys"] = if live { json!(manifest.iter().flat_map(|m| m.keys.iter().flatten()).map(|k| { let mut x = json!({ "key": k.key }); if let Some(a) = &k.action { x["action"] = json!(a) } if let Some(p) = &k.pane { x["pane"] = json!(p) } x["description"] = json!(k.description); x }).collect::<Vec<_>>()) } else { json!([]) };
    v["panes"] = if live { json!(manifest.iter().flat_map(|m| m.panes.iter().flatten()).map(|p| json!({ "id": p.id, "title": p.title, "placement": p.placement })).collect::<Vec<_>>()) } else { json!([]) };
    v["links"] = if pl.client.is_some() { json!(manifest.iter().flat_map(|m| m.links.iter().flatten()).filter(|l| pl.actions.contains(&l.action)).collect::<Vec<_>>()) } else { json!([]) };
    v
}

// What plugins show in the TUI, for the session's view: the running ones that show anything.
pub fn ui_view(srv: &Server) -> Value {
    json!(srv
        .host
        .plugins
        .values()
        .filter(|p| p.live())
        .map(ui_of)
        .filter(|v| ["actions", "status", "badges", "menu", "keys", "panes", "links"].iter().any(|k| v[k].as_array().is_some_and(|a| !a.is_empty())) || v.get("sidebar").is_some())
        .collect::<Vec<_>>())
}

// Views go to the clients that draw them on their own channel, only when one changes: a tree can be big, and the
// session's view is sent again on every change of anything.
pub fn views(srv: &Server) -> Value {
    json!(srv.host.plugins.values().filter(|p| p.live()).flat_map(|p| p.ui.views.values().map(|v| v.state.clone())).collect::<Vec<_>>())
}

fn viewers(srv: &Server) -> Vec<u64> {
    srv.clients.iter().filter(|(_, c)| c.attached && understands_views(c)).map(|(id, _)| *id).collect()
}

fn view_closed(srv: &Server, plugin: &str, id: &str) {
    srv.broadcast_to("plugin.view.closed", json!({ "plugin": plugin, "id": id }), &viewers(srv));
}

fn close_view(srv: &mut Server, plugin: &str, id: &str) -> bool {
    let Some(pl) = srv.host.plugins.get_mut(plugin) else { return false };
    if pl.ui.views.shift_remove(id).is_none() {
        return false;
    }
    view_closed(srv, plugin, id);
    true
}

// a pane is being closed: whether an overlay still had the focus then
pub fn pane_closing(srv: &mut Server, id: &str, focused: bool) {
    if let Some(o) = srv.host.overlays.get_mut(id) {
        o.had_focus = focused;
    }
}

// an overlay's or popup's process ended
pub fn pane_exited(srv: &mut Server, id: &str) {
    if let Some(o) = srv.host.overlays.remove(id) {
        let origin = o.origin.as_ref().filter(|origin| srv.s.panes.get(*origin).is_some_and(|p| Some(&p.info.instance) == o.instance.as_ref()));
        if let (true, Some(origin)) = (o.had_focus, origin) {
            let origin = origin.clone();
            if let Some((wi, ti)) = srv.s.locate(&origin) {
                if wi == srv.s.active && ti == srv.s.workspaces[wi].active {
                    srv.s.focus_pane(&origin);
                    srv.s.workspaces[wi].tabs[ti].zoomed = o.was_zoomed;
                    srv.s.layout();
                }
            }
        }
    }
    if srv.host.popups.remove(id).is_some() {
        srv.s.drop_hidden(id);
        srv.changed();
    }
}

pub fn disconnected(srv: &mut Server, c: u64) {
    let mut any = false;
    for pl in srv.host.plugins.values_mut() {
        if pl.client != Some(c) {
            continue;
        }
        pl.client = None;
        pl.actions.clear();
        any = true; // its palette actions go; what it put in the TUI stays until its run ends
    }
    // a popup is its client's: that client is gone
    let popups: Vec<String> = srv.host.popups.iter().filter(|(_, p)| p.client == c).map(|(id, _)| id.clone()).collect();
    for id in popups {
        srv.host.popups.remove(&id);
        srv.s.drop_hidden(&id);
    }
    if any {
        srv.changed();
    }
}

// an overlay stays over its origin: it isn't moved or swapped away
pub fn movable(srv: &Server, id: &str) -> bool {
    !srv.host.overlays.contains_key(id)
}

// unlinked through the plugin manager: out of this session's list, once nothing of it runs
pub fn forget(srv: &mut Server, name: &str) {
    let gone = srv.host.plugins.get(name).is_some_and(|p| p.source == "linked" && !p.run.as_ref().is_some_and(|r| r.group.alive()) && !p.starting);
    if gone {
        srv.host.plugins.shift_remove(name);
    }
}

// A ui.* call: from the plugin's bound connection, within its rate, naming only actions it offered. The plugin's name.
fn ui_call(srv: &mut Server, c: u64, action: Option<&str>) -> RpcResult<String> {
    let Some(name) = srv.host.plugins.values().find(|p| p.client == Some(c) && p.live()).map(|p| p.name.clone()) else {
        return Err(fail("plugin_unavailable", "ui calls work only on a plugin's bound connection: call plugin.hello first"));
    };
    let session = srv.host.session_ui.get_or_insert_with(|| Bucket::full(SESSION_BURST));
    session.refill(SESSION_BURST, SESSION_PER_SECOND);
    let session_tokens = session.tokens;
    let pl = srv.host.plugins.get_mut(&name).unwrap();
    let mine = pl.ui.tokens.get_or_insert_with(|| Bucket::full(UPDATES_BURST));
    mine.refill(UPDATES_BURST, UPDATES_PER_SECOND);
    // both checked before either is spent, so an update one refuses doesn't use up the other
    if mine.tokens < 1.0 {
        return Err(fail("rate_limited", format!("too many ui updates from {name}: at most {} a second", UPDATES_PER_SECOND)));
    }
    if session_tokens < 1.0 {
        return Err(fail("rate_limited", format!("too many ui updates from the session's plugins: at most {} a second", SESSION_PER_SECOND)));
    }
    mine.tokens -= 1.0;
    srv.host.session_ui.as_mut().unwrap().tokens -= 1.0;
    let pl = &srv.host.plugins[&name];
    if let Some(a) = action.filter(|a| !pl.actions.iter().any(|x| x == a)) {
        return Err(no_action(pl, a, " in hello"));
    }
    Ok(name)
}

fn no_action(pl: &Plugin, action: &str, how: &str) -> RpcError {
    let offers = if pl.actions.is_empty() { "none".to_string() } else { pl.actions.join(", ") };
    fail("no_such_action", format!("{} didn't offer action {action}{how} (it offers: {offers})", pl.name))
}

fn changed(srv: &mut Server, name: &str) -> RpcResult {
    srv.changed();
    Ok(ui_of(&srv.host.plugins[name]))
}

fn others<'a>(srv: &'a Server, name: &'a str) -> impl Iterator<Item = &'a Plugin> {
    srv.host.plugins.values().filter(move |x| x.name != name && x.live())
}

const TONES: &[&str] = &["fg", "dim", "accent", "warn", "working", "blocked", "done", "idle"];

pub fn route(method: &str) -> Option<Handler> {
    use Handler::Sync;
    Some(match method {
        "plugin.list" => Sync(|srv, _, _| Ok(json!(srv.host.plugins.values().map(|p| status_of(srv, p)).collect::<Vec<_>>()))),
        "plugin.stop" => async_handler!(plugin_stop),
        "plugin.start" => async_handler!(plugin_start),
        "plugin.hello" => Sync(plugin_hello),
        "plugin.invoke" => async_handler!(plugin_invoke),
        "ui.status.set" => Sync(status_set),
        "ui.status.clear" => Sync(status_clear),
        "ui.sidebar.set" => Sync(sidebar_set),
        "ui.sidebar.clear" => Sync(|srv, p, c| {
            Params::new(p)?;
            let name = ui_call(srv, c, None)?;
            srv.host.plugins.get_mut(&name).unwrap().ui.sidebar = None;
            changed(srv, &name)
        }),
        "ui.badge.set" => Sync(badge_set),
        "ui.badge.clear" => Sync(|srv, p, c| {
            let pr = Params::new(p)?;
            let pane = pr.len("pane", 1, None)?;
            let name = ui_call(srv, c, None)?;
            srv.host.plugins.get_mut(&name).unwrap().ui.badges.shift_remove(&pane);
            changed(srv, &name)
        }),
        "ui.menu.set" => Sync(menu_set),
        "ui.toast" => Sync(toast),
        "ui.state" => Sync(|srv, p, _| {
            let pr = Params::new(p)?;
            let plugin = pr.len("plugin", 1, None)?;
            let pl = need(srv, &plugin)?;
            let mut v = ui_of(pl);
            v["views"] = json!(pl.ui.views.values().map(|v| v.state.clone()).collect::<Vec<_>>());
            Ok(v)
        }),
        "plugin.pane.open" => Sync(pane_open),
        "plugin.popup.resize" => Sync(|srv, p, c| {
            let pr = Params::new(p)?;
            let pane = pr.len("pane", 1, None)?;
            let cols = pr.opt_num("cols", Some(10.0), Some(1000.0), true)?.unwrap_or(80.0) as u16;
            let rows = pr.opt_num("rows", Some(3.0), Some(500.0), true)?.unwrap_or(24.0) as u16;
            if srv.host.popups.get(&pane).is_none_or(|x| x.client != c) {
                return Err(fail("no_such_pane", format!("no popup {pane} opened by this client")));
            }
            if let Some(p) = srv.s.panes.get_mut(&pane) {
                p.resize(cols, rows);
            }
            Ok(json!(true))
        }),
        "plugin.popup.close" => Sync(|srv, p, c| {
            let pr = Params::new(p)?;
            let pane = pr.len("pane", 1, None)?;
            if srv.host.popups.get(&pane).is_none_or(|x| x.client != c) {
                return Err(fail("no_such_pane", format!("no popup {pane} opened by this client")));
            }
            srv.host.popups.remove(&pane);
            srv.s.drop_hidden(&pane);
            Ok(json!(true))
        }),
        "ui.view.set" => Sync(view_set),
        "ui.view.close" => Sync(|srv, p, c| {
            let pr = Params::new(p)?;
            let id = pr.len("id", 1, Some(40))?;
            let name = ui_call(srv, c, None)?;
            Ok(json!(close_view(srv, &name, &id)))
        }),
        "ui.blit" => Sync(blit),
        "plugin.view.close" => Sync(user_closed_view),
        "ui.popup.close" => Sync(|srv, p, c| {
            Params::new(p)?;
            let name = ui_call(srv, c, None)?;
            let popups: Vec<String> = srv.host.popups.iter().filter(|(_, x)| x.plugin == name).map(|(id, _)| id.clone()).collect();
            for id in popups {
                srv.host.popups.remove(&id);
                srv.s.drop_hidden(&id);
            }
            Ok(json!(true))
        }),
        _ => return None,
    })
}

// stop one plugin: revoke its run, then end its group if that's still there
async fn plugin_stop(shared: Shared, p: Value, _c: u64) -> RpcResult {
    let pr = Params::new(&p)?;
    let name = pr.len("name", 1, None)?;
    need(&mut shared.borrow_mut(), &name)?;
    stop(&shared, Some(&name)).await;
    let srv = shared.borrow();
    Ok(status_of(&srv, &srv.host.plugins[&name]))
}

// Start a plugin that isn't running, as a new run with a new token. Linked plugins are looked up again, so one linked
// since this server started can be started too. Never a second run: a running or starting plugin is already_running.
async fn plugin_start(shared: Shared, p: Value, _c: u64) -> RpcResult {
    let pr = Params::new(&p)?;
    let name = pr.len("name", 1, None)?;
    {
        let mut srv = shared.borrow_mut();
        let source = srv.host.plugins.get(&name).map(|p| p.source);
        if source.is_none() || source == Some("linked") {
            let Some(l) = linked_plugins().into_iter().find(|l| l.name == name) else {
                return Err(fail("no_such_plugin", format!("no plugin named {name} is linked (modisa plugin link <dir>)")));
            };
            if !srv.host.plugins.contains_key(&name) {
                add(&mut srv, &l.name, "linked", Some(l.dir.clone()));
            }
            let pl = srv.host.plugins.get_mut(&name).unwrap();
            pl.dir = Some(l.dir.clone());
            pl.install = install_of(&l.name, &l.dir);
            let busy = pl.run.as_ref().is_some_and(|r| r.group.alive()) || pl.starting;
            if let (Some(e), false) = (l.error, busy) {
                failed(pl, e);
                return Ok(status_of(&srv, &srv.host.plugins[&name]));
            }
        }
        let pl = srv.host.plugins.get_mut(&name).unwrap();
        if pl.run.as_ref().is_some_and(|r| r.group.alive()) || pl.starting {
            return Err(fail("already_running", format!("{name} is already running in this session (pid {}); not started again", pl.pid.map(|p| p.to_string()).unwrap_or("undefined".into()))));
        }
        pl.starting = true;
        let ready = prepare(pl);
        drop(srv);
        if ready {
            launch(&shared, &name);
        }
    }
    let mut srv = shared.borrow_mut();
    if let Some(pl) = srv.host.plugins.get_mut(&name) {
        pl.starting = false;
    }
    srv.settle();
    Ok(status_of(&srv, &srv.host.plugins[&name]))
}

// The token says which run of which plugin is talking. It tells plugins apart; it isn't a permission boundary against
// other code running as the user, which can reach the socket too.
fn plugin_hello(srv: &mut Server, p: &Value, c: u64) -> RpcResult {
    let pr = Params::new(p)?;
    pr.opt_str("caller")?;
    let token = pr.len("token", 1, None)?;
    let actions = pr.str_list("actions", 0, 1)?.unwrap_or_default();
    let Some(name) = srv.host.plugins.values().find(|x| x.run.as_ref().is_some_and(|r| !r.revoked && r.token == token)).map(|x| x.name.clone()) else {
        return Err(fail("plugin_unavailable", "that token doesn't belong to a running plugin of this server: it was stopped, exited, or never started"));
    };
    let old = srv.host.plugins[&name].client.filter(|old| *old != c);
    if let Some(conn) = old.and_then(|o| srv.clients.get(&o)).map(|o| o.conn.clone()) {
        conn.close(); // one live connection per run
    }
    let pl = srv.host.plugins.get_mut(&name).unwrap();
    pl.client = Some(c);
    pl.actions = actions;
    let log = pl.run.as_ref().unwrap().log.clone();
    if let Some(client) = srv.clients.get_mut(&c) {
        client.plugin = Some(name.clone());
        let plugin = name.clone();
        client.conn.on_late_reply(move |m| log.borrow_mut().note(&format!("modisa: a late reply from {plugin}, after its invocation had timed out (dropped): {}", m.to_string().chars().take(500).collect::<String>())));
    }
    srv.changed(); // its actions reach the palette
    Ok(json!({ "name": name, "protocol": PROTOCOL, "session": srv.session, "epoch": srv.epoch }))
}

async fn plugin_invoke(shared: Shared, p: Value, _c: u64) -> RpcResult {
    let pr = Params::new(&p)?;
    pr.opt_str("caller")?;
    let plugin = pr.len("plugin", 1, None)?;
    let action = pr.len("action", 1, None)?;
    let params = pr.record("params")?.map(Value::Object).unwrap_or(json!({}));
    let run = pr.opt_str("run")?;
    let target = match pr.nested("target")? {
        Some(t) => Some((t.len("pane", 1, None)?, t.len("instance", 1, None)?)),
        None => None,
    };
    let link = pr.opt_len("link", 1, Some(2048))?;
    let ui = pr.raw("ui").cloned();
    let (conn, invocation, log) = {
        let mut srv = shared.borrow_mut();
        let pl = need(&mut srv, &plugin)?;
        let (status, error) = (pl.status, pl.error.clone());
        let live = pl.run.as_ref().filter(|r| !r.revoked).map(|r| (r.id.clone(), r.log.clone()));
        let client = pl.client;
        let conn = client.and_then(|c| srv.clients.get(&c)).map(|c| c.conn.clone());
        let (Some(conn), Some((run_id, log))) = (conn, live) else {
            return Err(fail("plugin_unavailable", format!("{plugin} isn't connected ({status}{})", error.map(|e| format!(": {e}")).unwrap_or_default())));
        };
        if run.as_ref().is_some_and(|r| *r != run_id) {
            return Err(fail("plugin_unavailable", format!("{plugin} has restarted since that was shown; use what it shows now")));
        }
        // an action aimed at a pane (from a menu, key or the palette) reaches that process or nothing
        if let Some((pane, instance)) = &target {
            if srv.s.panes.get(pane).map(|p| &p.info.instance) != Some(instance) {
                return Err(fail("pane_gone", format!("pane {pane} has closed or restarted since")));
            }
        }
        let pl = &srv.host.plugins[&plugin];
        if !pl.actions.contains(&action) {
            let offers = if pl.actions.is_empty() { "none".to_string() } else { pl.actions.join(", ") };
            return Err(fail("no_such_action", format!("{plugin} has no action {action} (it offers: {offers})")));
        }
        // a clicked URL reaches only an action whose link pattern matches it
        if let Some(l) = &link {
            if !pl.manifest.iter().flat_map(|m| m.links.iter().flatten()).any(|e| e.action == action && link_matches(e, l)) {
                return Err(fail("invalid_params", format!("{plugin}'s {action} doesn't handle that link")));
            }
        }
        srv.host.invocations += 1;
        (conn, format!("{plugin}-{}", srv.host.invocations), log)
    };
    let mut req = json!({ "action": action, "params": params, "invocation": invocation });
    if let Some((pane, instance)) = target {
        req["target"] = json!({ "pane": pane, "instance": instance });
    }
    if let Some(l) = link {
        req["link"] = json!(l);
    }
    if let Some(u) = ui {
        req["ui"] = u;
    }
    let secs = *INVOKE_MS as f64 / 1000.0;
    match conn.request("plugin.action", req, Some(Duration::from_millis(*INVOKE_MS))).await {
        Ok(v) => Ok(v),
        Err(e) if e.code == "timeout" => {
            conn.notify("plugin.cancel", json!({ "invocation": invocation, "action": action })); // advisory: nothing proves the action stopped
            log.borrow_mut().note(&format!("modisa: {action} (invocation {invocation}) didn't answer within {secs}s; its outcome is unknown"));
            Err(fail("timeout", format!("{plugin} didn't answer {action} within {secs}s. Its outcome is unknown: it may still finish, and running it again can repeat its effects")))
        }
        Err(e) if e.message.starts_with("connection closed") => Err(fail("plugin_unavailable", format!("{plugin} disconnected before answering {action}; its outcome is unknown"))),
        Err(e) => Err(fail("plugin_error", format!("{plugin} {action}: {}", e.message))),
    }
}

// ---------- the TUI ----------

fn status_set(srv: &mut Server, p: &Value, c: u64) -> RpcResult {
    let pr = Params::new(p)?;
    let id = pr.len("id", 1, Some(40))?;
    let text = pr.str("text")?;
    let tone = pr.enum_or("tone", TONES, "fg")?;
    let action = pr.opt_len("action", 1, None)?;
    let name = ui_call(srv, c, action.as_deref())?;
    let has = srv.host.plugins[&name].ui.status.contains_key(&id);
    let size = srv.host.plugins[&name].ui.status.len();
    if !has && size >= STATUS_SEGMENTS {
        return Err(fail("error", format!("at most {STATUS_SEGMENTS} status segments per plugin")));
    }
    if !has && others(srv, &name).map(|x| x.ui.status.len()).sum::<usize>() + size >= SESSION_STATUS_SEGMENTS {
        return Err(fail("error", format!("at most {SESSION_STATUS_SEGMENTS} status segments across the session's plugins")));
    }
    let mut seg = json!({ "id": id, "text": clean_text(&text, STATUS_TEXT), "tone": tone });
    if let Some(a) = action {
        seg["action"] = json!(a);
    }
    srv.host.plugins.get_mut(&name).unwrap().ui.status.insert(id, seg);
    changed(srv, &name)
}

fn status_clear(srv: &mut Server, p: &Value, c: u64) -> RpcResult {
    let pr = Params::new(p)?;
    let id = pr.len("id", 1, Some(40))?;
    let name = ui_call(srv, c, None)?;
    srv.host.plugins.get_mut(&name).unwrap().ui.status.shift_remove(&id);
    changed(srv, &name)
}

fn sidebar_set(srv: &mut Server, p: &Value, c: u64) -> RpcResult {
    let pr = Params::new(p)?;
    let title = pr.str("title")?;
    let rows = pr.raw("rows").and_then(Value::as_array).cloned().ok_or_else(|| crate::protocol::schema::invalid("rows", "Invalid input: expected array, received undefined"))?;
    if rows.len() > 50 {
        return Err(crate::protocol::schema::invalid("rows", "Too big: expected array to have <=50 items"));
    }
    for (i, r) in rows.iter().enumerate() {
        let Value::Object(_) = r else { return Err(crate::protocol::schema::invalid(&format!("rows.{i}"), "Invalid input: expected object")) };
        let rp = Params::new(r)?;
        rp.opt_str("text")?;
        rp.opt_enum("tone", TONES).map_err(|e| crate::protocol::conn::fail(&e.code, e.message.replacen("invalid params: tone", &format!("invalid params: rows.{i}.tone"), 1)))?;
        if let Some(spans) = rp.raw("spans") {
            let Some(list) = spans.as_array().filter(|a| a.len() <= 16) else {
                return Err(crate::protocol::schema::invalid(&format!("rows.{i}.spans"), "Too big: expected array to have <=16 items"));
            };
            // each span is text (in a tone, bold or not) or an agent's icon
            for (j, x) in list.iter().enumerate() {
                let sp = Params::new(x)?;
                let ok = if sp.has("icon") { sp.opt_len("icon", 0, Some(40)).is_ok() } else { sp.opt_str("text").is_ok_and(|t| t.is_some()) && sp.opt_enum("tone", TONES).is_ok() && sp.opt_bool("bold").is_ok() };
                if !ok {
                    return Err(crate::protocol::schema::invalid(&format!("rows.{i}.spans.{j}"), "Invalid input"));
                }
            }
        }
    }
    let name = ui_call(srv, c, None)?;
    let pl = &srv.host.plugins[&name];
    for r in &rows {
        if let Some(a) = r["action"].as_str().filter(|a| !pl.actions.iter().any(|x| x == a)) {
            return Err(fail("no_such_action", format!("{name} didn't offer action {a} in hello")));
        }
    }
    if pl.ui.sidebar.is_none() && others(srv, &name).filter(|x| x.ui.sidebar.is_some()).count() >= SESSION_SIDEBAR_SECTIONS {
        return Err(fail("error", format!("at most {SESSION_SIDEBAR_SECTIONS} plugins' sidebar sections in a session")));
    }
    // a row that focuses a pane names the process it's for; clicking it later reaches that process or nothing
    for r in &rows {
        if let Some(pane) = r["pane"].as_str() {
            let instance = r["instance"].as_str();
            if srv.s.panes.get(pane).map(|p| p.info.instance.as_str()) != instance {
                return Err(fail("pane_gone", format!("no pane {pane} with instance {}", instance.unwrap_or("(none given: a row's pane needs its instance)"))));
            }
        }
    }
    let rows: Vec<Value> = rows
        .iter()
        .take(SIDEBAR_ROWS)
        .map(|r| {
            let spans = r["spans"].as_array().map(|s| clean_spans(s));
            let text = match &spans {
                Some(s) => s.iter().map(|x| x["icon"].as_str().map(|i| brand(i).glyph.to_string()).unwrap_or_else(|| x["text"].as_str().unwrap_or("").to_string())).collect(),
                None => clean_text(r["text"].as_str().unwrap_or(""), ROW_TEXT),
            };
            let mut v = json!({ "text": text, "tone": r["tone"].as_str().unwrap_or("fg") });
            if let Some(s) = spans {
                v["spans"] = json!(s);
            }
            if let Some(a) = r["action"].as_str() {
                v["action"] = json!(a);
            }
            if let Some(p) = r["pane"].as_str() {
                v["pane"] = json!(p);
                v["instance"] = r["instance"].clone();
            }
            v
        })
        .collect();
    srv.host.plugins.get_mut(&name).unwrap().ui.sidebar = Some(json!({ "title": clean_text(&title, SIDEBAR_TITLE), "rows": rows }));
    changed(srv, &name)
}

fn badge_set(srv: &mut Server, p: &Value, c: u64) -> RpcResult {
    let pr = Params::new(p)?;
    let pane = pr.len("pane", 1, None)?;
    let instance = pr.len("instance", 1, None)?;
    let text = pr.str("text")?;
    let tone = pr.enum_or("tone", TONES, "accent")?;
    let name = ui_call(srv, c, None)?;
    if srv.s.panes.get(&pane).map(|p| &p.info.instance) != Some(&instance) {
        return Err(fail("pane_gone", format!("no pane {pane} with instance {instance}: it closed or was restarted")));
    }
    let pl = &srv.host.plugins[&name];
    let has = pl.ui.badges.contains_key(&pane);
    if !has && pl.ui.badges.len() >= BADGES {
        return Err(fail("error", format!("at most {BADGES} badges per plugin")));
    }
    if !has && others(srv, &name).filter(|x| x.ui.badges.contains_key(&pane)).count() >= SESSION_BADGES_PER_PANE {
        return Err(fail("error", format!("at most {SESSION_BADGES_PER_PANE} plugins' badges on one pane")));
    }
    srv.host.plugins.get_mut(&name).unwrap().ui.badges.insert(pane.clone(), json!({ "pane": pane, "instance": instance, "text": clean_text(&text, BADGE_TEXT), "tone": tone }));
    changed(srv, &name)
}

fn menu_set(srv: &mut Server, p: &Value, c: u64) -> RpcResult {
    let pr = Params::new(p)?;
    let items = pr.raw("items").and_then(Value::as_array).cloned().ok_or_else(|| crate::protocol::schema::invalid("items", "Invalid input: expected array, received undefined"))?;
    if items.len() > 20 {
        return Err(crate::protocol::schema::invalid("items", "Too big: expected array to have <=20 items"));
    }
    for (i, it) in items.iter().enumerate() {
        let ip = Params::new(it)?;
        for (k, min, max) in [("id", 1, Some(40)), ("title", 0, None), ("action", 1, None)] {
            ip.len(k, min, max).map_err(|e| fail(&e.code, e.message.replacen(&format!("invalid params: {k}"), &format!("invalid params: items.{i}.{k}"), 1)))?;
        }
    }
    let name = ui_call(srv, c, None)?;
    let pl = &srv.host.plugins[&name];
    for it in &items {
        let a = it["action"].as_str().unwrap_or("");
        if !pl.actions.iter().any(|x| x == a) {
            return Err(fail("no_such_action", format!("{name} didn't offer action {a} in hello")));
        }
    }
    if others(srv, &name).map(|x| x.ui.menu.len()).sum::<usize>() + items.len().min(MENU_ITEMS) > SESSION_MENU_ITEMS {
        return Err(fail("error", format!("at most {SESSION_MENU_ITEMS} menu entries across the session's plugins")));
    }
    let menu = items.iter().take(MENU_ITEMS).map(|it| json!({ "id": it["id"], "title": clean_text(it["title"].as_str().unwrap_or(""), MENU_TITLE), "action": it["action"] })).collect();
    srv.host.plugins.get_mut(&name).unwrap().ui.menu = menu;
    changed(srv, &name)
}

// every attached client shows it (and a system notification, if asked and the user has those on); counted per run, so
// a restarted plugin starts with a fresh budget
fn toast(srv: &mut Server, p: &Value, c: u64) -> RpcResult {
    let pr = Params::new(p)?;
    let text = pr.str("text")?;
    let tone = pr.enum_or("tone", TONES, "fg")?;
    let system = pr.flag("system")?;
    let name = ui_call(srv, c, None)?;
    let run = srv.host.plugins[&name].run.as_ref().unwrap().id.clone();
    srv.toast(super::Toast { from: name.clone(), source: format!("plugin:{name}:{run}"), plugin: true, text, tone, system, sound: false })?;
    Ok(json!(true))
}

// ---------- panes ----------

fn pane_open(srv: &mut Server, p: &Value, c: u64) -> RpcResult {
    let pr = Params::new(p)?;
    pr.opt_str("caller")?;
    let plugin = pr.len("plugin", 1, None)?;
    let pane_id = pr.len("pane", 1, None)?;
    let params = pr.record("params")?.map(Value::Object).unwrap_or(json!({}));
    let run = pr.opt_str("run")?;
    let from = match pr.nested("from")? {
        Some(f) => Some((f.len("pane", 1, None)?, f.opt_str("instance")?)),
        None => None,
    };
    let pl = need(srv, &plugin)?;
    if !pl.live() {
        return Err(fail("plugin_unavailable", format!("{plugin} isn't running")));
    }
    if run.is_some_and(|r| Some(&r) != pl.run.as_ref().map(|x| &x.id)) {
        return Err(fail("plugin_unavailable", format!("{plugin} has restarted since that was shown; use what it shows now")));
    }
    let Some(def) = pl.manifest.as_ref().and_then(|m| m.panes.iter().flatten().find(|x| x.id == pane_id)).cloned() else {
        let ids = pl.manifest.as_ref().map(|m| m.panes.iter().flatten().map(|x| x.id.clone()).collect::<Vec<_>>().join(", ")).filter(|s| !s.is_empty()).unwrap_or("none".into());
        return Err(fail("no_such_action", format!("{plugin} has no pane {pane_id} (its plugin.json panes: {ids})")));
    };
    let dir = pl.dir.clone();
    // the pane it's for, as it was when asked for: never whatever pane has that id now
    if let Some((fp, Some(inst))) = &from {
        if srv.s.panes.get(fp).map(|p| &p.info.instance) != Some(inst) {
            return Err(fail("pane_gone", format!("pane {fp} has closed or restarted since")));
        }
    }
    let origin = match &from {
        Some((fp, _)) => srv.s.panes.get(fp).map(|p| (p.id().to_string(), p.info.instance.clone())),
        None => srv.s.focused_id().and_then(|f| srv.s.panes.get(&f).map(|p| (f.clone(), p.info.instance.clone()))),
    };
    let context = json!({ "plugin": plugin, "pane": origin.as_ref().map(|o| o.0.clone()), "instance": origin.as_ref().map(|o| o.1.clone()), "params": params });
    let mut env: IndexMap<String, String> = dirs_of(&plugin).into_iter().collect();
    env.insert("MODISA_PLUGIN_CONTEXT".into(), context.to_string());
    let opts = SpawnOpts { command: Some(def.run.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ")), cwd: dir, created_by: Some(format!("plugin:{plugin}")), env: Some(env), ..Default::default() };
    let in_layout = origin.as_ref().filter(|o| srv.s.locate(&o.0).is_some()).map(|o| o.0.clone()).or_else(|| srv.s.focused_id());
    let id = match def.placement.as_str() {
        "tab" => srv.s.new_tab(Some(def.title.clone()), opts)?,
        "popup" => {
            let client = srv.clients.get(&c);
            if !client.is_some_and(|c| c.attached) {
                return Err(fail("usage", "a popup opens in a TUI client (a key or the palette); from the CLI use a split, tab or zoomed pane"));
            }
            if !client.is_some_and(understands_plugins) {
                return Err(fail("usage", "this client is from before plugin popups: update modisa on this machine, or use a split, tab or zoomed pane"));
            }
            if !srv.host.popups.is_empty() {
                return Err(fail("ui_busy", "a popup is already open in this session"));
            }
            let id = srv.s.spawn_hidden(opts)?;
            srv.host.popups.insert(id.clone(), Popup { plugin: plugin.clone(), client: c });
            id
        }
        placement => {
            let was_zoomed = in_layout.as_ref().and_then(|p| srv.s.locate(p)).map(|(wi, ti)| srv.s.workspaces[wi].tabs[ti].zoomed).unwrap_or(false);
            let overlay = placement == "overlay";
            let id = srv.s.split(Axis::Row, SpawnOpts { ephemeral: overlay, ..opts }, in_layout.as_deref(), true, 0.5)?.ok_or_else(|| fail("error", "there's no pane to open it next to"))?;
            if placement != "split" {
                if let Some((wi, ti)) = srv.s.locate(&id) {
                    srv.s.workspaces[wi].tabs[ti].zoomed = true;
                }
                srv.s.layout();
            }
            if overlay {
                srv.host.overlays.insert(id.clone(), Overlay { plugin: plugin.clone(), origin: origin.as_ref().map(|o| o.0.clone()), instance: origin.map(|o| o.1), was_zoomed, had_focus: false });
            }
            id
        }
    };
    let pane = srv.s.panes.get_mut(&id).unwrap();
    pane.default_title = def.title.clone();
    pane.refresh_title();
    let instance = pane.info.instance.clone();
    srv.changed();
    let mut v = json!({ "pane": id, "instance": instance, "placement": def.placement, "title": def.title });
    if let Some(w) = def.width {
        v["width"] = w;
    }
    if let Some(h) = def.height {
        v["height"] = h;
    }
    Ok(v)
}

// ---------- views ----------

fn view_set(srv: &mut Server, p: &Value, c: u64) -> RpcResult {
    let pr = Params::new(p)?;
    pr.opt_str("caller")?;
    let id = pr.len("id", 1, Some(40))?;
    let title = pr.opt_str("title")?;
    let placement = pr.opt_enum("placement", &["popup", "overlay"])?;
    let w = pr.raw("width").cloned();
    let h = pr.raw("height").cloned();
    super::views::check_size("width", w.as_ref())?;
    super::views::check_size("height", h.as_ref())?;
    let from = match pr.nested("from")? {
        Some(f) => Some((f.len("pane", 1, None)?, f.len("instance", 1, None)?)),
        None => None,
    };
    let keys = pr.raw("keys").cloned();
    let close = pr.opt_len("close", 1, None)?;
    let focus = pr.opt_len("focus", 1, Some(80))?;
    let root = pr.raw("root").cloned().ok_or_else(|| crate::protocol::schema::invalid("root", "Invalid input"))?;
    super::views::check_shape(&root, "root")?;
    if let Some(k) = &keys {
        super::views::check_keys(k)?;
    }
    let name = ui_call(srv, c, None)?;
    let pl = &srv.host.plugins[&name];
    let old = pl.ui.views.get(&id).map(|v| v.state.clone());
    if old.is_none() && pl.ui.views.len() >= VIEW_LIMIT.views {
        return Err(fail("error", format!("at most {} views open per plugin", VIEW_LIMIT.views)));
    }
    if old.is_none() && views(srv).as_array().map_or(0, Vec::len) >= VIEW_LIMIT.session_views {
        return Err(fail("error", format!("at most {} views open across the session's plugins", VIEW_LIMIT.session_views)));
    }
    if let Some((fp, fi)) = &from {
        if srv.s.panes.get(fp).map(|p| &p.info.instance) != Some(fi) {
            return Err(fail("pane_gone", format!("pane {fp} has closed or restarted since")));
        }
    }
    // an update replaces what it shows; where and how it's shown stay as they were unless it says otherwise
    let was = old.as_ref();
    let close = close.or_else(|| was.and_then(|w| w["close"].as_str().map(String::from)));
    let keys = keys.or_else(|| was.map(|w| w["keys"].clone())).unwrap_or(json!([]));
    let (root, keys, rasters) = check_view(&root, &keys, close.as_deref(), &pl.actions)?;
    let width = w.or_else(|| was.and_then(|w| w.get("width").cloned()));
    let height = h.or_else(|| was.and_then(|w| w.get("height").cloned()));
    let from = from.map(|(p, i)| json!({ "pane": p, "instance": i })).or_else(|| was.and_then(|w| w.get("from").cloned()));
    let rev = was.and_then(|w| w["rev"].as_u64()).unwrap_or(0) + 1;
    let mut state = json!({
        "plugin": name, "run": pl.run.as_ref().unwrap().id, "id": id,
        "title": title.map(|t| clean_text(&t, MENU_TITLE)).or_else(|| was.and_then(|w| w["title"].as_str().map(String::from))).unwrap_or(id.clone()),
        "placement": placement.or_else(|| was.and_then(|w| w["placement"].as_str().map(String::from))).unwrap_or("popup".into()),
    });
    let o = state.as_object_mut().unwrap();
    if let Some(w) = width {
        o.insert("width".into(), w);
    }
    if let Some(h) = height {
        o.insert("height".into(), h);
    }
    if let Some(f) = from {
        o.insert("from".into(), f);
    }
    o.insert("keys".into(), keys);
    if let Some(c) = close {
        o.insert("close".into(), json!(c));
    }
    if let Some(f) = focus {
        o.insert("focus".into(), json!(f));
    }
    o.insert("root".into(), root);
    o.insert("rev".into(), json!(rev));
    let open = old.is_none();
    srv.host.plugins.get_mut(&name).unwrap().ui.views.insert(id.clone(), OpenView { state: state.clone(), rasters });
    srv.broadcast_to("plugin.view", state, &viewers(srv));
    Ok(json!({ "id": id, "rev": rev, "open": open }))
}

// A Raster repainted in place: the size it was drawn at, on its own budget so a plugin can animate one.
fn blit(srv: &mut Server, p: &Value, c: u64) -> RpcResult {
    let pr = Params::new(p)?;
    let view = pr.len("view", 1, Some(40))?;
    let id = pr.len("id", 1, Some(80))?;
    let cells = pr.str("cells")?;
    let name = ui_call(srv, c, None)?;
    let pl = srv.host.plugins.get_mut(&name).unwrap();
    let b = pl.ui.blits.get_or_insert_with(|| Bucket::full(BLITS.0));
    b.refill(BLITS.0, BLITS.1);
    if b.tokens < 1.0 {
        return Err(fail("rate_limited", format!("too many blits from {name}: at most {} a second", BLITS.1)));
    }
    b.tokens -= 1.0;
    let Some((cols, rows)) = pl.ui.views.get(&view).and_then(|v| v.rasters.get(&id)).copied() else {
        return Err(fail("invalid_params", format!("{name} has no raster {id} in an open view {view}")));
    };
    check_cells(&cells, cols, rows)?;
    // kept in the tree too, so a client attaching later draws what's there now
    fn swap(n: &mut Value, id: &str, cells: &str) {
        if n["type"] == "raster" && n["id"] == id {
            n["cells"] = json!(cells);
        } else if let Some(child) = n.get_mut("child") {
            swap(child, id, cells);
        } else if let Some(children) = n.get_mut("children").and_then(Value::as_array_mut) {
            for c in children {
                swap(c, id, cells);
            }
        }
    }
    if let Some(v) = pl.ui.views.get_mut(&view) {
        swap(&mut v.state["root"], &id, &cells);
    }
    srv.broadcast_to("plugin.blit", json!({ "plugin": name, "view": view, "id": id, "cells": cells }), &viewers(srv));
    Ok(json!(true))
}

// The user closed a view: gone for every client, and its plugin told, by its close action, if it gave one.
fn user_closed_view(srv: &mut Server, p: &Value, c: u64) -> RpcResult {
    if srv.clients.get(&c).is_some_and(|x| x.plugin.is_some()) {
        return Err(fail("usage", "plugin.view.close is the TUI's: a plugin closes its own views with ui.view.close"));
    }
    let pr = Params::new(p)?;
    let plugin = pr.len("plugin", 1, None)?;
    let id = pr.len("id", 1, None)?;
    let pl = need(srv, &plugin)?;
    let action = pl.ui.views.get(&id).and_then(|v| v.state["close"].as_str().map(String::from));
    let client = pl.client;
    let offered = action.as_ref().is_some_and(|a| pl.actions.contains(a));
    let log = pl.run.as_ref().map(|r| r.log.clone());
    if !close_view(srv, &plugin, &id) || !offered {
        return Ok(json!(true));
    }
    let Some(conn) = client.and_then(|c| srv.clients.get(&c)).map(|c| c.conn.clone()) else { return Ok(json!(true)) };
    srv.host.invocations += 1;
    let invocation = format!("{plugin}-{}", srv.host.invocations);
    let action = action.unwrap();
    tokio::task::spawn_local(async move {
        let req = json!({ "action": action, "params": {}, "invocation": invocation, "ui": { "view": id } });
        if let Err(e) = conn.request("plugin.action", req, Some(Duration::from_millis(*INVOKE_MS))).await {
            if let Some(log) = log {
                log.borrow_mut().note(&format!("modisa: {action}, run when view {id} closed: {}", e.message));
            }
        }
    });
    Ok(json!(true))
}

