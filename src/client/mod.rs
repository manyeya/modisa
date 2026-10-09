// TUI client: renders what the server holds, sends input back. Local or over ssh.
//
// The TypeScript client was a tree of OpenTUI renderables kept in step with the server's view; this one is drawn whole
// every frame from the App's state (ratatui), and remembers where each frame put what can be clicked (`hits`). Like the
// server, everything runs on one thread: the App is one Rc<RefCell<…>>, borrowed only between awaits. Dialogs are async
// (modals.rs), so actions read like the original's: `let name = prompt(...).await`.
pub mod actions;
pub mod commands;
pub mod design;
pub mod draw;
pub mod input;
pub mod keys;
pub mod modals;
pub mod pane;
pub mod plugin_ui;
pub mod plugins;
pub mod settings;
pub mod slots;
pub mod sound;
pub mod views;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event};
use indexmap::IndexMap;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use serde_json::{json, Value};
use tokio::sync::{mpsc, Notify};

use crate::cli::sessions::{ClientOptions, OnMessage};
use crate::cli::update::Manifest;
use crate::config::keys::{all_keys, Bindings, Mode};
use crate::config::themes::{theme, Theme};
use crate::config::{self, parse_prefix, Config, NotifyKind, Prefix};
use crate::core::layout::{panes as tree_panes, Rect};
use crate::core::paths::code_version;
use crate::protocol::conn::{unb64, Conn};
use crate::protocol::types::{AgentState, PaneInfo, TabView, View, WorkspaceView, PLUGIN_UI};
use crate::vt::Screen;
use draw::Hit;
use modals::Modal;
pub use plugin_ui::tone_color;

pub type Shared = Rc<RefCell<App>>;

// One pane on screen: its own emulator, fed the server's output for it.
pub struct ClientPane {
    pub screen: Screen,
}

#[derive(Default)]
pub struct Toast {
    pub id: u64,
    pub text: String,
    pub color: &'static str,
    pub title: Option<String>,
    pub lines: Vec<ratatui::text::Line<'static>>, // a plugin's rich text, in place of `text`
    pub buttons: Vec<(String, Hit)>,             // what its buttons say, and what pressing one does
}

// A pane border (or the sidebar's edge) being dragged.
pub struct Resizing {
    pub x: i32,
    pub y: i32,
    pub sidebar: bool,
    pub saw_drag: bool, // this drag has reported motion with the button held
}

// What the left button is doing in a pane: selecting its text (anchored in its grid), or handed to its program.
pub struct Selecting {
    pub pane: String,
}

pub struct Search {
    pub matches: Vec<usize>,
    pub total: usize,
    pub i: usize,
}

const TOASTS: usize = 3; // cards shown at once; an older one beyond that goes

pub struct App {
    pub opts: ClientOptions,
    pub conn: Option<Conn>,
    pub view: Option<View>,
    pub cfg: Config,
    pub th: Theme,
    pub prefix: Prefix,
    pub bindings: Bindings,
    pub more_keys: IndexMap<String, String>, // after the prefix, beyond modisa's actions: lists, commands, modes, plugin:, sh:
    pub root_keys: IndexMap<String, String>, // without the prefix
    pub modes: IndexMap<String, Mode>,
    pub mode: Option<String>,  // the key mode it's in
    pub mode_at: Instant,      // its last key, for its timeout
    pub panes: HashMap<String, ClientPane>,
    pub sidebar: bool,
    pub prefix_armed: bool,
    pub copy_mode: bool,
    pub term_focused: bool, // this terminal has the keyboard (as far as it says)
    pub reported_focus: Option<String>, // the pane last told it has focus
    pub modal: Option<Modal>,
    pub search: Option<Search>,
    pub resizing: Option<Resizing>,
    pub selecting: Option<Selecting>,
    pub toasts: Vec<Toast>, // newest first
    pub collapsed_tabs: HashSet<String>, // tabs (by id) whose agents the user folded in the sidebar's graph
    pub collapsed_plugins: HashSet<String>, // plugins' sidebar sections the user folded
    pub views: Vec<views::OpenView>, // plugins' views open now, in the order they opened: the last is on top
    pub slot_elems: HashMap<String, slots::SlotElem>, // what the user did in plugins' sidebar elements
    pub animating: bool, // a spinner is showing: frames come on their own
    pub prompt_ids: HashSet<u64>, // open permission prompts
    pub hits: Vec<(Rect, Hit)>, // what the last frame put where, topmost last
    pub hover: Option<(i32, i32)>, // where the pointer is
    pub pointer_shape: &'static str,
    pub width: i32,
    pub height: i32,
    pub quitting: Option<String>,
    pub restarting: bool, // the server told us it's restarting: wait for the new one instead of giving up
    pub restarted_by_us: bool, // we asked for it, so we start the new server
    pub update: Option<Manifest>, // a newer modisa release, when one is out
    pub logos: Option<crate::platform::logos::Loaded>, // agents' logos, as much of modisa's logo font as this terminal has
    pub cell_guess: f64, // this terminal's cell height in ems, from its font, when it doesn't say its size in pixels
    pub me: Weak<RefCell<App>>,
    redraw: Rc<Notify>,
    quit: Rc<Notify>,
    toast_seq: u64,
    connecting: bool,
}

