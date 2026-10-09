// Marketplaces: git repositories that list plugins, in a modisa-marketplace.json at the top (or
// .modisa/marketplace.json). `modisa plugin marketplace add owner/repo` clones one into <state>/marketplaces/<name>,
// with install's git policy, and records it in <state>/marketplaces.json; `plugin install <plugin>@<marketplace>`
// installs a plugin it lists. A marketplace is a list someone keeps, not a review: its plugins run as you.
use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::plugin_manage::{iso_now, mkdtemp, rm_rf};
use super::plugins::{inside, installs, is_dir, linked_plugins, real, without_credentials};
use crate::cli::plugin_git::{checkout_ref, fetch_latest, git, source_problem, JS_SPACE};
use crate::core::paths::DIR;
use crate::core::text::clean_text;
use crate::protocol::conn::{error, RpcResult};

pub static MARKETPLACES_DIR: LazyLock<String> = LazyLock::new(|| format!("{}/marketplaces", *DIR));
fn registry_file() -> String {
    format!("{}/marketplaces.json", *DIR)
}
const FILES: [&str; 2] = ["modisa-marketplace.json", ".modisa/marketplace.json"];
const MAX_BYTES: u64 = 1024 * 1024;

// JavaScript's \s: what /\s+/ collapses and trim() takes off
pub fn js_space(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\x0B' | '\x0C' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}

// ---------- the file: a stranger's, so every name is checked and every text cleaned ----------
// The original checked it with a zod schema; this checks the same things and words them the same way. An issue that
// isn't `soft` stops the checks that would look at what it's in (zod's abort), as a type error does.
#[derive(Clone, Debug, PartialEq)]
pub struct Issue {
    pub path: String,
    pub message: String,
    soft: bool,
}

pub fn describe(issues: &[Issue]) -> String {
    issues.iter().map(|i| format!("{}: {}", if i.path.is_empty() { "(top level)" } else { &i.path }, i.message)).collect::<Vec<_>>().join("; ")
}

static ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z0-9][a-z0-9-]*$").unwrap());
static GITHUB: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$").unwrap());
static UNSAFE: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"[{JS_SPACE}\x00-\x1f\x7f-\x9f]")).unwrap());
const SOURCE: &str = r#"a source is "./path" in this repository, "owner/repo" on GitHub, or { "git": url, "ref", "subdir" }"#;

pub fn is_id(s: &str) -> bool {
    s.encode_utf16().count() <= 64 && ID.is_match(s)
}

