// The `modisa plugin` commands that work without a session server (new, sdk, schema, check, dev, search, marketplace),
// and link, install, update and unlink, which change what's linked and then act on the running sessions they reach.
use std::io::Write as _;

use serde_json::Value;

use super::args::Args;
use super::commands::stringify;
use crate::config::plugin_manage::{conn_ask, session_name, start_with, StartOutcome};
use crate::config::plugins::{read_manifest, real, PLUGINS_DIR};
use crate::protocol::conn::RpcResult;
use crate::protocol::schema::{DESCRIBE, PROTOCOL};
use crate::protocol::transport::connect_existing;

// The client library plugins vendor, and what `plugin new` writes: TypeScript files, kept as they are.
pub const SDK_TEXT: &str = include_str!("../plugins/modisa-plugin.ts");
const PLUGIN: &str = include_str!("../plugins/template/plugin.ts.txt");
const TEST: &str = include_str!("../plugins/template/plugin.test.ts.txt");
const GUIDE: &str = include_str!("../plugins/template/AGENTS.md");

// its SDK_VERSION, read from the text
pub fn sdk_version(text: &str) -> Option<u64> {
    let at = text.find("SDK_VERSION = ")? + "SDK_VERSION = ".len();
    let digits: String = text[at..].chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok().filter(|&v| v != 0)
}

// The plugin commands main.rs's dispatch sends here (cli/mod.rs is_local_plugin_command): the rest go through a server.
pub async fn run_plugin_local(verb: &str, args: &[&str], a: &Args) -> i32 {
    let json = a.on("json");
    let arg = args.first().copied();
    match verb {
        "sdk" => {
            let _ = std::io::stdout().write_all(SDK_TEXT.as_bytes());
            0
        }
        "schema" => {
            outln!("{}", stringify(&serde_json::from_str::<Value>(DESCRIBE).unwrap_or_default(), true));
            0
        }
        "new" => scaffold(arg, a.str("dir")),
        "check" => super::plugin_check::check_plugin(arg.unwrap_or(".")).await,
        "dev" => super::plugin_check::dev_plugin(arg.unwrap_or("."), a.switch("watch")).await,
        "validate" => super::plugin_check::validate(arg.unwrap_or("."), json),
        "grant" | "revoke" => grant(verb, arg, &args[1.min(args.len())..]),
        "link" => link(arg, a.str("session"), json).await,
        "install" => super::plugin_install::install(arg, a.str("ref"), a.str("subdir"), a.str("session"), json).await,
        "update" => super::plugin_install::update(arg, json).await,
        "search" => super::plugin_search::search(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>(), json).await,
        "marketplace" => super::plugin_marketplace::marketplace(args, a).await,
        _ => super::plugin_install::unlink(arg, a.str("session"), json).await,
    }
}

// What each permission lets a plugin do, as `link` and `install` say it before it starts.
fn means(p: &str) -> &'static str {
    match p {
        "ui" => "show things in modisa's screen",
        "ui.replace" => "ask to draw instead of modisa's own widgets",
        "panes.read" => "read what's in your panes",
        "panes.control" => "type and run commands in panes, and open, close and move them",
        "agents" => "start agents",
        "messages" => "message agents and read their inbox",
        "notify" => "send system notifications and sounds",
        "sessions" => "create, rename and close spaces and tabs",
        _ => "something this modisa doesn't know",
    }
}

// Before a plugin starts: what it asks to do through modisa, and the user's answer recorded (TOOLING.md, Permissions).
// At a terminal it asks; a script (nobody to ask) gets what the plugin declares, which is what linking it means.
pub fn consent(m: &crate::protocol::plugin::PluginManifest, json: bool) {
    use std::io::IsTerminal;
    let Some(perms) = &m.permissions else {
        if !json {
            outln!("{} doesn't say what it does through modisa (no permissions in its plugin.json): it may do anything a plugin can", m.name);
        }
        return;
    };
    if !json && !perms.is_empty() {
        outln!("{} asks to:\n{}", m.name, perms.iter().map(|p| format!("  · {} ({p})", means(p))).collect::<Vec<_>>().join("\n"));
    }
    let yes = if !json && !perms.is_empty() && std::io::stdin().is_terminal() {
        eprint!("Allow? [Y/n] ");
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer).is_ok() && !answer.trim().eq_ignore_ascii_case("n")
    } else {
        true
    };
    let saved = crate::config::grants::revoke(&m.name, &[]).and_then(|_| if yes { crate::config::grants::grant(&m.name, perms) } else { Ok(()) });
    if let Err(e) = saved {
        errln!("modisa: {}: {e}", crate::config::grants::path());
    } else if !yes {
        outln!("not granted: it starts, but can't do those. modisa plugin grant {} when you change your mind", m.name);
    }
}

