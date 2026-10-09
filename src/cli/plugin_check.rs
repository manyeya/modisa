// `modisa plugin check <dir>` and `plugin dev <dir>`: a throwaway session (its own state and config directories,
// with only this plugin linked) to verify a plugin against the real server, or to try it by hand. It isn't a sandbox:
// the plugin runs as you, with your files and network.
use std::ffi::OsString;
use std::process::Stdio;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use regex::Regex;
use serde_json::{json, Value};

use super::commands::{stringify, tpl, truthy};
use super::plugin::{sdk_version, SDK_TEXT};
use crate::config::keys::{modisa_key, DEFAULT_KEYS};
use crate::config::plugin_manage::{mkdtemp, rm_rf};
use crate::config::plugins::{read_manifest, real};
use crate::core::paths::self_exe;
use crate::protocol::conn::{Conn, RpcResult};
use crate::protocol::plugin::PluginManifest;
use crate::protocol::schema::PROTOCOL;
use crate::protocol::transport::connect_unix;
use crate::server::plugins::OwnedGroup;

fn ok(step: &str, note: Option<&str>) {
    outln!("✓ {step}{}", note.map_or(String::new(), |n| format!(": {n}")));
}

fn skip(step: &str, why: &str) {
    outln!("– {step}: skipped ({why})");
}

fn bad(step: &str, detail: &str) -> bool {
    outln!("✗ {step}\n{}", detail.trim_end().split('\n').map(|l| format!("    {l}")).collect::<Vec<_>>().join("\n"));
    false
}

// its last lines, or (empty)
fn tail(file: Option<&str>, lines: usize) -> String {
    let text = file.and_then(|f| std::fs::read(f).ok()).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    let all: Vec<&str> = text.trim_end().split('\n').collect();
    let out = all[all.len().saturating_sub(lines)..].join("\n");
    if out.is_empty() { "(empty)".into() } else { out }
}

fn plugin_dir(arg: &str) -> Option<String> {
    let dir = real(arg);
    if dir.is_empty() {
        errln!("modisa: no such directory: {arg}");
        return None;
    }
    Some(dir)
}

fn tmpdir() -> String {
    std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into())
}

struct Throwaway {
    root: String,
    env: [(&'static str, String); 4],
    child_env: Vec<(OsString, OsString)>,
    session: &'static str,
    sock: String,
    data: String,
}

fn throwaway(dir: &str, name: &str) -> std::io::Result<Throwaway> {
    let root = mkdtemp(&format!("{}/modisa-plugin-", tmpdir()))?;
    let env = [("MODISA_DIR", format!("{root}/state")), ("MODISA_CONFIG_DIR", format!("{root}/config")), ("MODISA_SOUND", "off".into()), ("MODISA_UPDATE_URL", "off".into())];
    std::fs::create_dir_all(&env[0].1)?;
    std::fs::create_dir_all(format!("{}/plugins", env[1].1))?;
    std::os::unix::fs::symlink(dir, format!("{}/plugins/{name}", env[1].1))?;
    let mine: Vec<&str> = env.iter().map(|(k, _)| *k).collect();
    let mut child_env: Vec<(OsString, OsString)> = std::env::vars_os()
        .filter(|(k, _)| !k.to_str().is_some_and(|k| mine.contains(&k) || ["MODISA_SOCKET", "MODISA_PANE_ID", "MODISA_SESSION", "MODISA_CHECK"].contains(&k)))
        .collect();
    child_env.extend(env.iter().map(|(k, v)| (OsString::from(k), OsString::from(v))));
    let session = "check";
    Ok(Throwaway { sock: format!("{}/{session}.sock", env[0].1), data: format!("{}/plugins/{name}", env[0].1), root, env, child_env, session })
}

// a command's exit status and output, run in `dir`
async fn run_in(dir: &str, program: &str, args: &[&str], env: Option<&[(OsString, OsString)]>) -> (i32, String, String) {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args).current_dir(dir).stdin(Stdio::null());
    if let Some(env) = env {
        cmd.env_clear().envs(env.iter().cloned());
    }
    match cmd.output().await {
        Ok(o) => (o.status.code().unwrap_or(-1), String::from_utf8_lossy(&o.stdout).into_owned(), String::from_utf8_lossy(&o.stderr).into_owned()),
        Err(e) => (-1, String::new(), e.to_string()),
    }
}

// every *.test.ts / *.test.js under dir, node_modules and dotfiles aside (as Bun.Glob scans)
fn tests_in(dir: &str) -> Vec<String> {
    let mut out = vec![];
    let mut todo = vec![std::path::PathBuf::from(dir)];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name == "node_modules" {
                continue;
            }
            let path = e.path();
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                todo.push(path);
            } else if name.ends_with(".test.ts") || name.ends_with(".test.js") {
                out.push(path.to_string_lossy().into_owned());
            }
        }
    }
    out
}

