// The session server: owns every PTY and speaks the protocol to TUI clients, the CLI, the integrations and plugins.
// This file is the shared state (the TypeScript `ServerContext`) and the startup/shutdown order; the pieces live in
// session/, rpc/, agents/, persist/.
//
// One thread: every connection, pane and timer is a task on the LocalSet, and the state is one `Rc<RefCell<Server>>`
// borrowed only between awaits, never across one.
pub mod agents;
pub mod attach;
pub mod env;
pub mod git;
pub mod keys;
pub mod permissions;
pub mod persist;
pub mod plugin_manager;
pub mod plugins;
pub mod pty;
pub mod rpc;
pub mod session;
pub mod views;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::time::Duration;

use indexmap::IndexMap;
use serde_json::{json, Value};
use tokio::sync::oneshot;
use tokio::task::AbortHandle;

use crate::config::agents::AgentDef;
use crate::config::{self, Config};
use crate::core::paths::{self, socket_path, DIR};
use crate::core::text::clean_text;
use crate::protocol::conn::{b64, fail, Conn, RpcResult};
use crate::protocol::transport::{connect_unix, running_pid, unreachable};
use agents::detect::Detector;
use agents::mailbox::Mailbox;
use session::pane::{now_ms, random_hex, PaneEvent, PtyPane};
use session::{Effect, Session, SpawnOpts};

pub type Shared = Rc<RefCell<Server>>;

// plugin: set once a plugin's connection has said plugin.hello; it then acts as that plugin, never as a pane
pub struct Client {
    pub conn: Conn,
    pub attached: bool,
    pub events: bool,
    pub output: bool,
    pub plugin: Option<String>,
    pub ui: u32, // the plugin UI version it attached with
}

// what a client can draw, by the plugin UI version it attached with (types.rs PLUGIN_UI): 1 status segments, sidebar
// sections, badges, menu entries, popups and toasts; 3 views too (examples/plugins/VIEWS.md; 2's are gone)
pub fn understands_plugins(c: &Client) -> bool {
    c.ui >= 1
}
pub fn understands_views(c: &Client) -> bool {
    c.ui >= 3
}

// A toast for the TUI, from a plugin (ui.toast) or anyone else (notify): `from` titles it, `source` is who's counted (a
// plugin's run, a pane, the user), `plugin` says which shared budget it comes out of. system and sound ask each client
// for those too, which it gives only where its user has them on.
pub struct Toast {
    pub from: String,
    pub source: String,
    pub plugin: bool,
    pub text: String,
    pub tone: String,
    pub system: bool,
    pub sound: bool,
}
// at most 3 every 10s from one source, and 6 from the session's plugins together, or from all the rest together
const TOASTS_PER_SOURCE: usize = 3;
const TOASTS_PER_BUCKET: usize = 6;
const TOASTS_WINDOW_MS: u64 = 10_000;
const TOAST_TEXT: usize = 120;

pub struct Server {
    pub session: String,
    pub version: String,
    pub epoch: String, // this server run, new on every start
    pub seq: u64, // the last event's seq
    pub cfg: Config,
    pub adapters: Vec<AgentDef>,
    pub s: Session,
    pub mail: Mailbox,
    pub detector: Detector,
    pub clients: IndexMap<u64, Client>,
    pub watchers: HashMap<String, Vec<u64>>, // pane → connections attached to it alone (attach.rs): sent its output, nothing else
    pub takeovers: HashMap<String, u64>, // pane → the connection driving it (attach.rs): no one else's input reaches it
    pub watching: HashMap<u64, attach::Watch>, // connection → the pane it's attached to (attach.rs)
    pub down: bool,
    pub prompts: IndexMap<u64, oneshot::Sender<String>>, // permission prompts waiting on an answer (permissions.rs)
    pub prompt_seq: u64,
    pub always: HashSet<String>, // permissions granted for good: "caller:action:target"
    pub cooldown: HashMap<String, u64>, // agents.monitor: no second message typed into a pane until then
    pub host: plugins::Host, // the plugin host (plugins.rs)
    pub me: Weak<RefCell<Server>>,
    client_seq: u64,
    view_queued: bool,
    save_timer: Option<AbortHandle>,
    toasts: HashMap<String, Vec<u64>>,
}