impl App {
    // A client in a terminal `size` cells big, before it has connected.
    pub fn new(opts: ClientOptions, cfg: Config, size: (i32, i32), me: Weak<RefCell<App>>, redraw: Rc<Notify>, quit: Rc<Notify>) -> App {
        let keys = all_keys(&cfg).0;
        App {
            opts,
            conn: None,
            view: None,
            th: *theme(&cfg),
            prefix: parse_prefix(&cfg.prefix),
            bindings: keys.prefix,
            more_keys: keys.more,
            root_keys: keys.root,
            modes: keys.modes,
            mode: None,
            mode_at: Instant::now(),
            sidebar: cfg.sidebar.visible,
            cfg,
            panes: HashMap::new(),
            prefix_armed: false,
            copy_mode: false,
            term_focused: true,
            reported_focus: None,
            modal: None,
            search: None,
            resizing: None,
            selecting: None,
            toasts: vec![],
            collapsed_tabs: HashSet::new(),
            collapsed_plugins: HashSet::new(),
            views: vec![],
            slot_elems: HashMap::new(),
            animating: false,
            prompt_ids: HashSet::new(),
            hits: vec![],
            hover: None,
            pointer_shape: "default",
            width: size.0,
            height: size.1,
            quitting: None,
            restarting: false,
            restarted_by_us: false,
            update: None,
            logos: None,
            cell_guess: 1.2,
            me,
            redraw,
            quit,
            toast_seq: 0,
            connecting: false,
        }
    }

    // this terminal's cell height in ems of its font: where a logo's halves meet
    pub fn cell_ems(&self) -> f64 {
        match crossterm::terminal::window_size() {
            Ok(w) if w.width > 0 && w.height > 0 && w.columns > 0 && w.rows > 0 => design::cell_ems(w.width as f64, w.height as f64, w.columns as f64, w.rows as f64),
            _ => self.cell_guess,
        }
    }

    pub fn shared(&self) -> Shared {
        self.me.upgrade().expect("the client is gone")
    }

    pub fn dirty(&self) {
        self.redraw.notify_one();
    }

    // ---------- geometry ----------
    pub fn metrics(&self) -> design::Chrome {
        design::chrome(self.width, self.height, self.sidebar, self.cfg.sidebar.width)
    }
    pub fn side_width(&self) -> i32 {
        self.metrics().side
    }
    pub fn area(&self) -> Rect {
        self.metrics().area
    }

