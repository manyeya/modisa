// The settings page: the sections down the side, the section's rows beside them, what the selected row does under them.
// Typing searches every section at once. ↑↓ (or the pointer) moves the selection, ←→ changes a value, ↵ / space / a
// click applies, tab switches section, the wheel scrolls, esc closes. Every change applies at once and is saved to
// config.toml.
//
// Rows read the live config. Where the TypeScript page gave each row closures, a row here carries what it does (`Act`),
// which `activate` and `step` carry out on the App.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Modifier;
use serde_json::{json, Value};

use super::design::{fit, mix};
use super::draw::{Canvas, Hit};
use super::modals::{matches, typed, ListItem};
use super::{actions, App};
use crate::config::themes::THEMES;
use crate::config::{save_setting, Config, IndicatorStyle, NotifyKind, CONFIG_PATH};
use crate::core::layout::Rect;
use crate::core::paths::HOME;
use crate::core::text::width;
use crate::core::version::VERSION;
use crate::protocol::types::{AgentState, IntegrationStatus};

const HEIGHT: i32 = 32;
const WIDTH: i32 = 100;
const NAV: i32 = 18; // the side list's width
pub const SECTIONS: [&str; 11] = ["theme", "general", "layout", "git", "indicators", "sound", "alerts", "agents", "integrations", "plugins", "slots"];
const EVENTS: [(AgentState, &str); 3] = [(AgentState::Blocked, "needs you"), (AgentState::Done, "done"), (AgentState::Working, "started working")];
const ALERTS: [(NotifyKind, &str, &str); 3] = [
    (NotifyKind::Toast, "toast", "A line in the top right corner of modisa"),
    (NotifyKind::System, "system notification", "Your desktop's notification, for when you're in another window"),
    (NotifyKind::Bell, "terminal bell", "The terminal's bell: a sound or a flash, as your terminal does it"),
];
// What a plugin can draw instead of modisa (examples/plugins/CHROME.md), and who does is [slots] in config.toml.
const SLOTS: [(&str, &str, &str); 9] = [
    ("agent.row", "agent rows", "An agent's rows in the sidebar's AGENTS list"),
    ("sidebar.agents", "agents list", "The sidebar's AGENTS list, all of it"),
    ("pane.title", "pane titles", "A pane's border title: its name, agent and state"),
    ("tab", "tab labels", "A tab's name in the tab bar"),
    ("space", "space chip", "The space's name at the left of the tab bar"),
    ("status.agents", "agent counts", "The status row's working and need-you counts"),
    ("status.panes", "pane count", "The status row's pane count"),
    ("status.git", "git", "The status row's repository and branch"),
    ("status.theme", "theme name", "The status row's theme name"),
];
// prefixes that don't take a key shells and programs need (Ctrl+C, Ctrl+D, Ctrl+M is Enter, Ctrl+I is Tab…)
const PREFIXES: [&str; 9] = ["a", "b", "g", "o", "q", "s", "t", "x", "y"];