impl Server {
    pub fn shared(&self) -> Shared {
        self.me.upgrade().expect("the server is gone")
    }

    pub fn attached(&self) -> Vec<u64> {
        self.clients.iter().filter(|(_, c)| c.attached).map(|(id, _)| *id).collect()
    }

    pub fn broadcast(&self, event: &str, data: Value) {
        self.broadcast_to(event, data, &self.attached());
    }

    pub fn broadcast_to(&self, event: &str, data: Value, to: &[u64]) {
        for id in to {
            if let Some(c) = self.clients.get(id) {
                c.conn.notify(event, data.clone());
            }
        }
    }

    // shapes: `events` in protocol/schema.ts (an e2e test checks every emitted event against them)
    pub fn emit(&mut self, kind: &str, data: Value) {
        self.seq += 1;
        let mut ev = json!({ "type": kind, "at": now_ms(), "seq": self.seq, "epoch": self.epoch });
        if let (Some(ev), Value::Object(data)) = (ev.as_object_mut(), data) {
            ev.extend(data);
        }
        for c in self.clients.values() {
            if c.events && (kind != "pane.output" || c.output) {
                c.conn.notify("event", ev.clone());
            }
        }
    }

    // Not stored: a client attached later never sees it. Both limits are checked before either is spent. Returns how
    // many clients it reached.
    pub fn toast(&mut self, t: Toast) -> RpcResult<usize> {
        let now = now_ms();
        self.toasts.retain(|_, at| {
            at.retain(|x| now - x < TOASTS_WINDOW_MS);
            !at.is_empty()
        });
        let bucket = if t.plugin { " plugins" } else { " others" }; // no source starts with a space
        let within = format!("at most {TOASTS_PER_SOURCE} every {}s", TOASTS_WINDOW_MS / 1000);
        if self.toasts.get(&t.source).map_or(0, Vec::len) >= TOASTS_PER_SOURCE {
            return Err(fail("rate_limited", format!("too many toasts from {}: {within}", t.from)));
        }
        if self.toasts.get(bucket).map_or(0, Vec::len) >= TOASTS_PER_BUCKET {
            let who = if t.plugin { "plugins" } else { "panes and scripts" };
            return Err(fail("rate_limited", format!("too many toasts from the session's {who}: at most {TOASTS_PER_BUCKET} every {}s", TOASTS_WINDOW_MS / 1000)));
        }
        for k in [t.source.as_str(), bucket] {
            self.toasts.entry(k.to_string()).or_default().push(now);
        }
        let to: Vec<u64> = self.clients.iter().filter(|(_, c)| c.attached && understands_plugins(c)).map(|(id, _)| *id).collect(); // an older client can't draw one
        let mut data = json!({ "plugin": t.from, "text": clean_text(&t.text, TOAST_TEXT), "tone": t.tone, "system": t.system });
        if t.sound {
            data["sound"] = json!(true);
        }
        self.broadcast_to("plugin.toast", data, &to);
        Ok(to.len())
    }

    pub fn name(&self, id: &str) -> String {
        if id == "user" {
            return "user".into();
        }
        self.s.panes.get(id).and_then(|p| p.info.name.clone()).unwrap_or_else(|| id.into())
    }

    pub fn need(&self, target: Option<&str>, caller: Option<&str>) -> RpcResult<String> {
        if let Some(id) = self.s.resolve(target, caller) {
            return Ok(id);
        }
        let t = target.or(caller);
        if let Some((id, _)) = t.and_then(crate::protocol::types::instance_target) {
            return Err(fail("pane_gone", format!("{id} has gone: that pane was closed or restarted since its message was sent")));
        }
        Err(fail("no_such_pane", format!("no such pane: {}", t.unwrap_or("(none)"))))
    }