static ENTRY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\.[cm]?[jt]sx?$").unwrap());
static PASSED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d+) pass").unwrap());
static FAILURE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\(fail\)|error|Expected|Received|✗").unwrap());

pub async fn check_plugin(arg: &str) -> i32 {
    let Some(dir) = plugin_dir(arg) else { return 2 };
    let manifest = match read_manifest(&dir) {
        Ok(m) => m,
        Err(e) => {
            bad("manifest", &format!("{e}\nplugin.json needs at least: {{ \"name\": \"my-plugin\", \"protocol\": {PROTOCOL}, \"run\": [\"bun\", \"plugin.ts\"] }}"));
            return 1;
        }
    };
    if manifest.protocol != u64::from(PROTOCOL) {
        bad("manifest", &format!("it says protocol {}; this modisa speaks protocol {PROTOCOL}", manifest.protocol));
        return 1;
    }
    ok("manifest", Some(&format!("{}, protocol {}, runs {}", manifest.name, manifest.protocol, manifest.run.join(" "))));
    // a key that's one of modisa's own can never work (another plugin's, or a user's remap, can only be known in a session)
    let keys = manifest.keys.clone().unwrap_or_default();
    let clashes: Vec<String> = keys.iter().filter_map(|k| modisa_key(&k.key, &DEFAULT_KEYS).map(|why| format!("{}: {why}; pick another key in plugin.json", k.key))).collect();
    if !clashes.is_empty() {
        bad("keys", &clashes.join("\n"));
        return 1;
    }
    if !keys.is_empty() {
        ok("keys", Some(&keys.iter().map(|k| k.key.as_str()).collect::<Vec<_>>().join(", ")));
    }
    let mut pass = true;

    let sdk = sdk_version(SDK_TEXT);
    match std::fs::read(format!("{dir}/modisa-plugin.ts")) {
        Err(_) => skip("client library", "the plugin doesn't use modisa-plugin.ts"),
        Ok(lib) => {
            let version = sdk_version(&String::from_utf8_lossy(&lib));
            let shown = |v: Option<u64>| v.map_or("unknown".into(), |v| v.to_string());
            if version == sdk {
                ok("client library", Some(&format!("version {}", shown(version))));
            } else {
                pass = bad("client library", &format!("modisa-plugin.ts is version {}; this modisa's is {}. Refresh it: modisa plugin sdk > modisa-plugin.ts", shown(version), shown(sdk)));
            }
        }
    }

    let entry = manifest.run.iter().find(|a| ENTRY.is_match(a));
    match entry.filter(|_| manifest.run.first().is_some_and(|r| r == "bun")) {
        None => skip("builds", "not started as bun <file>"),
        Some(entry) => {
            let out = format!("{}/modisa-check-build-{}", tmpdir(), std::process::id());
            let (code, stdout, stderr) = run_in(&dir, "bun", &["build", entry, "--target=bun", &format!("--outdir={out}")], None).await;
            rm_rf(&out);
            if code == 0 {
                ok("builds", Some(entry));
            } else {
                pass = bad("builds", &(stderr + &stdout));
            }
        }
    }

    let tsc = format!("{dir}/node_modules/.bin/tsc");
    if !std::path::Path::new(&tsc).is_file() {
        skip("types", "no TypeScript in the plugin; bun add -d typescript @types/bun to check types");
    } else {
        let (code, stdout, _) = run_in(&dir, &tsc, &["--noEmit"], None).await;
        if code == 0 {
            ok("types", None);
        } else {
            pass = bad("types", &stdout);
        }
    }
    if !pass {
        outln!("\nfix the above, then run modisa plugin check again");
        return 1;
    }

    let t = match throwaway(&dir, &manifest.name) {
        Ok(t) => t,
        Err(e) => {
            errln!("modisa: {e}");
            return 1;
        }
    };
    let server_log = format!("{}/server.log", t.root);
    let server = std::fs::File::create(&server_log).and_then(|log| {
        tokio::process::Command::new(self_exe()).args(["server", "-s", t.session]).env_clear().envs(t.child_env.iter().cloned()).current_dir(&t.root).stdin(Stdio::null()).stdout(Stdio::null()).stderr(log).spawn()
    });
    let mut server = match server {
        Ok(s) => s,
        Err(e) => {
            rm_rf(&t.root);
            errln!("modisa: {e}");
            return 1;
        }
    };
    let mut s = Session { group: None, exited: false };
    let outcome = in_session(&t, &dir, &manifest, &mut server, &server_log, &mut s).await;
    // whatever happened: the plugin's group only while it's provably still the plugin's, the server only while it runs
    if !s.exited {
        if let Some(g) = &s.group {
            g.signal(libc::SIGKILL);
        }
    }
    if server.try_wait().ok().flatten().is_none() {
        let _ = server.start_kill();
        let _ = server.wait().await;
    }
    rm_rf(&t.root);
    let pass = match outcome {
        Ok(Some(pass)) => pass,
        Ok(None) => return 1, // the session never started: said so
        Err(e) => {
            errln!("modisa: {e}");
            return 1;
        }
    };
    if pass {
        outln!("\n{} passes", manifest.name);
    } else {
        outln!("\nfix the above, then run modisa plugin check again");
    }
    if pass { 0 } else { 1 }
}