    // ---------- the server's view ----------
    pub fn ws(&self) -> &WorkspaceView {
        let v = self.view.as_ref().unwrap();
        &v.workspaces[v.active]
    }
    pub fn tab(&self) -> &TabView {
        let ws = self.ws();
        &ws.tabs[ws.active.min(ws.tabs.len().saturating_sub(1))]
    }
    pub fn info(&self, id: &str) -> Option<&PaneInfo> {
        self.view.as_ref()?.panes.iter().find(|p| p.id == id)
    }
    pub fn ready(&self) -> bool {
        self.view.as_ref().is_some_and(|v| v.workspaces.get(v.active).is_some_and(|w| !w.tabs.is_empty()))
    }
    // the key that runs an action after the prefix, if one does
    pub fn key_for(&self, action: &str) -> Option<String> {
        self.bindings.iter().find(|(_, a)| **a == action).map(|(k, _)| k.clone())
    }
    pub fn icon(&self, state: AgentState) -> &'static str {
        use crate::config::IndicatorStyle::*;
        match (self.cfg.indicators.style, state) {
            (Dots, AgentState::Idle) | (Symbols, AgentState::Idle) => "○",
            (Dots, _) => "●",
            (Letters, s) => ["W", "B", "D", "I"][s as usize],
            (Symbols, AgentState::Blocked) => "!",
            (Symbols, AgentState::Working) => "◆",
            (Symbols, AgentState::Done) => "✓",
        }
    }
    pub fn state_color(&self, s: AgentState) -> &'static str {
        match s {
            AgentState::Working => self.th.working,
            AgentState::Blocked => self.th.blocked,
            AgentState::Done => self.th.done,
            AgentState::Idle => self.th.idle,
        }
    }
    // The current space's agents, what needs you first. Other spaces' agents reach you as notifications.
    pub fn sorted_agents(&self) -> Vec<PaneInfo> {
        let order = |s: AgentState| match s {
            AgentState::Blocked => 0,
            AgentState::Done => 1,
            AgentState::Working => 2,
            AgentState::Idle => 3,
        };
        let here: HashSet<String> = self.ws().tabs.iter().flat_map(|t| tree_panes(&t.tree)).collect();
        let mut agents: Vec<PaneInfo> = self.view.as_ref().unwrap().panes.iter().filter(|p| p.agent.is_some() && here.contains(&p.id)).cloned().collect();
        agents.sort_by_key(|p| order(p.agent.as_ref().unwrap().state));
        agents
    }

    // ---------- talking to the user and the server ----------
    pub fn call(&self, name: &str, args: Value) {
        let Some(conn) = self.conn.clone() else { return };
        let me = self.me.clone();
        let (name, args) = (name.to_string(), args);
        tokio::task::spawn_local(async move {
            if let Err(e) = conn.request("cmd", json!({ "name": name, "args": args }), None).await {
                if let Some(app) = me.upgrade() {
                    let th = app.borrow().th.blocked;
                    app.borrow_mut().toast(&e.message, th);
                }
            }
        });
    }

    pub fn notify_server(&self, method: &str, params: Value) {
        if let Some(c) = &self.conn {
            c.notify(method, params);
        }
    }

    // A card in the toast stack, in `color`'s border; `title` names who it's from (a plugin). Warnings and what needs
    // you stay twice as long as the rest.
    pub fn toast(&mut self, text: &str, color: &'static str) {
        self.toast_from(text, color, None, None);
    }

    pub fn toast_from(&mut self, text: &str, color: &'static str, ms: Option<u64>, title: Option<String>) {
        self.show_toast(Toast { text: text.to_string(), color, title, ..Default::default() }, ms);
    }

    pub fn show_toast(&mut self, mut t: Toast, ms: Option<u64>) {
        if self.quitting.is_some() {
            return;
        }
        let ms = ms.unwrap_or(if t.color == self.th.warn || t.color == self.th.blocked { 10_000 } else { 5_000 });
        self.toast_seq += 1;
        let id = self.toast_seq;
        t.id = id;
        for (_, h) in &mut t.buttons {
            if let Hit::Slot(s) = h {
                s.toast = Some(id); // pressed, a button takes its toast away
            }
        }
        self.toasts.insert(0, t);
        self.toasts.truncate(TOASTS);
        let me = self.me.clone();
        tokio::task::spawn_local(async move {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            if let Some(app) = me.upgrade() {
                let mut app = app.borrow_mut();
                app.toasts.retain(|t| t.id != id);
                app.dirty();
            }
        });
        self.dirty();
    }

    pub fn set_config(&mut self, cfg: Config) {
        self.th = *theme(&cfg);
        self.prefix = parse_prefix(&cfg.prefix);
        let keys = all_keys(&cfg).0;
        (self.bindings, self.more_keys, self.root_keys, self.modes) = (keys.prefix, keys.more, keys.root, keys.modes);
        if self.mode.as_ref().is_some_and(|m| !self.modes.contains_key(m)) {
            self.mode = None;
        }
        self.cfg = cfg;
        paint_background(self.th.bg);
        self.dirty();
    }

    // a copy of text, to the clipboard of the terminal this runs in (OSC 52), wherever that is: over ssh too
    pub fn copy(&self, text: &str) {
        let mut out = std::io::stdout();
        let _ = write!(out, "\x1b]52;c;{}\x07", crate::protocol::conn::b64(text.as_bytes()));
        let _ = out.flush();
    }
}

