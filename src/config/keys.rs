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
    pub problems: Vec<KeyProblem>,
}

// The bindings [keys] makes ("<action>" = "K", or a list of keys; "" or [] for none), and what in it can't be bound. An
// action set there has only the keys given, each taken from whatever had it by default.
pub fn resolve_keys(keys: &Map<String, Value>) -> ResolvedKeys {
    let mut table = DEFAULT_KEYS.clone();
    let mut problems = Vec::new();
    let mut given: HashMap<String, ActionId> = HashMap::new(); // key → the action [keys] gave it
    for (name, value) in keys {
        let Some(action) = action_id(name) else {
            let message = format!("there's no action {name} (the keyboard guide, prefix ?, lists them)");
            problems.push(KeyProblem { level: Level::Warning, action: name.clone(), message });
            continue;
        };
        let wanted: Vec<&Value> = match value {
            Value::Array(items) => items.iter().collect(),
            v => vec![v],
        };
        let Some(wanted) = wanted.iter().map(|k| k.as_str()).collect::<Option<Vec<&str>>>() else {
            problems.push(KeyProblem { level: Level::Error, action: name.clone(), message: "a key is a string (\"K\"), or a list of them".into() });
            continue;
        };
        table.retain(|key, a| *a != action || kept(key).is_some());
        for key in wanted.into_iter().filter(|k| !k.is_empty()) {
            let mut error = |message: String| problems.push(KeyProblem { level: Level::Error, action: name.clone(), message });
            if RESERVED_KEYS.contains(&key) && kept(key) != Some(action) {
                error(format!("{key} is reserved: it's how you get out of anything a plugin opens"));
                continue;
            }
            if !is_key(key) {
                let named = NAMED[..8].join(", ");
                error(format!("{} isn't a key: one character (H is shift+h), or {named} or f1 to f12", Value::from(key)));
                continue;
            }
            if let Some(other) = given.get(key) {
                if *other != action {
                    error(format!("{key} is given to {other} too; the last one wins"));
                }
            }
            given.insert(key.to_string(), action);
            table.insert(key.to_string(), action);
        }
    }
    ResolvedKeys { table, problems }
}

pub fn bindings(cfg: &Config) -> Bindings {
    resolve_keys(&cfg.keys).table
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
        let ResolvedKeys { table, problems } = resolve_keys(&keys(json!({ "zoom": "x", "palette": "escape", "close-pane": "q", "detach": "" })));
        assert_eq!(table.get("x").copied(), Some("close-pane"));
        assert_eq!(table.get("q").copied(), Some("close-pane"));
        assert_eq!(table.get("d").copied(), Some("detach"));
        assert_eq!(table.get("escape"), None);
        assert!(!table.values().any(|a| *a == "zoom")); // asked only for x, which it can't have
        let got: Vec<(&str, Level)> = problems.iter().map(|p| (p.action.as_str(), p.level)).collect();
        assert_eq!(got, [("zoom", Level::Error), ("palette", Level::Error)]);
        assert_eq!(resolve_keys(&keys(json!({ "close-pane": ["x", "q"] }))).problems, []); // its own key, said again
    }

    #[test]
    fn what_keys_gets_wrong_is_reported() {
        let ResolvedKeys { table, problems } = resolve_keys(&keys(json!({ "nope": "q", "zoom": "ctrl-z", "help": "g", "settings": "g", "copy-mode": 3 })));
        let got: Vec<(&str, Level)> = problems.iter().map(|p| (p.action.as_str(), p.level)).collect();
        assert_eq!(got, [("nope", Level::Warning), ("zoom", Level::Error), ("settings", Level::Error), ("copy-mode", Level::Error)]);
        assert!(problems[0].message.contains("no action nope"));
        assert_eq!(problems[1].message, "\"ctrl-z\" isn't a key: one character (H is shift+h), or left, right, up, down, home, end, pageup, pagedown or f1 to f12");
        assert!(problems[2].message.contains("given to help too"));
        assert!(problems[3].message.contains("a key is a string"));
        assert_eq!(table.get("g").copied(), Some("settings")); // the last one wins
        assert_eq!(table.get("[").copied(), Some("copy-mode")); // a value that isn't keys changes nothing
        assert_eq!(resolve_keys(&keys(json!({ "zoom": ["Z", "f5", "pageup", "é"] }))).problems, []);
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
}
