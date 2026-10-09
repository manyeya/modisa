// modisa's prefix bindings (key → action), shared by the client, which runs them, the server, which refuses plugin keys
// that would shadow them, and `config check`. [keys] in config.toml changes them.
use std::collections::HashMap;
use std::sync::LazyLock;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::check::Level;
use super::Config;

// Every action the client can run by name: a key, the palette, a menu or a button. The client's table of actions is
// keyed by these, so the two can't drift apart.
pub const ACTION_IDS: &[&str] = &[
    "theme-picker", "help", "pane-menu", "pane-picker", "working-agents", "blocked-agents",
    "split-right", "split-down", "focus-left", "focus-right", "focus-up", "focus-down",
    "resize-left", "resize-right", "resize-up", "resize-down", "zoom", "close-pane", "close-tab",
    "new-tab", "next-tab", "prev-tab", "workspace-picker", "new-workspace", "new-agent", "toggle-sidebar",
    "copy-mode", "search", "palette", "settings", "plugins", "edit-config", "reload-config", "update-modisa", "restart-server",
    "toggle-messaging", "message-log", "send-message", "rename-tab", "rename-pane", "rename-workspace", "delete-workspace",
    "detach", "agent-1", "agent-2", "agent-3", "agent-4", "agent-5", "agent-6", "agent-7", "agent-8", "agent-9",
];

// One of ACTION_IDS (the interned &'static str, so it can be compared and kept freely).
pub type ActionId = &'static str;

pub fn action_id(id: &str) -> Option<ActionId> {
    ACTION_IDS.iter().copied().find(|a| *a == id)
}

pub type Bindings = IndexMap<String, ActionId>; // key after the prefix → action

// The digits come first: they did in the TypeScript, whose objects list integer-like keys before the rest, and the
// keyboard guide lists bindings in this order.
pub static DEFAULT_KEYS: LazyLock<Bindings> = LazyLock::new(|| {
    let mut t: Bindings = (1..=9).map(|n| (n.to_string(), action_id(&format!("agent-{n}")).expect("a known action"))).collect();
    #[rustfmt::skip]
    let named: &[(&str, &str)] = &[
        ("v", "split-right"), ("%", "split-right"), ("-", "split-down"), ("\"", "split-down"),
        ("h", "focus-left"), ("j", "focus-down"), ("k", "focus-up"), ("l", "focus-right"),
        ("left", "focus-left"), ("down", "focus-down"), ("up", "focus-up"), ("right", "focus-right"),
        ("H", "resize-left"), ("J", "resize-down"), ("K", "resize-up"), ("L", "resize-right"),
        ("z", "zoom"), ("x", "close-pane"), ("X", "close-tab"),
        ("c", "new-tab"), ("n", "next-tab"), ("p", "prev-tab"),
        ("w", "workspace-picker"), ("W", "new-workspace"),
        ("a", "new-agent"), ("b", "toggle-sidebar"),
        ("o", "pane-picker"), ("e", "pane-menu"), ("?", "help"),
        ("t", "theme-picker"),
        ("[", "copy-mode"), ("/", "search"), (":", "palette"),
        ("s", "settings"), ("R", "reload-config"), ("P", "plugins"),
        ("m", "toggle-messaging"), ("i", "message-log"), ("M", "send-message"),
        (",", "rename-tab"), (".", "rename-pane"), ("$", "rename-workspace"), ("&", "delete-workspace"),
        ("d", "detach"),
    ];
    t.extend(named.iter().map(|(k, a)| (k.to_string(), action_id(a).expect("a known action"))));
    t
});

// Keys no plugin can have, and [keys] can't give away: x closes any pane or popup, d detaches, and escape (with the
// prefix twice, which types it through) are the ways out of anything a plugin opens. x and d stay on their actions
// whatever else [keys] gives those.
pub const RESERVED_KEYS: &[&str] = &["x", "d", "escape"];

