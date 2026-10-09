// Editing agents' own config files: add modisa's hook entries in each agent's native shape, and
// remove only ours again. Ours are recognised by the MODISA_HOOK=<version> marker in the command.
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{json, Map, Value};

pub const MARK: &str = "MODISA_HOOK=";
const COMMAND_KEYS: [&str; 3] = ["command", "bash", "powershell"];

// An object mapping event names to hook arrays.
pub type Hooks = Map<String, Value>;
pub type Res<T = ()> = Result<T, String>;

fn is_ours(value: Option<&Value>) -> bool {
    value.and_then(Value::as_str).is_some_and(|s| s.contains(MARK))
}
fn entry_ours(entry: &Value) -> bool {
    COMMAND_KEYS.iter().any(|k| is_ours(entry.get(k)))
}

// A file's text, or "" when there is none.
pub fn read_text(path: &str) -> Res<String> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(String::from_utf8_lossy(&bytes).into_owned()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!("{path}: {e}")),
    }
}

// Like Bun.write: the directories it goes in are made first.
pub fn write(path: &str, text: &str) -> Res {
    if let Some(dir) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(path, text).map_err(|e| format!("{path}: {e}"))
}

pub fn read_json(path: &str) -> Res<Map<String, Value>> {
    let text = read_text(path)?;
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str(&text) {
        Ok(Value::Object(value)) => Ok(value),
        _ => Err(format!("{path} isn't a JSON object; fix it by hand, then retry")),
    }
}

pub fn write_json(path: &str, value: &Map<String, Value>) -> Res {
    let mut value = Value::Object(value.clone());
    js_numbers(&mut value);
    write(path, &(serde_json::to_string_pretty(&value).unwrap_or_default() + "\n"))
}

// As JSON.stringify writes them: a whole number without a fraction (1e3 → 1000, 2.0 → 2), so the
// user's other settings come back as the original wrote them.
fn js_numbers(v: &mut Value) {
    match v {
        Value::Number(n) => {
            if let Some(f) = n.as_f64().filter(|f| n.is_f64() && f.fract() == 0.0 && f.abs() < 9007199254740992.0) {
                *v = (f as i64).into();
            }
        }
        Value::Array(items) => items.iter_mut().for_each(js_numbers),
        Value::Object(fields) => fields.values_mut().for_each(js_numbers),
        _ => {}
    }
}

// The object mapping event names to hook arrays, created if missing. (null is refused here too: the
// original let it through and failed a step later.)
pub fn hooks_of<'a>(root: &'a mut Map<String, Value>, key: &str) -> Res<&'a mut Hooks> {
    root.entry(key).or_insert_with(|| json!({})).as_object_mut().ok_or_else(|| format!("\"{key}\" must be an object"))
}

// An event's hook list, created if missing.
pub fn list<'a>(hooks: &'a mut Hooks, event: &str) -> Res<&'a mut Vec<Value>> {
    let entries = hooks.entry(event).or_insert(Value::Null);
    if entries.is_null() {
        *entries = json!([]);
    }
    entries.as_array_mut().ok_or_else(|| format!("hooks for {event} must be a list"))
}

// Claude-style group: { matcher?, hooks: [{ type: "command", command, timeout }] }
pub fn add_nested(hooks: &mut Hooks, event: &str, command: &str, matcher: Option<&str>, timeout: Option<u64>) -> Res {
    let mut group = Map::new();
    if let Some(m) = matcher.filter(|m| !m.is_empty()) {
        group.insert("matcher".into(), m.into());
    }
    group.insert("hooks".into(), json!([{ "type": "command", "command": command, "timeout": timeout.unwrap_or(10) }]));
    list(hooks, event)?.push(Value::Object(group));
    Ok(())
}

// A single handler: { type: "command", <key>: command, ...extra }
pub fn add_flat(hooks: &mut Hooks, event: &str, command: &str, extra: Value, key: &str) -> Res {
    let mut handler = Map::new();
    handler.insert("type".into(), "command".into());
    handler.insert(key.into(), command.into());
    if let Value::Object(extra) = extra {
        handler.extend(extra);
    }
    list(hooks, event)?.push(Value::Object(handler));
    Ok(())
}