// The theme's background as the terminal's default background (OSC 11), so the window padding a terminal draws around
// its cells matches the TUI instead of framing it in the terminal's own colour; it gets its own back on exit (OSC 111).
fn paint_background(bg: &str) {
    let hex = bg.trim_start_matches('#');
    if hex.len() == 6 {
        let mut out = std::io::stdout();
        let _ = write!(out, "\x1b]11;rgb:{}/{}/{}\x07", &hex[0..2], &hex[2..4], &hex[4..6]);
        let _ = out.flush();
    }
}

// Agents' logos in the sidebar: installed the first time the TUI starts (a terminal only picks up a new font when it
// restarts, so they show from then on), and drawn wherever this terminal can show them. [sidebar] logos = "auto" (that),
// "on" (always: for a terminal modisa doesn't recognise) or "off"; MODISA_LOGOS=off overrides it (the test suite, which
// must never touch a real home's fonts).
pub async fn setup_logos(shared: Shared) {
    use crate::config::Logos;
    use crate::platform::logos::{install_logos_once, logos_loaded, logos_off, logos_visible, terminal_font, Loaded};
    let mode = if logos_off() { Logos::Off } else { shared.borrow().cfg.sidebar.logos };
    if mode == Logos::Auto {
        if let Ok(Some(done)) = install_logos_once().await {
            let place = if done.terminals.is_empty() { String::new() } else { format!(" for {}", done.terminals.join(", ")) };
            let text = if done.updated { format!("agent logos updated{place}: quit and reopen the terminal to centre them") } else { format!("agent logos installed{place}: quit and reopen the terminal to see them") };
            let mut app = shared.borrow_mut();
            let a = app.th.accent;
            app.toast(&text, a);
        }
    }
    // drawn where the terminal is set up for them, as much as it has loaded: an older font's whole logos until it's
    // restarted after an update, then the halves
    let logos = match mode {
        Logos::On => Some(Loaded::Halves),
        Logos::Auto if logos_visible() => Some(logos_loaded().await),
        _ => None,
    };
    // where a logo's halves meet, for a terminal that doesn't say how big its cells are
    let guess = terminal_font().ok().and_then(|f| design::font_ems(&f.family, f.line_height)).unwrap_or(1.2);
    let mut app = shared.borrow_mut();
    app.cell_guess = guess;
    if logos != app.logos {
        app.logos = logos;
        app.dirty();
    }
}

// ---------- notifications ----------

pub fn system_notification(text: &str) {
    system_notification_at(text, None)
}

