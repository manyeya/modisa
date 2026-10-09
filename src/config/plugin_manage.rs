// Installing, updating and unlinking plugins, as results: the CLI prints them (cli/plugin_install.rs) and the server
// answers the plugin manager's requests with them (server/plugin_manager.rs). Each acts in the running sessions it
// reaches: over their sockets, or, in a session's own server, straight through its plugin host (`Here`).
//
// An install clones into a staging directory, resolves the ref to a commit, checks the directory and plugin.json, and
// only then takes the name: the checkout moves to <state>/plugins-src/<name>, beside modisa's install record, and is
// linked exactly like `plugin link`, for the caller to start. No dependencies are installed and no build scripts run.
// A failure before the name is taken leaves nothing behind.
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use regex::Regex;
use serde::Serialize;
use serde_json::{json, Value};

use super::marketplaces::{resolve_entry, update_marketplaces, MARKETPLACES_DIR};
use super::plugins::{inside, is_dir, read_install, read_manifest, real, without_credentials, write_install, InstallRecord, MANAGED_DIR, PLUGINS_DIR};
use crate::cli::commands::{tpl, truthy};
use crate::cli::plugin_git::{checkout_ref, fetch_latest, git, remote_commit, source_problem};
use crate::core::paths::{socket_path, which, DIR};
use crate::protocol::conn::{error, fail, Conn, RpcResult};
use crate::protocol::transport::{connect_existing, connect_unix, running_pid};

// Requests to a session, and the session they reach
pub type Ask = Rc<dyn Fn(String, Value) -> Pin<Box<dyn Future<Output = RpcResult<Value>>>>>;

#[derive(Clone)]
pub struct Here {
    pub session: String,
    pub ask: Ask,
}

// a session's requests over a connection to it
pub fn conn_ask(conn: &Conn) -> Ask {
    let conn = conn.clone();
    Rc::new(move |method, params| {
        let conn = conn.clone();
        Box::pin(async move { conn.request(&method, params, None).await })
    })
}

async fn ask(a: &Ask, method: &str, params: Value) -> RpcResult<Value> {
    a(method.to_string(), params).await
}

pub fn session_name(session: Option<&str>) -> String {
    session.map(String::from).or_else(|| std::env::var("MODISA_SESSION").ok()).unwrap_or_else(|| "default".into())
}

// plugin.json names are ids, so a name that isn't one never makes it into a path
static PLUGIN_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z0-9][a-z0-9-]*$").unwrap());
fn named(name: &str) -> RpcResult<()> {
    if PLUGIN_NAME.is_match(name) {
        return Ok(());
    }
    let short: String = name.chars().take(64).collect();
    Err(fail("no_such_plugin", format!("no plugin named {}: a plugin's name is lowercase letters, digits and dashes", serde_json::to_string(&short).unwrap())))
}

// ---------- small things the original asked the shell for ----------

// new Date().toISOString()
pub fn iso_now() -> String {
    iso_at(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64))
}

fn iso_at(ms: i64) -> String {
    let (secs, millis) = (ms.div_euclid(1000), ms.rem_euclid(1000));
    let (days, sod) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // the civil date of a day count (Howard Hinnant's days_from_civil, inverted)
    let z = days + 719_468;
    let (era, doe) = (z.div_euclid(146_097), z.rem_euclid(146_097));
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z", sod / 3600, sod % 3600 / 60, sod % 60)
}