    // need, but no target means the calling pane, else the focused one
    pub fn subject(&self, target: Option<&str>, caller: Option<&str>) -> RpcResult<String> {
        match target {
            Some(t) => self.need(Some(t), None),
            None => {
                let own = caller.filter(|c| self.s.panes.contains_key(*c)).map(String::from);
                self.need(own.or_else(|| self.s.focused_id()).as_deref(), None)
            }
        }
    }

    pub fn pane(&self, id: &str) -> RpcResult<&PtyPane> {
        self.s.panes.get(id).ok_or_else(|| fail("no_such_pane", format!("no such pane: {id}")))
    }

    pub fn agent_opts(&self, harness: &str, prompt: Option<&str>, name: Option<String>, created_by: Option<String>) -> SpawnOpts {
        let a = self.adapters.iter().find(|x| x.id == harness);
        let command = match a {
            Some(a) if !a.launch.is_empty() => [Some(a.launch.clone()), prompt.filter(|p| !p.is_empty()).map(persist::template::quote)].into_iter().flatten().collect::<Vec<_>>().join(" "),
            _ => harness.to_string(),
        };
        SpawnOpts { command: Some(command), harness: Some(a.map(|a| a.id.clone()).unwrap_or_else(|| "generic".into())), name, created_by, ..Default::default() }
    }

    // screen and recentOutput are what pane.read returned before it took a source and format: kept for older readers
    pub fn snapshot(&self, id: &str, lines: usize, source: &str, format: &str) -> RpcResult<Value> {
        let p = self.pane(id)?;
        let mut v = serde_json::to_value(&p.info).unwrap();
        v["screen"] = json!(p.read("visible", "text", lines));
        v["recentOutput"] = json!(p.read("recent", "text", lines));
        v["content"] = json!(p.read(source, format, lines));
        v["source"] = json!(source);
        v["format"] = json!(format);
        Ok(v)
    }

    // ---------- after every operation ----------

    // What the session queued, handled in order; then clients get one view for however many changes, and the session
    // is saved a second after the last one.
    pub fn settle(&mut self) {
        self.drain_effects();
        if std::mem::take(&mut self.s.dirty) {
            self.changed();
        }
    }

    fn drain_effects(&mut self) {
        loop {
            let effects = std::mem::take(&mut self.s.effects);
            if effects.is_empty() {
                break;
            }
            for e in effects {
                match e {
                    Effect::Created(id) => {
                        if let Some(p) = self.s.panes.get(&id) {
                            let mut data = json!({ "pane": id, "instance": p.info.instance });
                            if let Some(n) = &p.info.name {
                                data["name"] = json!(n);
                            }
                            if let Some(c) = &p.info.command {
                                data["command"] = json!(c);
                            }
                            self.emit("pane.created", data);
                        }
                    }
                    Effect::Closing { id, focused, info } => self.pane_closing(&id, focused, &info),
                    Effect::Empty => {
                        let shared = self.shared();
                        tokio::task::spawn_local(async move { shutdown(shared, true, "exit").await });
                    }
                }
            }
        }
    }

    // State changes: coalesce client updates into one view per tick, debounce saves by a second.
    pub fn changed(&mut self) {
        if !self.view_queued {
            self.view_queued = true;
            let shared = self.shared();
            tokio::task::spawn_local(async move {
                let mut srv = shared.borrow_mut();
                srv.view_queued = false;
                srv.push_view();
            });
        }
        if let Some(t) = self.save_timer.take() {
            t.abort();
        }
        let shared = self.shared();
        self.save_timer = Some(
            tokio::task::spawn_local(async move {
                tokio::time::sleep(Duration::from_secs(1)).await;
                persist::store::save_shared(&shared, true).await;
            })
            .abort_handle(),
        );
    }