// With `focus` ([notify] click = "focus", and terminal-notifier on the PATH), clicking it focuses that pane: (session,
// pane). Elsewhere a click does what the system does with it.
fn system_notification_at(text: &str, focus: Option<(&str, &str)>) {
    use crate::core::paths::{self_exe, which};
    if let (Some((session, pane)), Some(tn)) = (focus, which("terminal-notifier")) {
        let quote = |s: &str| format!("'{}'", s.replace('\'', r"'\''"));
        let run = format!("{} -s {} pane focus {}", quote(&self_exe()), quote(session), quote(pane));
        let _ = std::process::Command::new(tn).args(["-title", "modisa", "-message", text, "-execute", &run]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
        return;
    }
    if which("osascript").is_some() {
        let _ = std::process::Command::new("osascript").args(["-e", &format!("display notification {} with title \"modisa\"", serde_json::to_string(text).unwrap())]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
    } else if which("notify-send").is_some() {
        let _ = std::process::Command::new("notify-send").args(["modisa", text]).spawn();
    } else {
        let mut out = std::io::stdout();
        let _ = write!(out, "\x1b]9;{text}\x07");
        let _ = out.flush();
    }
}

// A built-in sound's name, or a file's path.
fn play_sound(name: &str, volume: f64) {
    if name.contains('/') {
        sound::play_file(&crate::core::paths::abs_path(name), volume);
    } else {
        sound::play(name, volume);
    }
}

// An agent's state changed while you weren't looking at it: told the ways [notify] (and [notify.<agent>]) say, but a
// muted pane makes no sound and no system notification.
fn notify(app: &mut App, state: AgentState, text: &str, pane: Option<&str>) {
    let info = pane.and_then(|p| app.info(p));
    let agent = info.and_then(|i| i.agent.as_ref()).map(|a| a.harness.clone());
    let muted = info.is_some_and(|i| i.muted);
    let kinds = app.cfg.notify.kinds_for(state, agent.as_deref()).to_vec();
    if kinds.contains(&NotifyKind::Toast) {
        let c = app.state_color(state);
        app.toast(text, c);
    }
    if kinds.contains(&NotifyKind::System) && !muted {
        let focus = (app.cfg.notify.click == "focus").then_some(pane).flatten().map(|p| (app.opts.session.as_str(), p));
        system_notification_at(text, focus);
    }
    if kinds.contains(&NotifyKind::Sound) && !muted {
        if let Some(s) = app.cfg.sound.for_agent(state, agent.as_deref()) {
            play_sound(&s, app.cfg.sound.volume);
        }
    }
    if kinds.contains(&NotifyKind::Bell) {
        let mut out = std::io::stdout();
        let _ = out.write_all(b"\x07");
        let _ = out.flush();
    }
}

// What a toast someone sent may do besides showing: a system notification and a sound only if it asked and the user has
// that kind on for some event. The sound is the one for the event its tone is, else the first event's that plays. A
// plugin's can be rich text, with buttons and its own timeout (slots.rs).
fn sent_toast(app: &mut App, d: &Value) {
    let (plugin, text, tone) = (d["plugin"].as_str().unwrap_or(""), d["text"].as_str().unwrap_or(""), d["tone"].as_str().unwrap_or("fg"));
    let color = tone_color(app, tone);
    let (lines, buttons, ms) = slots::toast(app, d);
    app.show_toast(Toast { text: text.to_string(), color, title: Some(plugin.to_string()), lines, buttons, ..Default::default() }, ms);
    let events = [AgentState::Blocked, AgentState::Done, AgentState::Working];
    let on = |kind: NotifyKind| events.iter().copied().filter(|e| app.cfg.notify.kinds(*e).contains(&kind)).collect::<Vec<_>>();
    if d["system"].as_bool() == Some(true) && !on(NotifyKind::System).is_empty() {
        system_notification(&format!("{plugin}: {text}"));
    }
    let sounding = on(NotifyKind::Sound);
    if d["sound"].as_bool() == Some(true) {
        if let Some(e) = sounding.iter().find(|e| e.as_str() == tone).or(sounding.first()) {
            if let Some(s) = app.cfg.sound.name(*e) {
                play_sound(s, app.cfg.sound.volume);
            }
        }
    }
}

// config.toml changed: apply it, unless it doesn't parse (half-edited, say: keep what's applied).
pub fn reload(app: &mut App, manual: bool) {
    let (next, error) = config::read_config();
    if let Some(e) = error {
        let w = app.th.warn;
        return app.toast(&format!("config.toml: {e} (modisa config check)"), w);
    }
    // unchanged: usually the settings page saving what it already applied
    if serde_json::to_value(&next).ok() == serde_json::to_value(&app.cfg).ok() {
        if manual {
            let d = app.th.dim;
            app.toast("config unchanged", d);
        }
        return;
    }
    if next.sidebar.visible != app.cfg.sidebar.visible {
        app.sidebar = next.sidebar.visible;
    }
    let logos = next.sidebar.logos != app.cfg.sidebar.logos;
    app.set_config(next);
    if logos {
        tokio::task::spawn_local(setup_logos(app.shared()));
    }
    let area = app.area();
    app.notify_server("area", json!({ "area": area }));
    let a = app.th.accent;
    app.toast("config reloaded", a);
}

// ---------- the connection ----------

pub fn quit(app: &mut App, why: &str) {
    if app.quitting.is_some() {
        return;
    }
    if why == "detached" {
        if let Some(conn) = app.conn.clone().filter(|c| !c.closed()) {
            tokio::task::spawn_local(async move {
                let _ = conn.request("detach", json!({}), Some(Duration::from_millis(500))).await;
            });
        }
    }
    app.quitting = Some(match why {
        "detached" => format!("[detached from {}]", app.opts.session),
        "exited" => "[modisa exited]".into(),
        other => other.into(),
    });
    app.modal = None;
    app.quit.notify_one();
}

fn on_message(app: &mut App, m: Value) {
    let d = &m["params"];
    match m["method"].as_str().unwrap_or("") {
        "view" => {
            if let Ok(v) = serde_json::from_value::<View>(d.clone()) {
                set_view(app, v);
            }
        }
        "output" => {
            if let Some(p) = d["pane"].as_str().and_then(|id| app.panes.get_mut(id)) {
                p.screen.write(&unb64(d["data"].as_str().unwrap_or("")));
                app.dirty();
            }
        }
        "notify" => {
            if let Some(s) = d["state"].as_str().and_then(AgentState::parse) {
                notify(app, s, d["text"].as_str().unwrap_or(""), d["pane"].as_str());
            }
        }
        "plugin.toast" => sent_toast(app, d),
        "plugin.view" => views::view_set(app, d.clone()),
        "plugin.view.closed" => views::view_closed(app, d["plugin"].as_str().unwrap_or(""), d["id"].as_str().unwrap_or("")),
        "plugin.blit" => views::view_blit(app, d),
        "prompt" => {
            let (id, text) = (d["id"].as_u64().unwrap_or(0), d["text"].as_str().unwrap_or("").to_string());
            modals::permission(app, id, text);
        }
        "prompt.done" => {
            let id = d["id"].as_u64().unwrap_or(0);
            if app.prompt_ids.remove(&id) {
                modals::close(app, None);
            }
        }
        "detach" => quit(app, "detached"),
        "exit" => quit(app, "exited"),
        "restart" => {
            app.restarting = true;
            let w = app.th.warn;
            app.toast("server restarting…", w);
        }
        _ => {}
    }
}

// What the server sent, and the panes' emulators made to match it: one per pane, at the pane's size.
fn set_view(app: &mut App, v: View) {
    let live: HashSet<&String> = v.panes.iter().map(|p| &p.id).collect();
    app.panes.retain(|id, _| live.contains(id));
    for p in &v.panes {
        let pane = app.panes.entry(p.id.clone()).or_insert_with(|| ClientPane { screen: Screen::new(p.cols, p.rows, 10_000) });
        if pane.screen.term.grid().columns() != p.cols as usize || pane.screen.rows() != p.rows as usize {
            pane.screen.resize(p.cols, p.rows);
        }
    }
    // a plugin popup this client opened: its process ended, so it's gone
    let popup_gone = plugin_ui::popup(app).is_some_and(|p| !v.panes.iter().any(|x| x.id == p.pane));
    app.view = Some(v);
    if popup_gone {
        modals::close(app, None);
    }
    slots::keep_elems(app);
    views::animate(app); // a spinner in a plugin's sidebar section
    // Older servers could leave a selected, empty workspace after a spawn error. Keep server indices intact and move
    // back to a workspace that has a terminal.
    let v = app.view.as_ref().unwrap();
    if v.workspaces.get(v.active).is_some_and(|w| w.tabs.is_empty()) {
        if let Some(i) = v.workspaces.iter().position(|w| !w.tabs.is_empty()) {
            app.view.as_mut().unwrap().active = i;
            app.call("selectWorkspace", json!({ "index": i }));
        }
    }
    app.dirty();
}

use alacritty_terminal::grid::Dimensions;

async fn attach(shared: &Shared, spawn: bool) -> crate::protocol::conn::RpcResult<()> {
    let (connect, me) = {
        let app = shared.borrow();
        (app.opts.connect.clone(), app.me.clone())
    };
    let handler: OnMessage = {
        let me = me.clone();
        Rc::new(move |_c: &Conn, m: Value| {
            if let Some(app) = me.upgrade() {
                on_message(&mut app.borrow_mut(), m);
            }
        })
    };
    let conn = connect(spawn, handler).await?;
    if shared.borrow().quitting.is_some() {
        conn.close();
        return Ok(());
    }
    shared.borrow_mut().conn = Some(conn.clone());
    let area = shared.borrow().area();
    // the view and the screens are applied the moment their replies are read: a view or output the server sent after
    // them is then applied on top, never overwritten by them
    let weak = me.clone();
    let res = conn
        .request_ordered("attach", json!({ "area": area, "ui": PLUGIN_UI }), move |r| {
            let (Ok(res), Some(app)) = (r, weak.upgrade()) else { return };
            let mut app = app.borrow_mut();
            app.panes.clear(); // reconnect: rebuild from the replay
            if let Ok(v) = serde_json::from_value::<View>(res.clone()) {
                set_view(&mut app, v);
            }
            views::clear_views(&mut app);
            for v in res["views"].as_array().into_iter().flatten() {
                views::view_set(&mut app, v.clone());
            }
        })
        .await?;
    let weak = me.clone();
    conn.request_ordered("replay", json!({}), move |r| {
        let (Ok(replay), Some(app)) = (r, weak.upgrade()) else { return };
        let mut app = app.borrow_mut();
        for r in replay.as_array().into_iter().flatten() {
            if let Some(p) = r["pane"].as_str().and_then(|id| app.panes.get_mut(id)) {
                p.screen.write(&unb64(r["data"].as_str().unwrap_or("")));
            }
        }
    })
    .await?;
    {
        let mut app = shared.borrow_mut();
        for id in res["prompts"].as_array().into_iter().flatten().filter_map(Value::as_u64) {
            modals::permission(&mut app, id, "(pending permission request)".into());
        }
        if res["version"].as_str() != Some(&code_version()) {
            let palette = app.key_for("palette");
            let how = match palette {
                Some(k) => format!("{} {k}", app.cfg.prefix.replace("C-", "^").to_uppercase()),
                None => "the command palette".into(),
            };
            let w = app.th.warn;
            app.toast_from(&format!("this client and the session's server run different modisa builds · detach and reattach, or {how} → Restart server"), w, Some(12_000), None);
        }
        app.dirty();
    }
    // when it drops: plugins' UI can't reach the session now, and one reconnect loop takes over
    let weak = me.clone();
    let mine = conn.clone();
    conn.on_close(move || {
        tokio::task::spawn_local(async move {
            let Some(app) = weak.upgrade() else { return };
            {
                let mut a = app.borrow_mut();
                if a.quitting.is_some() || a.conn.as_ref() != Some(&mine) {
                    return;
                }
                // plugins' actions can't reach the session now: hide what they show (the next attach brings it back), and
                // a popup this client had open is gone with the connection
                if plugin_ui::popup(&a).is_some() {
                    modals::close(&mut a, None);
                }
                views::clear_views(&mut a);
                if let Some(v) = a.view.as_mut() {
                    v.plugins = None;
                    v.slots.clear();
                }
                a.dirty();
            }
            connect_with_retry(&app, true).await;
        });
    });
    Ok(())
}

pub async fn connect_with_retry(shared: &Shared, reconnecting: bool) {
    {
        let mut app = shared.borrow_mut();
        if app.connecting || app.quitting.is_some() {
            return;
        }
        app.connecting = true;
    }
    let mut failure = String::new();
    let mut attempt = 0;
    loop {
        let (quitting, remote, restarting, restarted_by_us) = {
            let a = shared.borrow();
            (a.quitting.is_some(), a.opts.remote, a.restarting, a.restarted_by_us)
        };
        if quitting || !(remote || attempt < if restarting { 40 } else { 3 }) {
            break;
        }
        if reconnecting || attempt > 0 {
            if !restarting {
                let mut a = shared.borrow_mut();
                let w = a.th.warn;
                a.toast("connection lost — reconnecting…", w);
            }
            tokio::time::sleep(Duration::from_millis(if remote { 1000 } else { 350 })).await;
            if shared.borrow().quitting.is_some() {
                break;
            }
        }
        // first attach starts the server; after a restart, the client that asked starts it (others wait ~2s first)
        let spawn = !reconnecting || restarted_by_us || (restarting && attempt >= 6);
        match attach(shared, spawn).await {
            Ok(()) => {
                let mut a = shared.borrow_mut();
                let d = a.th.done;
                if a.restarting {
                    a.toast("server restarted", d);
                } else if reconnecting || attempt > 0 {
                    a.toast("reconnected", d);
                }
                a.restarting = false;
                a.restarted_by_us = false;
                a.connecting = false;
                return;
            }
            Err(e) => {
                failure = e.message;
                if let Some(c) = shared.borrow_mut().conn.take() {
                    c.close();
                }
            }
        }
        attempt += 1;
    }
    let mut a = shared.borrow_mut();
    a.connecting = false;
    if a.quitting.is_none() {
        let why = format!("[could not connect to {}: {failure}]", a.opts.session);
        a.quitting = Some(why);
        a.quit.notify_one();
        process_exit::set(1);
    }
}

// ---------- startup ----------

// the exit status the client leaves with: 1 when it couldn't connect
pub mod process_exit {
    use std::sync::atomic::{AtomicI32, Ordering};
    pub static CODE: AtomicI32 = AtomicI32::new(0);
    pub fn set(code: i32) {
        CODE.store(code, Ordering::Relaxed);
    }
}

fn setup_terminal() -> std::io::Result<Terminal<CrosstermBackend<std::io::Stdout>>> {
    use crossterm::event::{EnableBracketedPaste, EnableFocusChange, EnableMouseCapture, KeyboardEnhancementFlags, PushKeyboardEnhancementFlags};
    crossterm::terminal::enable_raw_mode()?;
    let mut out = std::io::stdout();
    crossterm::execute!(out, crossterm::terminal::EnterAlternateScreen, EnableMouseCapture, EnableBracketedPaste, EnableFocusChange)?;
    if crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false) {
        let _ = crossterm::execute!(out, PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES));
    }
    Terminal::new(CrosstermBackend::new(out))
}