// Remove every entry of ours, in any shape, from every event; empty events disappear.
pub fn remove_ours(hooks: &mut Hooks) -> bool {
    let mut changed = false;
    let events: Vec<String> = hooks.keys().cloned().collect();
    for event in events {
        let Some(Value::Array(entries)) = hooks.get(&event) else { continue };
        let kept: Vec<Value> = entries
            .iter()
            .filter_map(|entry| {
                if entry_ours(entry) {
                    changed = true;
                    return None;
                }
                if let Some(inner) = entry.get("hooks").and_then(Value::as_array).filter(|inner| inner.iter().any(entry_ours)) {
                    changed = true;
                    let inner: Vec<Value> = inner.iter().filter(|h| !entry_ours(h)).cloned().collect();
                    if inner.is_empty() {
                        return None;
                    }
                    let mut entry = entry.clone();
                    entry["hooks"] = Value::Array(inner);
                    return Some(entry);
                }
                Some(entry.clone())
            })
            .collect();
        if kept.is_empty() {
            hooks.shift_remove(&event);
        } else {
            hooks.insert(event, Value::Array(kept));
        }
    }
    changed
}

// Every command of ours, as "event: command", so status can compare with what install would write.
pub fn ours_in(hooks: Option<&Value>) -> Vec<String> {
    let mut found = vec![];
    for (event, entries) in hooks.and_then(Value::as_object).into_iter().flatten() {
        let Some(entries) = entries.as_array() else { continue };
        for entry in entries {
            let inner = entry.get("hooks").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
            for h in std::iter::once(entry).chain(inner) {
                for k in COMMAND_KEYS {
                    if let Some(command) = h.get(k).and_then(Value::as_str).filter(|c| c.contains(MARK)) {
                        found.push(format!("{event}: {command}"));
                    }
                }
            }
        }
    }
    found.sort();
    found
}

// ---------- TOML and YAML, by line, so the user's formatting and comments stay ----------

// `key =` at the start of a line, after any indentation.
fn assigns(line: &str, key: &str) -> bool {
    line.trim_start().strip_prefix(key).is_some_and(|rest| rest.trim_start().starts_with('='))
}

// Codex: [features] hooks = true (and drop the retired codex_hooks key).
pub fn with_codex_hooks_feature(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let Some(header) = lines.iter().position(|l| l.trim() == "[features]") else {
        return format!("{}{}[features]\nhooks = true\n", text.trim_end(), if text.trim().is_empty() { "" } else { "\n\n" });
    };
    let end = (header + 1..lines.len()).find(|&i| lines[i].trim_start().starts_with('[')).unwrap_or(lines.len());
    let mut section: Vec<&str> = lines[header + 1..end].iter().copied().filter(|l| !assigns(l, "codex_hooks")).collect();
    match section.iter().position(|l| assigns(l, "hooks")) {
        Some(at) => section[at] = "hooks = true",
        None => section.insert(0, "hooks = true"),
    }
    [&lines[..=header], &section[..], &lines[end..]].concat().join("\n")
}

static BLANK_RUN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n{3,}").unwrap());

// A block of lines between markers that we own entirely (Kimi's config.toml).
pub fn with_block(text: &str, begin: &str, end: &str, body: Option<&str>) -> String {
    let start = text.find(begin);
    let stop = start.and_then(|s| text[s..].find(end).map(|i| s + i));
    let rest = match (start, stop) {
        (Some(start), Some(stop)) => {
            let after = &text[stop + end.len()..];
            format!("{}{}", &text[..start], after.strip_prefix('\n').unwrap_or(after))
        }
        _ => text.to_string(),
    };
    match body {
        None => BLANK_RUN.replace_all(&rest, "\n\n").into_owned(),
        Some(body) => format!("{}{}{begin}\n{body}{end}\n", rest.trim_end(), if rest.trim().is_empty() { "" } else { "\n\n" }),
    }
}

static EMPTY_ENABLED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^ {2}enabled:\s*\[\s*\]\s*$").unwrap());

