// ~/.config/modisa/config.toml, merged over defaults; changes apply live.
pub mod adapters;
pub mod agents;
pub mod plugins;
pub mod check;
pub mod keys;
pub mod marketplaces;
pub mod plugin_manage;
pub mod themes;

use std::path::Path;
use std::sync::LazyLock;

use indexmap::IndexMap;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use crate::core::paths::HOME;
use crate::protocol::types::AgentState;

pub static CONFIG_DIR: LazyLock<String> = LazyLock::new(|| std::env::var("MODISA_CONFIG_DIR").unwrap_or_else(|_| format!("{}/.config/modisa", *HOME)));
pub static CONFIG_PATH: LazyLock<String> = LazyLock::new(|| format!("{}/config.toml", *CONFIG_DIR));

// The sounds [sound] can name: cuelume's recipes, in their order (copied from src/client/sound/recipes.ts, RECIPES).
pub const SOUND_NAMES: &[&str] = &[
    "chime", "sparkle", "droplet", "bloom", "whisper", "tick", "press", "release", "toggle", "success", "error", "page", "loading",
    "ready", "pulse", "scan", "arrival",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotifyKind {
    Toast,
    System,
    Sound,
    Bell,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Policy {
    Allow,
    Ask,
    Deny,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IndicatorStyle {
    Symbols,
    Dots,
    Letters,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BorderStyle {
    Single,
    Rounded,
    Double,
    Heavy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Logos {
    Auto,
    On,
    Off,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Stable,
    Staging,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Mouse {
    pub hover: bool, // the pointer resting on a list row selects it
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sidebar {
    pub visible: bool,
    pub width: i64,
    pub agents: String, // a plugin whose section replaces the AGENTS list
    pub logos: Logos,
    pub graph: bool,
}

// What the status row shows besides the buttons.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Status {
    pub agents: bool,
    pub panes: bool,
    pub theme: bool,
}

// The active space's repository in the status row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Git {
    pub status: bool,
    pub repo: bool,
    pub counts: bool,
    pub changes: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Panes {
    pub border: BorderStyle,
}

// How each agent event is told: per NotifyEvent (an AgentState but idle).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Notify {
    #[serde(deserialize_with = "known_kinds")]
    pub blocked: Vec<NotifyKind>,
    #[serde(deserialize_with = "known_kinds")]
    pub done: Vec<NotifyKind>,
    #[serde(deserialize_with = "known_kinds")]
    pub working: Vec<NotifyKind>,
}

impl Notify {
    pub fn kinds(&self, event: AgentState) -> &[NotifyKind] {
        match event {
            AgentState::Blocked => &self.blocked,
            AgentState::Done => &self.done,
            AgentState::Working => &self.working,
            AgentState::Idle => &[],
        }
    }
}

// A kind modisa doesn't know is left out: in the TS it stayed in the list and matched nothing.
fn known_kinds<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<NotifyKind>, D::Error> {
    Ok(Vec::<Value>::deserialize(d)?.into_iter().filter_map(|k| serde_json::from_value(k).ok()).collect())
}

// A cuelume sound name per event.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sound {
    pub volume: f64,
    pub blocked: String,
    pub done: String,
    pub working: String,
}

impl Sound {
    pub fn name(&self, event: AgentState) -> Option<&str> {
        match event {
            AgentState::Blocked => Some(&self.blocked),
            AgentState::Done => Some(&self.done),
            AgentState::Working => Some(&self.working),
            AgentState::Idle => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Indicators {
    pub style: IndicatorStyle,
    pub tab: bool,
    pub pane: bool,
    pub sidebar: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PaneLabels {
    pub agent: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Update {
    pub check: bool,
    pub channel: Channel,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Messaging {
    pub max_hops: i64,
    pub per_minute: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Permissions {
    pub keys_foreign: Policy,
    pub close_foreign: Policy,
    pub run_foreign: Policy,
}

// [[plugin]]: a program started with the session server.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginEntry {
    pub run: String,
}

// [agents.<id>]: any of the adapter's fields (launch, resume, …) over it; see adapters.rs.
pub type AgentOverride = Map<String, Value>;

fn yes() -> bool {
    true
}

fn split() -> String {
    "split".into()
}

// [modes.<name>]: a key mode, entered with `enter` after the prefix; its own keys work alone until escape.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ModeConfig {
    #[serde(default)]
    pub enter: String,
    #[serde(default = "yes")]
    pub sticky: bool, // stay after each key (false: one key, then back)
    #[serde(default)]
    pub timeout: u64, // ms of no key before it ends; 0 never
    #[serde(default)]
    pub keys: Map<String, Value>, // action → its key(s) in the mode, as [keys]
}

// A [[command]]'s question: typed (`default` to start from), or picked from what `pick` prints, a line each.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CommandPrompt {
    pub name: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub default: String,
    #[serde(default)]
    pub pick: String,
}

// [[command]]: an entry of the user's own in the palette, with keys and prompts.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CommandConfig {
    pub name: String,
    pub run: String,
    #[serde(default)]
    pub key: String, // after the prefix
    #[serde(default)]
    pub root: String, // without it
    #[serde(default = "split", rename = "in")]
    pub place: String, // split, split-down, tab, zoomed, background
    #[serde(default)]
    pub cwd: String, // default: the focused pane's
    #[serde(default)]
    pub prompts: Vec<CommandPrompt>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub prefix: String,
    pub theme: String,
    pub mouse: Mouse,
    pub sidebar: Sidebar,
    pub status: Status,
    pub git: Git,
    pub panes: Panes,
    pub notify: Notify,
    pub sound: Sound,
    pub indicators: Indicators,
    pub pane_labels: PaneLabels,
    pub update: Update,
    pub messaging: Messaging,
    pub permissions: Permissions,
    pub agents: IndexMap<String, AgentOverride>,
    pub plugin: Vec<PluginEntry>,
    pub plugin_keys: IndexMap<String, String>, // "<plugin>.<action or pane>" → key ("" turns it off); outranks plugin.json
    pub keys: Map<String, Value>, // action → its key(s) after the prefix, in place of modisa's ("" for none): see keys.rs
    pub actions: IndexMap<String, Vec<String>>, // [actions]: a name for a list of actions, run in order
    pub root_keys: Map<String, Value>, // action → its key(s) with no prefix (a modifier needed): see keys.rs
    pub modes: IndexMap<String, ModeConfig>,
    pub command: Vec<CommandConfig>,
    pub remote_command: String,
}

pub fn defaults() -> Config {
    Config {
        prefix: "C-b".into(),
        theme: "ion".into(),
        mouse: Mouse { hover: true },
        sidebar: Sidebar { visible: true, width: 26, agents: String::new(), logos: Logos::Auto, graph: false },
        status: Status { agents: true, panes: true, theme: false },
        git: Git { status: true, repo: true, counts: true, changes: true },
        panes: Panes { border: BorderStyle::Single },
        notify: Notify { blocked: vec![NotifyKind::Toast, NotifyKind::System, NotifyKind::Sound], done: vec![NotifyKind::Toast], working: vec![] },
        sound: Sound { volume: 0.7, blocked: "chime".into(), done: "success".into(), working: "loading".into() },
        indicators: Indicators { style: IndicatorStyle::Symbols, tab: true, pane: true, sidebar: true },
        pane_labels: PaneLabels { agent: true },
        update: Update { check: true, channel: Channel::Stable },
        messaging: Messaging { max_hops: 10, per_minute: 5 },
        permissions: Permissions { keys_foreign: Policy::Ask, close_foreign: Policy::Ask, run_foreign: Policy::Ask },
        agents: IndexMap::new(),
        plugin: vec![],
        plugin_keys: IndexMap::new(),
        keys: Map::new(),
        actions: IndexMap::new(),
        root_keys: Map::new(),
        modes: IndexMap::new(),
        command: vec![],
        remote_command: "modisa".into(),
    }
}

// Every table's default is its part of defaults(), so a partial table deserializes over them.
macro_rules! default_from_defaults {
    ($($ty:ident . $field:ident),*) => {
        $(impl Default for $ty { fn default() -> Self { defaults().$field } })*
    };
}
default_from_defaults!(
    Mouse.mouse, Sidebar.sidebar, Status.status, Git.git, Panes.panes, Notify.notify, Sound.sound, Indicators.indicators,
    PaneLabels.pane_labels, Update.update, Messaging.messaging, Permissions.permissions
);

impl Default for Config {
    fn default() -> Self {
        defaults()
    }
}

pub const SAMPLE: &str = r#"# modisa config — changes apply live (Ctrl+B s opens the settings page)
prefix = "C-b"              # C-<key>
theme = "ion"               # ion, tokyonight, catppuccin-mocha, gruvbox, nord, dracula, bearded-* (see settings)

[sidebar]
visible = true
width = 26                  # 20 to 48 columns, at most a third of the terminal; dragging its edge sets it
agents = ""                 # a plugin whose sidebar section takes the AGENTS list's place ("radar"); "" keeps modisa's
logos = "auto"              # agents' logos where the terminal can show them (modisa logos); "on", or "off" for plain marks
graph = false               # true draws the AGENTS list as a git graph of its tabs; the dots stay either way

[status]                    # the bottom row, besides its buttons
agents = true               # how many agents are working and need you
panes = true                # how many panes this tab has
theme = false               # the theme's name (click it to change theme)

[git]                       # the active space's repository, on the right of the status row
status = true               # its branch (green when clean and in step with its upstream)
repo = true                 # the repository's name before it
counts = true               # ↑ commits to push, ↓ commits to pull
changes = true              # ● files changed

[panes]
border = "single"           # single, rounded, double or heavy

[mouse]                     # clicks, drags, the wheel and right-click always work (Shift-drag selects text natively)
hover = true                # the pointer resting on a row of a menu, picker or the settings page selects it

[notify]                    # toast, system, sound, bell — when an agent you're not looking at…
blocked = ["toast", "system", "sound"]   # …needs you
done = ["toast"]                         # …finished
working = []                             # …started working

[sound]                     # the sound each event plays (cuelume): chime, sparkle, droplet, bloom,
volume = 0.7                # whisper, tick, press, release, toggle, success, error, page, loading,
blocked = "chime"           # ready, pulse, scan, arrival
done = "success"
working = "loading"

[indicators]
style = "symbols"           # symbols ! ◆ ✓ ○ · dots ● ● ● ○ · letters B W D I
tab = true                  # badge on tabs with an agent that needs you
pane = true                 # in pane border titles
sidebar = true

[pane_labels]
agent = true                # agent and state in the pane's border title

[update]
check = true                # tell me when a new modisa is out (modisa update installs it)
channel = "stable"          # stable, or staging for prerelease builds

[messaging]
max_hops = 10               # stop reply chains after this many hops
per_minute = 5              # per sender→recipient pair

[permissions]               # allow, ask, deny — for agents acting on panes they didn't create
keys_foreign = "ask"
close_foreign = "ask"
run_foreign = "ask"

# [agents.claude-code]
# launch = "claude --model opus"

# Programs started with the session server, with $MODISA_SOCKET set. See examples/plugins.
# [[plugin]]
# run = "my-plugin --socket $MODISA_SOCKET"

# A plugin's keys (after the prefix) are the ones its plugin.json asks for unless you change them here:
# "<plugin>.<action or pane>" = "K", or "" to turn one off.
# [plugin_keys]
# "attention-log.log" = "A"

# modisa's own keys (after the prefix): "<action>" = "K", or a list of keys, or "" for none. An action set here
# loses its default keys; the keyboard guide (prefix ?) names every action. x (close pane), d (detach) and escape
# can't be given away. modisa config check finds mistakes; modisa config reset-keys puts the defaults back.
# [keys]
# zoom = "f"
# split-right = ["v", "|"]

# A name for a list of actions, run in order; bind it like any action ([keys], [root_keys], a mode).
# [actions]
# "dev-layout" = ["split-right", "focus-left", "zoom"]

# Keys that work without the prefix (they never reach the panes, so each needs C- or M-, or is an f-key).
# [root_keys]
# focus-left = "M-h"
# focus-right = "M-l"
# palette = "M-return"

# A key mode: `enter` after the prefix, then its keys work alone until escape (sticky = false: one key, then back).
# [modes.resize]
# enter = "r"
# keys = { resize-left = "h", resize-right = "l", resize-down = "j", resize-up = "k" }

# Your own commands, in the palette and on keys. `in`: split, split-down, tab, zoomed or background. {cwd}, {pane},
# {name}, {space}, {tab}, {session} and each prompt's name are filled in (prompt answers shell-quoted).
# [[command]]
# name = "Run tests"
# key = "T"
# run = "bun test {filter}"
# prompts = [{ name = "filter", title = "Test filter" }]

# How --remote starts modisa on the far side of ssh. Set an absolute path when it isn't on the
# PATH of a non-interactive ssh shell (~/.local/bin often isn't).
# remote_command = "modisa"
"#;

// A TOML parse error's message, and where it is: the line and column (both from 1) its span starts at.
#[derive(Clone, Debug, PartialEq)]
pub struct TomlError {
    pub message: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
}

impl TomlError {
    // How a bad config is reported: "line 3: <message>".
    pub fn describe(&self) -> String {
        match self.line {
            Some(line) => format!("line {line}: {}", self.message),
            None => self.message.clone(),
        }
    }
}

pub fn toml_error(e: &toml::de::Error, source: &str) -> TomlError {
    let at = e.span().map(|s| {
        let before = source.get(..s.start).unwrap_or(source);
        let line_start = before.rfind('\n').map_or(0, |i| i + 1);
        (before.matches('\n').count() + 1, before[line_start..].chars().count() + 1)
    });
    TomlError { message: e.message().to_string(), line: at.map(|a| a.0), column: at.map(|a| a.1) }
}

// A TOML document as the JSON-ish value modisa reads settings from, with tables in the order they're written (the toml
// crate sorts them; the spans of the keys put them back). Numbers are JavaScript's: a float with no fraction is an
// integer. JSON has no nan or inf, and Bun reads them as strings, so they're strings here too; a datetime (which Bun
// can't read at all) is its TOML text.
pub fn parse_toml(source: &str) -> Result<Map<String, Value>, TomlError> {
    let values: toml::Table = toml::from_str(source).map_err(|e| toml_error(&e, source))?;
    let doc = toml::de::DeTable::parse(source).map_err(|e| toml_error(&e, source))?;
    Ok(ordered(&values, Some(doc.get_ref())))
}

fn ordered(values: &toml::Table, doc: Option<&toml::de::DeTable>) -> Map<String, Value> {
    let mut keys: Vec<(usize, &str)> = match doc {
        Some(doc) => doc.keys().map(|k| (k.span().start, k.get_ref().as_ref())).collect(),
        None => values.keys().map(|k| (0, k.as_str())).collect(),
    };
    keys.sort_by_key(|(at, _)| *at);
    let inner = |k: &str| doc.and_then(|d| d.get(k)).map(|v| v.get_ref());
    keys.into_iter().filter_map(|(_, k)| values.get(k).map(|v| (k.to_string(), json(v, inner(k))))).collect()
}

fn json(v: &toml::Value, doc: Option<&toml::de::DeValue>) -> Value {
    use toml::de::DeValue;
    match v {
        toml::Value::String(s) => Value::String(s.clone()),
        toml::Value::Integer(i) => Value::from(*i),
        toml::Value::Float(f) if f.is_nan() => Value::String("nan".into()),
        toml::Value::Float(f) if f.is_infinite() => Value::String(if *f > 0.0 { "inf" } else { "-inf" }.into()),
        toml::Value::Float(f) if f.fract() == 0.0 && f.abs() < 9007199254740992.0 => Value::from(*f as i64),
        toml::Value::Float(f) => Value::from(*f),
        toml::Value::Boolean(b) => Value::Bool(*b),
        toml::Value::Datetime(d) => Value::String(d.to_string()),
        toml::Value::Array(items) => {
            let at = |i: usize| match doc {
                Some(DeValue::Array(a)) => a.get(i).map(|v| v.get_ref()),
                _ => None,
            };
            Value::Array(items.iter().enumerate().map(|(i, v)| json(v, at(i))).collect())
        }
        toml::Value::Table(t) => Value::Object(ordered(
            t,
            match doc {
                Some(DeValue::Table(d)) => Some(d),
                _ => None,
            },
        )),
    }
}

// `{ ...base, ...user }` for one typed table: each of the user's settings over the default, and settings modisa
// doesn't know left out.
// ponytail: the TS kept a value of the wrong type as it was and let whatever read it cope; a typed table can't hold it,
// so it's ignored and the default stays (`modisa config check` reports it either way).
pub(crate) fn overlay<T: Serialize + DeserializeOwned>(base: T, user: Option<&Value>) -> T {
    let (Some(Value::Object(user)), Ok(Value::Object(mut merged))) = (user, serde_json::to_value(&base)) else { return base };
    for (k, v) in user {
        let before = merged.insert(k.clone(), v.clone());
        if serde_json::from_value::<T>(Value::Object(merged.clone())).is_err() {
            match before {
                Some(b) => merged.insert(k.clone(), b),
                None => merged.remove(k),
            };
        }
    }
    serde_json::from_value(Value::Object(merged)).unwrap_or(base)
}

fn typed<T: DeserializeOwned>(v: Option<&Value>) -> Option<T> {
    v.and_then(|v| serde_json::from_value(v.clone()).ok())
}

// A table of the user's own entries (no defaults), each that has the right type.
fn entries<T: DeserializeOwned>(v: Option<&Value>) -> IndexMap<String, T> {
    v.and_then(Value::as_object).map(|m| m.iter().filter_map(|(k, v)| Some((k.clone(), typed(Some(v))?))).collect()).unwrap_or_default()
}

// config.toml's settings merged over the defaults.
pub fn merge(user: &Map<String, Value>) -> Config {
    let d = defaults();
    let get = |k: &str| user.get(k);
    let mut git = d.git;
    // [sidebar] git was where turning git off lived before it moved to the status row
    if get("sidebar").and_then(|s| s.get("git")) == Some(&Value::Bool(false)) {
        git.status = false;
    }
    Config {
        prefix: typed(get("prefix")).unwrap_or(d.prefix),
        theme: typed(get("theme")).unwrap_or(d.theme),
        mouse: overlay(d.mouse, get("mouse")),
        sidebar: overlay(d.sidebar, get("sidebar")),
        status: overlay(d.status, get("status")),
        git: overlay(git, get("git")),
        panes: overlay(d.panes, get("panes")),
        notify: overlay(d.notify, get("notify")),
        sound: overlay(d.sound, get("sound")),
        indicators: overlay(d.indicators, get("indicators")),
        pane_labels: overlay(d.pane_labels, get("pane_labels")),
        update: overlay(d.update, get("update")),
        messaging: overlay(d.messaging, get("messaging")),
        permissions: overlay(d.permissions, get("permissions")),
        agents: entries(get("agents")),
        // ponytail: a [[plugin]] without a run string is left out (the TS ran `sh -lc undefined`)
        plugin: get("plugin").and_then(Value::as_array).map(|p| p.iter().filter_map(|e| typed(Some(e))).collect()).unwrap_or_default(),
        plugin_keys: entries(get("plugin_keys")),
        keys: get("keys").and_then(Value::as_object).cloned().unwrap_or_default(),
        actions: entries(get("actions")),
        root_keys: get("root_keys").and_then(Value::as_object).cloned().unwrap_or_default(),
        modes: entries(get("modes")),
        command: get("command").and_then(Value::as_array).map(|c| c.iter().filter_map(|e| typed(Some(e))).collect()).unwrap_or_default(),
        remote_command: typed(get("remote_command")).unwrap_or(d.remote_command),
    }
}

// config.toml merged over the defaults; when it can't be read, the defaults and why.
pub fn read_config() -> (Config, Option<String>) {
    read_config_from(Path::new(&*CONFIG_PATH))
}

pub fn read_config_from(path: &Path) -> (Config, Option<String>) {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (defaults(), None),
        Err(e) => return (defaults(), Some(e.to_string())),
    };
    match parse_toml(&text) {
        Ok(user) => (merge(&user), None),
        Err(e) => (defaults(), Some(e.describe())),
    }
}

pub fn load_config() -> Config {
    let (cfg, error) = read_config();
    if let Some(error) = error {
        eprintln!("modisa: bad config {}: {error}", *CONFIG_PATH);
    }
    cfg
}

pub fn ensure_config_file() -> std::io::Result<String> {
    let path = Path::new(&*CONFIG_PATH);
    if !path.exists() {
        std::fs::create_dir_all(&*CONFIG_DIR)?;
        std::fs::write(path, SAMPLE)?;
    }
    Ok(CONFIG_PATH.clone())
}

// A value as TOML writes it: a string quoted (JSON's escapes are TOML's), a number as JavaScript prints it.
fn literal(v: &Value) -> String {
    match v {
        Value::Array(items) => format!("[{}]", items.iter().map(literal).collect::<Vec<_>>().join(", ")),
        Value::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => i.to_string(),
            (None, Some(f)) => f.to_string(),
            _ => n.to_string(),
        },
        other => other.to_string(),
    }
}

// JSON.stringify(a) === JSON.stringify(b), where 1 and 1.0 are one number.
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(a, b)| same(a, b)),
        _ => a == b,
    }
}

// Where a value that starts on lines[at] (text = what follows "key =") ends: its last line, and the comment
// after it there (with the spaces before it). Arrays may span lines; # inside strings isn't a comment.
fn value_end<'a, S: AsRef<str>>(lines: &'a [S], at: usize, mut text: &'a str) -> (usize, String) {
    let (mut depth, mut quote) = (0i32, None::<u8>);
    let mut line = at;
    loop {
        let b = text.as_bytes();
        let mut i = 0;
        while i < b.len() {
            let c = b[i];
            if let Some(q) = quote {
                if c == b'\\' && q == b'"' {
                    i += 1;
                } else if c == q {
                    quote = None;
                }
            } else if c == b'"' || c == b'\'' {
                quote = Some(c);
            } else if c == b'[' {
                depth += 1;
            } else if c == b']' {
                depth -= 1;
            } else if c == b'#' {
                if depth <= 0 {
                    return (line, text[text[..i].trim_end().len()..].to_string());
                }
                break; // a comment inside a multiline array
            }
            i += 1;
        }
        if depth <= 0 || line + 1 >= lines.len() {
            return (line, String::new());
        }
        line += 1;
        text = lines[line].as_ref();
    }
}

fn header(l: &str) -> bool {
    l.trim_start().starts_with('[')
}

fn assignment(key: &str) -> regex::Regex {
    let k = regex::escape(key);
    regex::Regex::new(&format!(r#"^(\s*(?:{k}|"{k}"|'{k}')\s*=\s*)(.*)$"#)).expect("an escaped key makes a valid pattern")
}

// Where [table] (None: the top of the file) is in config.toml's lines: the table's first line and the line past its
// last, and the line `key` is set on, if it is. None: there's no such table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyAt {
    pub start: usize,
    pub end: usize,
    pub at: Option<usize>,
}

pub fn find_key<S: AsRef<str>>(lines: &[S], table: Option<&str>, key: Option<&str>) -> Option<KeyAt> {
    let mut start = 0;
    if let Some(table) = table {
        let want = format!("[{table}]");
        start = lines.iter().position(|l| l.as_ref().split('#').next().unwrap_or("").trim() == want)? + 1;
    }
    let end = (start..lines.len()).find(|&i| header(lines[i].as_ref())).unwrap_or(lines.len());
    let at = key.and_then(|key| {
        let set = assignment(key);
        (start..end).find(|&i| set.is_match(lines[i].as_ref()))
    });
    Some(KeyAt { start, end, at })
}

// Set one key — at the root (table None) or inside [table] — keeping comments and every other setting.
// A missing key goes at the end of its table; a missing table is added at the end of the file.
pub fn with_value(source: &str, table: Option<&str>, key: &str, value: &Value) -> Result<String, String> {
    let mut lines: Vec<String> = source.split('\n').map(String::from).collect();
    let Some(KeyAt { start, end, at }) = find_key(&lines, table, Some(key)) else {
        return Ok(format!("{}\n\n[{}]\n{key} = {}\n", source.trim_end(), table.unwrap_or_default(), literal(value)));
    };
    if let Some(at) = at {
        let (lead, stop, comment) = {
            let caps = assignment(key).captures(&lines[at]).expect("find_key matched this line");
            let (stop, comment) = value_end(&lines, at, caps.get(2).map_or("", |m| m.as_str()));
            (caps[1].to_string(), stop, comment)
        };
        lines.splice(at..=stop, [format!("{lead}{}{comment}", literal(value))]);
    } else {
        let mut last = end;
        while last > start && lines[last - 1].trim().is_empty() {
            last -= 1;
        }
        lines.insert(last, format!("{key} = {}", literal(value)));
    }
    let result = lines.join("\n");
    let check = parse_toml(&result).ok();
    let got = check.as_ref().and_then(|c| match table {
        Some(t) => c.get(t).and_then(|t| t.get(key)),
        None => c.get(key),
    });
    if !got.is_some_and(|g| same(g, value)) {
        return Err(format!("couldn't update {}{key} in config.toml safely; edit it by hand", table.map(|t| format!("{t}.")).unwrap_or_default()));
    }
    Ok(result)
}

// Take [table] out: its header and every setting in it, keeping the comments around and in it, and everything else.
pub fn without_table(source: &str, table: &str) -> Result<String, String> {
    let mut lines: Vec<String> = source.split('\n').map(String::from).collect();
    while let Some(t) = find_key(&lines, Some(table), None) {
        let mut kept: Vec<String> = Vec::new();
        let mut i = t.start;
        while i < t.end {
            let l = &lines[i];
            if l.trim().is_empty() || l.trim_start().starts_with('#') {
                kept.push(l.clone());
            } else {
                i = value_end(&lines, i, &l[l.find('=').map_or(0, |p| p + 1)..]).0; // a setting, however many lines its value takes
            }
            i += 1;
        }
        // no blank line left doubled where the header was
        while kept.first().is_some_and(|k| k.trim().is_empty()) && (t.start == 1 || lines[t.start - 2].trim().is_empty()) {
            kept.remove(0);
        }
        lines.splice(t.start - 1..t.end, kept);
    }
    let result = lines.join("\n");
    match parse_toml(&result) {
        Ok(check) if !check.contains_key(table) => Ok(result),
        _ => Err(format!("couldn't take [{table}] out of config.toml safely; edit it by hand")),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct KeysReset {
    pub result: String,
    pub changes: Vec<String>,
}

// modisa's own keys back (`config reset-keys`): [keys] and [plugin_keys] out and the prefix C-b again, each change
// said. Fails when the file doesn't parse: nothing in it can be edited safely then.
pub fn with_default_keys(source: &str) -> Result<KeysReset, String> {
    let user = parse_toml(source).map_err(|e| format!("config.toml doesn't parse ({}); fix that first", e.describe()))?;
    let mut result = source.to_string();
    let mut changes = Vec::new();
    for table in ["keys", "plugin_keys"] {
        let Some(settings) = user.get(table) else { continue };
        result = without_table(&result, table)?;
        let n = settings.as_object().map_or(0, |m| m.len());
        changes.push(format!("[{table}] removed ({n} {})", if n == 1 { "setting" } else { "settings" }));
    }
    let prefix = Value::String(defaults().prefix);
    if let Some(p) = user.get("prefix").filter(|p| **p != prefix) {
        result = with_value(&result, None, "prefix", &prefix)?;
        changes.push(format!("prefix {p} → {prefix}"));
    }
    Ok(KeysReset { result, changes })
}

// Write one setting to config.toml (creating it from SAMPLE first), preserving everything else.
pub fn save_setting(table: Option<&str>, key: &str, value: &Value) -> Result<(), String> {
    let path = ensure_config_file().map_err(|e| e.to_string())?;
    let source = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    std::fs::write(&path, with_value(&source, table, key, value)?).map_err(|e| e.to_string())
}

// Hot reload: poll the file once a second. Must run inside the LocalSet; like the TS's unref'd interval, it doesn't
// keep the program alive.
// ponytail: polling, not fs events; a change waits up to a second, and an edit undone within one tick goes unseen.
pub fn watch_config(on_change: impl Fn() + 'static) {
    watch_file(CONFIG_PATH.clone(), on_change);
}

fn watch_file(path: String, on_change: impl Fn() + 'static) {
    // its modification time, and its size for an edit within the mtime's resolution; None: no file
    let stamp = move || std::fs::metadata(&path).ok().map(|m| (m.modified().ok(), m.len()));
    let mut last = stamp();
    tokio::task::spawn_local(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
        tick.tick().await; // the first tick is now
        loop {
            tick.tick().await;
            let now = stamp();
            if now != last {
                last = now;
                on_change();
            }
        }
    });
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prefix {
    pub ctrl: bool,
    pub name: String,
}

// "C-b" → ctrl+b; anything else is C-b.
pub fn parse_prefix(p: &str) -> Prefix {
    let key = p.get(..2).filter(|c| c.eq_ignore_ascii_case("C-")).and_then(|_| one_js_char(&p[2..]));
    Prefix { ctrl: true, name: key.map_or_else(|| "b".into(), |c| c.to_lowercase().collect()) }
}

// The one character `s` is, as a JavaScript regex's `.` matches it: one UTF-16 unit that doesn't end a line.
pub(crate) fn one_js_char(s: &str) -> Option<char> {
    let mut chars = s.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.len_utf16() == 1 && !matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}') => Some(c),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(s: &str) -> Value {
        Value::Object(parse_toml(s).unwrap())
    }

    // A scratch directory of the test's own: never the user's ~/.config.
    pub(crate) fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("modisa-config-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn theme_saving_preserves_comments_tables_and_unrelated_settings() {
        let source = "# my setup\nprefix = \"C-a\"\ntheme = \"ion\" # keep this\n\n[sidebar]\nvisible = false\nwidth = 30\n";
        let result = with_value(source, None, "theme", &Value::from("dracula")).unwrap();
        assert_eq!(result, source.replace("theme = \"ion\"", "theme = \"dracula\""));
        assert_eq!(parse(&result), json!({ "prefix": "C-a", "theme": "dracula", "sidebar": { "visible": false, "width": 30 } }));
        assert_eq!(parse(&with_value("[sidebar]\nvisible = false\n", None, "theme", &Value::from("nord")).unwrap()), json!({ "theme": "nord", "sidebar": { "visible": false } }));
    }

    #[test]
    fn settings_edit_one_key_in_place_in_any_table_keeping_comments() {
        let source = "# mine\ntheme = \"ion\"\n\n[notify]            # kinds\nblocked = [\"toast\", \"sound\"]   # needs you\ndone = [\"toast\"]\n\n[sidebar]\nvisible = true\n";
        // an existing key inside a table: value replaced, its comment kept
        let a = with_value(source, Some("notify"), "blocked", &json!(["toast"])).unwrap();
        assert_eq!(a, source.replace("blocked = [\"toast\", \"sound\"]", "blocked = [\"toast\"]"));
        // a new key in an existing table goes at the end of that table, not the file
        let b = with_value(source, Some("notify"), "working", &json!([])).unwrap();
        assert!(b.contains("done = [\"toast\"]\nworking = []\n\n[sidebar]"));
        // a new table is appended; booleans and numbers are written as TOML literals
        let c = with_value(&with_value(source, Some("sound"), "volume", &json!(0.5)).unwrap(), Some("indicators"), "tab", &json!(false)).unwrap();
        assert!(c.starts_with(source.trim_end()));
        let c = parse(&c);
        assert_eq!((&c["theme"], &c["sound"], &c["indicators"], &c["sidebar"]), (&json!("ion"), &json!({ "volume": 0.5 }), &json!({ "tab": false }), &json!({ "visible": true })));
        // a # inside a string isn't mistaken for a comment
        assert_eq!(with_value("[sound]\nblocked = \"a#b\"  # c\n", Some("sound"), "blocked", &json!("chime")).unwrap(), "[sound]\nblocked = \"chime\"  # c\n");
        // a multiline array is replaced whole, with the comment after it kept
        let multi = "[notify]\nblocked = [\n  \"toast\", # mine\n  \"sound\",\n]  # end\ndone = []\n";
        assert_eq!(with_value(multi, Some("notify"), "blocked", &json!(["bell"])).unwrap(), "[notify]\nblocked = [\"bell\"]  # end\ndone = []\n");
        // a root key, and a whole number written as one
        assert_eq!(with_value("theme = \"ion\"\n\n[sidebar]\nwidth = 30\n", Some("sidebar"), "width", &json!(31.0)).unwrap(), "theme = \"ion\"\n\n[sidebar]\nwidth = 31\n");
        assert_eq!(with_value("# top\n\n[git]\nrepo = true\n", None, "prefix", &json!("C-a")).unwrap(), "# top\nprefix = \"C-a\"\n\n[git]\nrepo = true\n");
    }

    #[test]
    fn a_key_is_matched_literally_dots_and_dashes_in_a_quoted_key_arent_patterns() {
        let source = "[plugin_keys]\n\"radarXlog\" = \"A\"\n\"radar.log\" = \"B\"\n";
        assert_eq!(with_value(source, Some("plugin_keys"), "radar.log", &json!("C")).unwrap(), "[plugin_keys]\n\"radarXlog\" = \"A\"\n\"radar.log\" = \"C\"\n");
        let lines: Vec<&str> = source.split('\n').collect();
        assert_eq!(find_key(&lines, Some("plugin_keys"), Some("radar.log")), Some(KeyAt { start: 1, end: 4, at: Some(2) }));
        assert_eq!(find_key(&lines, Some("plugin_keys"), Some("radar.lo")).unwrap().at, None);
        assert_eq!(find_key(&lines, Some("keys"), None), None);
        // a new dotted key would be a table of its own: refused rather than written wrong
        assert!(with_value(source, Some("plugin_keys"), "new.key", &json!("D")).unwrap_err().contains("edit it by hand"));
    }

    #[test]
    fn without_table_takes_a_table_and_its_settings_out_keeping_every_comment_and_the_other_tables() {
        let source = "# mine\nprefix = \"C-a\" # p\n\n[keys]   # my keys\nzoom = \"f\"   # z\n# a note\nsplit-right = [\n  \"v\", # one\n  \"|\",\n]\n\n[sidebar]\nwidth = 30\n";
        let result = without_table(source, "keys").unwrap();
        assert_eq!(result, "# mine\nprefix = \"C-a\" # p\n\n# a note\n\n[sidebar]\nwidth = 30\n");
        assert_eq!(parse(&result), json!({ "prefix": "C-a", "sidebar": { "width": 30 } }));
        assert_eq!(without_table("theme = \"ion\"\n\n[keys]\nzoom = \"f\"\n", "keys").unwrap(), "theme = \"ion\"\n");
        assert_eq!(without_table("[keys]\nzoom = \"f\"\n\n[git]\nrepo = false\n", "keys").unwrap(), "[git]\nrepo = false\n");
        assert_eq!(without_table(source, "absent").unwrap(), source);
        assert!(without_table("keys = { zoom = \"f\" }\n", "keys").unwrap_err().contains("by hand")); // not a [keys] table: left alone
    }

    #[test]
    fn reset_keys_takes_out_keys_and_plugin_keys_and_puts_the_prefix_back_saying_what_changed() {
        let KeysReset { result, changes } = with_default_keys("prefix = \"C-a\"   # mine\n\n[keys]\nzoom = \"f\"\n\n[plugin_keys]\n\"a.b\" = \"Y\"\n\"c.d\" = \"\"\n").unwrap();
        assert_eq!(result, "prefix = \"C-b\"   # mine\n");
        assert_eq!(changes, ["[keys] removed (1 setting)", "[plugin_keys] removed (2 settings)", "prefix \"C-a\" → \"C-b\""]);
        assert_eq!(with_default_keys("theme = \"ion\"\n").unwrap().changes, Vec::<String>::new());
        assert!(with_default_keys("a = \n").unwrap_err().starts_with("config.toml doesn't parse (line 1"));
    }

    #[test]
    fn settings_merge_over_the_defaults_table_by_table() {
        assert_eq!(merge(&Map::new()), defaults());
        assert_eq!(parse_toml(SAMPLE).map(|u| merge(&u)), Ok(defaults())); // the sample says the defaults
        let user = parse_toml(
            "prefix = \"C-a\"\ntheme = 5\nshiny = true\n[sidebar]\nwidth = 30.0\ngit = false\nlogos = \"maybe\"\n[notify]\nblocked = [\"toast\", \"pager\"]\nworking = \"x\"\n\
             [sound]\nvolume = 1\n[agents.claude]\nlaunch = \"claude --model opus\"\n[agents]\nbad = 3\n[[plugin]]\nrun = \"p1\"\n[[plugin]]\nnope = 1\n\
             [plugin_keys]\n\"a.b\" = \"Y\"\n\"c.d\" = 3\n[keys]\nzoom = \"f\"\nhelp = 3\n",
        )
        .unwrap();
        let cfg = merge(&user);
        assert_eq!((cfg.prefix.as_str(), cfg.theme.as_str()), ("C-a", "ion")); // a wrong type keeps the default
        assert_eq!(cfg.sidebar, Sidebar { width: 30, ..defaults().sidebar }); // logos "maybe" isn't one
        assert!(!cfg.git.status && cfg.git.repo); // [sidebar] git = false, the old place
        assert_eq!(cfg.notify, Notify { blocked: vec![NotifyKind::Toast], ..defaults().notify });
        assert_eq!(cfg.sound.volume, 1.0);
        assert_eq!(serde_json::to_value(&cfg.agents).unwrap(), json!({ "claude": { "launch": "claude --model opus" } }));
        assert_eq!(cfg.plugin, [PluginEntry { run: "p1".into() }]);
        assert_eq!(cfg.plugin_keys, IndexMap::from([("a.b".to_string(), "Y".to_string())]));
        assert_eq!(Value::Object(cfg.keys.clone()), json!({ "zoom": "f", "help": 3 })); // keys.rs reports what's wrong in it
        let mut git = parse_toml("[sidebar]\ngit = false\n[git]\nstatus = true\n").unwrap();
        assert!(merge(&git).git.status); // [git] has the last word
        git.remove("git");
        assert!(!merge(&git).git.status);
    }

    #[test]
    fn the_config_serializes_with_the_ts_field_names() {
        let v = serde_json::to_value(defaults()).unwrap();
        let fields: Vec<&str> = v.as_object().unwrap().keys().map(|k| k.as_str()).collect();
        let ts = ["prefix", "theme", "mouse", "sidebar", "status", "git", "panes", "notify", "sound", "indicators", "pane_labels", "update", "messaging", "permissions", "agents", "plugin", "plugin_keys", "keys"];
        let since = ["actions", "root_keys", "modes", "command", "remote_command"]; // the Rust build's, then the TS's last
        assert_eq!(fields, [&ts[..], &since[..]].concat());
        assert_eq!(v["sidebar"], json!({ "visible": true, "width": 26, "agents": "", "logos": "auto", "graph": false }));
        assert_eq!(v["notify"], json!({ "blocked": ["toast", "system", "sound"], "done": ["toast"], "working": [] }));
        assert_eq!(v["sound"], json!({ "volume": 0.7, "blocked": "chime", "done": "success", "working": "loading" }));
        assert_eq!(v["permissions"], json!({ "keys_foreign": "ask", "close_foreign": "ask", "run_foreign": "ask" }));
        let partial: Config = serde_json::from_value(json!({ "theme": "nord", "sidebar": { "width": 40 } })).unwrap();
        assert_eq!((partial.theme.as_str(), partial.sidebar.width, partial.sidebar.visible, partial.prefix.as_str()), ("nord", 40, true, "C-b"));
    }

    #[test]
    fn toml_keeps_the_order_it_was_written_in_and_javascripts_numbers() {
        let v = parse("zeta = 1\nalpha = 2.0\nmid = 2.5\nn = nan\n[b]\ny = 1\nx = [{ q = 1, p = 2 }]\n[a]\nk = 1979-05-27T07:32:00Z\n");
        assert_eq!(serde_json::to_string(&v).unwrap(), r#"{"zeta":1,"alpha":2,"mid":2.5,"n":"nan","b":{"y":1,"x":[{"q":1,"p":2}]},"a":{"k":"1979-05-27T07:32:00Z"}}"#);
        let e = parse_toml("theme = \"ion\"\n[sidebar]\nwidth = [1, 2\n").unwrap_err();
        assert_eq!((e.line, e.column), (Some(3), Some(14)));
        let e = parse_toml("x = 1\nx = 2\n").unwrap_err();
        assert_eq!((e.line, e.column), (Some(2), Some(1)));
    }

    #[test]
    fn reading_a_config_file() {
        let dir = scratch("read");
        let path = dir.join("config.toml");
        assert_eq!(read_config_from(&path), (defaults(), None)); // no file: the defaults
        std::fs::write(&path, "theme = \"nord\"\n[sidebar]\nvisible = false\n").unwrap();
        let (cfg, error) = read_config_from(&path);
        assert_eq!((cfg.theme.as_str(), cfg.sidebar.visible, error), ("nord", false, None));
        std::fs::write(&path, "theme = \"nord\"\n[sidebar\n").unwrap();
        let (cfg, error) = read_config_from(&path);
        assert_eq!(cfg, defaults());
        assert!(error.unwrap().starts_with("line 2: "));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prefixes() {
        assert_eq!(parse_prefix("C-a"), Prefix { ctrl: true, name: "a".into() });
        assert_eq!(parse_prefix("c-B"), Prefix { ctrl: true, name: "b".into() });
        assert_eq!(parse_prefix("C-é").name, "é");
        for bad in ["Ctrl-a", "C-ab", "C-", "C-😀", "", "C-\n"] {
            assert_eq!(parse_prefix(bad).name, "b");
        }
    }

    #[test]
    fn watching_sees_the_file_change() {
        let dir = scratch("watch");
        let path = dir.join("config.toml");
        std::fs::write(&path, "theme = \"ion\"\n").unwrap();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let seen = std::rc::Rc::new(std::cell::Cell::new(0));
        tokio::task::LocalSet::new().block_on(&rt, async {
            let s = seen.clone();
            watch_file(path.to_string_lossy().into_owned(), move || s.set(s.get() + 1));
            tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
            assert_eq!(seen.get(), 0); // nothing changed yet
            std::fs::write(&path, "theme = \"nord\"\n").unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        });
        assert_eq!(seen.get(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