// modisa plugin grant <name> [permission…] (none: all it asks for) | revoke <name> [permission…] (none: all)
fn grant(verb: &str, name: Option<&str>, perms: &[&str]) -> i32 {
    let Some(name) = name else {
        errln!("usage: modisa plugin {verb} <name> [permission…]");
        return 2;
    };
    let mut perms: Vec<String> = perms.iter().map(|s| s.to_string()).collect();
    if let Some(p) = perms.iter().find(|p| !crate::protocol::plugin::PERMISSIONS.contains(&p.as_str())) {
        errln!("modisa: {p} isn't a permission: {}", crate::protocol::plugin::PERMISSIONS.join(", "));
        return 2;
    }
    if verb == "grant" && perms.is_empty() {
        let declared = crate::config::plugins::linked_plugins().into_iter().find(|l| l.name == name).and_then(|l| read_manifest(&l.dir).ok()).and_then(|m| m.permissions);
        perms = declared.unwrap_or_default();
    }
    let done = if verb == "grant" { crate::config::grants::grant(name, &perms) } else { crate::config::grants::revoke(name, &perms) };
    match done {
        Ok(()) => {
            let have = crate::config::grants::read().get(name).cloned().unwrap_or_default();
            outln!("{name} may now: {} (modisa plugin restart {name} for a running one to have it)", if have.is_empty() { "nothing beyond what every plugin can".into() } else { have.join(", ") });
            0
        }
        Err(e) => {
            errln!("modisa: {}: {e}", crate::config::grants::path());
            1
        }
    }
}

fn valid_name(name: &str) -> bool {
    let mut cs = name.chars();
    cs.next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) && cs.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn scaffold(name: Option<&str>, dir_flag: Option<&str>) -> i32 {
    let Some(name) = name.filter(|n| valid_name(n)) else {
        errln!("usage: modisa plugin new <name> [--dir d]   (name: lowercase letters, digits and dashes)");
        return 2;
    };
    let dir = dir_flag.unwrap_or(name);
    let path = std::path::Path::new(dir);
    let existing = if path.is_dir() { std::fs::read_dir(path).is_ok_and(|mut d| d.next().is_some()) } else { path.exists() };
    if existing {
        errln!("modisa: {dir} already exists and isn't empty; plugin new never overwrites");
        return 1;
    }
    let fill = |text: &str| text.replace("{{name}}", name);
    let manifest = serde_json::json!({ "name": name, "protocol": PROTOCOL, "run": ["bun", "plugin.ts"], "description": format!("{name}: a modisa plugin"), "permissions": ["ui"] });
    let files = [
        ("plugin.json", stringify(&manifest, true) + "\n"),
        ("plugin.ts", fill(PLUGIN)),
        ("plugin.test.ts", fill(TEST)),
        ("modisa-plugin.ts", SDK_TEXT.to_string()),
        ("AGENTS.md", fill(GUIDE)),
        ("CLAUDE.md", "Read AGENTS.md: it explains how to write, test and install this modisa plugin.\n".to_string()),
    ];
    let written = std::fs::create_dir_all(dir).and_then(|_| files.iter().try_for_each(|(f, text)| std::fs::write(format!("{dir}/{f}"), text)));
    if let Err(e) = written {
        errln!("modisa: {dir}: {e}");
        return 1;
    }
    outln!("created {dir}: plugin.json, plugin.ts, plugin.test.ts, modisa-plugin.ts, AGENTS.md\nnext: put the plugin's logic in plugin.ts (AGENTS.md explains how), then\n  modisa plugin check {dir}\n  modisa plugin link {dir}");
    0
}

// the keys off in that session, on a line of their own
fn keys_off(keys: Option<&Vec<String>>) -> String {
    match keys {
        Some(k) if !k.is_empty() => format!("\n  keys off in the server's config: {}", k.join("; ")),
        _ => String::new(),
    }
}

