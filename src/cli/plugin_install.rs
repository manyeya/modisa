// `modisa plugin install <git-url | plugin@marketplace> [--ref r] [--subdir d]`, `plugin update <name>` and
// `plugin unlink <name>`: config/plugin_manage.rs does each, and this prints what it did. An install starts the plugin
// in the running session reached (the default, or -s); an update restarts it wherever it was running.
use std::sync::LazyLock;

use regex::Regex;

use super::commands::stringify;
use super::plugin::{describe_start, start_in};
use crate::config::plugin_manage::{install_plugin, session_name, unlink_plugin, update_plugin, InstallFrom, InstallResult, StartOutcome, UpdateResult};
use crate::config::plugins::MANAGED_DIR;

// <plugin>@<marketplace>: both ids, so no git URL looks like one
static MARKETPLACE_PLUGIN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z0-9][a-z0-9-]*@[a-z0-9][a-z0-9-]*$").unwrap());

fn started(start: Option<&StartOutcome>) -> bool {
    start.is_none_or(|s| ["started", "already-running", "not-started"].contains(&s.state))
}

fn at(r#ref: Option<&str>, commit: Option<&str>) -> String {
    format!("{}commit {}", r#ref.filter(|r| !r.is_empty()).map_or(String::new(), |r| format!("ref {r}, ")), commit.unwrap_or("undefined"))
}

fn print(x: &impl serde::Serialize) {
    outln!("{}", stringify(&serde_json::to_value(x).unwrap_or_default(), true));
}

fn report(result: &InstallResult, json: bool) -> i32 {
    let already = result.already_installed == Some(true);
    if json {
        print(result);
    } else if !result.installed && !already {
        errln!("modisa: not installed ({}): {}", result.stage.unwrap_or("undefined"), result.reason.as_deref().unwrap_or("undefined"));
    } else {
        outln!(
            "{} {} from {} ({})\n  {}",
            if already { "already installed" } else { "installed" },
            result.name.as_deref().unwrap_or_default(),
            result.source,
            at(result.r#ref.as_deref(), result.commit.as_deref()),
            result.dir.as_deref().unwrap_or_default()
        );
        if let Some(start) = &result.start {
            outln!("{}", describe_start(start, if already { "already installed" } else { "installed" }));
        }
        for hint in result.hints.iter().flatten() {
            outln!("note: {hint}");
        }
    }
    if (result.installed || already) && started(result.start.as_ref()) { 0 } else { 1 }
}

pub async fn install(arg: Option<&str>, r#ref: Option<&str>, subdir: Option<&str>, session: Option<&str>, json: bool) -> i32 {
    let Some(arg) = arg else {
        errln!("usage: modisa plugin install <git-url | plugin@marketplace> [--ref branch|tag|commit] [--subdir path] [--json]");
        return 2;
    };
    let listed = MARKETPLACE_PLUGIN.is_match(arg);
    if listed && (r#ref.is_some_and(|r| !r.is_empty()) || subdir.is_some_and(|s| !s.is_empty())) {
        errln!("usage: {arg} is installed from where its marketplace says: --ref and --subdir are for a git URL");
        return 2;
    }
    let from = if listed {
        InstallFrom { marketplace_plugin: Some(arg.into()), ..Default::default() }
    } else {
        InstallFrom { source: Some(arg.into()), r#ref: r#ref.map(String::from), subdir: subdir.map(String::from), ..Default::default() }
    };
    let cloning = |source: &str| outln!("installing from {source}: only https, ssh, git and file transports are used, and no build or dependency scripts run before it starts. A plugin runs as you, with your files and network; it isn't sandboxed.");
    let mut result = match install_plugin(&from, None, if json { None } else { Some(&cloning) }).await {
        Ok(r) => r,
        Err(e) => {
            errln!("modisa: {e}");
            return 1;
        }
    };
    if !result.installed && result.already_installed != Some(true) {
        return report(&result, json);
    }
    // what it asks to do through modisa, before it starts
    let name = result.name.clone().unwrap_or_default();
    if let Some(m) = crate::config::plugins::linked_plugins().into_iter().find(|l| l.name == name).and_then(|l| crate::config::plugins::read_manifest(&l.dir).ok()) {
        super::plugin::consent(&m, json);
    }
    // the start goes before the hints, as it always has
    result.start = match start_in(session, result.name.as_deref().unwrap_or_default()).await {
        Ok(s) => Some(s),
        Err(e) => {
            errln!("modisa: {e}");
            return 1;
        }
    };
    report(&result, json)
}

fn report_update(r: &UpdateResult) {
    let short = |c: Option<&str>| c.map_or(String::new(), |c| c.chars().take(7).collect());
    if let Some(stage) = r.stage {
        return errln!("modisa: not updated ({stage}): {}", r.reason.as_deref().unwrap_or("undefined"));
    }
    let r#ref = r.r#ref.clone().flatten();
    let source = r.source.as_deref().unwrap_or("undefined");
    if r.up_to_date == Some(true) {
        return outln!("{} is up to date: {source} ({})", r.name, at(r#ref.as_deref(), r.commit.as_deref()));
    }
    outln!("updated {} from {source} ({}{} → {})", r.name, r#ref.as_deref().filter(|r| !r.is_empty()).map_or(String::new(), |r| format!("ref {r}, ")), short(r.from.as_deref()), short(r.commit.as_deref()));
    for start in &r.restarted {
        let line = describe_start(start, "updated");
        outln!("{}", line.strip_prefix("started in").map_or(line.clone(), |rest| format!("restarted in{rest}")));
    }
    if r.restarted.is_empty() {
        outln!("it wasn't running in any session: each starts the new version when it starts it");
    }
    for hint in r.hints.iter().flatten() {
        outln!("note: {hint}");
    }
}

pub async fn update(name: Option<&str>, json: bool) -> i32 {
    let Some(name) = name else {
        errln!("usage: modisa plugin update <name> [--json]");
        return 2;
    };
    let result = match update_plugin(name, None).await {
        Ok(r) => r,
        Err(e) => {
            errln!("modisa: {e}");
            return 1;
        }
    };
    if json {
        print(&result);
    } else {
        report_update(&result);
    }
    if result.stage.is_none() && result.restarted.iter().all(|s| started(Some(s))) { 0 } else { 1 }
}

pub async fn unlink(name: Option<&str>, session: Option<&str>, json: bool) -> i32 {
    let Some(name) = name else {
        errln!("usage: modisa plugin unlink <name> [--json]");
        return 2;
    };
    let result = match unlink_plugin(name, session, None).await {
        Ok(r) => r,
        Err(e) => {
            errln!("modisa: {e}");
            return 1;
        }
    };
    if json {
        print(&result);
    } else if !result.managed {
        let at = session_name(session);
        let stopped = if result.stopped_in.is_empty() { format!("no running copy in session {at}") } else { format!("stopped it in session {at}") };
        outln!("unlinked {name}\n{stopped}; other running sessions keep theirs until they restart. Its directory is untouched.");
    } else {
        outln!("unlinked {name}{}", if result.stopped_in.is_empty() { String::new() } else { format!("; stopped it in session {}", result.stopped_in.join(", ")) });
        let checkout = result.checkout.as_ref().expect("a managed unlink says what became of its checkout");
        if checkout.deleted {
            outln!("deleted its checkout ({}); its data and logs are kept", checkout.path);
        } else {
            let why: Vec<String> = result.still_using.iter().map(|s| format!("session {s} still runs it")).chain(result.unreachable.iter().map(|s| format!("session {s} can't be reached (a sandbox?)"))).collect();
            outln!("kept its checkout at {}: {}. Once that's stopped, remove {}/{name}.", checkout.path, why.join("; "), *MANAGED_DIR);
        }
    }
    0
}
