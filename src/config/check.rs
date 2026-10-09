// `modisa config check`: config.toml read the way modisa reads it, with no session needed. What modisa can't parse or
// would misread is an error; a setting it doesn't know (a typo, or one from another version) is a warning, since
// modisa ignores it.
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::keys::all_keys;
use super::{find_key, one_js_char, parse_toml, themes, SOUND_NAMES};
use crate::protocol::types::{REPLACEABLE, SLOTS};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Error,
    Warning,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ConfigProblem {
    pub level: Level,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>, // the setting's dotted path; none: the file doesn't parse
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>, // from 1
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<usize>,
}

// The "config check" result (protocol schema.ts, cliResults). ok: no errors (warnings are settings modisa ignores).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CheckReport {
    pub file: String,
    pub exists: bool,
    pub ok: bool,
    pub problems: Vec<ConfigProblem>,
}

// The config file at `path` checked; no file is fine (modisa uses its defaults).
// ponytail: a file that exists but can't be read is one error here; the TS threw.
pub fn check(path: &str) -> CheckReport {
    let (exists, problems) = match std::fs::read_to_string(path) {
        Ok(text) => (true, check_config(&text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (false, vec![]),
        Err(e) => (true, vec![ConfigProblem { level: Level::Error, key: None, message: e.to_string(), line: None, column: None }]),
    };
    let ok = !problems.iter().any(|p| p.level == Level::Error);
    CheckReport { file: path.to_string(), exists, ok, problems }
}

// What a setting may be: the zod schemas of the TS, reduced to what config.toml uses, with zod's messages.
enum Schema {
    Bool,
    Str { min: usize },
    Prefix, // C- and one key
    Theme,  // a built-in theme's name
    Num { int: bool, min: Option<Bound>, max: Option<Bound> },
    Enum(&'static [&'static str]),
    Sound, // one of SOUND_NAMES, which its message lists
    Array(&'static Schema),
    Object { fields: &'static [(&'static str, Schema)], partial: bool }, // strict: other keys are warnings
    Open { fields: &'static [(&'static str, Schema)], rest: &'static Schema }, // its fields, and any other key is a `rest`
    Record(&'static Schema),
    Keys, // one key, or a list of them
    Unknown,
}

struct Bound {
    value: f64,
    inclusive: bool,
    message: Option<&'static str>,
}

const BOOL: Schema = Schema::Bool;
const STR: Schema = Schema::Str { min: 0 };
const KINDS: Schema = Schema::Array(&Schema::Enum(&["toast", "system", "sound", "bell"]));
const SOUND: Schema = Schema::Sound;
const POLICY: Schema = Schema::Enum(&["allow", "ask", "deny"]);
const AT_LEAST: Bound = Bound { value: 0.0, inclusive: true, message: None };
const WIDTH: Bound = Bound { value: 20.0, inclusive: true, message: Some("20 to 48 columns") };
const POSITIVE: Schema = Schema::Num { int: true, min: Some(Bound { value: 0.0, inclusive: false, message: None }), max: None };

const fn table(fields: &'static [(&'static str, Schema)]) -> Schema {
    Schema::Object { fields, partial: true }
}

const AGENT_NOTIFY: Schema = table(&[("blocked", KINDS), ("done", KINDS), ("working", KINDS)]);
const AGENT_SOUND: Schema = table(&[("blocked", STR), ("done", STR), ("working", STR)]); // a sound's name or a file