// mktemp -d <prefix>XXXXXX: a new directory only this process made
pub fn mkdtemp(prefix: &str) -> std::io::Result<String> {
    use std::os::unix::fs::DirBuilderExt;
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    loop {
        let mut b = [0u8; 6];
        let _ = getrandom::fill(&mut b);
        let path = format!("{prefix}{}", b.iter().map(|x| CHARS[*x as usize % CHARS.len()] as char).collect::<String>());
        match std::fs::DirBuilder::new().mode(0o700).create(&path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
}

// rm -rf, quietly
pub fn rm_rf(path: &str) {
    let _ = std::fs::remove_dir_all(path).or_else(|_| std::fs::remove_file(path));
}

fn readlink(path: &str) -> String {
    std::fs::read_link(path).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
}

fn short(commit: &str) -> &str {
    &commit[..commit.len().min(7)]
}

// ---------- starting ----------
// Starting a plugin in the one session a command reaches: started (and connected), already running, not started (no
// session running), failed (it didn't start, or exited), or no-hello (started, but never connected in time).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartOutcome {
    pub session: String,
    pub state: &'static str, // started | already-running | not-started | failed | no-hello
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_keys: Option<Vec<String>>, // "<key>: <why>"
}

impl StartOutcome {
    pub fn new(session: &str, state: &'static str) -> StartOutcome {
        StartOutcome { session: session.into(), state, reason: None, pid: None, log: None, disabled_keys: None }
    }
}

const HELLO_MS: u64 = 10_000;

// "<key>: <why>" for each of a plugin's keys that's off in that session
fn off_keys(status: &Value) -> Vec<String> {
    let keys = status["keys"].as_array().cloned().unwrap_or_default();
    keys.iter()
        .filter(|k| k["state"] == "disabled")
        .map(|k| format!("{}: {}", if truthy(k.get("key")) { tpl(k.get("key")) } else { "(none)".into() }, tpl(k.get("reason"))))
        .collect()
}

fn pid_of(s: &Value) -> Option<i64> {
    s["pid"].as_i64()
}

fn log_of(s: &Value) -> Option<String> {
    s["log"].as_str().map(String::from)
}

async fn listed(a: &Ask, name: &str) -> RpcResult<Option<Value>> {
    let list = ask(a, "plugin.list", json!({})).await?;
    Ok(list.as_array().and_then(|ps| ps.iter().find(|p| p["name"] == name).cloned()))
}

// Start a linked plugin in a session, and wait for it to connect: a process that started isn't a plugin that's ready.
pub async fn start_with(a: &Ask, at: &str, name: &str) -> RpcResult<StartOutcome> {
    let status = match ask(a, "plugin.start", json!({ "name": name })).await {
        Ok(s) => s,
        Err(e) if e.code == "already_running" => {
            let s = listed(a, name).await?.unwrap_or(Value::Null);
            return Ok(StartOutcome { pid: pid_of(&s), log: log_of(&s), disabled_keys: Some(off_keys(&s)), ..StartOutcome::new(at, "already-running") });
        }
        Err(e) => return Ok(StartOutcome { reason: Some(e.message), ..StartOutcome::new(at, "failed") }),
    };
    let end = Instant::now() + Duration::from_millis(HELLO_MS);
    loop {
        let s = listed(a, name).await?.unwrap_or_else(|| status.clone());
        if truthy(s.get("connected")) {
            return Ok(StartOutcome { pid: pid_of(&s), log: log_of(&s), disabled_keys: Some(off_keys(&s)), ..StartOutcome::new(at, "started") });
        }
        if s["status"] != "running" && s["status"] != "starting" {
            let reason = match s.get("error").filter(|e| !e.is_null()) {
                Some(e) => tpl(Some(e)),
                None => format!("it {}", tpl(s.get("status"))),
            };
            return Ok(StartOutcome { reason: Some(reason), pid: pid_of(&s), log: log_of(&s), ..StartOutcome::new(at, "failed") });
        }
        if Instant::now() > end {
            return Ok(StartOutcome { reason: Some(format!("it started but didn't connect within {}s", HELLO_MS / 1000)), pid: pid_of(&s), log: log_of(&s), ..StartOutcome::new(at, "no-hello") });
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

// stopped and started again where it runs; nothing where it doesn't
async fn restart_with(a: &Ask, at: &str, name: &str) -> RpcResult<Option<StartOutcome>> {
    let list = ask(a, "plugin.list", json!({})).await.unwrap_or(json!([]));
    let running = list.as_array().is_some_and(|ps| ps.iter().any(|p| p["name"] == name && (p["status"] == "running" || p["status"] == "starting")));
    if !running {
        return Ok(None);
    }
    let _ = ask(a, "plugin.stop", json!({ "name": name })).await;
    start_with(a, at, name).await.map(Some)
}

// running sessions modisa can see: each socket in the state directory (as `modisa ls` finds them)
pub fn sessions() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(&*DIR)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|f| !f.starts_with('.'))
        .filter_map(|f| f.strip_suffix(".sock").map(String::from))
        .collect();
    names.sort();
    names
}

// `act` in every running session: `here` through its own host, the rest over their sockets. What each act returned,
// and those running but unreachable (a sandbox?).
async fn every_session<T, F, Fut>(here: Option<&Here>, act: F) -> RpcResult<(Vec<T>, Vec<String>)>
where
    F: Fn(String, Ask) -> Fut,
    Fut: Future<Output = RpcResult<T>>,
{
    let mut names: Vec<String> = here.map(|h| h.session.clone()).into_iter().collect();
    for s in sessions() {
        if !names.contains(&s) {
            names.push(s);
        }
    }
    let (mut out, mut unreachable) = (vec![], vec![]);
    for s in names {
        if let Some(h) = here.filter(|h| h.session == s) {
            out.push(act(s, h.ask.clone()).await?);
            continue;
        }
        let sock = socket_path(&s);
        let Ok(conn) = connect_unix(&sock, |_, _| {}).await else {
            if running_pid(&sock).is_some() {
                unreachable.push(s);
            }
            continue;
        };
        let r = act(s, conn_ask(&conn)).await;
        conn.close();
        out.push(r?);
    }
    Ok((out, unreachable))
}

// ---------- installing ----------
// installed: the checkout, record and link are in place (then `start` says whether it started). Not installed:
// `stage` and `reason` say where it failed, and nothing was left behind.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallResult {
    pub installed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub already_installed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub source: String, // without credentials
    pub r#ref: Option<String>, // as requested; null: the default branch
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>, // what the ref resolved to
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>, // the plugin's directory (the checkout, or --subdir inside it)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<&'static str>, // source | git | clone | ref | subdir | manifest | collision | marketplace
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marketplace: Option<String>, // installed as <name>@<marketplace>
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<StartOutcome>, // set by whoever starts it, after the install
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hints: Option<Vec<String>>,
}

// a git URL with a ref and a subdir, or a marketplace's entry (name@marketplace), which has its own
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InstallFrom {
    pub source: Option<String>,
    pub marketplace_plugin: Option<String>,
    pub r#ref: Option<String>,
    pub subdir: Option<String>,
}

// setup modisa doesn't do for you; its own wording, never commands from the plugin
fn setup_hints(dir: &str, name: &str) -> Vec<String> {
    let pkg: Option<Value> = std::fs::read(format!("{dir}/package.json")).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let deps: std::collections::HashSet<String> = pkg.iter().flat_map(|p| ["dependencies", "devDependencies"].map(|k| p.get(k).and_then(Value::as_object).map(|m| m.keys().cloned().collect::<Vec<_>>()).unwrap_or_default())).flatten().collect();
    if !deps.is_empty() && !is_dir(&format!("{dir}/node_modules")) {
        vec![format!("it has package.json dependencies, which install doesn't fetch: run bun install in {dir}, then modisa plugin start {name}")]
    } else {
        vec![]
    }
}

// The plugin's directory, and its plugin.json, must really be inside the checkout (no .., no symlink out): the
// directory and the plugin's name, or the stage and why not.
fn checked(checkout: &str, subdir: Option<&str>) -> Result<(String, String), (&'static str, String)> {
    let root = real(checkout);
    let dir = real(&subdir.map_or(checkout.to_string(), |s| format!("{checkout}/{s}")));
    if dir.is_empty() || !inside(&dir, &root) || !is_dir(&dir) {
        return Err(("subdir", format!("{} isn't a directory inside the repository", subdir.unwrap_or("."))));
    }
    let manifest_file = real(&format!("{dir}/plugin.json"));
    if !manifest_file.is_empty() && !inside(&manifest_file, &root) {
        return Err(("manifest", "plugin.json points outside the repository".into()));
    }
    let manifest = read_manifest(&dir).map_err(|e| ("manifest", e))?;
    Ok((dir, manifest.name))
}

struct Target {
    url: String,
    r#ref: Option<String>,
    subdir: Option<String>,
    from: String,
    name: Option<String>,
    marketplace: Option<String>,
}

fn target(o: &InstallFrom) -> RpcResult<Target> {
    if let Some(spec) = &o.marketplace_plugin {
        let t = resolve_entry(spec)?;
        return Ok(Target { url: t.url, r#ref: t.r#ref, subdir: t.subdir, from: t.from, name: Some(t.name), marketplace: Some(t.marketplace) });
    }
    let source = o.source.clone().unwrap_or_default();
    Ok(Target { from: without_credentials(&source), url: source, r#ref: o.r#ref.clone(), subdir: o.subdir.clone(), name: None, marketplace: None })
}

// source: what install fetches (for a plugin inside a marketplace, its checkout here); from: where the code comes
// from, as you'd know it. commit: what the ref is at now (null: an abbreviated commit, only known once fetched)
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Resolution {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marketplace: Option<String>,
    pub source: String,
    pub from: String,
    pub r#ref: Option<String>,
    pub subdir: Option<String>,
    pub commit: Option<String>,
}

// Where an install would fetch from, and the commit its ref is at now: what the user is shown before installing.
// Nothing is fetched or written.
pub async fn resolve_plugin(o: &InstallFrom) -> RpcResult<Resolution> {
    let t = target(o)?;
    let commit = remote_commit(&t.url, t.r#ref.as_deref()).await.map_err(|reason| error(without_credentials(&reason)))?;
    Ok(Resolution { name: t.name, marketplace: t.marketplace, source: without_credentials(&t.url), from: t.from, r#ref: t.r#ref, subdir: t.subdir, commit })
}

// Install it and link it (not start it). commit: what the user was shown; the install fails if the ref has moved.
// cloning: told the source once it's been checked, before git runs.
pub async fn install_plugin(o: &InstallFrom, commit: Option<&str>, cloning: Option<&dyn Fn(&str)>) -> RpcResult<InstallResult> {
    let Some(spec) = &o.marketplace_plugin else {
        let fetch = Fetch { r#ref: o.r#ref.clone(), subdir: o.subdir.clone(), name: None, marketplace: None, commit, cloning };
        return fetch_plugin(o.source.as_deref().unwrap_or_default(), fetch).await;
    };
    match resolve_entry(spec) {
        Ok(t) => fetch_plugin(&t.url, Fetch { r#ref: t.r#ref, subdir: t.subdir, name: Some(t.name), marketplace: Some(t.marketplace), commit, cloning }).await,
        Err(e) => Ok(InstallResult { source: spec.clone(), stage: Some("marketplace"), reason: Some(e.message), ..Default::default() }),
    }
}

struct Fetch<'a> {
    r#ref: Option<String>,
    subdir: Option<String>,
    name: Option<String>,
    marketplace: Option<String>,
    commit: Option<&'a str>,
    cloning: Option<&'a dyn Fn(&str)>,
}

async fn fetch_plugin(url: &str, o: Fetch<'_>) -> RpcResult<InstallResult> {
    let source = without_credentials(url);
    let r#ref = o.r#ref.clone();
    let mut staging: Option<String> = None;
    let failed = |staging: &mut Option<String>, stage: &'static str, reason: &str| {
        if let Some(s) = staging.take() {
            rm_rf(&s); // this attempt's files only
        }
        InstallResult { source: source.clone(), r#ref: r#ref.clone(), stage: Some(stage), reason: Some(without_credentials(reason)), marketplace: o.marketplace.clone(), ..Default::default() }
    };

    if let Some(unsupported) = source_problem(url) {
        return Ok(failed(&mut staging, "source", &unsupported));
    }
    if let Some(cloning) = o.cloning {
        cloning(&source);
    }
    if which("git").is_none() {
        return Ok(failed(&mut staging, "git", "git isn't installed"));
    }
    if let Some(r) = r#ref.as_deref().filter(|r| r.starts_with('-')) {
        return Ok(failed(&mut staging, "ref", &format!("not a ref: {r}")));
    }
    let subdir = o.subdir.as_deref().map(|s| s.trim_end_matches('/').to_string()).filter(|s| !s.is_empty());
    if let Some(s) = subdir.as_deref().filter(|s| s.starts_with('/') || s.split('/').any(|p| p == "..")) {
        return Ok(failed(&mut staging, "subdir", &format!("--subdir must be a relative path inside the repository: {s}")));
    }

    std::fs::create_dir_all(&*MANAGED_DIR)?;
    staging = Some(mkdtemp(&format!("{}/.staging-", *MANAGED_DIR))?);
    let checkout = format!("{}/checkout", staging.as_deref().unwrap());
    let cloned = git(&["clone", "--quiet", "--", url, &checkout], None).await;
    if cloned.code != 0 {
        return Ok(failed(&mut staging, "clone", if cloned.err.is_empty() { "git clone failed" } else { &cloned.err }));
    }
    git(&["remote", "set-url", "origin", &source], Some(&checkout)).await; // no credentials left in the checkout's own config
    let commit = match checkout_ref(&checkout, r#ref.as_deref(), &source).await {
        Ok(c) => c,
        Err(reason) => return Ok(failed(&mut staging, "ref", &reason)),
    };
    if let Some(shown) = o.commit.filter(|c| !c.is_empty() && *c != commit) {
        let reason = format!("{} of {source} is at {} now, not {} as shown before installing: look at it again", r#ref.as_deref().unwrap_or("the default branch"), short(&commit), short(shown));
        return Ok(failed(&mut staging, "ref", &reason));
    }

    let name = match checked(&checkout, subdir.as_deref()) {
        Ok((_, name)) => name,
        Err((stage, reason)) => return Ok(failed(&mut staging, stage, &reason)),
    };
    if let Some(listed) = o.name.as_deref().filter(|n| *n != name) {
        let reason = format!("marketplace {} lists it as {listed}, but its plugin.json names it {name}", o.marketplace.as_deref().unwrap_or("undefined"));
        return Ok(failed(&mut staging, "manifest", &reason));
    }

    let existing = read_install(&name);
    let link = format!("{}/{name}", *PLUGINS_DIR);
    if let Some(existing) = existing.as_ref().filter(|e| e.source == source && e.subdir == subdir) {
        rm_rf(staging.as_deref().unwrap());
        return Ok(InstallResult {
            already_installed: Some(true),
            name: Some(name),
            source,
            r#ref: existing.r#ref.clone(),
            commit: Some(existing.commit.clone()),
            checkout: Some(existing.checkout.clone()),
            dir: Some(existing.dir.clone()),
            marketplace: o.marketplace.clone(),
            hints: Some(vec![]),
            ..Default::default()
        });
    }
    let linked_to = readlink(&link);
    if let Some(existing) = existing {
        return Ok(failed(&mut staging, "collision", &format!("{name} is already installed from {}; modisa plugin unlink {name} first", existing.source)));
    }
    if !linked_to.is_empty() {
        return Ok(failed(&mut staging, "collision", &format!("{name} is already linked to {linked_to}; modisa plugin unlink {name} first")));
    }

    // Take the name. mkdir and ln -s both refuse to overwrite, so two installs racing for it can't both get it.
    let home = format!("{}/{name}", *MANAGED_DIR);
    if std::fs::create_dir(&home).is_err() {
        return Ok(failed(&mut staging, "collision", &format!("{name} is already being installed, or left behind at {home}")));
    }
    std::fs::rename(&checkout, format!("{home}/checkout"))?;
    rm_rf(&staging.take().unwrap());
    let final_checkout = real(&format!("{home}/checkout"));
    let final_dir = real(&subdir.as_deref().map_or(final_checkout.clone(), |s| format!("{final_checkout}/{s}")));
    let record = InstallRecord {
        name: name.clone(),
        source: source.clone(),
        r#ref: r#ref.clone(),
        commit: commit.clone(),
        checkout: final_checkout.clone(),
        dir: final_dir.clone(),
        subdir,
        installed_at: iso_now(),
        updated_at: None,
        marketplace: o.marketplace.clone(),
    };
    write_install(&record)?;
    std::fs::create_dir_all(&*PLUGINS_DIR)?;
    if std::os::unix::fs::symlink(&final_dir, &link).is_err() {
        rm_rf(&home); // ours: taken above, nothing else uses it
        return Ok(failed(&mut None, "collision", &format!("{name} was linked by something else during the install")));
    }
    Ok(InstallResult {
        installed: true,
        name: Some(name.clone()),
        source,
        r#ref,
        commit: Some(commit),
        checkout: Some(final_checkout),
        hints: Some(setup_hints(&final_dir, &name)),
        dir: Some(final_dir),
        marketplace: o.marketplace.clone(),
        ..Default::default()
    })
}

// ---------- updating ----------
// An installed plugin moved to what its ref (none: its source's HEAD) is at now, its directory and plugin.json checked
// again, then restarted in each running session that ran it (`restarted`). Not updated: upToDate, or `stage` and
// `reason` say why, and it's as it was. A plugin you linked is never updated (stage linked).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateResult {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#ref: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>, // the commit it was at
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marketplace: Option<String>,
    pub updated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub up_to_date: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>, // the commit it's at now
    pub restarted: Vec<StartOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hints: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<&'static str>, // linked | marketplace | fetch | checkout | subdir | manifest
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

// A plugin inside a marketplace's repository comes from its checkout here, so that's brought up to date first.
// Anything wrong at the new commit puts the old one back.
pub async fn update_plugin(name: &str, here: Option<&Here>) -> RpcResult<UpdateResult> {
    named(name)?;
    let linked = real(&format!("{}/{name}", *PLUGINS_DIR));
    if linked.is_empty() {
        return Err(fail("no_such_plugin", format!("no linked plugin named {name} in {}", *PLUGINS_DIR)));
    }
    let Some(record) = read_install(name).filter(|r| r.dir == linked) else {
        return Ok(UpdateResult { name: name.into(), stage: Some("linked"), reason: Some(format!("linked from {linked}: update it there")), ..Default::default() });
    };
    let base = UpdateResult { name: name.into(), source: Some(record.source.clone()), r#ref: Some(record.r#ref.clone()), from: Some(record.commit.clone()), marketplace: record.marketplace.clone(), ..Default::default() };
    let failed = |stage: &'static str, reason: &str| UpdateResult { commit: Some(record.commit.clone()), stage: Some(stage), reason: Some(without_credentials(reason)), ..base.clone() };

    if let Some(market) = record.marketplace.as_deref().filter(|m| record.source == format!("file://{}", real(&format!("{}/{m}", *MARKETPLACES_DIR)))) {
        let reason = match update_marketplaces(Some(market)).await {
            Ok(list) => list.into_iter().next().and_then(|m| m.reason).filter(|r| !r.is_empty()),
            Err(e) => Some(e.message).filter(|r| !r.is_empty()),
        };
        if let Some(reason) = reason {
            return Ok(failed("marketplace", &format!("marketplace {market}: {reason}")));
        }
    }
    let latest = match fetch_latest(&record.checkout, &record.source, record.r#ref.as_deref()).await {
        Ok(c) => c,
        Err(reason) => return Ok(failed("fetch", &reason)),
    };
    if latest == record.commit {
        return Ok(UpdateResult { up_to_date: Some(true), commit: Some(record.commit.clone()), ..base });
    }
    let moved = git(&["checkout", "--quiet", "--detach", &latest], Some(&record.checkout)).await;
    if moved.code != 0 {
        return Ok(failed("checkout", &if moved.err.is_empty() { format!("couldn't check out {latest}") } else { moved.err }));
    }
    let problem = match checked(&record.checkout, record.subdir.as_deref()) {
        Err(p) => Some(p),
        Ok((dir, _)) if dir != record.dir => Some(("subdir", format!("{} is somewhere else at {}", record.subdir.as_deref().unwrap_or("."), short(&latest)))),
        Ok((_, n)) if n != name => Some(("manifest", format!("at {} its plugin.json names it {n}: unlink it and install that", short(&latest)))),
        Ok(_) => None,
    };
    if let Some((stage, reason)) = problem {
        git(&["checkout", "--quiet", "--detach", &record.commit], Some(&record.checkout)).await;
        return Ok(failed(stage, &reason));
    }
    write_install(&InstallRecord { commit: latest.clone(), updated_at: Some(iso_now()), ..record.clone() })?;
    let (restarted, _) = every_session(here, |s, a| async move { restart_with(&a, &s, name).await }).await?;
    Ok(UpdateResult { updated: true, commit: Some(latest), restarted: restarted.into_iter().flatten().collect(), hints: Some(setup_hints(&record.dir, name)), ..base })
}

// ---------- unlinking ----------
// managed: installed with `plugin install` (stopped in every reachable session; its checkout deleted only if none
// still runs it). Otherwise a directory you linked: stopped in the session reached, and never deleted.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnlinkResult {
    pub name: String,
    pub unlinked: bool,
    pub managed: bool,
    pub stopped_in: Vec<String>,
    pub still_using: Vec<String>,
    pub unreachable: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkout: Option<Checkout>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Checkout {
    pub path: String,
    pub deleted: bool,
}

async fn stop(a: &Ask, name: &str) -> bool {
    ask(a, "plugin.stop", json!({ "name": name })).await.is_ok()
}

// The link goes, so no session starts it again. Installed by modisa: stopped in every running session, then its
// checkout and install record deleted, unless a session still runs it or can't be reached. Linked by you: stopped in
// the session reached (here, else the default or `session`), and never deleted.
pub async fn unlink_plugin(name: &str, session: Option<&str>, here: Option<&Here>) -> RpcResult<UnlinkResult> {
    named(name)?;
    let link = format!("{}/{name}", *PLUGINS_DIR);
    if !std::fs::symlink_metadata(&link).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(fail("no_such_plugin", format!("no linked plugin named {name} in {}", *PLUGINS_DIR)));
    }
    let record = read_install(name);
    let managed = record.as_ref().is_some_and(|r| real(&link) == r.dir);
    std::fs::remove_file(&link)?; // the link only
    let mut result = UnlinkResult { name: name.into(), unlinked: true, managed, stopped_in: vec![], still_using: vec![], unreachable: vec![], checkout: None };

    let Some(record) = record.filter(|_| managed) else {
        let at = here.map_or_else(|| session_name(session), |h| h.session.clone());
        let conn = match here {
            Some(_) => None,
            None => connect_existing(session, |_, _| {}).await.ok(),
        };
        let a = here.map(|h| h.ask.clone()).or_else(|| conn.as_ref().map(conn_ask));
        if let Some(a) = a {
            if stop(&a, name).await {
                result.stopped_in.push(at);
            }
        }
        if let Some(c) = conn {
            c.close();
        }
        return Ok(result);
    };
    let (seen, unreachable) = every_session(here, |s, a| async move {
        let stopped = stop(&a, name).await;
        let list = ask(&a, "plugin.list", json!({})).await.unwrap_or(json!([]));
        let still = list.as_array().is_some_and(|ps| ps.iter().any(|p| p["name"] == name && p["group"] == "running"));
        Ok::<_, crate::protocol::conn::RpcError>((s, stopped, still))
    })
    .await?;
    for (s, stopped, still) in seen {
        if stopped {
            result.stopped_in.push(s.clone());
        }
        if still {
            result.still_using.push(s);
        }
    }
    result.unreachable = unreachable;
    let keep = !result.unreachable.is_empty() || !result.still_using.is_empty();
    if !keep {
        rm_rf(&format!("{}/{name}", *MANAGED_DIR)); // the checkout and install record; never its data or logs
    }
    result.checkout = Some(Checkout { path: record.checkout, deleted: !keep });
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_dates_read_like_javascripts() {
        let now = iso_now();
        assert_eq!(now.len(), 24, "{now}");
        assert!(now.ends_with('Z') && now.as_bytes()[10] == b'T', "{now}");
        assert!(now.as_str() > "2025-01-01T00:00:00.000Z");
        for (ms, js) in [(0, "1970-01-01T00:00:00.000Z"), (951_782_400_000, "2000-02-29T00:00:00.000Z"), (1_791_504_000_123, "2026-10-09T00:00:00.123Z"), (4_102_444_799_999, "2099-12-31T23:59:59.999Z")] {
            assert_eq!(iso_at(ms), js);
        }
    }

    #[test]
    fn names_that_arent_ids_never_reach_a_path() {
        assert!(named("demo-1").is_ok());
        let e = named("../x").unwrap_err();
        assert_eq!((e.code.as_str(), e.message.as_str()), ("no_such_plugin", r#"no plugin named "../x": a plugin's name is lowercase letters, digits and dashes"#));
    }
}