// Start a linked plugin in the one running session this reaches, and wait for it to connect. Never starts a session.
pub async fn start_in(session: Option<&str>, name: &str) -> RpcResult<StartOutcome> {
    let at = session_name(session);
    let Ok(conn) = connect_existing(session, |_, _| {}).await else {
        return Ok(StartOutcome { reason: Some(format!("no running session {at}; it starts with the next one")), ..StartOutcome::new(&at, "not-started") });
    };
    let r = start_with(&conn_ask(&conn), &at, name).await;
    conn.close();
    r
}

pub fn describe_start(start: &StartOutcome, what: &str) -> String {
    let reason = start.reason.as_deref().unwrap_or("undefined");
    let pid = start.pid.map_or("undefined".into(), |p| p.to_string());
    let log = start.log.as_deref().filter(|l| !l.is_empty()).map_or(String::new(), |l| format!("\n  log: {l}"));
    match start.state {
        "started" => format!("started in session {}, connected (pid {pid}){}", start.session, keys_off(start.disabled_keys.as_ref())),
        "already-running" => format!("already running in session {} (pid {pid}); not started again{}", start.session, keys_off(start.disabled_keys.as_ref())),
        "not-started" => format!("not started: {reason}"),
        "failed" => format!("{what}, but failed to start in session {}: {reason}{log}", start.session),
        _ => format!("{what} and started in session {}, but {reason}{log}", start.session),
    }
}

// Registers the plugin for every session (they start it when they start), then starts it in the one running session
// this reaches. Exit 0 when it's connected, already running or there's no session; 1 when it's linked but didn't
// start or connect.
async fn link(arg: Option<&str>, session: Option<&str>, json: bool) -> i32 {
    let Some(arg) = arg else {
        errln!("usage: modisa plugin link <dir> [--json]");
        return 2;
    };
    let dir = real(arg);
    if dir.is_empty() {
        errln!("modisa: no such directory: {arg}");
        return 1;
    }
    let manifest = match read_manifest(&dir) {
        Ok(m) => m,
        Err(e) => {
            errln!("modisa: {e}");
            return 1;
        }
    };
    let target = format!("{}/{}", *PLUGINS_DIR, manifest.name);
    let existing = std::fs::read_link(&target).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    if !existing.is_empty() && existing != dir {
        errln!("modisa: {} is already linked to {existing}; modisa plugin unlink {} first", manifest.name, manifest.name);
        return 1;
    }
    if existing.is_empty() {
        let linked = std::fs::create_dir_all(&*PLUGINS_DIR).and_then(|_| {
            let _ = std::fs::remove_file(&target); // ln -sfn: whatever was there
            std::os::unix::fs::symlink(&dir, &target)
        });
        if let Err(e) = linked {
            errln!("modisa: {target}: {e}");
            return 1;
        }
    }
    consent(&manifest, json);
    let start = match start_in(session, &manifest.name).await {
        Ok(s) => s,
        Err(e) => {
            errln!("modisa: {e}");
            return 1;
        }
    };
    if json {
        let result = serde_json::json!({ "name": manifest.name, "dir": dir, "linked": true, "alreadyLinked": !existing.is_empty(), "start": start });
        outln!("{}", stringify(&result, true));
    } else {
        outln!("{} {} → {dir} (every session starts it)\n{}", if existing.is_empty() { "linked" } else { "already linked" }, manifest.name, describe_start(&start, "linked"));
    }
    if start.state == "failed" || start.state == "no-hello" { 1 } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_sdk_says_its_version() {
        assert!(sdk_version(SDK_TEXT).is_some_and(|v| v > 0));
        assert_eq!(sdk_version("export const SDK_VERSION = 7;"), Some(7));
        assert_eq!(sdk_version("nothing here"), None);
        assert!(PLUGIN.contains("{{name}}") && TEST.contains("checkSession") && GUIDE.contains("{{name}}"));
    }

    #[test]
    fn plugin_names_are_ids() {
        assert!(valid_name("demo-2") && valid_name("9lives"));
        assert!(!valid_name("Demo") && !valid_name("-x") && !valid_name("") && !valid_name("a_b"));
    }
}