// What a row does when it's applied (↵, a click) or stepped (←→).
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    None,
    Theme(String),
    Flip(Option<&'static str>, &'static str), // a boolean setting: [table] key
    Prefix,
    Channel,
    SidebarWidth,
    Logos,
    Border,
    Style(IndicatorStyle),
    Sound(AgentState),
    Volume,
    Alert(AgentState, NotifyKind),
    Policy(&'static str),
    Number(&'static str, i64, i64),
    Run(&'static str),
    Integrations(Vec<String>, bool), // ids, install (else remove)
    Plugin(String, bool), // name, running
    Plugins(&'static str), // the plugin manager, at one of its views
    Slot(&'static str),    // who draws a slot
}

#[derive(Clone, Debug)]
pub enum Kind {
    Heading,
    Radio { current: bool, swatches: Vec<&'static str> },
    Toggle { on: bool },
    Choice { value: String },
    Action { status: String, tone: &'static str, note: Option<String>, hint: String },
}

#[derive(Clone, Debug)]
pub struct Row {
    pub kind: Kind,
    pub label: String,
    pub about: String,
    pub act: Act,
}

fn heading(label: &str) -> Row {
    Row { kind: Kind::Heading, label: label.into(), about: String::new(), act: Act::None }
}

fn toggle(label: &str, on: bool, act: Act, about: &str) -> Row {
    Row { kind: Kind::Toggle { on }, label: label.into(), about: about.into(), act }
}

fn choice(label: &str, value: impl Into<String>, act: Act, about: &str) -> Row {
    Row { kind: Kind::Choice { value: value.into() }, label: label.into(), about: about.into(), act }
}

fn action(label: &str, status: impl Into<String>, tone: &'static str, note: Option<String>, hint: &str, act: Act, about: &str) -> Row {
    Row { kind: Kind::Action { status: status.into(), tone, note, hint: hint.into() }, label: label.into(), about: about.into(), act }
}

fn focusable(r: Option<&Row>) -> bool {
    r.is_some_and(|r| !matches!(r.kind, Kind::Heading))
}

pub struct Settings {
    pub current: usize,
    pub sel: usize,
    pub query: String,
    saved_theme: String, // the theme before a preview
    pub close: bool, // a row asked for the page to close (it opens something of its own)
    integrations: Option<Vec<IntegrationStatus>>,
    plugins: Option<Vec<Value>>,
    busy: String,
}

impl Settings {
    pub fn new(app: &App, start: &str) -> Settings {
        Settings { current: SECTIONS.iter().position(|s| *s == start).unwrap_or(0), sel: 0, query: String::new(), saved_theme: app.cfg.theme.clone(), close: false, integrations: None, plugins: None, busy: String::new() }
    }
}

// ---------- the rows ----------

fn rows(app: &App, s: &Settings, section: usize) -> Vec<Row> {
    let cfg = &app.cfg;
    match SECTIONS[section] {
        "theme" => THEMES
            .iter()
            .map(|(name, p)| Row { kind: Kind::Radio { current: *name == s.saved_theme, swatches: vec![p.focus, p.accent, p.working, p.blocked, p.done] }, label: name.to_string(), about: "Previewed as you move; ↵ or a click keeps it".into(), act: Act::Theme(name.to_string()) })
            .collect(),
        "general" => vec![
            heading("keyboard"),
            choice("prefix key", &cfg.prefix, Act::Prefix, "The key that starts every modisa shortcut: Ctrl and a letter. Press it twice to send it to the pane"),
            action("keyboard guide", "every shortcut", "dim", None, "↵ open", Act::Run("help"), "The shortcuts, searchable; choosing one runs it"),
            heading("mouse"),
            toggle("select on hover", cfg.mouse.hover, Act::Flip(Some("mouse"), "hover"), "The pointer resting on a row selects it, here and in every menu and picker. Off: only a click does"),
            heading("updates"),
            toggle("check for updates", cfg.update.check, Act::Flip(Some("update"), "check"), "Look for a new release every few hours; the status row says when one is out"),
            choice("channel", json!(cfg.update.channel).as_str().unwrap_or("stable"), Act::Channel, "stable: releases. staging: a prerelease of what's coming"),
            action("check now", VERSION, "dim", None, "↵ check", Act::Run("update-modisa"), "Look for a newer modisa now"),
            heading("config file"),
            action("edit config.toml", "in $EDITOR", "dim", None, "↵ open", Act::Run("edit-config"), "Everything on this page, and more (agents' launch commands, plugins), as text"),
            action("reload config", "", "dim", None, "↵ reload", Act::Run("reload-config"), "Read config.toml again (it's also read whenever it changes)"),
        ],
        "layout" => vec![
            heading("sidebar"),
            toggle("show the sidebar", cfg.sidebar.visible, Act::Flip(Some("sidebar"), "visible"), "Whether it's open when modisa starts; the status row's ◧ button and prefix b toggle it"),
            choice("width", format!("{} columns", cfg.sidebar.width), Act::SidebarWidth, "Or drag its edge. It never takes more than a third of the terminal"),
            choice("agent logos", json!(cfg.sidebar.logos).as_str().unwrap_or("auto"), Act::Logos, "auto: agents' real logos where the terminal can show them. off: plain marks"),
            toggle("branch lines", cfg.sidebar.graph, Act::Flip(Some("sidebar"), "graph"), "Draw the AGENTS list as a git graph of its tabs; off: just the tab names over their agents"),
            heading("status row"),
            toggle("agent counts", cfg.status.agents, Act::Flip(Some("status"), "agents"), "How many agents are working and how many need you; click one to list them"),
            toggle("pane count", cfg.status.panes, Act::Flip(Some("status"), "panes"), "How many panes this tab has; click it to switch pane"),
            toggle("theme name", cfg.status.theme, Act::Flip(Some("status"), "theme"), "The theme in use; click it to change theme"),
            heading("panes"),
            choice("border style", json!(cfg.panes.border).as_str().unwrap_or("single"), Act::Border, "The line around each pane"),
            toggle("agent in the border title", cfg.pane_labels.agent, Act::Flip(Some("pane_labels"), "agent"), "The agent and what it's doing, in its pane's border"),
        ],
        "git" => vec![
            heading("in the status row"),
            toggle("branch", cfg.git.status, Act::Flip(Some("git"), "status"), "The branch of the repository the active space's focused pane is in; green when clean and in step with its upstream"),
            toggle("repository name", cfg.git.repo, Act::Flip(Some("git"), "repo"), "The repository's name (its folder), before the branch"),
            toggle("commits to push and pull", cfg.git.counts, Act::Flip(Some("git"), "counts"), "↑ commits to push and ↓ commits to pull, against the branch's upstream"),
            toggle("changed files", cfg.git.changes, Act::Flip(Some("git"), "changes"), "● files changed, staged or not, and new files"),
        ],
        "indicators" => {
            let mut v = vec![heading("style")];
            for (style, name, glyphs) in [(IndicatorStyle::Symbols, "symbols", "! ◆ ✓ ○"), (IndicatorStyle::Dots, "dots", "● ● ● ○"), (IndicatorStyle::Letters, "letters", "B W D I")] {
                v.push(Row { kind: Kind::Radio { current: cfg.indicators.style == style, swatches: vec![] }, label: format!("{name:<9} {glyphs}"), about: "How an agent's state is drawn: needs you, working, done, idle".into(), act: Act::Style(style) });
            }
            v.push(heading("show in"));
            v.push(toggle("tab bar badge", cfg.indicators.tab, Act::Flip(Some("indicators"), "tab"), "A mark on each tab with an agent that needs you"));
            v.push(toggle("pane border title", cfg.indicators.pane, Act::Flip(Some("indicators"), "pane"), "The agent's state in its pane's border"));
            v.push(toggle("sidebar", cfg.indicators.sidebar, Act::Flip(Some("indicators"), "sidebar"), "The agent's state next to its name"));
            v
        }
        "sound" => {
            let mut v = vec![heading("when an agent…")];
            for (event, label) in EVENTS {
                let value = if cfg.notify.kinds(event).contains(&NotifyKind::Sound) { cfg.sound.name(event).unwrap_or("off").to_string() } else { "off".into() };
                v.push(choice(label, value, Act::Sound(event), "←→ to hear the others; ↵ plays it again"));
            }
            v.push(heading("level"));
            v.push(choice("volume", format!("{}%", (cfg.sound.volume * 100.0).round()), Act::Volume, "For every sound modisa plays"));
            v
        }
        "alerts" => EVENTS
            .iter()
            .flat_map(|(event, _)| {
                let title = match event {
                    AgentState::Blocked => "when an agent needs you",
                    AgentState::Done => "when an agent is done",
                    _ => "when an agent starts working",
                };
                let kinds = cfg.notify.kinds(*event);
                std::iter::once(heading(title)).chain(ALERTS.iter().map(move |(kind, name, about)| toggle(name, kinds.contains(kind), Act::Alert(*event, *kind), &format!("{about}. Only for agents you're not looking at"))))
            })
            .collect(),
        "agents" => {
            let policy = |label: &str, key: &'static str, about: &str| {
                let p = match key {
                    "keys_foreign" => cfg.permissions.keys_foreign,
                    "run_foreign" => cfg.permissions.run_foreign,
                    _ => cfg.permissions.close_foreign,
                };
                choice(label, json!(p).as_str().unwrap_or("ask"), Act::Policy(key), &format!("{about}. ask: you're asked each time"))
            };
            vec![
                heading("on panes an agent didn't start"),
                policy("send keys", "keys_foreign", "Typing into another pane"),
                policy("run commands", "run_foreign", "Running a command in another pane"),
                policy("close panes", "close_foreign", "Closing another pane"),
                heading("messaging"),
                choice("reply chain limit", cfg.messaging.max_hops.to_string(), Act::Number("max_hops", 1, 50), "Agents replying to each other stop after this many messages"),
                choice("messages a minute", cfg.messaging.per_minute.to_string(), Act::Number("per_minute", 1, 60), "From one agent to another, at most"),
            ]
        }
        // Installed on the machine the server runs on (where the agents are), so it asks the server. Every agent is
        // listed: installed, out of date, available (the agent is here), or not found.
        "integrations" => {
            let Some(list) = &s.integrations else { return vec![heading("checking…")] };
            let about = "State is read from every agent's screen; an integration adds its session, so it resumes after a restart";
            let todo: Vec<&IntegrationStatus> = list.iter().filter(|i| i.status == "outdated" || (i.available && i.status == "none")).collect();
            let mut v = vec![];
            if !todo.is_empty() {
                v.push(action(&format!("Install all ({})", todo.len()), todo.iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join(", "), "accent", None, "↵ install", Act::Integrations(todo.iter().map(|i| i.id.clone()).collect(), true), about));
            }
            let order = |i: &IntegrationStatus| if i.status != "none" { 0 } else if i.available { 1 } else { 2 };
            let mut sorted: Vec<&IntegrationStatus> = list.iter().collect();
            sorted.sort_by_key(|i| order(i));
            for i in sorted {
                let status = if s.busy == i.id { "working…" } else if i.status == "current" { "✓ installed" } else if i.status == "outdated" { "↻ update available" } else if i.available { "+ available" } else { "not found" };
                let tone = if i.status == "current" { "ok" } else if i.status == "outdated" { "warn" } else if i.available { "accent" } else { "dim" };
                let hint = if i.status == "current" { "↵ remove" } else if i.status == "outdated" { "↵ update" } else { "↵ install" };
                let note = Some(if i.kind == "lifecycle" { "state + session" } else { "session" }.to_string());
                v.push(action(&i.name, status, tone, note, hint, Act::Integrations(vec![i.id.clone()], i.status != "current"), about));
            }
            v
        }
        // Who draws each part of modisa a plugin can draw instead: modisa (builtin), or one of the plugins that ask to.
        // With nothing chosen, the first of them by name does.
        "slots" => {
            let mut v = vec![heading("drawn by")];
            for (slot, label, about) in SLOTS {
                let asking = super::slots::askers(app, slot);
                let who = if asking.is_empty() { "No plugin asks to draw it".to_string() } else { format!("Asking to draw it: {}", asking.join(", ")) };
                v.push(choice(label, slot_holder(app, slot), Act::Slot(slot), &format!("{about}. builtin: modisa's own. {who}")));
            }
            v
        }
        // The plugins on the server's machine, where they run: each one's state, and ↵ starts or stops it. The plugin
        // manager (prefix P) finds, installs, updates and removes them.
        _ => {
            let mut v = vec![
                heading("plugin manager"),
                action("discover", "", "dim", None, "↵ open", Act::Plugins("discover"), "Plugins in the index and in your marketplaces; choosing one shows where it comes from before it installs (prefix P)"),
                action("installed", "", "dim", None, "↵ open", Act::Plugins("installed"), "Start, stop, restart, read the log of, update and remove installed plugins"),
                action("marketplaces", "", "dim", None, "↵ open", Act::Plugins("marketplaces"), "Git repositories that list plugins: modisa-marketplace.json at the top"),
                action("add from URL", "", "dim", None, "↵ open", Act::Plugins("url"), "Install a plugin from a git URL, at a branch, tag or commit"),
            ];
            v.push(heading(match &s.plugins {
                Some(l) if l.is_empty() => "installed: none yet",
                Some(_) => "installed",
                None => "checking…",
            }));
            for p in s.plugins.iter().flatten() {
                let (name, status) = (p["name"].as_str().unwrap_or(""), p["status"].as_str().unwrap_or(""));
                let shown = if s.busy == name { "working…".to_string() } else if status == "running" { "● running".into() } else if status == "failed" { "✕ failed".into() } else { format!("○ {status}") };
                let tone = if status == "running" { "ok" } else if status == "failed" { "warn" } else { "dim" };
                let note = match (p["install"]["commit"].as_str(), p["source"].as_str()) {
                    (Some(c), _) => format!("@{}", &c[..7.min(c.len())]),
                    (None, Some("config")) => "config.toml".into(),
                    _ => "linked".into(),
                };
                let about = p["error"].as_str().map(String::from).unwrap_or_else(|| match (p["install"]["source"].as_str(), p["dir"].as_str()) {
                    (Some(src), _) => format!("From {src}{}", p["install"]["ref"].as_str().map(|r| format!(" ({r})")).unwrap_or_default()),
                    (None, Some(d)) => format!("Linked from {d}"),
                    _ => "A [[plugin]] run line in config.toml".into(),
                });
                v.push(action(name, shown, tone, Some(note), if status == "running" { "↵ stop" } else { "↵ start" }, Act::Plugin(name.into(), status == "running"), &about));
            }
            v
        }
    }
}

// What's listed: the current section's rows, or while searching, every section's matches under headings that say where
// each one lives; a blank line between groups.
fn entries(app: &App, s: &Settings) -> Vec<(Row, usize)> {
    let mut list: Vec<(Row, usize)> = vec![];
    if s.query.is_empty() {
        list = rows(app, s, s.current).into_iter().map(|r| (r, s.current)).collect();
    } else {
        for i in 0..SECTIONS.len() {
            let mut group = String::new();
            let mut found: Vec<(Row, String)> = vec![];
            for r in rows(app, s, i) {
                if matches!(r.kind, Kind::Heading) {
                    group = r.label.clone();
                    continue;
                }
                found.push((r, group.clone()));
            }
            let items: Vec<ListItem> = found
                .iter()
                .map(|(r, g)| {
                    let value = if let Kind::Choice { value } = &r.kind { value.clone() } else { String::new() };
                    ListItem::new(&r.label, format!("{} {g} {} {value}", SECTIONS[i], r.about), "")
                })
                .collect();
            let mut last: Option<String> = None;
            let mut any = false;
            for k in matches(&items, &s.query) {
                let (r, g) = &found[k];
                if last.as_ref() != Some(g) || !any {
                    list.push((heading(&if g.is_empty() { SECTIONS[i].to_string() } else { format!("{} · {g}", SECTIONS[i]) }), i));
                }
                last = Some(g.clone());
                any = true;
                list.push((r.clone(), i));
            }
        }
    }
    let mut spaced = vec![];
    for (k, e) in list.into_iter().enumerate() {
        if k > 0 && matches!(e.0.kind, Kind::Heading) {
            spaced.push((heading(""), e.1));
        }
        spaced.push(e);
    }
    spaced
}

// ---------- what rows do ----------

// A setting changed: applied at once, and saved to config.toml.
fn save(app: &mut App, table: Option<&str>, key: &str, value: Value) {
    let mut v = serde_json::to_value(&app.cfg).unwrap();
    match table {
        Some(t) => v[t][key] = value.clone(),
        None => v[key] = value.clone(),
    }
    if let Ok(cfg) = serde_json::from_value::<Config>(v) {
        app.set_config(cfg);
    }
    if let Err(e) = save_setting(table, key, &value) {
        let w = app.th.warn;
        app.toast(&format!("not saved: {e}"), w);
    }
}

// the next of `options` after `current` (by 1 or -1), round the end
fn cycle<'a>(options: &[&'a str], current: &str, by: i64) -> &'a str {
    let n = options.len() as i64;
    let at = options.iter().position(|o| *o == current).map(|i| i as i64).unwrap_or(-1);
    options[((at + by).rem_euclid(n)) as usize]
}

// the sidebar's width moves the panes: the server lays them out again for the new area
fn relayout(app: &App) {
    let area = app.area();
    app.notify_server("area", json!({ "area": area }));
}

// who draws a slot: [slots] says, else the plugin the server chose, else modisa
fn slot_holder(app: &App, slot: &str) -> String {
    app.cfg.slots.get(slot).cloned().or_else(|| super::slots::holder(app, slot).map(String::from)).unwrap_or_else(|| "builtin".into())
}

// modisa, and each plugin that asks to draw it
// ponytail: once chosen, a slot can't go back to "the first that asks" from here; taking its line out of config.toml does
fn slot_choices(app: &App, slot: &str) -> Vec<String> {
    let mut v = vec!["builtin".to_string()];
    for p in super::slots::askers(app, slot).into_iter().chain(app.cfg.slots.get(slot).cloned()) {
        if !v.contains(&p) {
            v.push(p);
        }
    }
    v
}

fn sound_options() -> Vec<&'static str> {
    std::iter::once("off").chain(super::sound::SOUNDS.iter().copied()).collect()
}

fn play(app: &App, name: &str, volume: Option<f64>) {
    if name != "off" {
        super::sound::play(name, volume.unwrap_or(app.cfg.sound.volume));
    }
}

fn notify_list(app: &App, event: AgentState) -> Vec<NotifyKind> {
    app.cfg.notify.kinds(event).to_vec()
}

fn step(app: &mut App, s: &mut Settings, act: &Act, by: i64) {
    match act {
        Act::Prefix => save(app, None, "prefix", json!(format!("C-{}", cycle(&PREFIXES, &app.prefix.name, by)))),
        Act::Channel => {
            let now = json!(app.cfg.update.channel).as_str().unwrap_or("stable").to_string();
            save(app, Some("update"), "channel", json!(cycle(&["stable", "staging"], &now, by)));
        }
        Act::SidebarWidth => {
            save(app, Some("sidebar"), "width", json!((app.cfg.sidebar.width + by * 2).clamp(20, 48)));
            relayout(app);
        }
        Act::Logos => {
            let now = json!(app.cfg.sidebar.logos).as_str().unwrap_or("auto").to_string();
            save(app, Some("sidebar"), "logos", json!(cycle(&["auto", "on", "off"], &now, by)));
            tokio::task::spawn_local(super::setup_logos(app.shared()));
        }
        Act::Border => {
            let now = json!(app.cfg.panes.border).as_str().unwrap_or("single").to_string();
            save(app, Some("panes"), "border", json!(cycle(&["single", "rounded", "double", "heavy"], &now, by)));
        }
        Act::Sound(event) => {
            let options = sound_options();
            let kinds = notify_list(app, *event);
            let value = if kinds.contains(&NotifyKind::Sound) { app.cfg.sound.name(*event).unwrap_or("off").to_string() } else { "off".into() };
            let next = cycle(&options, &value, by);
            let ev = event.as_str();
            if next == "off" {
                return save(app, Some("notify"), ev, json!(kinds.iter().filter(|k| **k != NotifyKind::Sound).collect::<Vec<_>>()));
            }
            if !kinds.contains(&NotifyKind::Sound) {
                let mut with = kinds.clone();
                with.push(NotifyKind::Sound);
                save(app, Some("notify"), ev, json!(with));
            }
            save(app, Some("sound"), ev, json!(next));
            play(app, next, None);
        }
        Act::Volume => {
            let volume = (((app.cfg.sound.volume * 10.0) + by as f64).round() / 10.0).clamp(0.0, 1.0);
            save(app, Some("sound"), "volume", json!(volume));
            let blocked = app.cfg.sound.blocked.clone();
            play(app, &blocked, Some(volume));
        }
        Act::Policy(key) => {
            let now = json!(match *key {
                "keys_foreign" => app.cfg.permissions.keys_foreign,
                "run_foreign" => app.cfg.permissions.run_foreign,
                _ => app.cfg.permissions.close_foreign,
            });
            save(app, Some("permissions"), key, json!(cycle(&["ask", "allow", "deny"], now.as_str().unwrap_or("ask"), by)));
        }
        Act::Number(key, lo, hi) => {
            let now = if *key == "max_hops" { app.cfg.messaging.max_hops } else { app.cfg.messaging.per_minute };
            save(app, Some("messaging"), key, json!((now + by).clamp(*lo, *hi)));
        }
        Act::Slot(slot) => {
            let choices = slot_choices(app, slot);
            let next = cycle(&choices.iter().map(String::as_str).collect::<Vec<_>>(), &slot_holder(app, slot), by).to_string();
            save(app, Some("slots"), slot, json!(next));
        }
        _ => {
            let _ = s;
        }
    }
}

fn activate(app: &mut App, s: &mut Settings, row: &Row) {
    match (&row.kind, &row.act) {
        (Kind::Radio { .. }, Act::Theme(name)) => {
            s.saved_theme = name.clone();
            save(app, None, "theme", json!(name));
            let a = app.th.accent;
            app.toast(&format!("Theme saved: {name}"), a);
        }
        (Kind::Radio { .. }, Act::Style(style)) => save(app, Some("indicators"), "style", json!(style)),
        (Kind::Toggle { on }, Act::Flip(table, key)) => {
            save(app, *table, key, json!(!on));
            if (*table, *key) == (Some("sidebar"), "visible") {
                app.sidebar = !on;
                relayout(app);
            }
        }
        (Kind::Toggle { on }, Act::Alert(event, kind)) => {
            let mut kinds = notify_list(app, *event);
            if *on {
                kinds.retain(|k| k != kind);
            } else {
                kinds.push(*kind);
            }
            save(app, Some("notify"), event.as_str(), json!(kinds));
        }
        // ↵ on a choice: sounds play again
        (Kind::Choice { value }, Act::Sound(_)) => play(app, value, None),
        (Kind::Choice { .. }, Act::Volume) => {
            let blocked = app.cfg.sound.blocked.clone();
            play(app, &blocked, None);
        }
        (Kind::Action { .. }, Act::Run(a)) => {
            // the page closes first: what it opens is a dialog of its own
            s.close = true;
            actions::run(&app.shared(), a);
        }
        (Kind::Action { .. }, Act::Integrations(ids, install)) => {
            if s.busy.is_empty() {
                apply_integrations(app, ids.clone(), *install);
            }
        }
        (Kind::Action { .. }, Act::Plugins(view)) => {
            // the page closes first: the manager's views are dialogs of their own
            s.close = true;
            let (shared, view) = (app.shared(), *view);
            tokio::task::spawn_local(async move { super::plugins::open(&shared, view).await });
        }
        (Kind::Action { .. }, Act::Plugin(name, running)) => {
            if s.busy.is_empty() {
                toggle_plugin(app, name.clone(), *running);
            }
        }
        _ => {}
    }
}

// ---------- what the server is asked ----------

fn with_settings(app: &mut App, f: impl FnOnce(&mut Settings)) {
    if let Some(super::modals::Modal { kind: super::modals::Kind::Settings(s), .. }) = app.modal.as_mut() {
        f(s);
        app.dirty();
    }
}

fn load(app: &App, method: &'static str) {
    let Some(conn) = app.conn.clone() else { return };
    let me = app.me.clone();
    tokio::task::spawn_local(async move {
        let r = conn.request(method, json!({}), None).await;
        let Some(a) = me.upgrade() else { return };
        let mut app = a.borrow_mut();
        match r {
            Ok(v) if method == "integrations" => with_settings(&mut app, |s| s.integrations = serde_json::from_value(v).ok()),
            Ok(v) => with_settings(&mut app, |s| s.plugins = v.as_array().cloned()),
            Err(e) => {
                let b = app.th.blocked;
                app.toast(&e.message, b);
            }
        }
    });
}

fn apply_integrations(app: &App, ids: Vec<String>, install: bool) {
    let Some(conn) = app.conn.clone() else { return };
    let me = app.me.clone();
    tokio::task::spawn_local(async move {
        for id in ids {
            let Some(a) = me.upgrade() else { return };
            with_settings(&mut a.borrow_mut(), |s| s.busy = id.clone());
            drop(a);
            let r = conn.request("integration", json!({ "id": id, "install": install }), None).await;
            let Some(a) = me.upgrade() else { return };
            let mut app = a.borrow_mut();
            match r {
                Ok(v) => {
                    let d = app.th.done;
                    app.toast(v.as_str().unwrap_or(""), d);
                }
                Err(e) => {
                    let b = app.th.blocked;
                    app.toast(&e.message, b);
                }
            }
        }
        if let Some(a) = me.upgrade() {
            let mut app = a.borrow_mut();
            with_settings(&mut app, |s| s.busy.clear());
            load(&app, "integrations");
        }
    });
}

fn toggle_plugin(app: &App, name: String, running: bool) {
    let Some(conn) = app.conn.clone() else { return };
    let me = app.me.clone();
    tokio::task::spawn_local(async move {
        if let Some(a) = me.upgrade() {
            with_settings(&mut a.borrow_mut(), |s| s.busy = name.clone());
        }
        let r = conn.request(if running { "plugin.stop" } else { "plugin.start" }, json!({ "name": name }), None).await;
        let Some(a) = me.upgrade() else { return };
        let mut app = a.borrow_mut();
        match r {
            Ok(s) => {
                let failed = s["status"] == "failed";
                let text = format!("{name}: {}{}", s["status"].as_str().unwrap_or(""), s["error"].as_str().map(|e| format!(" ({e})")).unwrap_or_default());
                let c = if failed { app.th.blocked } else { app.th.done };
                app.toast(&text, c);
            }
            Err(e) => {
                let b = app.th.blocked;
                app.toast(&e.message, b);
            }
        }
        with_settings(&mut app, |s| s.busy.clear());
        load(&app, "plugin.list");
    });
}

// ---------- the page ----------

// A section shown: the theme section remembers what's in use, a server-backed one asks for its list.
fn enter(app: &App, s: &mut Settings) {
    match SECTIONS[s.current] {
        "theme" => s.saved_theme = app.cfg.theme.clone(),
        "integrations" => load(app, "integrations"),
        "plugins" => load(app, "plugin.list"),
        _ => {}
    }
}

// The page moves away from a section or closes: a theme previewed and not applied goes.
pub fn leave(app: &mut App, s: &Settings) {
    if SECTIONS[s.current] == "theme" && app.cfg.theme != s.saved_theme {
        let mut cfg = app.cfg.clone();
        cfg.theme = s.saved_theme.clone();
        app.set_config(cfg);
    }
}

pub fn show(app: &mut App, s: &mut Settings, index: i64) {
    leave(app, s);
    s.current = index.rem_euclid(SECTIONS.len() as i64) as usize;
    s.query.clear();
    enter(app, s);
    let list = entries(app, s);
    s.sel = list.iter().position(|(r, _)| matches!(r.kind, Kind::Radio { current: true, .. })).or_else(|| list.iter().position(|(r, _)| focusable(Some(r)))).unwrap_or(0);
}

fn select(app: &mut App, s: &mut Settings, index: usize) {
    s.sel = index;
    let row = entries(app, s).get(index).map(|e| e.0.clone());
    if let Some(Row { kind: Kind::Radio { .. }, act: Act::Theme(name), .. }) = row {
        // live, until you apply or leave
        let mut cfg = app.cfg.clone();
        cfg.theme = name;
        app.set_config(cfg);
    }
}

fn move_by(app: &mut App, s: &mut Settings, by: i64) {
    let list = entries(app, s);
    let mut to = s.sel;
    let mut left = by.unsigned_abs();
    let mut i = s.sel as i64 + by.signum();
    while i >= 0 && (i as usize) < list.len() && left > 0 {
        if focusable(Some(&list[i as usize].0)) {
            to = i as usize;
            left -= 1;
        }
        i += by.signum();
    }
    if to != s.sel {
        select(app, s, to);
    }
}

// A key while the page is open (Escape is the dialog's own: it closes it).
pub fn key(app: &mut App, s: &mut Settings, k: &KeyEvent) {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let row = entries(app, s).get(s.sel).map(|e| e.0.clone());
    match k.code {
        KeyCode::Tab => show(app, s, s.current as i64 + 1),
        KeyCode::BackTab => show(app, s, s.current as i64 - 1),
        KeyCode::Up => move_by(app, s, -1),
        KeyCode::Char('p') if ctrl => move_by(app, s, -1),
        KeyCode::Down => move_by(app, s, 1),
        KeyCode::Char('n') if ctrl => move_by(app, s, 1),
        KeyCode::PageUp => move_by(app, s, -8),
        KeyCode::PageDown => move_by(app, s, 8),
        KeyCode::Left if matches!(row.as_ref().map(|r| &r.kind), Some(Kind::Choice { .. })) => step(app, s, &row.unwrap().act, -1),
        KeyCode::Right if matches!(row.as_ref().map(|r| &r.kind), Some(Kind::Choice { .. })) => step(app, s, &row.unwrap().act, 1),
        KeyCode::Enter => {
            if let Some(r) = row {
                activate(app, s, &r);
            }
        }
        KeyCode::Char(' ') if s.query.is_empty() => {
            if let Some(r) = row {
                activate(app, s, &r);
            }
        }
        _ => {
            if let Some(next) = typed(&s.query, k).filter(|n| *n != s.query) {
                s.query = next;
                let list = entries(app, s);
                s.sel = list.iter().position(|(r, _)| focusable(Some(r))).unwrap_or(0);
            }
        }
    }
}

// A click: "nav:<i>" a section, "row:<i>" a row, "step:<i>:<by>" a choice's arrow.
pub fn click(app: &mut App, s: &mut Settings, what: &str) {
    let mut parts = what.split(':');
    match (parts.next(), parts.next().and_then(|i| i.parse::<usize>().ok()), parts.next()) {
        (Some("nav"), Some(i), _) => show(app, s, i as i64),
        (Some("row"), Some(i), _) => {
            s.sel = i;
            if let Some((r, _)) = entries(app, s).get(i).cloned() {
                activate(app, s, &r);
            }
        }
        (Some("step"), Some(i), Some(by)) => {
            s.sel = i;
            if let Some((r, _)) = entries(app, s).get(i).cloned() {
                step(app, s, &r.act, by.parse().unwrap_or(1));
            }
        }
        _ => {}
    }
}

pub fn hover(app: &mut App, s: &mut Settings, what: &str) {
    if let Some(i) = what.strip_prefix("row:").and_then(|i| i.parse::<usize>().ok()) {
        if app.cfg.mouse.hover && i != s.sel {
            select(app, s, i);
        }
    }
}

pub fn scroll(app: &mut App, s: &mut Settings, by: i64) {
    move_by(app, s, by);
}

pub fn draw(app: &App, s: &mut Settings, c: &mut Canvas) {
    let th = &app.th;
    let r = super::design::floating(c.w, c.h, WIDTH, HEIGHT, None, None);
    c.fill(r, th.bar);
    c.border(r, crate::config::BorderStyle::Rounded, &mix(th.border, th.focus, 0.45), Some(th.bar), None);
    c.hit(r, Hit::Inert);
    let (x, inner) = (r.x + 2, (r.w - 4).max(1));
    let mut y = r.y + 1;
    let body = ((r.h - 2) - 6).max(3); // header, search, gap … gap, about, footer
    let list = entries(app, s);
    if !focusable(list.get(s.sel).map(|e| &e.0)) {
        s.sel = list.iter().position(|(r, _)| focusable(Some(r))).unwrap_or(0); // e.g. once integrations have loaded
    }
    // header and search
    let meta = fit(&CONFIG_PATH.replace(HOME.as_str(), "~"), (inner / 2) as usize);
    c.text(x, y, &fit("Settings", (inner - width(&meta) as i32 - 1).max(1) as usize), th.fg, Some(th.bar), Modifier::BOLD, inner as usize);
    c.text(x + inner - width(&meta) as i32, y, &meta, th.dim, Some(th.bar), Modifier::empty(), meta.len());
    y += 1;
    c.fill(Rect { x, y, w: inner, h: 1 }, th.bg);
    let mut sx = x + c.text(x, y, " ⌕ ", if s.query.is_empty() { th.dim } else { th.accent }, Some(th.bg), Modifier::empty(), 3);
    if s.query.is_empty() {
        sx += c.text(sx, y, "▏", th.focus, Some(th.bg), Modifier::empty(), 1);
        c.text(sx, y, "Search settings", th.dim, Some(th.bg), Modifier::empty(), (inner - 5).max(0) as usize);
    } else {
        sx += c.text(sx, y, &s.query, th.fg, Some(th.bg), Modifier::empty(), (inner - 5).max(0) as usize);
        c.text(sx, y, "▏", th.focus, Some(th.bg), Modifier::empty(), 1);
    }
    y += 2;
    // the sections down the side; while searching, how many matches each has
    let found: Vec<usize> = (0..SECTIONS.len()).map(|i| if s.query.is_empty() { 0 } else { list.iter().filter(|(r, sec)| *sec == i && focusable(Some(r))).count() }).collect();
    for (i, name) in SECTIONS.iter().enumerate() {
        if i as i32 >= body {
            break;
        }
        let on = s.query.is_empty() && i == s.current;
        let line = Rect { x, y: y + i as i32, w: NAV, h: 1 };
        let bg = if on { mix(th.bar, th.focus, 0.16) } else if c.hovered(line) { mix(th.bar, th.fg, 0.07) } else { th.bar.to_string() };
        c.fill(line, &bg);
        let dim = !s.query.is_empty() && found[i] == 0;
        let label = fit(&format!(" {}{}", name[..1].to_uppercase(), &name[1..]), (NAV - 5) as usize);
        let fg = if on { th.fg.to_string() } else if dim { mix(th.dim, th.bar, 0.4) } else { th.dim.to_string() };
        c.text(x + 1, line.y, &label, &fg, Some(&bg), if on { Modifier::BOLD } else { Modifier::empty() }, (NAV - 1) as usize);
        if !s.query.is_empty() && found[i] > 0 {
            let n = format!("{} ", found[i]);
            c.text(x + NAV - width(&n) as i32, line.y, &n, th.accent, Some(&bg), Modifier::empty(), n.len());
        }
        c.hit(line, Hit::Modal(format!("nav:{i}")));
    }
    for k in 0..body {
        c.text(x + NAV, y + k, " │ ", th.border, Some(th.bar), Modifier::empty(), 3);
    }
    // the rows, scrolled so the selection stays in view
    let rx = x + NAV + 3;
    let w = (inner - NAV - 3 - 1).max(1); // the scrollbar's column
    let first = (s.sel as i64 - body as i64 / 2).min(list.len() as i64 - body as i64).max(0) as usize;
    let bar = thumb(list.len(), body as usize, first, body as usize);
    for (k, (row, _)) in list.iter().enumerate().skip(first).take(body as usize) {
        let ly = y + (k - first) as i32;
        let selected = k == s.sel;
        let line = Rect { x: rx, y: ly, w: w + 1, h: 1 };
        let hovered = !app.cfg.mouse.hover && c.hovered(line) && !selected && !matches!(row.kind, Kind::Heading);
        let bg = if selected { mix(th.bar, th.focus, 0.16) } else if hovered { mix(th.bar, th.fg, 0.07) } else { th.bar.to_string() };
        c.fill(line, &bg);
        let sbar = bar.is_some_and(|(t, size)| k - first >= t && k - first < t + size);
        c.text(rx + w, ly, if sbar { "▐" } else { " " }, th.border, Some(&bg), Modifier::empty(), 1);
        if let Kind::Heading = row.kind {
            c.text(rx, ly, &fit(&format!(" {}", row.label.to_uppercase()), w as usize), th.dim, Some(&bg), Modifier::BOLD, w as usize);
            continue;
        }
        c.hit(Rect { x: rx, y: ly, w, h: 1 }, Hit::Modal(format!("row:{k}")));
        let mut at = rx + c.text(rx, ly, " ", th.focus, Some(&bg), Modifier::empty(), 1); // left padding inside the highlight
        let label = |c: &mut Canvas, at: i32, room: i32| {
            let padded = format!("{:<room$}", fit(&row.label, room.max(0) as usize), room = room.max(0) as usize);
            at + c.text(at, ly, &format!(" {padded}"), th.fg, Some(&bg), if selected { Modifier::BOLD } else { Modifier::empty() }, (room + 1).max(0) as usize)
        };
        let right_edge = rx + w;
        match &row.kind {
            Kind::Radio { current, swatches } => {
                at += c.text(at, ly, if *current { " ◉" } else { " ○" }, if *current { th.accent } else { th.dim }, Some(&bg), Modifier::empty(), 2);
                label(c, at, w - 23);
                let tail = (if *current { 8 } else { 0 }) + swatches.len() as i32 * 2 + 1;
                let mut tx = right_edge - tail;
                if *current {
                    tx += c.text(tx, ly, "in use  ", th.done, Some(&bg), Modifier::empty(), 8);
                }
                for col in swatches {
                    tx += c.text(tx, ly, "  ", th.fg, Some(col), Modifier::empty(), 2);
                }
            }
            Kind::Toggle { on } => {
                label(c, at, w - 10);
                let t = if *on { "● on " } else { "○ off" };
                c.text(right_edge - 7, ly, t, if *on { th.accent } else { th.dim }, Some(&bg), if *on { Modifier::BOLD } else { Modifier::empty() }, 5);
            }
            Kind::Choice { value } => {
                label(c, at, w - 24);
                let mut tx = right_edge - (3 + 14 + 3 + 1);
                for (glyph, by) in [(" ‹ ", -1), (" › ", 1)] {
                    let cell = Rect { x: tx, y: ly, w: 3, h: 1 };
                    let fg = if c.hovered(cell) { th.accent } else if selected { th.fg } else { th.dim };
                    c.text(tx, ly, glyph, fg, Some(&bg), Modifier::empty(), 3);
                    c.hit(cell, Hit::Modal(format!("step:{k}:{by}")));
                    tx += 3;
                    if by == -1 {
                        tx += c.text(tx, ly, &format!("{:<14}", fit(value, 14)), if value == "off" { th.dim } else { th.accent }, Some(&bg), Modifier::empty(), 14);
                    }
                }
            }
            Kind::Action { status, tone, note, hint } => {
                at = label(c, at, 20);
                let color = match *tone {
                    "ok" => th.done,
                    "warn" => th.warn,
                    "accent" => th.accent,
                    _ => th.dim,
                };
                at += c.text(at, ly, &format!("{:<22}", fit(&format!(" {status}"), 22)), color, Some(&bg), Modifier::empty(), 22);
                if let Some(n) = note.as_ref().filter(|_| w >= 70) {
                    c.text(at, ly, &fit(n, 16), th.dim, Some(&bg), Modifier::empty(), 16);
                }
                if selected {
                    let h = format!(" {hint} ");
                    c.text(right_edge - 1 - width(&h) as i32, ly, &h, th.fg, Some(&mix(th.bar, th.fg, 0.14)), Modifier::empty(), width(&h));
                }
            }
            Kind::Heading => {}
        }
    }
    if list.is_empty() {
        c.text(rx, y, &fit(&format!("  No settings match “{}”", s.query), w as usize), th.dim, Some(th.bar), Modifier::empty(), w as usize);
    }
    // what the selected row does
    let about = list.get(s.sel).map(|(r, _)| r.about.clone()).unwrap_or_default();
    let fy = r.y + r.h - 2;
    c.text(x, fy - 1, &fit(&about, inner as usize), th.dim, Some(th.bar), Modifier::empty(), inner as usize);
    let mut hx = x;
    let right = if s.query.is_empty() { "" } else { "^u clear" };
    let mut used = width(right) as i32 + 1;
    for (key, what) in [("↑↓", "move"), ("←→", "change"), ("↵", "apply"), ("tab", "section"), ("esc", "close")] {
        let wd = (width(key) + width(what) + 3) as i32;
        if used + wd > inner {
            break;
        }
        used += wd;
        c.text(hx, fy, key, th.fg, Some(th.bar), Modifier::empty(), wd as usize);
        c.text(hx + width(key) as i32, fy, &format!(" {what}   "), th.dim, Some(th.bar), Modifier::empty(), wd as usize);
        hx += wd;
    }
    if !right.is_empty() {
        c.text(x + inner - width(right) as i32, fy, right, th.dim, Some(th.bar), Modifier::empty(), right.len());
    }
}

// A scrollbar for `total` rows, `shown` of them from `first`, `height` cells tall: the thumb's cells.
fn thumb(total: usize, shown: usize, first: usize, height: usize) -> Option<(usize, usize)> {
    if total <= shown {
        return None;
    }
    let size = ((shown as f64 / total as f64) * height as f64).round().max(1.0) as usize;
    let top = ((first as f64 / (total - shown).max(1) as f64) * (height - size) as f64).round() as usize;
    Some((top, size))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_slots_section_says_who_draws_each_and_who_asks_to() {
        let asks = |plugin: &str, replaces: bool| json!({ "plugin": plugin, "run": "r", "slot": "agent.row", "id": "x", "pane": "p1", "instance": "p1i", "position": "replace", "replaces": replaces, "lines": ["mine"] });
        let a = crate::client::slots::tests::app(120, 30, json!([asks("radar", true), asks("atlas", false)]));
        let mut app = a.borrow_mut();
        let s = Settings::new(&app, "slots");
        let section = |app: &App| rows(app, &s, SECTIONS.iter().position(|x| *x == "slots").unwrap());
        let row = |app: &App, slot: &'static str| section(app).into_iter().find(|r| r.act == Act::Slot(slot)).unwrap();
        let value = |r: Row| if let Kind::Choice { value } = r.kind { value } else { unreachable!() };
        let agents = row(&app, "agent.row");
        assert!(agents.about.contains("Asking to draw it: atlas, radar"), "{}", agents.about);
        assert_eq!(value(agents), "radar"); // the server chose it: nothing in [slots] says otherwise
        assert_eq!(slot_choices(&app, "agent.row"), ["builtin", "atlas", "radar"]);
        let titles = row(&app, "pane.title");
        assert!(titles.about.contains("No plugin asks"));
        assert_eq!(value(titles), "builtin");
        app.cfg.slots.insert("agent.row".into(), "builtin".into());
        assert_eq!(value(row(&app, "agent.row")), "builtin");
        app.cfg.slots.insert("agent.row".into(), "gone".into()); // a plugin that's stopped asking stays a choice while it's chosen
        assert_eq!(slot_choices(&app, "agent.row"), ["builtin", "atlas", "radar", "gone"]);
    }
}