    pub fn push_view(&self) {
        let mut view = serde_json::to_value(self.s.view()).unwrap();
        view["paused"] = json!(self.mail.paused);
        let plugins = self.plugin_ui(); // once per push, however many clients
        for id in self.attached() {
            let c = &self.clients[&id];
            let mut v = view.clone();
            if understands_plugins(c) {
                v["plugins"] = plugins.clone();
            } else if let Some(o) = v.as_object_mut() {
                o.remove("plugins");
            }
            c.conn.notify("view", v);
        }
    }

    // what plugins show in the TUI
    pub fn plugin_ui(&self) -> Value {
        plugins::ui_view(self)
    }
    // the views plugins have open, for a client attaching
    pub fn plugin_views(&self) -> Value {
        plugins::views(self)
    }
    // whether a pane may leave its place (plugins: not an overlay, which belongs over its origin)
    pub fn movable(&self, id: &str) -> bool {
        plugins::movable(self, id)
    }

    // a pane is being closed; it's out of the session already
    fn pane_closing(&mut self, id: &str, focused: bool, info: &crate::protocol::types::PaneInfo) {
        plugins::pane_closing(self, id, focused);
        attach::pane_closing(self, id, info);
    }

    // ---------- panes' processes ----------

    pub fn on_pane_event(&mut self, id: &str, instance: &str, ev: PaneEvent) {
        let live = self.s.panes.get(id).is_some_and(|p| p.info.instance == instance);
        match ev {
            PaneEvent::Output(bytes) if live => self.pane_output(id, &bytes),
            PaneEvent::SyncTimeout if live => {
                if self.s.panes.get_mut(id).unwrap().flush_sync() {
                    self.s.changed();
                }
            }
            PaneEvent::Exited(code) if live => self.pane_exited(id, code),
            // a closed pane's process ending: its exit is still news, to events and whoever watched it
            PaneEvent::Exited(code) => {
                if let Some((info, _)) = self.s.retired.get_mut(instance).filter(|(i, _)| i.running()) {
                    info.status = "exited".into();
                    info.exit_code = Some(code);
                    let info = info.clone();
                    self.exited_event(&info);
                    self.s.changed();
                }
            }
            _ => {}
        }
        self.settle();
    }

    fn pane_output(&mut self, id: &str, bytes: &[u8]) {
        let p = self.s.panes.get_mut(id).unwrap();
        let retitled = p.feed(bytes);
        if let Some(at) = p.screen.sync_deadline() {
            let (id, instance, me) = (id.to_string(), p.info.instance.clone(), self.me.clone());
            tokio::task::spawn_local(async move {
                tokio::time::sleep_until(at.into()).await;
                if let Some(s) = me.upgrade() {
                    s.borrow_mut().on_pane_event(&id, &instance, PaneEvent::SyncTimeout);
                }
            });
        }
        if retitled {
            self.s.changed();
        }
        let p = &self.s.panes[id];
        let data = b64(bytes);
        self.broadcast("output", json!({ "pane": id, "data": data }));
        if let Some(watchers) = self.watchers.get(id).filter(|w| !w.is_empty()) {
            let watched = json!({ "pane": id, "data": attach::for_watchers(p, bytes, &data) });
            for c in watchers {
                if let Some(c) = self.clients.get(c).filter(|c| !c.attached) {
                    c.conn.notify("output", watched.clone()); // an attached one has it
                }
            }
        }
        let instance = p.info.instance.clone();
        if self.clients.values().any(|c| c.events && c.output) {
            self.emit("pane.output", json!({ "pane": id, "instance": instance, "text": String::from_utf8_lossy(bytes) }));
        } else {
            self.seq += 1; // every event this run emits takes a seq, subscribed or not
        }
    }