// What each top-level setting may be: one schema per table.
const SETTINGS: &[(&str, Schema)] = &[
    ("prefix", Schema::Prefix),
    ("theme", Schema::Theme),
    ("remote_command", Schema::Str { min: 1 }),
    ("mouse", table(&[("hover", BOOL)])),
    (
        "sidebar",
        table(&[
            ("visible", BOOL),
            ("width", Schema::Num { int: true, min: Some(WIDTH), max: Some(Bound { value: 48.0, ..WIDTH }) }),
            ("agents", STR),
            ("logos", Schema::Enum(&["auto", "on", "off"])),
            ("graph", BOOL),
            ("git", BOOL),
            ("position", Schema::Enum(&["left", "right"])),
            ("sections", Schema::Array(&Schema::Str { min: 1 })),
            ("row", Schema::Array(&STR)),
            ("sort", Schema::Enum(&["attention", "name", "created"])),
            ("show", Schema::Enum(&["space", "tab", "all"])),
        ]),
    ),
    ("status", table(&[("agents", BOOL), ("panes", BOOL), ("theme", BOOL), ("left", STR), ("right", STR)])),
    ("git", table(&[("status", BOOL), ("repo", BOOL), ("counts", BOOL), ("changes", BOOL)])),
    (
        "panes",
        table(&[
            ("border", Schema::Enum(crate::config::BorderStyle::NAMES)),
            ("title", STR),
            ("title_position", Schema::Enum(&["top_left", "top_center", "top_right", "bottom_left", "bottom_center", "bottom_right"])),
            ("dim_unfocused", Schema::Num { int: false, min: Some(AT_LEAST), max: Some(Bound { value: 0.8, ..AT_LEAST }) }),
        ]),
    ),
    ("tabs", table(&[("position", Schema::Enum(&["top", "bottom", "hidden"])), ("format", STR)])),
    ("window", table(&[("title", STR)])),
    (
        "notify",
        Schema::Open { fields: &[("blocked", KINDS), ("done", KINDS), ("working", KINDS), ("unread", BOOL), ("click", Schema::Enum(&["focus", "none"]))], rest: &AGENT_NOTIFY },
    ),
    (
        "sound",
        Schema::Open {
            fields: &[
                ("volume", Schema::Num { int: false, min: Some(Bound { value: 0.0, ..AT_LEAST }), max: Some(Bound { value: 1.0, ..AT_LEAST }) }),
                ("blocked", SOUND),
                ("done", SOUND),
                ("working", SOUND),
                ("pack", STR),
            ],
            rest: &AGENT_SOUND,
        },
    ),
    ("indicators", table(&[("style", Schema::Enum(&["symbols", "dots", "letters"])), ("tab", BOOL), ("pane", BOOL), ("sidebar", BOOL)])),
    ("pane_labels", table(&[("agent", BOOL)])),
    ("update", table(&[("check", BOOL), ("channel", Schema::Enum(&["stable", "staging"]))])),
    ("messaging", table(&[("max_hops", POSITIVE), ("per_minute", POSITIVE)])),
    ("permissions", table(&[("keys_foreign", POLICY), ("close_foreign", POLICY), ("run_foreign", POLICY)])),
    ("agents", Schema::Record(&Schema::Record(&Schema::Unknown))), // per agent, over its adapter's fields
    ("plugin", Schema::Array(&Schema::Object { fields: &[("run", Schema::Str { min: 1 })], partial: false })),
    ("plugin_keys", Schema::Record(&STR)),
    ("keys", Schema::Record(&Schema::Keys)),
    ("actions", Schema::Record(&Schema::Array(&Schema::Str { min: 1 }))),
    ("root_keys", Schema::Record(&Schema::Keys)),
    (
        "modes",
        Schema::Record(&Schema::Object {
            fields: &[("enter", Schema::Str { min: 1 }), ("sticky", BOOL), ("timeout", Schema::Num { int: true, min: Some(AT_LEAST), max: None }), ("keys", Schema::Record(&Schema::Keys))],
            partial: true,
        }),
    ),
    (
        "hook",
        Schema::Array(&Schema::Object {
            fields: &[("on", Schema::Str { min: 1 }), ("when", STR), ("run", Schema::Str { min: 1 }), ("timeout", Schema::Num { int: true, min: Some(AT_LEAST), max: None })],
            partial: true,
        }),
    ),
    (
        "command",
        Schema::Array(&Schema::Object {
            fields: &[
                ("name", Schema::Str { min: 1 }),
                ("run", Schema::Str { min: 1 }),
                ("key", STR),
                ("root", STR),
                ("in", Schema::Enum(&["split", "split-down", "tab", "zoomed", "background"])),
                ("cwd", STR),
                (
                    "prompts",
                    Schema::Array(&Schema::Object { fields: &[("name", Schema::Str { min: 1 }), ("title", STR), ("default", STR), ("pick", STR)], partial: true }),
                ),
            ],
            partial: true,
        }),
    ),
    ("slots", Schema::Record(&STR)), // who draws each slot (examples/plugins/CHROME.md)
];