fn kept(key: &str) -> Option<ActionId> {
    match key {
        "x" => Some("close-pane"),
        "d" => Some("detach"),
        _ => None,
    }
}

// What a key is called after the prefix: one character as typed (H is shift+h), or a key with a name.
static NAMED: LazyLock<Vec<String>> = LazyLock::new(|| {
    let mut n: Vec<String> = ["left", "right", "up", "down", "home", "end", "pageup", "pagedown"].map(String::from).to_vec();
    n.extend((1..=12).map(|i| format!("f{i}")));
    n
});

// JavaScript's \s, which isn't quite Unicode's White_Space: it has U+FEFF and lacks U+0085.
fn js_space(c: char) -> bool {
    c == '\u{feff}' || (c.is_whitespace() && c != '\u{85}')
}

fn is_key(key: &str) -> bool {
    let mut chars = key.chars();
    matches!((chars.next(), chars.next()), (Some(c), None) if !js_space(c)) || NAMED.iter().any(|n| n == key)
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct KeyProblem {
    pub level: Level,
    pub action: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedKeys {
    pub table: Bindings,
    pub more: IndexMap<String, String>, // key → what else it runs (a list, a command, a mode, plugin:, sh:)
    pub problems: Vec<KeyProblem>,
}

// The bindings [keys] makes ("<action>" = "K", or a list of keys; "" or [] for none), and what in it can't be bound. An
// action set there has only the keys given, each taken from whatever had it by default. `named` says which other
// names (not modisa's actions) can be bound: their keys go in `more`.
pub fn resolve_keys(keys: &Map<String, Value>, named: &dyn Fn(&str) -> bool) -> ResolvedKeys {
    let mut table = DEFAULT_KEYS.clone();
    let mut more = IndexMap::new();
    let mut problems = Vec::new();
    let mut given: HashMap<String, String> = HashMap::new(); // key → what [keys] gave it
    for (name, value) in keys {
        let action = action_id(name);
        if action.is_none() && !named(name) {
            let message = format!("there's no action {name} (the keyboard guide, prefix ?, lists them)");
            problems.push(KeyProblem { level: Level::Warning, action: name.clone(), message });
            continue;
        }
        let wanted: Vec<&Value> = match value {
            Value::Array(items) => items.iter().collect(),
            v => vec![v],
        };
        let Some(wanted) = wanted.iter().map(|k| k.as_str()).collect::<Option<Vec<&str>>>() else {
            problems.push(KeyProblem { level: Level::Error, action: name.clone(), message: "a key is a string (\"K\"), or a list of them".into() });
            continue;
        };
        if let Some(action) = action {
            table.retain(|key, a| *a != action || kept(key).is_some());
        }
        for key in wanted.into_iter().filter(|k| !k.is_empty()) {
            let mut error = |message: String| problems.push(KeyProblem { level: Level::Error, action: name.clone(), message });
            if RESERVED_KEYS.contains(&key) && (action.is_none() || kept(key) != action) {
                error(format!("{key} is reserved: it's how you get out of anything a plugin opens"));
                continue;
            }
            if !is_key(key) {
                let named = NAMED[..8].join(", ");
                error(format!("{} isn't a key: one character (H is shift+h), or {named} or f1 to f12", Value::from(key)));
                continue;
            }
            if let Some(other) = given.get(key) {
                if other != name {
                    error(format!("{key} is given to {other} too; the last one wins"));
                }
            }
            given.insert(key.to_string(), name.clone());
            match action {
                Some(a) => {
                    more.shift_remove(key);
                    table.insert(key.to_string(), a);
                }
                None => {
                    table.shift_remove(key);
                    more.insert(key.to_string(), name.clone());
                }
            }
        }
    }
    ResolvedKeys { table, more, problems }
}

pub fn bindings(cfg: &Config) -> Bindings {
    all_keys(cfg).0.prefix
}

// ---------- everything a key can run, and keys beyond the prefix ----------

// What a key, a mode or an [actions] list can run by name: one of modisa's actions, an [actions] list, a [[command]],
// a mode (mode:<name>), a plugin's action (plugin:<name>.<action>), or a shell command run in the background
// (sh:<command>).
pub fn known(cfg: &Config, name: &str) -> bool {
    action_id(name).is_some()
        || cfg.actions.contains_key(name)
        || cfg.command.iter().any(|c| c.name == name)
        || name.strip_prefix("mode:").is_some_and(|m| cfg.modes.contains_key(m))
        || name.strip_prefix("plugin:").and_then(|p| p.split_once('.')).is_some_and(|(p, a)| !p.is_empty() && !a.is_empty())
        || name.strip_prefix("sh:").is_some_and(|c| !c.trim().is_empty())
}

// Keys with names beyond the one-character ones, for chords and modes.
const SPECIAL: &[&str] = &["return", "tab", "space", "backspace", "escape", "delete"];

// A key with its modifiers, written C- (ctrl), M- (alt), S- (shift, for a named key: a shifted character is just that
// character) before its name: "M-h", "C-M-left", "S-tab". The one way modisa writes it (C-, M-, S- in that order), or
// None when it isn't one.
pub fn chord(s: &str) -> Option<String> {
    let (mut ctrl, mut alt, mut shift, mut rest) = (false, false, false, s);
    loop {
        let flag = match rest.get(..2) {
            Some("C-") => &mut ctrl,
            Some("M-") => &mut alt,
            Some("S-") => &mut shift,
            _ => break,
        };
        if rest.len() == 2 {
            break; // "C-" alone isn't a key ("C--" is ctrl and -)
        }
        *flag = true;
        rest = &rest[2..];
    }
    if !(is_key(rest) || SPECIAL.contains(&rest)) || (shift && rest.chars().count() == 1) {
        return None;
    }
    Some(format!("{}{}{}{rest}", if ctrl { "C-" } else { "" }, if alt { "M-" } else { "" }, if shift { "S-" } else { "" }))
}

// A key that works without the prefix mustn't be one typing needs: it has C- or M-, or it's an f-key.
fn rootable(chord: &str) -> bool {
    chord.starts_with("C-") || chord.starts_with("M-") || chord.strip_prefix("S-").unwrap_or(chord).strip_prefix('f').is_some_and(|n| n.parse::<u8>().is_ok())
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mode {
    pub sticky: bool,
    pub timeout: u64,
    pub keys: IndexMap<String, String>, // key (as chord() writes it) → what it runs
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Keys {
    pub prefix: Bindings,               // modisa's actions after the prefix
    pub more: IndexMap<String, String>, // the rest after the prefix: lists, commands, modes' `enter`, plugin:, sh:
    pub root: IndexMap<String, String>, // without the prefix
    pub modes: IndexMap<String, Mode>,
}

// One table's "<action>" = key(s): each key as chord() writes it, or a problem.
fn table_keys(table: &str, map: &Map<String, Value>, cfg: &Config, root: bool, problems: &mut Vec<(String, KeyProblem)>) -> IndexMap<String, String> {
    let mut out = IndexMap::new();
    for (name, value) in map {
        let mut problem = |level: Level, message: String| problems.push((table.to_string(), KeyProblem { level, action: name.clone(), message }));
        if !known(cfg, name) {
            problem(Level::Warning, format!("there's no action {name} (the keyboard guide, prefix ?, lists them)"));
            continue;
        }
        let wanted: Vec<&Value> = match value {
            Value::Array(items) => items.iter().collect(),
            v => vec![v],
        };
        for k in wanted {
            let Some(k) = k.as_str() else {
                problem(Level::Error, "a key is a string (\"M-h\"), or a list of them".into());
                continue;
            };
            if k.is_empty() {
                continue;
            }
            let Some(c) = chord(k) else {
                problem(Level::Error, format!("{} isn't a key: C-, M- or S- and a key's name (one character, {}, f1 to f12)", Value::from(k), SPECIAL.join(", ")));
                continue;
            };
            if root && !rootable(&c) {
                problem(Level::Error, format!("{c} would take a key the panes need: give it C- or M- (or use an f-key)"));
                continue;
            }
            if root && format!("C-{}", cfg.prefix.strip_prefix("C-").unwrap_or("")) == c {
                problem(Level::Error, format!("{c} is the prefix"));
                continue;
            }
            if let Some(other) = out.get(&c).filter(|o| *o != name) {
                problem(Level::Error, format!("{c} is given to {other} too; the last one wins"));
            }
            out.insert(c, name.clone());
        }
    }
    out
}

// Every key the config makes, and what's wrong with them: (the table it's in, the problem).
pub fn all_keys(cfg: &Config) -> (Keys, Vec<(String, KeyProblem)>) {
    let mut problems = Vec::new();
    // after the prefix: [keys], each [[command]]'s key, each mode's enter
    let mut prefix = cfg.keys.clone();
    for c in cfg.command.iter().filter(|c| !c.key.is_empty()) {
        prefix.insert(c.name.clone(), Value::from(c.key.clone()));
    }
    for (name, m) in cfg.modes.iter().filter(|(_, m)| !m.enter.is_empty()) {
        prefix.insert(format!("mode:{name}"), Value::from(m.enter.clone()));
    }
    let r = resolve_keys(&prefix, &|n| known(cfg, n));
    problems.extend(r.problems.into_iter().map(|p| ("keys".to_string(), p)));
    let mut root_map = cfg.root_keys.clone();
    for c in cfg.command.iter().filter(|c| !c.root.is_empty()) {
        root_map.insert(c.name.clone(), Value::from(c.root.clone()));
    }
    let root = table_keys("root_keys", &root_map, cfg, true, &mut problems);
    let modes = cfg
        .modes
        .iter()
        .map(|(name, m)| {
            let keys = table_keys(&format!("modes.{name}.keys"), &m.keys, cfg, false, &mut problems);
            (name.clone(), Mode { sticky: m.sticky, timeout: m.timeout, keys })
        })
        .collect();
    for (name, list) in &cfg.actions {
        for a in list.iter().filter(|a| !known(cfg, a)) {
            problems.push(("actions".into(), KeyProblem { level: Level::Warning, action: name.clone(), message: format!("there's no action {a}, so the list skips it") }));
        }
    }
    (Keys { prefix: r.table, more: r.more, root, modes }, problems)
}

// Why a plugin can't have `key` under these bindings (DEFAULT_KEYS when it's no config's), if it can't.
pub fn modisa_key(key: &str, table: &Bindings) -> Option<String> {
    if RESERVED_KEYS.contains(&key) {
        return Some("reserved for getting out of plugin panes".into());
    }
    table.get(key).map(|a| format!("modisa's {a}"))
}

// A plugin key as plugin.json declares it, and as one config binds it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DeclaredKey {
    pub plugin: String,
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<String>,
    pub description: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyState {
    Active,
    Disabled,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoundKey {
    #[serde(flatten)]
    pub declared: DeclaredKey, // its key as bound: the [plugin_keys] remap, if any
    pub state: KeyState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

// Bind plugins' declared keys under one config's [plugin_keys] ("<plugin>.<action or pane>" → key; "" turns it off),
// then turn off a key that config's bindings use or modisa reserves, and one that two plugins (or two of one plugin's)
// want. Every client binds with its own config, so clients attached to one session can differ; the server binds with
// its own, only for what `modisa plugin list` and `plugin check` report.
pub fn bind_plugin_keys(declared: &[DeclaredKey], remaps: &IndexMap<String, String>, table: &Bindings) -> Vec<BoundKey> {
    let wanted: Vec<DeclaredKey> = declared
        .iter()
        .map(|k| {
            let name = format!("{}.{}", k.plugin, k.action.as_deref().or(k.pane.as_deref()).unwrap_or("undefined"));
            DeclaredKey { key: remaps.get(&name).unwrap_or(&k.key).clone(), ..k.clone() }
        })
        .collect();
    let mut by_key: HashMap<&str, Vec<&str>> = HashMap::new();
    for w in wanted.iter().filter(|w| !w.key.is_empty()) {
        by_key.entry(&w.key).or_default().push(&w.plugin);
    }
    wanted
        .iter()
        .map(|w| {
            let all = by_key.get(w.key.as_str()).map(|v| v.as_slice()).unwrap_or(&[]);
            let others: Vec<&str> = all.iter().copied().filter(|p| *p != w.plugin).collect();
            let shared = !others.is_empty() || all.len() > 1;
            let reason = if w.key.is_empty() {
                Some("turned off in [plugin_keys]".to_string())
            } else {
                modisa_key(&w.key, table).or_else(|| {
                    let by = if others.is_empty() { format!("another key of {}", w.plugin) } else { others.join(", ") };
                    shared.then(|| format!("also wanted by {by}"))
                })
            };
            BoundKey { declared: w.clone(), state: if reason.is_some() { KeyState::Disabled } else { KeyState::Active }, reason }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    // Prefix bindings: modisa's defaults, what [keys] changes in them, the keys it can't, and plugin keys bound around them.
    use super::*;
    use serde_json::json;

    fn keys(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }
    fn resolve_keys_plain(keys: &Map<String, Value>) -> ResolvedKeys {
        resolve_keys(keys, &|_| false)
    }
    fn with_keys(v: Value) -> Bindings {
        bindings(&Config { keys: keys(v), ..Config::default() })
    }

    #[test]
    fn with_no_keys_the_bindings_are_the_defaults_and_every_default_runs_a_known_action() {
        assert_eq!(bindings(&Config::default()), *DEFAULT_KEYS);
        assert_eq!(with_keys(json!({})), *DEFAULT_KEYS);
        for action in DEFAULT_KEYS.values() {
            assert!(ACTION_IDS.contains(action));
        }
        assert_eq!(DEFAULT_KEYS.get("5").copied(), Some("agent-5"));
        assert_eq!(DEFAULT_KEYS.len(), 53);
    }

    #[test]
    fn an_action_set_in_keys_has_only_the_keys_given_there_taken_from_whatever_had_them() {
        let t = with_keys(json!({ "zoom": "f", "split-right": ["v", "|"], "split-down": "%" }));
        assert_eq!(t.get("f").copied(), Some("zoom"));
        assert_eq!(t.get("z"), None); // zoom's default went
        assert_eq!(t.get("|").copied(), Some("split-right"));
        assert_eq!(t.get("%").copied(), Some("split-down")); // split-right's default, given to another action
        assert_eq!(t.get("-"), None);
        assert_eq!(t.get("h").copied(), Some("focus-left")); // untouched actions keep theirs
    }

    #[test]
    fn empty_string_or_list_leaves_an_action_with_no_key() {
        for none in [json!(""), json!([])] {
            let t = with_keys(json!({ "help": none }));
            assert!(!t.values().any(|a| *a == "help"));
            assert_eq!(t.get("?"), None);
        }
    }

    #[test]
    fn x_d_and_escape_cant_be_given_away_and_x_and_d_stay_on_close_pane_and_detach() {
        let ResolvedKeys { table, problems, .. } = resolve_keys_plain(&keys(json!({ "zoom": "x", "palette": "escape", "close-pane": "q", "detach": "" })));
        assert_eq!(table.get("x").copied(), Some("close-pane"));
        assert_eq!(table.get("q").copied(), Some("close-pane"));
        assert_eq!(table.get("d").copied(), Some("detach"));
        assert_eq!(table.get("escape"), None);
        assert!(!table.values().any(|a| *a == "zoom")); // asked only for x, which it can't have
        let got: Vec<(&str, Level)> = problems.iter().map(|p| (p.action.as_str(), p.level)).collect();
        assert_eq!(got, [("zoom", Level::Error), ("palette", Level::Error)]);
        assert_eq!(resolve_keys_plain(&keys(json!({ "close-pane": ["x", "q"] }))).problems, []); // its own key, said again
    }

    #[test]
    fn what_keys_gets_wrong_is_reported() {
        let ResolvedKeys { table, problems, .. } = resolve_keys_plain(&keys(json!({ "nope": "q", "zoom": "ctrl-z", "help": "g", "settings": "g", "copy-mode": 3 })));
        let got: Vec<(&str, Level)> = problems.iter().map(|p| (p.action.as_str(), p.level)).collect();
        assert_eq!(got, [("nope", Level::Warning), ("zoom", Level::Error), ("settings", Level::Error), ("copy-mode", Level::Error)]);
        assert!(problems[0].message.contains("no action nope"));
        assert_eq!(problems[1].message, "\"ctrl-z\" isn't a key: one character (H is shift+h), or left, right, up, down, home, end, pageup, pagedown or f1 to f12");
        assert!(problems[2].message.contains("given to help too"));
        assert!(problems[3].message.contains("a key is a string"));
        assert_eq!(table.get("g").copied(), Some("settings")); // the last one wins
        assert_eq!(table.get("[").copied(), Some("copy-mode")); // a value that isn't keys changes nothing
        assert_eq!(resolve_keys_plain(&keys(json!({ "zoom": ["Z", "f5", "pageup", "é"] }))).problems, []);
        assert!(!is_key("\u{feff}") && is_key("\u{85}") && !is_key(" ") && !is_key("ab"));
    }

    #[test]
    fn plugin_keys_are_refused_for_the_keys_these_bindings_use_and_given_the_ones_they_free() {
        assert_eq!(modisa_key("z", &DEFAULT_KEYS).as_deref(), Some("modisa's zoom"));
        assert!(modisa_key("x", &DEFAULT_KEYS).unwrap().contains("reserved"));
        let table = with_keys(json!({ "zoom": "Y" }));
        assert_eq!(modisa_key("z", &table), None);
        assert_eq!(modisa_key("Y", &table).as_deref(), Some("modisa's zoom"));
        let key = |plugin: &str, key: &str, action: &str| DeclaredKey { plugin: plugin.into(), key: key.into(), action: Some(action.into()), pane: None, description: action.into() };
        let declared = [key("p", "z", "a"), key("q", "Y", "b")];
        let states = |b: Vec<BoundKey>| b.into_iter().map(|k| (k.state, k.reason)).collect::<Vec<_>>();
        assert_eq!(states(bind_plugin_keys(&declared, &IndexMap::new(), &DEFAULT_KEYS)), [(KeyState::Disabled, Some("modisa's zoom".into())), (KeyState::Active, None)]);
        assert_eq!(states(bind_plugin_keys(&declared, &IndexMap::new(), &table)), [(KeyState::Active, None), (KeyState::Disabled, Some("modisa's zoom".into()))]);
    }

    #[test]
    fn plugin_keys_remapped_turned_off_or_wanted_twice() {
        let key = |plugin: &str, key: &str, pane: &str| DeclaredKey { plugin: plugin.into(), key: key.into(), action: None, pane: Some(pane.into()), description: String::new() };
        let declared = [key("p", "A", "log"), key("q", "A", "view"), key("r", "B", "one"), key("r", "B", "two"), key("s", "C", "off")];
        let remaps: IndexMap<String, String> = [("s.off".to_string(), String::new())].into();
        let bound = bind_plugin_keys(&declared, &remaps, &DEFAULT_KEYS);
        let reasons: Vec<Option<&str>> = bound.iter().map(|k| k.reason.as_deref()).collect();
        assert_eq!(reasons, [Some("also wanted by q"), Some("also wanted by p"), Some("also wanted by another key of r"), Some("also wanted by another key of r"), Some("turned off in [plugin_keys]")]);
        let json = serde_json::to_value(&bound[4]).unwrap();
        assert_eq!(json, json!({ "plugin": "s", "key": "", "pane": "off", "description": "", "state": "disabled", "reason": "turned off in [plugin_keys]" }));
    }

    fn cfg(v: Value) -> Config {
        crate::config::merge(v.as_object().unwrap())
    }

    #[test]
    fn chords_are_written_one_way() {
        assert_eq!(chord("M-h").as_deref(), Some("M-h"));
        assert_eq!(chord("M-C-left").as_deref(), Some("C-M-left"));
        assert_eq!(chord("S-tab").as_deref(), Some("S-tab"));
        assert_eq!(chord("C--").as_deref(), Some("C--"));
        assert_eq!(chord("f5").as_deref(), Some("f5"));
        for bad in ["S-a", "C-", "M-ab", "ctrl-x", ""] {
            assert_eq!(chord(bad), None, "{bad}");
        }
    }

    #[test]
    fn root_keys_need_a_modifier_and_lists_commands_and_modes_bind_like_actions() {
        let c = cfg(json!({
            "actions": { "dev": ["split-right", "zoom", "nope"] },
            "root_keys": { "focus-left": "M-h", "zoom": "h", "dev": ["f5", "C-b"], "palette": "M-C-p" },
            "modes": { "resize": { "enter": "r", "keys": { "resize-left": "h", "resize-right": ["l", "right"] } }, "once": { "enter": "O", "sticky": false, "timeout": 500, "keys": { "zoom": "z" } } },
            "command": [{ "name": "Tests", "run": "bun test", "key": "T", "root": "M-t" }, { "name": "Theme", "run": "x", "key": "t" }],
            "keys": { "dev": "D" },
        }));
        let (k, problems) = all_keys(&c);
        assert_eq!(k.root.get("M-h").map(String::as_str), Some("focus-left"));
        assert_eq!(k.root.get("f5").map(String::as_str), Some("dev"));
        assert_eq!(k.root.get("C-M-p").map(String::as_str), Some("palette"));
        assert_eq!(k.root.get("M-t").map(String::as_str), Some("Tests"));
        assert!(!k.root.contains_key("h") && !k.root.contains_key("C-b"));
        assert_eq!(k.more.get("D").map(String::as_str), Some("dev"));
        assert_eq!(k.more.get("T").map(String::as_str), Some("Tests"));
        assert_eq!(k.more.get("t").map(String::as_str), Some("Theme"));
        assert_eq!(k.prefix.get("t"), None); // the command took the theme picker's key
        assert_eq!(k.more.get("r").map(String::as_str), Some("mode:resize"));
        assert_eq!(k.modes["resize"].keys.get("right").map(String::as_str), Some("resize-right"));
        assert!(k.modes["resize"].sticky && !k.modes["once"].sticky && k.modes["once"].timeout == 500);
        let got: Vec<(&str, &str, Level)> = problems.iter().map(|(t, p)| (t.as_str(), p.action.as_str(), p.level)).collect();
        assert_eq!(got, [("root_keys", "zoom", Level::Error), ("root_keys", "dev", Level::Error), ("actions", "dev", Level::Warning)]);
        assert!(problems[0].1.message.contains("C- or M-") && problems[1].1.message.contains("the prefix"));
    }

    #[test]
    fn what_a_key_can_run() {
        let c = cfg(json!({ "actions": { "dev": ["zoom"] }, "modes": { "resize": { "enter": "r" } }, "command": [{ "name": "Tests", "run": "x" }] }));
        for ok in ["zoom", "dev", "Tests", "mode:resize", "plugin:radar.order", "sh:make"] {
            assert!(known(&c, ok), "{ok}");
        }
        for bad in ["nope", "mode:other", "plugin:radar", "plugin:.x", "sh: "] {
            assert!(!known(&c, bad), "{bad}");
        }
    }
}