// what goes to git or names a path: no option, whitespace or control character
fn unsafe_text(s: &str) -> Option<String> {
    (s.starts_with('-') || UNSAFE.is_match(s)).then(|| format!("{} can't start with - or hold spaces or control characters", serde_json::to_string(&clean_text(s, 60)).unwrap()))
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Source {
    Path(String),
    Git {
        git: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        r#ref: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        subdir: Option<String>,
    },
}

fn source_problem_of(s: &Source) -> Option<String> {
    match s {
        Source::Path(s) if s.starts_with("./") => unsafe_text(s),
        Source::Path(s) => if GITHUB.is_match(s) { None } else { Some(SOURCE.into()) },
        Source::Git { git, r#ref, subdir } => {
            let subdir = subdir.as_deref().filter(|s| !s.is_empty()).and_then(|s| unsafe_text(s).or_else(|| (s.starts_with('/') || s.split('/').any(|p| p == "..")).then(|| "subdir must be a relative path inside the repository".to_string())));
            unsafe_text(git).or_else(|| source_problem(git)).or_else(|| r#ref.as_deref().filter(|r| !r.is_empty()).and_then(unsafe_text)).or(subdir)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Entry {
    pub name: String,
    pub description: String,
    pub source: Source,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MarketplaceFile {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>, // or Claude Code's { name, email }
    pub plugins: Vec<Entry>,
}

// zod's names for what it got
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
    fn hard(&mut self, path: &str, message: impl Into<String>) {
        self.issues.push(Issue { path: path.into(), message: message.into(), soft: false });
    }
    fn soft(&mut self, path: &str, message: impl Into<String>) {
        self.issues.push(Issue { path: path.into(), message: message.into(), soft: true });
    }
    fn aborted_since(&self, start: usize) -> bool {
        self.issues[start..].iter().any(|i| !i.soft)
    }

    fn string<'a>(&mut self, path: &str, v: Option<&'a Value>) -> Option<&'a str> {
        match v {
            Some(Value::String(s)) => Some(s),
            other => {
                self.hard(path, format!("Invalid input: expected string, received {}", kind(other)));
                None
            }
        }
    }

    // a string of at most `max` (and at least `min`) UTF-16 units, as JavaScript counts a string's length
    fn sized(&mut self, path: &str, v: Option<&Value>, min: usize, max: usize) -> Option<String> {
        let s = self.string(path, v)?;
        let n = s.encode_utf16().count();
        if n < min {
            self.soft(path, format!("Too small: expected string to have >={min} characters"));
        }
        if n > max {
            self.soft(path, format!("Too big: expected string to have <={max} characters"));
        }
        Some(s.to_string())
    }

    fn id(&mut self, path: &str, v: Option<&Value>) -> Option<String> {
        let s = self.sized(path, v, 0, 64)?;
        if !ID.is_match(&s) {
            self.soft(path, "use lowercase letters, digits and dashes");
        }
        Some(s)
    }

    // one line, cleaned of escapes and control characters, and cut to `cells` terminal cells
    fn text(&mut self, path: &str, v: Option<&Value>, cells: usize) -> Option<String> {
        self.string(path, v).map(|s| one_line(s, cells))
    }

    fn source(&mut self, path: &str, v: Option<&Value>) -> Option<Source> {
        match v {
            Some(Value::String(s)) => {
                // a string's only problem can be its length, so it's the option that's taken
                self.sized(path, v, 0, 500);
                Some(Source::Path(s.clone()))
            }
            Some(Value::Object(o)) => {
                let mut inner = Check { issues: vec![] };
                let git = inner.sized(&format!("{path}.git"), o.get("git"), 0, 500);
                let r#ref = o.get("ref").and_then(|r| inner.sized(&format!("{path}.ref"), Some(r), 1, 200));
                let subdir = o.get("subdir").and_then(|s| inner.sized(&format!("{path}.subdir"), Some(s), 1, 300));
                if inner.aborted_since(0) {
                    self.hard(path, SOURCE); // neither option fits: the union's own message
                    return None;
                }
                self.issues.extend(inner.issues);
                Some(Source::Git { git: git?, r#ref, subdir })
            }
            _ => {
                self.hard(path, SOURCE);
                None
            }
        }
    }

    fn entry(&mut self, path: &str, v: &Value) -> Option<Entry> {
        let Value::Object(o) = v else {
            self.hard(path, format!("Invalid input: expected object, received {}", kind(Some(v))));
            return None;
        };
        let start = self.issues.len();
        let name = self.id(&format!("{path}.name"), o.get("name"));
        let description = match o.get("description") {
            None => Some(String::new()),
            d => self.text(&format!("{path}.description"), d, 200),
        };
        let source = self.source(&format!("{path}.source"), o.get("source"));
        if self.aborted_since(start) {
            return None;
        }
        let source = source?;
        if let Some(why) = source_problem_of(&source) {
            self.soft(&format!("{path}.source"), why);
        }
        Some(Entry { name: name?, description: description?, source })
    }
}

fn one_line(s: &str, cells: usize) -> String {
    let mut collapsed = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars() {
        if js_space(c) {
            if !space {
                collapsed.push(' ');
            }
            space = true;
        } else {
            collapsed.push(c);
            space = false;
        }
    }
    clean_text(&collapsed, cells).trim_matches(js_space).to_string()
}

pub fn check_marketplace(raw: &Value) -> Result<MarketplaceFile, Vec<Issue>> {
    let mut c = Check { issues: vec![] };
    let Value::Object(o) = raw else {
        c.hard("", format!("Invalid input: expected object, received {}", kind(Some(raw))));
        return Err(c.issues);
    };
    let name = c.id("name", o.get("name"));
    let description = o.get("description").map(|d| c.text("description", Some(d), 200));
    let owner = o.get("owner").map(|v| match v {
        Value::String(s) => Some(one_line(s, 100)),
        Value::Object(m) if matches!(m.get("name"), Some(Value::String(_))) => Some(one_line(m["name"].as_str().unwrap(), 100)),
        _ => {
            c.hard("owner", "Invalid input");
            None
        }
    });
    let mut plugins = vec![];
    match o.get("plugins") {
        Some(Value::Array(list)) => {
            for (i, p) in list.iter().enumerate() {
                plugins.push(c.entry(&format!("plugins.{i}"), p));
            }
            if list.len() > 1000 {
                c.soft("plugins", "Too big: expected array to have <=1000 items");
            }
        }
        other => c.hard("plugins", format!("Invalid input: expected array, received {}", kind(other))),
    }
    if !c.aborted_since(0) {
        let mut seen = HashSet::new();
        for (i, p) in plugins.iter().enumerate() {
            let Some(p) = p else { continue };
            if !seen.insert(p.name.clone()) {
                c.soft(&format!("plugins.{i}.name"), format!("a second plugin named {}", p.name));
            }
        }
    }
    if !c.issues.is_empty() {
        return Err(c.issues);
    }
    Ok(MarketplaceFile { name: name.unwrap_or_default(), description: description.flatten(), owner: owner.flatten(), plugins: plugins.into_iter().flatten().collect() })
}

// A checkout's marketplace file, validated, and itself inside the checkout
pub fn read_marketplace(dir: &str) -> Result<MarketplaceFile, String> {
    let root = real(dir);
    if root.is_empty() {
        return Err(format!("its checkout is gone ({dir})"));
    }
    for name in FILES {
        let path = real(&format!("{dir}/{name}"));
        if path.is_empty() {
            continue;
        }
        if !inside(&path, &root) {
            return Err(format!("{name} points outside the repository"));
        }
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_BYTES) {
            return Err(format!("{name} is over {} MB", MAX_BYTES / 1024 / 1024));
        }
        let raw: Value = match std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| serde_json::from_slice(&b).map_err(|e| e.to_string())) {
            Ok(v) => v,
            Err(e) => return Err(clean_text(&format!("{name} isn't valid JSON: {e}"), 300)),
        };
        return check_marketplace(&raw).map_err(|issues| clean_text(&format!("{name}: {}", describe(&issues)), 500));
    }
    Err(format!("no {} at the top of the repository", FILES.join(" or ")))
}

// ---------- the registry: which are added, from where, at which commit ----------
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Marketplace {
    pub name: String,
    pub source: String,
    #[serde(default)]
    pub r#ref: Option<String>,
    #[serde(default)]
    pub commit: String,
    #[serde(default)]
    pub added_at: String,
    #[serde(default)]
    pub updated_at: String,
}

// a name that isn't a plain id never makes it into a path
pub fn registry() -> Vec<Marketplace> {
    let list: Value = std::fs::read(registry_file()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null);
    let Value::Array(list) = list else { return vec![] };
    list.into_iter()
        .filter(|m| m["name"].as_str().is_some_and(is_id) && m["source"].is_string())
        .filter_map(|m| serde_json::from_value(m).ok())
        .collect()
}

fn save(list: &[Marketplace]) -> std::io::Result<()> {
    std::fs::create_dir_all(&*DIR)?;
    let mut b = [0u8; 4];
    let _ = getrandom::fill(&mut b);
    let tmp = format!("{}.{}.tmp", registry_file(), b.iter().map(|x| format!("{x:02x}")).collect::<String>());
    std::fs::write(&tmp, serde_json::to_string_pretty(list).unwrap() + "\n")?;
    std::fs::rename(&tmp, registry_file()) // whole or not at all
}

fn dir_of(name: &str) -> String {
    format!("{}/{name}", *MARKETPLACES_DIR)
}

fn changed(name: &str, commit: &str, updated_at: &str) -> std::io::Result<()> {
    let list: Vec<Marketplace> = registry().into_iter().map(|m| if m.name == name { Marketplace { commit: commit.into(), updated_at: updated_at.into(), ..m } } else { m }).collect();
    save(&list)
}

// owner/repo is GitHub's; anything else is a git URL as install takes them
pub fn marketplace_url(arg: &str) -> String {
    if GITHUB.is_match(arg) && !arg.starts_with('.') { format!("https://github.com/{arg}.git") } else { arg.to_string() }
}

// not added: `stage` and `reason` say where it failed, and nothing was left behind (marketplaceResults.add)
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Added {
    pub added: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub already_added: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub source: String,
    pub r#ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugins: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

// Clone it into a staging directory, check its file, and only then take its name. A failure leaves nothing behind.
pub async fn add_marketplace(arg: &str, r#ref: Option<&str>) -> RpcResult<Added> {
    let url = marketplace_url(arg);
    let source = without_credentials(&url);
    let r#ref = r#ref.map(String::from);
    let failed = |staging: Option<&str>, stage: &'static str, reason: &str| {
        if let Some(s) = staging {
            rm_rf(s);
        }
        Added { source: source.clone(), r#ref: r#ref.clone(), stage: Some(stage), reason: Some(without_credentials(reason)), ..Default::default() }
    };
    if let Some(problem) = source_problem(&url) {
        return Ok(failed(None, "source", &problem));
    }
    if crate::core::paths::which("git").is_none() {
        return Ok(failed(None, "git", "git isn't installed"));
    }
    if let Some(r) = r#ref.as_deref().filter(|r| !r.is_empty() && unsafe_text(r).is_some()) {
        return Ok(failed(None, "ref", &format!("not a ref: {r}")));
    }

    std::fs::create_dir_all(&*MARKETPLACES_DIR)?;
    let staging = mkdtemp(&format!("{}/.staging-", *MARKETPLACES_DIR))?;
    let at = Some(staging.as_str());
    let checkout = format!("{staging}/checkout");
    let cloned = git(&["clone", "--quiet", "--", &url, &checkout], None).await;
    if cloned.code != 0 {
        return Ok(failed(at, "clone", if cloned.err.is_empty() { "git clone failed" } else { &cloned.err }));
    }
    git(&["remote", "set-url", "origin", &source], Some(&checkout)).await; // no credentials left in the checkout's own config
    let commit = match checkout_ref(&checkout, r#ref.as_deref(), &source).await {
        Ok(c) => c,
        Err(reason) => return Ok(failed(at, "ref", &reason)),
    };
    let file = match read_marketplace(&checkout) {
        Ok(f) => f,
        Err(e) => return Ok(failed(at, "manifest", &e)),
    };
    let plugins: Vec<String> = file.plugins.iter().map(|p| p.name.clone()).collect();

    let existing = registry().into_iter().find(|m| m.name == file.name);
    if let Some(existing) = existing.as_ref().filter(|m| m.source == source) {
        rm_rf(&staging);
        return Ok(Added { already_added: Some(true), name: Some(file.name), source, r#ref: existing.r#ref.clone(), commit: Some(existing.commit.clone()), description: file.description, plugins: Some(plugins), ..Default::default() });
    }
    if let Some(existing) = existing {
        return Ok(failed(at, "collision", &format!("a marketplace named {} is already added from {}; modisa plugin marketplace remove {} first", file.name, existing.source, file.name)));
    }
    // rename won't replace a directory with anything in it, so two adds racing for the name can't both get it
    if std::fs::rename(&checkout, dir_of(&file.name)).is_err() {
        return Ok(failed(at, "collision", &format!("{} is already there", dir_of(&file.name))));
    }
    rm_rf(&staging);
    let now = iso_now();
    let mut list: Vec<Marketplace> = registry().into_iter().filter(|m| m.name != file.name).collect();
    list.push(Marketplace { name: file.name.clone(), source: source.clone(), r#ref: r#ref.clone(), commit: commit.clone(), added_at: now.clone(), updated_at: now });
    save(&list)?;
    Ok(Added { added: true, name: Some(file.name), source, r#ref, commit: Some(commit), description: file.description, plugins: Some(plugins), ..Default::default() })
}

// error: its file can't be read now (it lists no plugins until that's fixed)
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Listed {
    #[serde(flatten)]
    pub m: Marketplace,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    pub plugins: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub fn list_marketplaces() -> Vec<Listed> {
    registry()
        .into_iter()
        .map(|m| {
            let read = read_marketplace(&dir_of(&m.name));
            let file = read.as_ref().ok();
            Listed {
                description: file.and_then(|f| f.description.clone()).filter(|d| !d.is_empty()),
                owner: file.and_then(|f| f.owner.clone()).filter(|o| !o.is_empty()),
                plugins: file.map_or(0, |f| f.plugins.len()),
                error: read.as_ref().err().cloned(),
                m,
            }
        })
        .collect()
}

// each one asked for: moved to the latest (updated), already there, or left as it was (reason)
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Updated {
    pub name: String,
    pub updated: bool,
    pub from: String,
    pub commit: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugins: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

// Each one (or the one named) moved to what its ref (none: its source's HEAD) is at now. A new commit whose file
// doesn't check out, or that renames the marketplace, is undone: it stays where it was.
pub async fn update_marketplaces(name: Option<&str>) -> RpcResult<Vec<Updated>> {
    let chosen: Vec<Marketplace> = registry().into_iter().filter(|m| name.is_none_or(|n| m.name == n)).collect();
    if let Some(name) = name.filter(|_| chosen.is_empty()) {
        return Err(error(format!("no marketplace named {name} (modisa plugin marketplace list)")));
    }
    let mut out = vec![];
    for m in chosen {
        let dir = dir_of(&m.name);
        let left = |reason: &str| Updated { name: m.name.clone(), updated: false, from: m.commit.clone(), commit: m.commit.clone(), plugins: None, reason: Some(without_credentials(reason)) };
        let latest = match fetch_latest(&dir, &m.source, m.r#ref.as_deref()).await {
            Ok(c) => c,
            Err(reason) => {
                out.push(left(&reason));
                continue;
            }
        };
        if latest == m.commit {
            out.push(Updated { name: m.name.clone(), updated: false, from: m.commit.clone(), commit: m.commit.clone(), plugins: Some(read_marketplace(&dir).map_or(0, |f| f.plugins.len())), reason: None });
            continue;
        }
        let moved = git(&["checkout", "--quiet", "--detach", &latest], Some(&dir)).await;
        if moved.code != 0 {
            out.push(left(&if moved.err.is_empty() { format!("couldn't check out {latest}") } else { moved.err }));
            continue;
        }
        match read_marketplace(&dir) {
            Ok(file) if file.name == m.name => {
                changed(&m.name, &latest, &iso_now())?;
                out.push(Updated { name: m.name.clone(), updated: true, from: m.commit.clone(), commit: latest, plugins: Some(file.plugins.len()), reason: None });
            }
            read => {
                git(&["checkout", "--quiet", "--detach", &m.commit], Some(&dir)).await;
                out.push(left(&match read {
                    Ok(file) => format!("it calls itself {} now: remove it and add it again", file.name),
                    Err(e) => format!("at {}: {e}", &latest[..latest.len().min(7)]),
                }));
            }
        }
    }
    Ok(out)
}

// installed: plugins installed from it, which stay installed
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Removed {
    pub name: String,
    pub removed: bool,
    pub dir: String,
    pub installed: Vec<String>,
}

// Its checkout and its record go; plugins installed from it stay installed (`installed` names them)
pub fn remove_marketplace(name: &str) -> RpcResult<Removed> {
    let list = registry();
    let Some(m) = list.iter().find(|x| x.name == name).cloned() else {
        return Err(error(format!("no marketplace named {name} (modisa plugin marketplace list)")));
    };
    save(&list.into_iter().filter(|x| x.name != m.name).collect::<Vec<_>>())?;
    rm_rf(&dir_of(&m.name));
    Ok(Removed { dir: dir_of(&m.name), removed: true, installed: installs().into_iter().filter(|r| r.marketplace.as_deref() == Some(&m.name)).map(|r| r.name).collect(), name: m.name })
}

// ---------- the plugins they list ----------
// Where a listed plugin's code comes from, as shown: the marketplace's own repository for one inside it
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Shown {
    pub from: String,
    pub r#ref: Option<String>,
    pub subdir: Option<String>,
}

fn shown(source: &Source, m: &Marketplace) -> Shown {
    match source {
        Source::Git { git, r#ref, subdir } => Shown { from: without_credentials(git), r#ref: r#ref.clone(), subdir: subdir.clone() },
        Source::Path(s) if s.starts_with("./") => Shown { from: m.source.clone(), r#ref: m.r#ref.clone(), subdir: Some(s[2..].trim_end_matches('/').to_string()).filter(|s| !s.is_empty()) },
        Source::Path(s) => Shown { from: marketplace_url(s), r#ref: None, subdir: None },
    }
}

// A plugin a marketplace lists. installed: a plugin by that name is linked.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ListedPlugin {
    pub name: String,
    pub marketplace: String,
    pub description: String,
    #[serde(flatten)]
    pub shown: Shown,
    pub install: String,
    pub installed: bool,
}

// Every plugin the marketplaces list that matches every word (none: all of them); installed: one by that name is linked
pub fn marketplace_plugins(words: &[String]) -> Vec<ListedPlugin> {
    let linked: HashSet<String> = linked_plugins().into_iter().map(|l| l.name).collect();
    let want: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();
    let mut out = vec![];
    for m in registry() {
        for p in read_marketplace(&dir_of(&m.name)).map(|f| f.plugins).unwrap_or_default() {
            let haystack = format!("{} {} {}", p.name, p.description, m.name).to_lowercase();
            if !want.iter().all(|w| haystack.contains(w.as_str())) {
                continue;
            }
            out.push(ListedPlugin { install: format!("modisa plugin install {}@{}", p.name, m.name), installed: linked.contains(&p.name), shown: shown(&p.source, &m), name: p.name, marketplace: m.name.clone(), description: p.description });
        }
    }
    out
}

// What installs <plugin>@<marketplace>: a git URL, ref and subdir.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    pub name: String,
    pub marketplace: String,
    pub from: String,
    pub r#ref: Option<String>,
    pub subdir: Option<String>,
    pub url: String,
}

// A plugin inside the marketplace's repository comes from its checkout here (a file:// URL, at the commit the
// marketplace is at), from a directory that's really inside it.
pub fn resolve_entry(spec: &str) -> RpcResult<Resolved> {
    let at = spec.rfind('@').filter(|&i| i > 0);
    let Some((name, market)) = at.map(|i| (&spec[..i], &spec[i + 1..])).filter(|(_, m)| is_id(m)) else {
        return Err(error(format!("not <plugin>@<marketplace>: {}", clean_text(spec, 100))));
    };
    let Some(m) = registry().into_iter().find(|x| x.name == market) else {
        return Err(error(format!("no marketplace named {market} (modisa plugin marketplace list)")));
    };
    let dir = dir_of(&m.name);
    let file = read_marketplace(&dir).map_err(|e| error(format!("marketplace {}: {e}", m.name)))?;
    let Some(p) = file.plugins.into_iter().find(|x| x.name == name) else {
        return Err(error(format!("marketplace {} lists no plugin named {}", m.name, clean_text(name, 64))));
    };
    let s = shown(&p.source, &m);
    let base = |url: String, r#ref: Option<String>, subdir: Option<String>| Resolved { name: p.name.clone(), marketplace: m.name.clone(), from: s.from.clone(), r#ref, subdir, url };
    match &p.source {
        Source::Git { git, .. } => Ok(base(git.clone(), s.r#ref.clone(), s.subdir.clone())),
        Source::Path(path) if !path.starts_with("./") => Ok(base(s.from.clone(), s.r#ref.clone(), s.subdir.clone())),
        Source::Path(path) => {
            let (root, subdir) = within(&dir, path)?;
            Ok(base(format!("file://{root}"), None, subdir))
        }
    }
}

// A relative source's directory, which must really be inside the checkout at `dir` (no .., no symlink out): the
// checkout's real path, and the directory's path in it (None: the top).
pub fn within(dir: &str, source: &str) -> RpcResult<(String, Option<String>)> {
    let root = real(dir);
    let target = real(&format!("{dir}/{source}"));
    if root.is_empty() || target.is_empty() || !inside(&target, &root) || !is_dir(&target) {
        return Err(error(format!("{} isn't a directory inside the marketplace's repository", clean_text(source, 100))));
    }
    let subdir = (target != root).then(|| target[root.len() + 1..].to_string());
    Ok((root, subdir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn problems(raw: Value) -> String {
        match check_marketplace(&raw) {
            Ok(_) => String::new(),
            Err(issues) => issues.iter().map(|i| format!("{}: {}", i.path, i.message)).collect::<Vec<_>>().join("; "),
        }
    }
    fn source_problem_in(source: Value) -> String {
        problems(json!({ "name": "acme", "plugins": [{ "name": "p", "source": source }] }))
    }

    #[test]
    fn the_three_source_forms_parse_and_extra_fields_are_ignored() {
        let m = check_marketplace(&json!({
            "name": "acme", "description": "Acme's plugins", "owner": "acme", "version": 3,
            "plugins": [
                { "name": "worktrees", "description": "a space per worktree", "source": "./plugins/worktrees", "author": "someone" },
                { "name": "x", "source": { "git": "https://github.com/a/x.git", "ref": "v1", "subdir": "plugin" } },
                { "name": "y", "source": "owner/repo" },
            ],
        }))
        .unwrap();
        assert_eq!(serde_json::to_value(m.plugins.iter().map(|p| &p.source).collect::<Vec<_>>()).unwrap(), json!(["./plugins/worktrees", { "git": "https://github.com/a/x.git", "ref": "v1", "subdir": "plugin" }, "owner/repo"]));
        assert_eq!(m.plugins[1].description, ""); // none given
        assert_eq!(check_marketplace(&json!({ "name": "acme", "plugins": [], "owner": { "name": "Acme", "email": "a@b.c" } })).unwrap().owner.as_deref(), Some("Acme")); // Claude Code's owner
    }

    #[test]
    fn its_text_cant_write_to_the_terminal() {
        let m = check_marketplace(&json!({
            "name": "acme", "description": "safe\x1b]0;owned\x07 text\u{202e}gpj.exe\nnext line", "owner": "a\x1b[31mred",
            "plugins": [{ "name": "p", "description": "日".repeat(150), "source": "./p" }],
        }))
        .unwrap();
        assert_eq!(m.description.as_deref(), Some("safe textgpj.exe next line"));
        assert_eq!(m.owner.as_deref(), Some("ared"));
        assert_eq!(crate::core::text::width(&m.plugins[0].description), 200);
    }

    #[test]
    fn names_are_ids_each_plugin_named_once() {
        let base = |extra: Value| {
            let mut m = json!({ "name": "acme", "plugins": [] });
            m.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            problems(m)
        };
        assert!(base(json!({ "name": "Acme Inc" })).contains("lowercase letters, digits and dashes"));
        assert!(!base(json!({ "name": "../up" })).is_empty());
        assert!(!base(json!({ "plugins": [{ "name": "a/b", "source": "./x" }] })).is_empty());
        assert!(base(json!({ "plugins": [{ "name": "a", "source": "./x" }, { "name": "a", "source": "./y" }] })).contains("a second plugin named a"));
    }

    #[test]
    fn a_source_never_reaches_git_as_an_option_or_a_helper() {
        assert!(source_problem_in(json!("plugins")).contains(r#"a source is "./path""#)); // neither form
        assert!(source_problem_in(json!("https://github.com/a/b.git")).contains(r#"a source is "./path""#)); // a URL goes in { git }
        assert!(source_problem_in(json!(42)).contains(r#"a source is "./path""#));
        assert!(source_problem_in(json!("./has space")).contains("can't start with - or hold spaces"));
        assert!(source_problem_in(json!({ "git": "ext::sh" })).contains("remote-helper"));
        assert!(source_problem_in(json!({ "git": "ext::sh -c touch% /tmp/pwned" })).contains("can't start with - or hold spaces"));
        assert!(source_problem_in(json!({ "git": "--upload-pack=touch /tmp/x" })).contains("can't start with -"));
        assert!(source_problem_in(json!({ "git": "http://example.com/a.git" })).contains("use https"));
        assert!(source_problem_in(json!({ "git": "https://example.com/a.git", "ref": "--force" })).contains("can't start with -"));
        assert!(source_problem_in(json!({ "git": "https://example.com/a.git", "subdir": "../out" })).contains("relative path inside the repository"));
        assert!(source_problem_in(json!({ "git": "https://example.com/a.git", "subdir": "/etc" })).contains("relative path inside the repository"));
        assert!(source_problem_in(json!({ "git": "https://example.com/a.git\nx" })).contains("control characters"));
        assert_eq!(source_problem_in(json!("./plugins/p")), "");
        assert_eq!(source_problem_in(json!({ "git": "git@github.com:a/b.git", "ref": "main", "subdir": "p" })), "");
    }

    #[test]
    fn owner_repo_means_github() {
        assert_eq!(marketplace_url("acme/plugins"), "https://github.com/acme/plugins.git");
        assert_eq!(marketplace_url("https://example.com/m.git"), "https://example.com/m.git");
        assert_eq!(marketplace_url("git@example.com:m.git"), "git@example.com:m.git");
        assert_eq!(marketplace_url("./local"), "./local"); // not GitHub, and install refuses it as a URL
    }

    #[test]
    fn a_relative_source_resolves_only_inside_the_checkout() {
        let tmp = format!("{}/modisa-marketplace-unit-{}-{}", std::env::temp_dir().display().to_string().trim_end_matches('/'), std::process::id(), crate::server::session::pane::random_hex(4));
        let checkout = format!("{tmp}/checkout");
        std::fs::create_dir_all(format!("{checkout}/plugins/p")).unwrap();
        std::fs::create_dir_all(format!("{tmp}/outside/p")).unwrap();
        std::os::unix::fs::symlink(format!("{tmp}/outside/p"), format!("{checkout}/away")).unwrap();
        std::fs::write(format!("{checkout}/plugins/file"), "not a directory").unwrap();
        let root = real(&checkout);
        assert_eq!(within(&checkout, "./plugins/p").unwrap(), (root.clone(), Some("plugins/p".into())));
        assert_eq!(within(&checkout, "./plugins/../plugins/p/").unwrap(), (root.clone(), Some("plugins/p".into())));
        assert_eq!(within(&checkout, "./").unwrap(), (root, None));
        for escape in ["./../outside/p", "./away", "./plugins/file", "./missing"] {
            assert!(within(&checkout, escape).unwrap_err().message.contains("isn't a directory inside the marketplace's repository"), "{escape}");
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
