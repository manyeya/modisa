// plugin.json: who the plugin is, the protocol version it speaks, and how to start it (argv, run in the plugin's
// directory, no shell). Optionally what it offers the TUI: actions (listed before it connects; hello must offer each),
// panes it can open, keys under the prefix, and URL globs that Ctrl+click hands to an action.
//
// The TypeScript original checked it with a zod schema; this checks the same things and says the same things: a list
// of (path, message), path "" for the manifest as a whole.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::links::{glob_problem, regex_problem, LinkEntry, LINK};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ManifestAction {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ManifestPane {
    pub id: String,
    pub title: String,
    pub run: Vec<String>,
    #[serde(default)]
    pub placement: String, // overlay (the default) | popup | split | tab | zoomed
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<Value>, // cells, or "80%"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ManifestKey {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<String>,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub protocol: u64,
    pub run: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<Vec<ManifestAction>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panes: Option<Vec<ManifestPane>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keys: Option<Vec<ManifestKey>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub links: Option<Vec<LinkEntry>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<Vec<String>>, // what it may do through modisa (examples/plugins/TOOLING.md); none: undeclared
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<Vec<ManifestSetting>>, // what the settings page offers for it
}

// A setting a plugin offers the user: boolean, number (min, max), enum (options) or string (max characters).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ManifestSetting {
    pub key: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub default: Value,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
}

impl ManifestSetting {
    // `v` as this setting takes it, or why not
    pub fn check(&self, v: &Value) -> Result<(), String> {
        let ok = match self.kind.as_str() {
            "boolean" => v.is_boolean(),
            "number" => v.as_f64().is_some_and(|n| self.min.is_none_or(|m| n >= m) && self.max.is_none_or(|m| n <= m)),
            "enum" => v.as_str().is_some_and(|s| self.options.iter().flatten().any(|o| o == s)),
            "string" => v.as_str().is_some_and(|s| s.chars().count() as f64 <= self.max.unwrap_or(200.0)),
            _ => false,
        };
        if ok {
            return Ok(());
        }
        Err(match self.kind.as_str() {
            "number" => format!("{} is a number{}{}", self.key, self.min.map(|m| format!(" from {m}")).unwrap_or_default(), self.max.map(|m| format!(" to {m}")).unwrap_or_default()),
            "enum" => format!("{} is one of {}", self.key, self.options.iter().flatten().cloned().collect::<Vec<_>>().join(", ")),
            k => format!("{} is a {k}", self.key),
        })
    }
}

// What a plugin can ask to do through modisa (TOOLING.md, Permissions).
pub const PERMISSIONS: &[&str] = &["ui", "ui.replace", "panes.read", "panes.control", "agents", "messages", "notify", "sessions"];

// The permission a plugin needs to make this request, if it needs one; "never" for what no plugin may do.
pub fn needs(method: &str, params: &Value) -> Option<&'static str> {
    Some(match method {
        "ui.slot.set" if params["position"] == "replace" => "ui.replace",
        m if m.starts_with("ui.") => "ui",
        "pane.meta.set" | "pane.meta.clear" => "ui",
        "pane.read" => "panes.read",
        "events.subscribe" if params["output"] == true => "panes.read",
        "pane.keys" | "pane.run" | "pane.split" | "pane.close" | "pane.move" | "pane.resize" | "pane.focus" | "pane.zoom" | "pane.swap" | "pane.rename" | "plugin.pane.open" | "plugin.popup.close" | "plugin.popup.resize" => "panes.control",
        "agent.spawn" => "agents",
        "send" | "inbox" | "messages" | "messaging.pause" => "messages",
        "notify" => "notify",
        "tab.create" | "workspace.create" | "workspace.rename" | "workspace.close" => "sessions",
        "restart" | "kill" => "never",
        _ => return None,
    })
}

pub type Issue = (String, String);