fn restore_terminal() {
    use crossterm::event::{DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, PopKeyboardEnhancementFlags};
    let mut out = std::io::stdout();
    let _ = crossterm::execute!(out, PopKeyboardEnhancementFlags, DisableMouseCapture, DisableBracketedPaste, DisableFocusChange, crossterm::cursor::Show, crossterm::terminal::LeaveAlternateScreen);
    let _ = write!(out, "\x1b]111\x07\x1b]22;default\x07");
    let _ = out.flush();
    let _ = crossterm::terminal::disable_raw_mode();
}

pub async fn run_client(opts: ClientOptions) -> i32 {
    let cfg = config::load_config();
    let mut terminal = match setup_terminal() {
        Ok(t) => t,
        Err(e) => {
            restore_terminal();
            eprintln!("modisa: can't start the TUI: {e}");
            return 1;
        }
    };
    // panics put the terminal back before they print
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        hook(info);
    }));
    let size = terminal.size().unwrap_or_default();
    let redraw = Rc::new(Notify::new());
    let quit_signal = Rc::new(Notify::new());
    let shared: Shared = Rc::new_cyclic(|me| RefCell::new(App::new(opts, cfg, (size.width as i32, size.height as i32), me.clone(), redraw.clone(), quit_signal.clone())));
    // which graphics protocol the terminal speaks, for plugins' images: asked before anything else reads it or is waiting
    // on what the client writes
    views::image::start(&shared);
    paint_background(shared.borrow().th.bg);

    // input on a thread of its own: crossterm's reads block
    let (tx, mut rx) = mpsc::unbounded_channel::<Event>();
    std::thread::spawn(move || {
        while let Ok(e) = event::read() {
            if tx.send(e).is_err() {
                return;
            }
        }
    });
    {
        let me = Rc::downgrade(&shared);
        config::watch_config(move || {
            if let Some(app) = me.upgrade() {
                reload(&mut app.borrow_mut(), false);
            }
        });
    }
    // a newer release lights the ↑ badge in the status row (checked in the background, cached 6h)
    {
        let me = Rc::downgrade(&shared);
        tokio::task::spawn_local(async move {
            tokio::time::sleep(Duration::from_millis(1500)).await;
            loop {
                let found = crate::cli::update::check_for_update(false).await;
                let Some(app) = me.upgrade() else { return };
                app.borrow_mut().update = found;
                app.borrow().dirty();
                drop(app);
                tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
            }
        });
    }
    tokio::task::spawn_local(setup_logos(shared.clone()));
    {
        let s = shared.clone();
        tokio::task::spawn_local(async move { connect_with_retry(&s, false).await });
    }

    loop {
        tokio::select! {
            _ = quit_signal.notified() => break,
            Some(e) = rx.recv() => {
                // one event at a time, letting what each one started run before the next: a key that opens a dialog
                // has it open before the keys typed after it arrive, as the TypeScript client's synchronous handlers did
                input::event(&shared, e);
                tokio::task::yield_now().await;
                while let Ok(e) = rx.try_recv() {
                    input::event(&shared, e);
                    tokio::task::yield_now().await;
                }
                if shared.borrow().quitting.is_some() { break; }
                draw_now(&shared, &mut terminal);
            }
            _ = redraw.notified() => {
                // a burst of output draws once: what's queued in the next few ms joins this frame
                tokio::time::sleep(Duration::from_millis(4)).await;
                if shared.borrow().quitting.is_some() { break; }
                draw_now(&shared, &mut terminal);
            }
        }
    }
    drop(terminal);
    restore_terminal();
    let why = shared.borrow().quitting.clone().unwrap_or_default();
    if let Some(c) = shared.borrow().conn.clone() {
        // a moment for the detach request to leave
        tokio::time::sleep(Duration::from_millis(50)).await;
        c.close();
    }
    println!("{why}");
    process_exit::CODE.load(std::sync::atomic::Ordering::Relaxed)
}

fn draw_now(shared: &Shared, terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>) {
    let mut app = shared.borrow_mut();
    input::report_focus(&mut app);
    let _ = terminal.draw(|f| draw::render(&mut app, f));
}