const UNKNOWN: &str = "modisa has no such setting, so it's ignored";

#[derive(Clone)]
enum Seg {
    Key(String),
    Index(usize),
}

struct Issue {
    level: Level,
    path: Vec<Seg>,
    message: String,
}

// zod's name for what it got.
fn received(v: Option<&Value>) -> &'static str {
    match v {
        None => "undefined",
        Some(Value::Null) => "null",
        Some(Value::Bool(_)) => "boolean",
        Some(Value::Number(_)) => "number",
        Some(Value::String(_)) => "string",
        Some(Value::Array(_)) => "array",
        Some(Value::Object(_)) => "object",
    }
}

fn one_of(v: Option<&Value>, options: &[&str]) -> bool {
    v.and_then(Value::as_str).is_some_and(|s| options.contains(&s))
}

// Every issue `value` has under `schema`, depth first, in the order zod finds them.
fn validate(schema: &Schema, value: Option<&Value>, path: &mut Vec<Seg>, out: &mut Vec<Issue>) {
    let mut error = |message: String| out.push(Issue { level: Level::Error, path: path.clone(), message });
    let expected = |what: &str| format!("Invalid input: expected {what}, received {}", received(value));
    match schema {
        Schema::Unknown => {}
        Schema::Bool => {
            if !matches!(value, Some(Value::Bool(_))) {
                error(expected("boolean"));
            }
        }
        Schema::Theme if matches!(value, Some(Value::Object(_))) => {
            // { dark = "name", light = "name" }: which theme for a dark terminal and which for a light one
            for (k, v) in value.and_then(Value::as_object).into_iter().flatten() {
                path.push(Seg::Key(k.clone()));
                if k == "dark" || k == "light" {
                    validate(&Schema::Theme, Some(v), path, out);
                } else {
                    out.push(Issue { level: Level::Warning, path: path.clone(), message: "a theme pair has dark and light".into() });
                }
                path.pop();
            }
        }
        Schema::Str { .. } | Schema::Prefix | Schema::Theme => {
            let Some(Value::String(s)) = value else { return error(expected("string")) };
            match schema {
                Schema::Str { min } if s.encode_utf16().count() < *min => error(format!("Too small: expected string to have >={min} characters")),
                Schema::Prefix if !(s.starts_with("C-") && one_js_char(&s[2..]).is_some()) => error("C- and one key, like \"C-b\"".into()),
                Schema::Theme if themes::find_theme(s).is_none() && !std::path::Path::new(&format!("{}/{s}.toml", themes::themes_dir())).exists() => {
                    error(format!("there's no theme {} (the settings page lists them)", Value::from(s.as_str())))
                }
                _ => {}
            }
        }
        Schema::Num { int, min, max } => {
            let Some(n) = value.and_then(Value::as_f64) else { return error(expected("number")) };
            if *int && n.fract() != 0.0 {
                return error("Invalid input: expected int, received number".into());
            }
            let say = |b: &Bound, word: &str, op: &str| match b.message {
                Some(m) => m.to_string(),
                None => format!("{word}: expected number to be {op}{}{}", if b.inclusive { "=" } else { "" }, b.value),
            };
            if let Some(b) = min.as_ref().filter(|b| if b.inclusive { n < b.value } else { n <= b.value }) {
                error(say(b, "Too small", ">"));
            }
            if let Some(b) = max.as_ref().filter(|b| if b.inclusive { n > b.value } else { n >= b.value }) {
                error(say(b, "Too big", "<"));
            }
        }
        Schema::Enum(options) => {
            if !one_of(value, options) {
                error(format!("Invalid option: expected one of {}", options.iter().map(|o| format!("\"{o}\"")).collect::<Vec<_>>().join("|")));
            }
        }
        Schema::Sound => {
            if !one_of(value, SOUND_NAMES) {
                error(format!("a sound: {}", SOUND_NAMES.join(", ")));
            }
        }
        Schema::Keys => {
            let keys = match value {
                Some(Value::String(_)) => true,
                Some(Value::Array(items)) => items.iter().all(Value::is_string),
                _ => false,
            };
            if !keys {
                error("Invalid input".into());
            }
        }
        Schema::Array(item) => {
            let Some(Value::Array(items)) = value else { return error(expected("array")) };
            for (i, v) in items.iter().enumerate() {
                path.push(Seg::Index(i));
                validate(item, Some(v), path, out);
                path.pop();
            }
        }
        Schema::Record(item) => {
            let Some(Value::Object(m)) = value else { return error(expected("record")) };
            for (k, v) in m {
                path.push(Seg::Key(k.clone()));
                validate(item, Some(v), path, out);
                path.pop();
            }
        }
        Schema::Open { fields, rest } => {
            let Some(Value::Object(m)) = value else { return error(expected("object")) };
            for (k, v) in m {
                path.push(Seg::Key(k.clone()));
                let field = fields.iter().find(|(name, _)| *name == k.as_str()).map_or(*rest, |(_, f)| f);
                validate(field, Some(v), path, out);
                path.pop();
            }
        }
        Schema::Object { fields, partial } => {
            let Some(Value::Object(m)) = value else { return error(expected("object")) };
            for (name, field) in fields.iter() {
                if m.contains_key(*name) || !partial {
                    path.push(Seg::Key(name.to_string()));
                    validate(field, m.get(*name), path, out);
                    path.pop();
                }
            }
            for k in m.keys().filter(|k| !fields.iter().any(|(name, _)| *name == k.as_str())) {
                let mut at = path.clone();
                at.push(Seg::Key(k.clone()));
                out.push(Issue { level: Level::Warning, path: at, message: UNKNOWN.into() });
            }
        }
    }
}