const PLACEMENTS: &[&str] = &["overlay", "popup", "split", "tab", "zoomed"];

fn kind(v: Option<&Value>) -> &'static str {
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

struct Check {
    issues: Vec<Issue>,
}

impl Check {
    fn issue(&mut self, path: &str, message: impl Into<String>) {
        self.issues.push((path.to_string(), message.into()));
    }
    fn expected(&mut self, path: &str, want: &str, got: Option<&Value>) {
        self.issue(path, format!("Invalid input: expected {want}, received {}", kind(got)));
    }
    // a string, at least `min` and at most `max` characters, and (pluginId) lowercase letters, digits and dashes
    fn string(&mut self, path: &str, v: Option<&Value>, min: usize, max: Option<usize>, id: bool) {
        let Some(Value::String(s)) = v else { return self.expected(path, "string", v) };
        let n = s.chars().count();
        if n < min {
            self.issue(path, format!("Too small: expected string to have >={min} characters"));
        } else if let Some(max) = max.filter(|&m| n > m) {
            self.issue(path, format!("Too big: expected string to have <={max} characters"));
        }
        if id && !(s.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')) {
            self.issue(path, "use lowercase letters, digits and dashes");
        }
    }
    fn opt_string(&mut self, path: &str, v: Option<&Value>, min: usize, max: Option<usize>, id: bool) {
        if v.is_some() {
            self.string(path, v, min, max, id);
        }
    }
    fn argv(&mut self, path: &str, v: Option<&Value>) {
        let Some(Value::Array(a)) = v else { return self.expected(path, "array", v) };
        if a.is_empty() {
            self.issue(path, "Too small: expected array to have >=1 items");
        }
        for (i, x) in a.iter().enumerate() {
            self.string(&format!("{path}.{i}"), Some(x), 1, None, false);
        }
    }
    // items of a strict object: no keys but these
    fn strict(&mut self, path: &str, v: &Value, keys: &[&str]) -> Option<Map<String, Value>> {
        let Value::Object(m) = v else {
            self.expected(path, "object", Some(v));
            return None;
        };
        let unknown: Vec<&String> = m.keys().filter(|k| !keys.contains(&k.as_str())).collect();
        if !unknown.is_empty() {
            let list = unknown.iter().map(|k| format!("\"{k}\"")).collect::<Vec<_>>().join(", ");
            self.issue(path, format!("Unrecognized key{}: {list}", if unknown.len() > 1 { "s" } else { "" }));
        }
        Some(m.clone())
    }
    fn list<'a>(&mut self, path: &str, v: Option<&'a Value>) -> Option<&'a Vec<Value>> {
        match v {
            None => None,
            Some(Value::Array(a)) => Some(a),
            other => {
                self.expected(path, "array", other);
                None
            }
        }
    }
    // a number of cells, or a percentage like "80%"
    fn cells(&mut self, path: &str, v: Option<&Value>) {
        match v {
            None => {}
            Some(Value::Number(n)) if n.as_f64().is_some_and(|f| f > 0.0 && f.fract() == 0.0) => {}
            Some(Value::String(s)) if s.ends_with('%') && (1..=3).contains(&(s.len() - 1)) && s[..s.len() - 1].bytes().all(|b| b.is_ascii_digit()) => {}
            Some(_) => self.issue(path, "Invalid input"),
        }
    }
}