// Hermes config.yaml: add or remove a name under plugins: enabled:.
pub fn with_hermes_plugin(text: &str, name: &str, enabled: bool) -> String {
    let mut lines: Vec<String> = text.split('\n').map(String::from).collect();
    let Some(plugins) = lines.iter().position(|l| l.strip_prefix("plugins:").is_some_and(|rest| rest.chars().all(char::is_whitespace))) else {
        return if enabled { format!("{}{}plugins:\n  enabled:\n    - {name}\n", text.trim_end(), if text.trim().is_empty() { "" } else { "\n" }) } else { text.to_string() };
    };
    let end = (plugins + 1..lines.len()).find(|&i| lines[i].starts_with(|c: char| !c.is_whitespace())).unwrap_or(lines.len());
    let item = format!("    - {name}");
    let Some(en) = (plugins + 1..end).find(|&i| lines[i].starts_with("  enabled:")) else {
        if enabled {
            lines.splice(plugins + 1..plugins + 1, ["  enabled:".to_string(), item]);
        }
        return lines.join("\n");
    };
    if EMPTY_ENABLED.is_match(&lines[en]) {
        lines[en] = "  enabled:".into();
    }
    let listed = format!("- {name}");
    let items = (en + 1..end).find(|&i| lines[i].trim() == listed);
    match items {
        None if enabled => lines.insert(en + 1, item),
        Some(i) if !enabled => drop(lines.remove(i)),
        _ => {}
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn our_entries_go_in_and_come_out_in_every_shape_leaving_the_users_own() {
        let ours = "MODISA_HOOK=2 /bin/modisa hook x session";
        let mut hooks: Hooks = serde_json::from_value(json!({
            "SessionStart": [{ "hooks": [{ "type": "command", "command": "echo mine" }] }],
            "Stop": [{ "type": "command", "bash": "echo mine too" }],
        }))
        .unwrap();
        add_nested(&mut hooks, "SessionStart", ours, Some("*"), None).unwrap();
        add_flat(&mut hooks, "sessionStart", ours, json!({ "timeoutSec": 10 }), "bash").unwrap();
        add_flat(&mut hooks, "Stop", ours, json!({}), "command").unwrap();
        hooks.insert("Mixed".into(), json!([{ "hooks": [{ "type": "command", "command": "echo keep" }, { "type": "command", "command": ours }] }]));
        assert_eq!(ours_in(Some(&Value::Object(hooks.clone()))).len(), 4);
        assert!(remove_ours(&mut hooks));
        assert_eq!(
            Value::Object(hooks.clone()),
            json!({
                "SessionStart": [{ "hooks": [{ "type": "command", "command": "echo mine" }] }],
                "Stop": [{ "type": "command", "bash": "echo mine too" }],
                "Mixed": [{ "hooks": [{ "type": "command", "command": "echo keep" }] }],
            })
        );
        assert!(!remove_ours(&mut hooks));
    }

    #[test]
    fn config_text_edits_keep_the_users_lines() {
        assert_eq!(with_codex_hooks_feature("model = \"o3\"\n"), "model = \"o3\"\n\n[features]\nhooks = true\n");
        assert_eq!(with_codex_hooks_feature("[features]\ncodex_hooks = true\nweb = true\n[tui]\nx = 1\n"), "[features]\nhooks = true\nweb = true\n[tui]\nx = 1\n");
        assert_eq!(with_codex_hooks_feature("[features]\nhooks = false\n"), "[features]\nhooks = true\n");
        let block = with_block("theme = \"x\"\n", "# >>> s", "# <<< s", Some("[[hooks]]\nevent = \"Stop\"\n"));
        assert_eq!(block, "theme = \"x\"\n\n# >>> s\n[[hooks]]\nevent = \"Stop\"\n# <<< s\n");
        assert_eq!(with_block(&block, "# >>> s", "# <<< s", None), "theme = \"x\"\n\n");
        let yaml = "model: x\nplugins:\n  enabled:\n    - other\nui: y\n";
        let on = with_hermes_plugin(yaml, "modisa-agent-state", true);
        assert_eq!(on, "model: x\nplugins:\n  enabled:\n    - modisa-agent-state\n    - other\nui: y\n");
        assert_eq!(with_hermes_plugin(&on, "modisa-agent-state", false), yaml);
        assert_eq!(with_hermes_plugin("", "p", true), "plugins:\n  enabled:\n    - p\n");
    }

    #[test]
    fn whole_numbers_are_written_as_javascript_writes_them() {
        let mut v = json!({ "a": 1e3, "b": [2.0, 1.5, { "c": -0.0 }], "d": 7 });
        js_numbers(&mut v);
        assert_eq!(v.to_string(), r#"{"a":1000,"b":[2,1.5,{"c":0}],"d":7}"#);
    }
}