pub fn check_config(source: &str) -> Vec<ConfigProblem> {
    let user = match parse_toml(source) {
        Ok(user) => user,
        Err(e) => return vec![ConfigProblem { level: Level::Error, key: None, message: e.message, line: e.line, column: e.column }],
    };
    let lines: Vec<&str> = source.split('\n').collect();
    // the line a setting is on, else the header of the nearest table around it, else nothing
    let line_of = |path: &[Seg]| -> Option<usize> {
        let keys: Vec<&str> = path.iter().filter_map(|s| if let Seg::Key(k) = s { Some(k.as_str()) } else { None }).collect();
        for n in (1..=keys.len()).rev() {
            let table = (n > 1).then(|| keys[..n - 1].join("."));
            if let Some(at) = find_key(&lines, table.as_deref(), Some(keys[n - 1])).and_then(|t| t.at) {
                return Some(at + 1);
            }
            if let Some(t) = find_key(&lines, Some(&keys[..n].join(".")), None) {
                return Some(t.start);
            }
        }
        None
    };
    let mut problems: Vec<ConfigProblem> = Vec::new();
    let mut add = |level: Level, path: &[Seg], message: String| {
        let key = path.iter().map(|s| match s { Seg::Key(k) => k.clone(), Seg::Index(i) => i.to_string() }).collect::<Vec<_>>().join(".");
        problems.push(ConfigProblem { level, key: Some(key), message, line: line_of(path), column: None });
    };
    for (name, value) in &user {
        let Some((_, schema)) = SETTINGS.iter().find(|(n, _)| *n == name.as_str()) else {
            add(Level::Warning, &[Seg::Key(name.clone())], UNKNOWN.into());
            continue;
        };
        let mut issues = Vec::new();
        validate(schema, Some(value), &mut vec![Seg::Key(name.clone())], &mut issues);
        for issue in issues {
            add(issue.level, &issue.path, issue.message);
        }
    }
    for (i, h) in user.get("hook").and_then(Value::as_array).into_iter().flatten().enumerate() {
        if let Some(problem) = h.get("when").and_then(Value::as_str).and_then(crate::server::hooks::check_when) {
            add(Level::Error, &[Seg::Key("hook".into()), Seg::Index(i), Seg::Key("when".into())], problem);
        }
    }
    // [notify.<id>] and [sound.<id>] are per agent: one modisa knows, or one [agents.<id>] adds
    let agents: Vec<String> = crate::config::agents::builtin_agents().into_iter().map(|a| a.id).chain(user.get("agents").and_then(Value::as_object).into_iter().flat_map(|m| m.keys().cloned())).collect();
    for (table, own) in [("notify", &["blocked", "done", "working", "unread", "click"][..]), ("sound", &["volume", "blocked", "done", "working", "pack"][..])] {
        for id in user.get(table).and_then(Value::as_object).into_iter().flat_map(|m| m.keys()).filter(|k| !own.contains(&k.as_str()) && !agents.contains(k)) {
            add(Level::Warning, &[Seg::Key(table.into()), Seg::Key(id.clone())], format!("there's no agent {id}, so this is ignored"));
        }
    }
    if user.get("sidebar").and_then(|s| s.get("git")).is_some() {
        add(Level::Warning, &[Seg::Key("sidebar".into()), Seg::Key("git".into())], "this moved: [git] status = false turns the branch off".into());
    }
    // the user's theme files, read as the TUI reads them
    for (name, problem) in themes::load_custom() {
        add(Level::Error, &[Seg::Key("themes".into()), Seg::Key(name)], problem);
    }
    // keys after the prefix, without it, and in modes, as modisa binds them (lists, commands and modes included)
    for (table, p) in all_keys(&super::merge(&user)).1 {
        let mut at: Vec<Seg> = table.split('.').map(|k| Seg::Key(k.to_string())).collect();
        at.push(Seg::Key(p.action));
        add(p.level, &at, p.message);
    }
    // a slot no plugin draws instead of modisa in is ignored
    for slot in user.get("slots").and_then(Value::as_object).into_iter().flat_map(|m| m.keys()) {
        let why = if !SLOTS.contains(&slot.as_str()) {
            format!("there's no slot {slot} (examples/plugins/CHROME.md lists them)")
        } else if !REPLACEABLE.contains(&slot.as_str()) {
            format!("plugins only add to {slot}: nothing replaces it")
        } else {
            continue;
        };
        add(Level::Warning, &[Seg::Key("slots".into()), Seg::Key(slot.clone())], why);
    }
    problems.sort_by_key(|p| p.line.unwrap_or(0)); // stable: same-line problems keep their order
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SAMPLE;
    use serde_json::json;

    fn problem(level: Level, key: &str, message: &str, line: Option<usize>) -> ConfigProblem {
        ConfigProblem { level, key: Some(key.into()), message: message.into(), line, column: None }
    }
    use Level::{Error as E, Warning as W};

    #[test]
    fn the_sample_and_an_empty_file_are_fine_mistakes_are_errors_with_their_line_unknown_settings_warnings() {
        assert_eq!(check_config(SAMPLE), []);
        assert_eq!(check_config(""), []);
        let problems = check_config("prefix = \"Ctrl-b\"\ntheme = \"neon\"\nshiny = true\n\n[sidebar]\nwidth = 60\ngit = false\n\n[notify]\nblocked = [\"toast\", \"pager\"]\n\n[sound]\ndone = \"boom\"\n\n[permissions]\nkeys_foreign = \"maybe\"\n\n[update]\nchannel = \"nightly\"\n\n[keys]\nzoom = \"x\"\nbogus = \"q\"\n");
        let sounds = "a sound: chime, sparkle, droplet, bloom, whisper, tick, press, release, toggle, success, error, page, loading, ready, pulse, scan, arrival";
        // exactly what the TS reports, in file order
        assert_eq!(
            problems,
            [
                problem(E, "prefix", "C- and one key, like \"C-b\"", Some(1)),
                problem(E, "theme", "there's no theme \"neon\" (the settings page lists them)", Some(2)),
                problem(W, "shiny", UNKNOWN, Some(3)),
                problem(E, "sidebar.width", "20 to 48 columns", Some(6)),
                problem(W, "sidebar.git", "this moved: [git] status = false turns the branch off", Some(7)),
                problem(E, "notify.blocked.1", "Invalid option: expected one of \"toast\"|\"system\"|\"sound\"|\"bell\"", Some(10)),
                problem(E, "sound.done", sounds, Some(13)),
                problem(E, "permissions.keys_foreign", "Invalid option: expected one of \"allow\"|\"ask\"|\"deny\"", Some(16)),
                problem(E, "update.channel", "Invalid option: expected one of \"stable\"|\"staging\"", Some(19)),
                problem(E, "keys.zoom", "x is reserved: it's how you get out of anything a plugin opens", Some(22)),
                problem(W, "keys.bogus", "there's no action bogus (the keyboard guide, prefix ?, lists them)", Some(23)),
            ]
        );
    }

    #[test]
    fn a_file_that_doesnt_parse_is_one_error_at_its_line_and_column() {
        let problems = check_config("theme = \"ion\"\n[sidebar]\nwidth = [1, 2\n");
        assert_eq!(problems.len(), 1);
        assert_eq!((problems[0].level, problems[0].key.as_deref(), problems[0].line, problems[0].column), (E, None, Some(3), Some(14)));
        assert!(!problems[0].message.is_empty());
    }

    // Each against what the TS's checkConfig says for it (run under Bun).
    #[test]
    fn zods_messages_and_where_they_point() {
        let cases: &[(&str, Value)] = &[
            ("prefix = 5\ntheme = 3\nremote_command = \"\"\n", json!([["error", "prefix", "Invalid input: expected string, received number", 1], ["error", "theme", "Invalid input: expected string, received number", 2], ["error", "remote_command", "Too small: expected string to have >=1 characters", 3]])),
            ("[sidebar]\nwidth = 60.5\nvisible = \"yes\"\nlogos = \"maybe\"\nagents = 3\ngraph = 1\nfoo = 1\n", json!([["error", "sidebar.width", "Invalid input: expected int, received number", 2], ["error", "sidebar.visible", "Invalid input: expected boolean, received string", 3], ["error", "sidebar.logos", "Invalid option: expected one of \"auto\"|\"on\"|\"off\"", 4], ["error", "sidebar.agents", "Invalid input: expected string, received number", 5], ["error", "sidebar.graph", "Invalid input: expected boolean, received number", 6], ["warning", "sidebar.foo", UNKNOWN, 7]])),
            ("[sidebar]\nwidth = 19\n", json!([["error", "sidebar.width", "20 to 48 columns", 2]])),
            ("[sidebar]\nwidth = 20.0\n", json!([])),
            ("sidebar = [1]\n", json!([["error", "sidebar", "Invalid input: expected object, received array", 1]])),
            ("[sound]\nvolume = 2\n", json!([["error", "sound.volume", "Too big: expected number to be <=1", 2]])),
            ("[sound]\nvolume = -1\n", json!([["error", "sound.volume", "Too small: expected number to be >=0", 2]])),
            ("[sound]\nvolume = nan\n", json!([["error", "sound.volume", "Invalid input: expected number, received string", 2]])),
            ("[messaging]\nmax_hops = 0\nper_minute = 1.5\n", json!([["error", "messaging.max_hops", "Too small: expected number to be >0", 2], ["error", "messaging.per_minute", "Invalid input: expected int, received number", 3]])),
            ("[messaging]\nmax_hops = -1.5\n", json!([["error", "messaging.max_hops", "Invalid input: expected int, received number", 2]])),
            ("[notify]\nblocked = \"toast\"\ndone = [1]\n", json!([["error", "notify.blocked", "Invalid input: expected array, received string", 2], ["error", "notify.done.0", "Invalid option: expected one of \"toast\"|\"system\"|\"sound\"|\"bell\"", 3]])),
            ("[panes]\nborder = 3\n", json!([["error", "panes.border", format!("Invalid option: expected one of {}", crate::config::BorderStyle::NAMES.iter().map(|n| format!("\"{n}\"")).collect::<Vec<_>>().join("|")), 2]])),
            ("[agents]\nclaude = \"x\"\n", json!([["error", "agents.claude", "Invalid input: expected record, received string", 2]])),
            ("[agents.claude]\nname = 3\n[agents.codex]\nlaunch = \"x\"\n", json!([])),
            ("[[plugin]]\nrun = \"\"\n[[plugin]]\nfoo = 1\n", json!([["error", "plugin.0.run", "Too small: expected string to have >=1 characters", null], ["error", "plugin.1.run", "Invalid input: expected string, received undefined", null], ["warning", "plugin.1.foo", UNKNOWN, null]])),
            ("plugin = [1]\n", json!([["error", "plugin.0", "Invalid input: expected object, received number", 1]])),
            ("[plugin_keys]\n\"a.b\" = 3\n", json!([["error", "plugin_keys.a.b", "Invalid input: expected string, received number", 2]])),
            ("[slots]\n\"agent.row\" = \"radar\"\n\"pane.title\" = \"builtin\"\n", json!([])),
            ("[slots]\n\"agent.row\" = 3\n\"status.left\" = \"x\"\n\"nope\" = \"x\"\n", json!([["error", "slots.agent.row", "Invalid input: expected string, received number", 2], ["warning", "slots.status.left", "plugins only add to status.left: nothing replaces it", 3], ["warning", "slots.nope", "there's no slot nope (examples/plugins/CHROME.md lists them)", 4]])),
            ("keys = \"x\"\n", json!([["error", "keys", "Invalid input: expected record, received string", 1]])),
            ("[keys]\nzoom = [\"a\", 3]\n", json!([["error", "keys.zoom", "Invalid input", 2], ["error", "keys.zoom", "a key is a string (\"K\"), or a list of them", 2]])),
            ("[keys]\nzoom = \"ctrl-z\"\nhelp = \"g\"\nsettings = \"g\"\nnope = \"q\"\n", json!([["error", "keys.zoom", "\"ctrl-z\" isn't a key: one character (H is shift+h), or left, right, up, down, home, end, pageup, pagedown or f1 to f12", 2], ["error", "keys.settings", "g is given to help too; the last one wins", 4], ["warning", "keys.nope", "there's no action nope (the keyboard guide, prefix ?, lists them)", 5]])),
            ("[notify.extra]\nx = 1\n", json!([["warning", "notify.extra", "there's no agent extra, so this is ignored", 1], ["warning", "notify.extra.x", UNKNOWN, 2]])),
            ("[notify.codex]\ndone = []\n[sound.claude-code]\ndone = \"~/ding.wav\"\n", json!([])),
            ("sidebar = { width = 60, visible = \"x\", zz = 1 }\n", json!([["error", "sidebar.visible", "Invalid input: expected boolean, received string", 1], ["error", "sidebar.width", "20 to 48 columns", 1], ["warning", "sidebar.zz", UNKNOWN, 1]])),
            ("[sidebar]\nwidth = 30\n[sidebar.deep]\na = 1\n", json!([["warning", "sidebar.deep", UNKNOWN, 3]])),
            ("width = 1\n[sidebar]\nwidth=1\n", json!([["warning", "width", UNKNOWN, 1], ["error", "sidebar.width", "20 to 48 columns", 3]])),
            ("prefix = \"C-😀\"\n", json!([["error", "prefix", "C- and one key, like \"C-b\"", 1]])),
            ("prefix = \"C-é\"\nremote_command = \"  \"\ntheme = \"bearded-oled\"\n", json!([])),
            ("theme = \"\"\n", json!([["error", "theme", "there's no theme \"\" (the settings page lists them)", 1]])),
        ];
        for (source, want) in cases {
            let got: Vec<Value> = check_config(source).into_iter().map(|p| json!([p.level, p.key, p.message, p.line])).collect();
            assert_eq!(Value::Array(got), *want, "for {source:?}");
        }
    }

    #[test]
    fn the_report_has_the_cli_shape() {
        let dir = crate::config::tests::scratch("check");
        let path = dir.join("config.toml");
        let path = path.to_str().unwrap();
        assert_eq!(serde_json::to_value(check(path)).unwrap(), json!({ "file": path, "exists": false, "ok": true, "problems": [] }));
        std::fs::write(path, "shiny = true\n[sidebar\n").unwrap();
        let report = serde_json::to_value(check(path)).unwrap();
        assert_eq!((&report["exists"], &report["ok"], report["problems"][0].as_object().unwrap().keys().map(String::as_str).collect::<Vec<_>>()), (&json!(true), &json!(false), vec!["level", "message", "line", "column"]));
        std::fs::write(path, "shiny = true\n").unwrap();
        assert_eq!(serde_json::to_value(check(path)).unwrap(), json!({ "file": path, "exists": true, "ok": true, "problems": [{ "level": "warning", "key": "shiny", "message": UNKNOWN, "line": 1 }] }));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
