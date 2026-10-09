// Entry point: route a command line to the session commands, the server, the integrations, or the API CLI.
// (Port of src/main.ts's dispatch.) Panes no longer start through `modisa __pty-exec`: the server's PTY spawn makes
// the pane's shell own its terminal itself.

// console.log and console.error: a closed stdout (`modisa pane list | head -1`) is no reason to panic.
macro_rules! outln {
    ($($t:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stdout(), $($t)*);
    }};
}
macro_rules! errln {
    ($($t:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stderr(), $($t)*);
    }};
}

pub mod args;
pub mod commands;
pub mod help;
pub mod logos;
pub mod pane_attach;
pub mod plugin;
pub mod plugin_check;
pub mod plugin_git;
pub mod plugin_install;
pub mod plugin_marketplace;
pub mod plugin_search;
pub mod profile;
pub mod theme;
pub mod sessions;
pub mod uninstall;
pub mod update;
pub mod view;

use crate::core::paths::{abs_path, cwd};
use args::parse_args;
use help::HELP;

// The plugin commands that need no server (src/cli/plugin.ts): the rest go through it.
const LOCAL_PLUGIN: &[&str] = &["new", "sdk", "schema", "check", "dev", "link", "unlink", "install", "update", "search", "marketplace"];

pub fn is_local_plugin_command(verb: Option<&str>) -> bool {
    verb.is_some_and(|v| LOCAL_PLUGIN.contains(&v))
}

// The process exit status for a command line (argv without the program name).
pub async fn run(argv: Vec<String>) -> i32 {
    let a = parse_args(&argv);
    let words: Vec<String> = if a.on("help") {
        vec!["help".into()]
    } else if a.on("version") {
        vec!["version".into()]
    } else {
        a.pos.clone()
    };
    let cmd = words.first().map(String::as_str);
    let rest: Vec<&str> = words.iter().skip(1).map(String::as_str).collect();
    let arg = |i: usize| rest.get(i).copied();
    let session = a.str("session").unwrap_or("default").to_string();
    let named = arg(0).unwrap_or(&session).to_string(); // a command's session: named, else -s, else default

    match cmd {
        None | Some("attach" | "a") => sessions::attach(&named, &cwd(), a.str("remote")).await,
        Some("new") => sessions::attach(&named, &abs_path(&a.str("cwd").map(String::from).unwrap_or_else(cwd)), a.str("remote")).await,
        Some("server") => crate::server::run_server(&session).await,
        Some("proxy") => sessions::proxy(&session).await,
        Some("ls" | "list-sessions") => sessions::list_sessions().await,
        Some("restart") => sessions::restart_session(&named).await,
        Some("kill") => sessions::kill_session(&named).await,
        Some("integration") => crate::integrations::run_integration(arg(0), arg(1)).await,
        Some("logos") => logos::run_logos(arg(0)).await,
        Some("update") => {
            update::run_update().await; // as the original's main.ts: its status isn't the exit status
            0
        }
        Some("version") => update::run_version().await,
        Some("uninstall") => uninstall::run_uninstall(a.switch("purge"), a.switch("yes")).await,
        Some("hook") => crate::integrations::hook::run_hook(arg(0), arg(1)).await, // run by agents' hooks: modisa hook <agent> <action>
        Some("config") => sessions::config_command(arg(0), &a).await,
        Some("__site-data") => site_data().await,
        Some("theme") => theme::run(arg(0), arg(1), a.str("name")),
        Some("profile") => match arg(0) {
            Some("export") => profile::export(),
            Some("import") => profile::import(arg(1), a.switch("yes")).await,
            _ => {
                eprintln!("usage: modisa profile export > my-setup.toml | modisa profile import <file or URL> [--yes]");
                2
            }
        },
        // new, sdk, schema, check, dev, link and unlink need no server; the rest go through it
        Some("plugin") if is_local_plugin_command(arg(0)) => plugin::run_plugin_local(arg(0).unwrap_or_default(), &rest[1..], &a).await,
        Some("view") => view::run_view(arg(0), rest.get(1..).unwrap_or_default(), &a), // a view drawn without a session
        Some("help" | "--help" | "-h") => {
            outln!("{HELP}");
            0
        }
        _ => commands::run_cli(&a).await,
    }
}

// What the docs site (site/build.ts) shows that comes from the code: the agents, every setting, the integrations and
// the plugin directory.
async fn site_data() -> i32 {
    use crate::integrations::targets::{LIFECYCLE, SESSION, TARGETS};
    let agents: Vec<_> = crate::config::agents::builtin_agents().into_iter().filter(|a| a.id != "generic").collect();
    let names = |kind: &str| TARGETS.iter().filter(|t| t.kind == kind).map(|t| t.name).collect::<Vec<_>>().join(", ");
    let plugins = match plugin_search::find(&[], 100).await {
        Ok(s) => serde_json::json!(s.results),
        Err(e) => serde_json::json!({ "error": e }),
    };
    let data = serde_json::json!({
        "agents": agents,
        "sample": crate::config::SAMPLE,
        "integrations": { LIFECYCLE: names(LIFECYCLE), SESSION: names(SESSION) },
        "topic": plugin_search::TOPIC,
        "plugins": plugins,
    });
    outln!("{data}");
    0
}