// what the cleanup needs to know
struct Session {
    group: Option<OwnedGroup>,
    exited: bool,
}

// The checks in the throwaway session: whether they passed, or None when the session itself didn't start.
async fn in_session(t: &Throwaway, dir: &str, manifest: &PluginManifest, server: &mut tokio::process::Child, server_log: &str, s: &mut Session) -> RpcResult<Option<bool>> {
    let mut conn: Option<Conn> = None;
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if let Ok(c) = connect_unix(&t.sock, |_, _| {}).await {
            conn = Some(c);
            break;
        }
    }
    let Some(conn) = conn else {
        bad("throwaway session", &format!("the server didn't start:\n{}", tail(Some(server_log), 20)));
        return Ok(None);
    };
    let (c, name) = (&conn, manifest.name.as_str());
    let me = move || async move {
        let list = c.request("plugin.list", json!({}), None).await?;
        RpcResult::Ok(list.as_array().and_then(|ps| ps.iter().find(|p| p["name"] == name)).cloned())
    };
    let mut pass = true;

    let mut state: Option<Value> = None;
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        state = me().await?;
        // starting (not launched yet) and running are still on their way; failed, exited and stopped are final
        if let Some(st) = &state {
            if truthy(st.get("connected")) || (st["status"] != "running" && st["status"] != "starting") {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let pid = |st: &Option<Value>| st.as_ref().and_then(|st| st["pid"].as_i64()).filter(|&p| p != 0);
    if !state.as_ref().is_some_and(|st| truthy(st.get("connected"))) {
        let how = match &state {
            Some(st) => format!(
                "status {}{}{}{}",
                tpl(st.get("status")),
                st.get("exitCode").map_or(String::new(), |c| format!(", exit code {}", tpl(Some(c)))),
                if truthy(st.get("signal")) { format!(", signal {}", tpl(st.get("signal"))) } else { String::new() },
                if truthy(st.get("error")) { format!(": {}", tpl(st.get("error"))) } else { String::new() }
            ),
            None => "modisa didn't find it".into(),
        };
        let log = state.as_ref().and_then(|st| st["log"].as_str().map(String::from));
        pass = bad("starts and connects", &format!("{how}. Within 10s it should connect and call hello (runPlugin and modisa.hello do).\nits log:\n{}", tail(log.as_deref(), 20)));
        if let Some(p) = pid(&state) {
            s.group = Some(OwnedGroup::new(p as i32));
        }
        conn.close();
        return Ok(Some(pass));
    }
    let st = state.clone().unwrap_or_default();
    let actions: Vec<String> = st["actions"].as_array().map(|a| a.iter().map(|x| tpl(Some(x))).collect()).unwrap_or_default();
    ok("starts and connects", Some(&format!("actions: {}", if actions.is_empty() { "none".into() } else { actions.join(", ") })));
    let declared: Vec<String> = manifest.actions.iter().flatten().map(|a| a.id.clone()).collect();
    let missing: Vec<&String> = declared.iter().filter(|id| !actions.contains(id)).collect();
    if !missing.is_empty() {
        let (it, them) = if missing.len() == 1 { ("it", "it") } else { ("them", "them") };
        pass = bad("offers its actions", &format!("plugin.json declares {}, but hello didn't offer {it}: pass {them} to modisa.hello({{ … }})", missing.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")));
    } else if !declared.is_empty() {
        ok("offers its actions", Some(&declared.join(", ")));
    }
    s.group = pid(&state).map(|p| OwnedGroup::new(p as i32));
    tokio::time::sleep(Duration::from_millis(1000)).await;
    let state = me().await?;
    let status = state.as_ref().map_or("undefined".into(), |st| tpl(st.get("status")));
    let connected = state.as_ref().is_some_and(|st| truthy(st.get("connected")));
    if connected && status == "running" {
        ok("stays up", None);
    } else {
        let log = state.as_ref().and_then(|st| st["log"].as_str().map(String::from));
        pass = bad("stays up", &format!("it's {status}{}; its log:\n{}", if connected { "" } else { " and disconnected" }, tail(log.as_deref(), 20)));
    }

    if tests_in(dir).is_empty() {
        skip("its tests", "none: add plugin.test.ts (AGENTS.md shows how)");
    } else {
        let env: serde_json::Map<String, Value> = t.env.iter().map(|(k, v)| (k.to_string(), json!(v))).collect();
        let check = stringify(&json!({ "bin": [self_exe()], "session": t.session, "env": env, "plugin": manifest.name, "data": t.data }), false);
        let mut env = t.child_env.clone();
        env.push(("MODISA_CHECK".into(), check.into()));
        let (code, stdout, stderr) = run_in(dir, "bun", &["test"], Some(&env)).await;
        let output = stdout + &stderr;
        if code == 0 {
            ok("its tests", PASSED.find(&output).map(|m| m.as_str()));
        } else {
            let failures: Vec<&str> = output.split('\n').filter(|l| FAILURE.is_match(l)).take(40).collect();
            let detail = if failures.is_empty() { output.chars().rev().take(4000).collect::<Vec<_>>().into_iter().rev().collect() } else { failures.join("\n") };
            pass = bad("its tests", &detail);
        }
    }

    // The server dies outright, as in a crash: nothing stops the plugin but the plugin itself.
    let _ = server.start_kill();
    let _ = server.wait().await;
    conn.close();
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end {
        s.exited = !s.group.as_ref().is_some_and(OwnedGroup::alive);
        if s.exited {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    if s.exited {
        ok("exits when the session dies", None);
    } else {
        pass = bad("exits when the session dies", "it kept running after its session's server went away. Exit when the connection closes (runPlugin does); otherwise each restart leaves another copy running.");
    }
    Ok(Some(pass))
}

// Each file under a plugin's directory (but node_modules, .git and the like) with when it changed and how big it is:
// what `plugin dev --watch` compares from one second to the next.
fn files_of(dir: &str) -> Vec<(String, Option<std::time::SystemTime>, u64)> {
    let mut out = vec![];
    let mut todo = vec![(std::path::PathBuf::from(dir), 0)];
    while let Some((d, depth)) = todo.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name == "node_modules" || out.len() > 2000 {
                continue;
            }
            match e.metadata() {
                Ok(m) if m.is_dir() && depth < 4 => todo.push((e.path(), depth + 1)),
                Ok(m) if m.is_file() => out.push((e.path().to_string_lossy().into_owned(), m.modified().ok(), m.len())),
                _ => {}
            }
        }
    }
    out.sort();
    out
}

// `modisa plugin validate <dir>`: a plugin checked without running it (TOOLING.md).
pub fn validate(arg: &str, json: bool) -> i32 {
    let Some(dir) = plugin_dir(arg) else { return 2 };
    let mut problems: Vec<String> = vec![];
    let mut notes: Vec<String> = vec![];
    let manifest = read_manifest(&dir);
    if let Err(e) = &manifest {
        problems.push(e.clone());
    }
    let m = manifest.ok();
    let current = sdk_version(SDK_TEXT);
    let vendored = std::fs::read_to_string(format!("{dir}/modisa-plugin.ts")).ok().and_then(|t| sdk_version(&t));
    if let (Some(v), Some(c)) = (vendored, current) {
        if v < c {
            notes.push(format!("its client library is version {v}; this modisa's is {c}: modisa plugin sdk > modisa-plugin.ts"));
        }
    }
    let run = m.as_ref().and_then(|m| m.run.first().cloned());
    let found = run.as_ref().is_some_and(|r| crate::core::paths::which(r).is_some() || std::path::Path::new(&dir).join(r).exists());
    if run.is_some() && !found {
        problems.push(format!("{} isn't on the PATH or in its directory", run.clone().unwrap_or_default()));
    }
    if let Some(m) = &m {
        if m.permissions.is_none() {
            notes.push("it doesn't say what it does through modisa (no permissions): it's treated as allowed everything a plugin can do".into());
        }
    }
    let count = |v: Option<usize>| v.unwrap_or(0);
    if json {
        let r = json!({
            "ok": problems.is_empty(),
            "name": m.as_ref().map(|m| m.name.clone()),
            "permissions": m.as_ref().and_then(|m| m.permissions.clone()),
            "settings": count(m.as_ref().and_then(|m| m.settings.as_ref().map(Vec::len))),
            "actions": count(m.as_ref().and_then(|m| m.actions.as_ref().map(Vec::len))),
            "keys": count(m.as_ref().and_then(|m| m.keys.as_ref().map(Vec::len))),
            "panes": count(m.as_ref().and_then(|m| m.panes.as_ref().map(Vec::len))),
            "links": count(m.as_ref().and_then(|m| m.links.as_ref().map(Vec::len))),
            "sdk": { "vendored": vendored, "current": current },
            "run": { "command": run, "found": found },
            "problems": problems,
            "notes": notes,
        });
        outln!("{}", stringify(&r, true));
    } else {
        if let Some(m) = &m {
            ok("manifest", Some(&format!("{} (protocol {})", m.name, m.protocol)));
            let perms = m.permissions.as_ref().map(|p| if p.is_empty() { "none".to_string() } else { p.join(", ") }).unwrap_or_else(|| "undeclared".into());
            ok("permissions", Some(&perms));
            ok(
                "offers",
                Some(&format!(
                    "{} actions, {} keys, {} panes, {} links, {} settings",
                    count(m.actions.as_ref().map(Vec::len)),
                    count(m.keys.as_ref().map(Vec::len)),
                    count(m.panes.as_ref().map(Vec::len)),
                    count(m.links.as_ref().map(Vec::len)),
                    count(m.settings.as_ref().map(Vec::len))
                )),
            );
        }
        if run.is_some() && found {
            ok("runs", run.as_deref());
        }
        for n in &notes {
            outln!("! {n}");
        }
        for p in &problems {
            bad("problem", p);
        }
    }
    if problems.is_empty() { 0 } else { 1 }
}

pub async fn dev_plugin(arg: &str, watch: bool) -> i32 {
    let Some(dir) = plugin_dir(arg) else { return 2 };
    let manifest = match read_manifest(&dir) {
        Ok(m) => m,
        Err(e) => {
            errln!("modisa: {e}");
            return 1;
        }
    };
    let t = match throwaway(&dir, &manifest.name) {
        Ok(t) => t,
        Err(e) => {
            errln!("modisa: {e}");
            return 1;
        }
    };
    outln!(
        "a throwaway session with {} running. It isn't a sandbox: the plugin runs as you, with your files and network.\nits files (removed when you exit): {}\nfrom another terminal:\n  MODISA_DIR={} MODISA_CONFIG_DIR={} modisa -s {} plugin logs {}",
        manifest.name, t.root, t.env[0].1, t.env[1].1, t.session, manifest.name
    );
    tokio::time::sleep(Duration::from_millis(1500)).await;
    // --watch: a change to its files restarts it in the throwaway session
    let watcher = watch.then(|| {
        let (dir, env, session, name) = (dir.clone(), t.child_env.clone(), t.session, manifest.name.clone());
        tokio::task::spawn_local(async move {
            let mut last = files_of(&dir);
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let now = files_of(&dir);
                if now == last {
                    continue;
                }
                last = now;
                for verb in ["stop", "start"] {
                    let _ = tokio::process::Command::new(self_exe()).args(["-s", session, "plugin", verb, &name]).env_clear().envs(env.iter().cloned()).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().await;
                }
            }
        })
    });
    let attached = tokio::process::Command::new(self_exe()).args(["-s", t.session]).env_clear().envs(t.child_env.iter().cloned()).current_dir(&dir).status().await;
    if let Some(w) = watcher {
        w.abort();
    }
    let _ = tokio::process::Command::new(self_exe()).args(["kill", t.session]).env_clear().envs(t.child_env.iter().cloned()).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().await;
    rm_rf(&t.root);
    attached.map_or(1, |s| s.code().unwrap_or(1))
}