// plugin.json, checked: the manifest, or what's wrong with it.
pub fn check_manifest(raw: &Value) -> Result<PluginManifest, Vec<Issue>> {
    let mut c = Check { issues: vec![] };
    let Value::Object(m) = raw else {
        c.expected("", "object", Some(raw));
        return Err(c.issues);
    };
    c.string("name", m.get("name"), 0, None, true);
    match m.get("protocol") {
        Some(Value::Number(n)) if n.as_f64().is_some_and(|f| f.fract() == 0.0) => {
            if n.as_f64().unwrap() <= 0.0 {
                c.issue("protocol", "Too small: expected number to be >0");
            }
        }
        other => c.expected("protocol", if matches!(other, Some(Value::Number(_))) { "int" } else { "number" }, other),
    }
    c.argv("run", m.get("run"));
    if let Some(d) = m.get("description") {
        c.string("description", Some(d), 0, None, false);
    }
    if let Some(list) = c.list("actions", m.get("actions")) {
        for (i, a) in list.iter().enumerate() {
            let p = format!("actions.{i}");
            if let Some(o) = c.strict(&p, a, &["id", "title", "description"]) {
                c.string(&format!("{p}.id"), o.get("id"), 0, None, true);
                c.string(&format!("{p}.title"), o.get("title"), 1, Some(60), false);
                c.opt_string(&format!("{p}.description"), o.get("description"), 0, Some(200), false);
            }
        }
    }
    if let Some(list) = c.list("panes", m.get("panes")) {
        for (i, a) in list.iter().enumerate() {
            let p = format!("panes.{i}");
            if let Some(o) = c.strict(&p, a, &["id", "title", "run", "placement", "width", "height"]) {
                c.string(&format!("{p}.id"), o.get("id"), 0, None, true);
                c.string(&format!("{p}.title"), o.get("title"), 1, Some(60), false);
                c.argv(&format!("{p}.run"), o.get("run"));
                if let Some(pl) = o.get("placement") {
                    if !pl.as_str().is_some_and(|s| PLACEMENTS.contains(&s)) {
                        c.issue(&format!("{p}.placement"), format!("Invalid option: expected one of {}", PLACEMENTS.iter().map(|x| format!("\"{x}\"")).collect::<Vec<_>>().join("|")));
                    }
                }
                c.cells(&format!("{p}.width"), o.get("width"));
                c.cells(&format!("{p}.height"), o.get("height"));
            }
        }
    }
    if let Some(list) = c.list("keys", m.get("keys")) {
        for (i, a) in list.iter().enumerate() {
            let p = format!("keys.{i}");
            if let Some(o) = c.strict(&p, a, &["key", "action", "pane", "description"]) {
                c.string(&format!("{p}.key"), o.get("key"), 1, Some(12), false);
                c.opt_string(&format!("{p}.action"), o.get("action"), 0, None, true);
                c.opt_string(&format!("{p}.pane"), o.get("pane"), 0, None, true);
                c.string(&format!("{p}.description"), o.get("description"), 1, Some(80), false);
            }
        }
    }
    if let Some(list) = c.list("permissions", m.get("permissions")) {
        for (i, p) in list.iter().enumerate() {
            if !p.as_str().is_some_and(|p| PERMISSIONS.contains(&p)) {
                c.issue(&format!("permissions.{i}"), format!("Invalid option: expected one of {}", PERMISSIONS.iter().map(|x| format!("\"{x}\"")).collect::<Vec<_>>().join("|")));
            }
        }
    }
    if let Some(list) = c.list("settings", m.get("settings")) {
        let mut keys = std::collections::HashSet::new();
        for (i, a) in list.iter().enumerate() {
            let p = format!("settings.{i}");
            if let Some(o) = c.strict(&p, a, &["key", "type", "default", "title", "min", "max", "options"]) {
                c.string(&format!("{p}.key"), o.get("key"), 0, None, true);
                c.string(&format!("{p}.title"), o.get("title"), 1, Some(60), false);
                if !o.get("type").and_then(Value::as_str).is_some_and(|t| ["boolean", "number", "enum", "string"].contains(&t)) {
                    c.issue(&format!("{p}.type"), "Invalid option: expected one of \"boolean\"|\"number\"|\"enum\"|\"string\"");
                } else if let Ok(s) = serde_json::from_value::<ManifestSetting>(a.clone()) {
                    if s.kind == "enum" && s.options.as_ref().is_none_or(|o| o.is_empty()) {
                        c.issue(&format!("{p}.options"), "an enum setting needs its options");
                    }
                    if let Err(e) = s.check(&s.default) {
                        c.issue(&format!("{p}.default"), e);
                    }
                    if !keys.insert(s.key.clone()) {
                        c.issue(&format!("{p}.key"), format!("a second setting {}", s.key));
                    }
                } else {
                    c.issue(&p, "Invalid input");
                }
            }
        }
    }
    if let Some(list) = c.list("links", m.get("links")) {
        if list.len() > LINK.per_plugin {
            c.issue("links", format!("Too big: expected array to have <={} items", LINK.per_plugin));
        }
        for (i, a) in list.iter().enumerate() {
            let p = format!("links.{i}");
            if let Some(o) = c.strict(&p, a, &["pattern", "regex", "action"]) {
                c.opt_string(&format!("{p}.pattern"), o.get("pattern"), 1, Some(LINK.source), false);
                c.opt_string(&format!("{p}.regex"), o.get("regex"), 1, Some(LINK.source), false);
                c.string(&format!("{p}.action"), o.get("action"), 0, None, true);
            }
        }
    }
    if !c.issues.is_empty() {
        return Err(c.issues);
    }
    let mut manifest: PluginManifest = serde_json::from_value(raw.clone()).map_err(|e| vec![(String::new(), e.to_string())])?;
    for p in manifest.panes.iter_mut().flatten() {
        if p.placement.is_empty() {
            p.placement = "overlay".into();
        }
    }
    // what the shapes alone can't say
    for (list, label) in [(manifest.actions.as_ref().map(|a| a.iter().map(|x| x.id.clone()).collect::<Vec<_>>()), "action"), (manifest.panes.as_ref().map(|a| a.iter().map(|x| x.id.clone()).collect()), "pane")] {
        let mut seen = std::collections::HashSet::new();
        for (i, id) in list.iter().flatten().enumerate() {
            if !seen.insert(id.clone()) {
                c.issue(&format!("{label}s.{i}.id"), format!("a second {label} with id {id}"));
            }
        }
    }
    let actions: std::collections::HashSet<&String> = manifest.actions.iter().flatten().map(|a| &a.id).collect();
    let panes: std::collections::HashSet<&String> = manifest.panes.iter().flatten().map(|a| &a.id).collect();
    let mut keys = std::collections::HashSet::new();
    for (i, k) in manifest.keys.iter().flatten().enumerate() {
        if !keys.insert(&k.key) {
            c.issue(&format!("keys.{i}.key"), format!("key {} is bound twice", k.key));
        }
        if k.action.is_some() == k.pane.is_some() {
            c.issue(&format!("keys.{i}"), format!("key {} needs exactly one of action or pane", k.key));
        }
        if let Some(a) = k.action.as_ref().filter(|a| !actions.contains(a)) {
            c.issue(&format!("keys.{i}.action"), format!("key {} runs action {a}, which isn't in actions", k.key));
        }
        if let Some(p) = k.pane.as_ref().filter(|p| !panes.contains(p)) {
            c.issue(&format!("keys.{i}.pane"), format!("key {} opens pane {p}, which isn't in panes", k.key));
        }
    }
    for (i, l) in manifest.links.iter().flatten().enumerate() {
        match (&l.pattern, &l.regex) {
            (Some(p), None) => {
                if let Some(why) = glob_problem(p) {
                    c.issue(&format!("links.{i}.pattern"), why);
                }
            }
            (None, Some(r)) => {
                if let Some(why) = regex_problem(r) {
                    c.issue(&format!("links.{i}.regex"), why);
                }
            }
            _ => c.issue(&format!("links.{i}"), "a link needs exactly one of pattern (a URL glob) or regex"),
        }
        if !actions.contains(&l.action) {
            c.issue(&format!("links.{i}.action"), format!("links to action {}, which isn't in actions", l.action));
        }
    }
    if c.issues.is_empty() { Ok(manifest) } else { Err(c.issues) }
}