    fn pane_exited(&mut self, id: &str, code: i32) {
        let p = self.s.panes.get_mut(id).unwrap();
        p.info.status = "exited".into();
        p.info.exit_code = Some(code);
        let info = p.info.clone();
        // shells close their pane; command/agent panes stay so their output and exit code can be read
        if info.command.is_none() || p.ephemeral {
            if self.s.locate(id).is_some() {
                self.s.close(Some(id));
            } else {
                self.s.drop_hidden(id);
            }
        } else {
            self.s.changed();
        }
        // the close first, as it happened: an overlay learns whether it had the focus before its exit puts focus back
        self.drain_effects();
        self.exited_event(&info);
    }

    fn exited_event(&mut self, info: &crate::protocol::types::PaneInfo) {
        let mut data = json!({ "pane": info.id, "instance": info.instance });
        if let Some(n) = &info.name {
            data["name"] = json!(n);
        }
        data["exitCode"] = json!(info.exit_code.unwrap_or(0));
        self.emit("process.exited", data);
        plugins::pane_exited(self, &info.id);
        attach::pane_exited(self, info);
    }

    // ---------- connections ----------

    fn connected(&mut self, conn: Conn) -> u64 {
        self.client_seq += 1;
        let id = self.client_seq;
        self.clients.insert(id, Client { conn, attached: false, events: false, output: false, plugin: None, ui: 0 });
        id
    }

    fn disconnected(&mut self, id: u64) {
        self.clients.shift_remove(&id);
        plugins::disconnected(self, id);
        attach::disconnected(self, id);
        self.s.drag_end(id);
        self.s.changed();
        self.settle();
    }
}

// Save, tell clients why, stop the panes, and exit. empty: the session ended (its saved layout goes too).
pub async fn shutdown(shared: Shared, empty: bool, why: &str) {
    let session = {
        let mut srv = shared.borrow_mut();
        if srv.down {
            return;
        }
        srv.down = true;
        if let Some(t) = srv.save_timer.take() {
            t.abort();
        }
        srv.session.clone()
    };
    if empty {
        persist::store::forget(&session);
    } else {
        persist::store::save_shared(&shared, false).await;
    }
    let sock = socket_path(&session);
    let pid_file = format!("{}.pid", sock.trim_end_matches(".sock"));
    for c in shared.borrow().clients.values() {
        c.conn.notify(why, json!({}));
    }
    plugins::stop(&shared, None).await; // each plugin's whole process group, within its time limit
    shared.borrow_mut().s.destroy();
    // The pid file goes first, so nothing mistakes this exiting server for a running one it can't reach. Then the
    // socket file, while it's still ours: once the listener stops, `modisa restart` starts the next server at this same
    // path, and deleting it after that would cut the new server off.
    if std::fs::read_to_string(&pid_file).ok().as_deref() == Some(&std::process::id().to_string()) {
        let _ = std::fs::remove_file(&pid_file);
    }
    let _ = std::fs::remove_file(&sock);
    // let the goodbyes reach the clients, then go: a connection closing mid-shutdown must not keep the process alive
    tokio::time::sleep(Duration::from_millis(200)).await;
    std::process::exit(0);
}