// The issues, as readManifest said them: "path: message; …", "(top level)" for the manifest as a whole.
pub fn describe(issues: &[Issue]) -> String {
    issues.iter().map(|(p, m)| format!("{}: {m}", if p.is_empty() { "(top level)" } else { p })).collect::<Vec<_>>().join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn problems(v: Value) -> Vec<String> {
        check_manifest(&v).err().unwrap_or_default().into_iter().map(|(_, m)| m).collect()
    }

    #[test]
    fn checks_manifests() {
        let base = json!({ "name": "demo", "protocol": 1, "run": ["bun", "plugin.ts"], "actions": [{ "id": "open", "title": "Open" }] });
        let m = check_manifest(&base).unwrap();
        assert_eq!(m.name, "demo");
        let mut bad = base.clone();
        bad["name"] = json!("Demo");
        assert_eq!(problems(bad), ["use lowercase letters, digits and dashes"]);
        let mut keys = base.clone();
        keys["keys"] = json!([{ "key": "z", "description": "both", "action": "open", "pane": "x" }]);
        assert!(problems(keys).iter().any(|m| m.contains("needs exactly one of action or pane")));
        let mut links = base.clone();
        links["links"] = json!([{ "action": "open" }]);
        assert_eq!(problems(links), ["a link needs exactly one of pattern (a URL glob) or regex"]);
        let mut panes = base;
        panes["panes"] = json!([{ "id": "p", "title": "P", "run": ["x"] }]);
        assert_eq!(check_manifest(&panes).unwrap().panes.unwrap()[0].placement, "overlay");
    }

    #[test]
    fn permissions_and_settings_are_checked() {
        let ok = json!({ "name": "p", "protocol": 1, "run": ["x"], "permissions": ["ui", "panes.read"], "settings": [
            { "key": "threshold", "type": "number", "default": 80, "min": 0, "max": 100, "title": "Warn above" },
            { "key": "show", "type": "enum", "options": ["a", "b"], "default": "a", "title": "Show" },
        ] });
        let m = check_manifest(&ok).unwrap();
        assert_eq!(m.permissions.as_deref(), Some(&["ui".to_string(), "panes.read".to_string()][..]));
        let s = &m.settings.unwrap()[0];
        assert!(s.check(&json!(50)).is_ok() && s.check(&json!(101)).is_err() && s.check(&json!("x")).is_err());
        for (bad, at) in [
            (json!({ "name": "p", "protocol": 1, "run": ["x"], "permissions": ["root"] }), "permissions.0"),
            (json!({ "name": "p", "protocol": 1, "run": ["x"], "settings": [{ "key": "k", "type": "enum", "default": "z", "options": ["a"], "title": "K" }] }), "settings.0.default"),
            (json!({ "name": "p", "protocol": 1, "run": ["x"], "settings": [{ "key": "k", "type": "color", "default": 1, "title": "K" }] }), "settings.0.type"),
        ] {
            let issues = check_manifest(&bad).unwrap_err();
            assert!(issues.iter().any(|(p, _)| p == at), "{bad}: {issues:?}");
        }
    }

    #[test]
    fn what_each_request_needs() {
        assert_eq!(needs("ui.status.set", &json!({})), Some("ui"));
        assert_eq!(needs("ui.slot.set", &json!({ "position": "replace" })), Some("ui.replace"));
        assert_eq!(needs("ui.slot.set", &json!({ "position": "after" })), Some("ui"));
        assert_eq!(needs("events.subscribe", &json!({ "output": true })), Some("panes.read"));
        assert_eq!(needs("events.subscribe", &json!({})), None);
        assert_eq!(needs("pane.keys", &json!({})), Some("panes.control"));
        assert_eq!(needs("kill", &json!({})), Some("never"));
        assert_eq!(needs("list", &json!({})), None);
    }
}