pub async fn run_server(session: &str) -> i32 {
    let sock = socket_path(session);
    let pid_file = format!("{}.pid", sock.trim_end_matches(".sock"));
    // Two clients reconnecting after a restart can both start a server; the second one bows out.
    if let Ok(other) = connect_unix(&sock, |_, _| {}).await {
        other.close();
        println!("modisa server \"{session}\" is already running");
        return 0;
    }
    // Running but unreachable from here (a sandbox): taking its socket over would orphan every pane in it.
    if let Some(pid) = running_pid(&sock) {
        eprintln!("{}", unreachable(pid));
        return 0;
    }
    env::prepare_pane_env(session, &sock);
    let _ = std::fs::create_dir_all(&*DIR);

    let cfg = config::load_config();
    let adapters = config::adapters::load_adapters(&cfg);
    let shared: Shared = Rc::new_cyclic(|me: &Weak<RefCell<Server>>| {
        let sink_me = me.clone();
        let sink: session::pane::Sink = Rc::new(move |id: &str, instance: &str, ev: PaneEvent| {
            if let Some(s) = sink_me.upgrade() {
                s.borrow_mut().on_pane_event(id, instance, ev);
            }
        });
        RefCell::new(Server {
            session: session.to_string(),
            version: paths::code_version(),
            epoch: random_hex(4),
            seq: 0,
            cfg,
            adapters,
            s: Session::new(sink),
            mail: Mailbox::new(),
            detector: Detector::new(),
            clients: IndexMap::new(),
            watchers: HashMap::new(),
            takeovers: HashMap::new(),
            watching: HashMap::new(),
            down: false,
            prompts: IndexMap::new(),
            prompt_seq: 0,
            always: HashSet::new(),
            cooldown: HashMap::new(),
            host: plugins::Host::default(),
            me: me.clone(),
            client_seq: 0,
            view_queued: false,
            save_timer: None,
            toasts: HashMap::new(),
        })
    });
    {
        let me = Rc::downgrade(&shared);
        config::watch_config(move || {
            if let Some(s) = me.upgrade() {
                let mut srv = s.borrow_mut();
                srv.cfg = config::load_config();
                srv.adapters = config::adapters::load_adapters(&srv.cfg);
                srv.broadcast("config", json!({}));
            }
        });
    }
    agents::monitor::start(shared.clone());
    git::start(shared.clone());

    // ---------- socket ----------
    let _ = std::fs::remove_file(&sock);
    let listener = match crate::protocol::transport::bind(&sock) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("modisa: can't listen on {sock}: {e}");
            return 1;
        }
    };
    let _ = std::fs::write(&pid_file, std::process::id().to_string()); // lets clients tell "running but unreachable" from "dead"
    // connections are answered from now on, while plugins start (a test can hold that) and the first panes open
    {
        let shared = shared.clone();
        tokio::task::spawn_local(async move {
            loop {
                if let Ok((stream, _)) = listener.accept().await {
                    accept(&shared, stream);
                }
            }
        });
    }
    plugins::start(shared.clone()).await;

    // ---------- initial contents: saved session, else modisa.toml, else a shell ----------
    {
        let mut srv = shared.borrow_mut();
        let srv = &mut *srv;
        let saved = persist::store::load(session);
        let started = match saved.filter(|s| !s.workspaces.is_empty()) {
            Some(saved) => persist::restore::restore(srv, &saved),
            None => persist::template::apply_template(srv, &paths::cwd()),
        };
        if let Err(e) = started.and_then(|done| if done { Ok(()) } else { srv.s.new_workspace(None, Some(paths::cwd()), SpawnOpts::default()).map(|_| ()) }) {
            eprintln!("modisa: {e}");
        }
        srv.settle();
    }
    println!("modisa server \"{session}\" listening on {sock}");

    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();
    match term.as_mut() {
        Some(t) => {
            t.recv().await;
        }
        None => std::future::pending::<()>().await,
    }
    shutdown(shared.clone(), false, "exit").await;
    0
}

fn accept(shared: &Shared, stream: tokio::net::UnixStream) {
    let (r, w) = stream.into_split();
    let id = Rc::new(std::cell::Cell::new(0u64));
    let me = Rc::downgrade(shared);
    let mine = id.clone();
    let conn = Conn::spawn(r, w, move |_, m| {
        if let Some(s) = me.upgrade() {
            rpc::dispatch::dispatch(&s, mine.get(), m);
        }
    });
    id.set(shared.borrow_mut().connected(conn.clone()));
    let me = Rc::downgrade(shared);
    let client = id.get();
    // never handled inside the close itself: a close can happen while the server is borrowed (a full write queue)
    conn.on_close(move || {
        tokio::task::spawn_local(async move {
            if let Some(s) = me.upgrade() {
                s.borrow_mut().disconnected(client);
            }
        });
    });
}
