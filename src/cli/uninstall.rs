// `modisa uninstall [--purge] [--yes]`: take modisa back off this machine. Package managers only
// remove the files they installed, so this does the rest: every agent integration and the shared
// skill, running sessions and saved state (config too, with --purge). The binary goes as well when
// install.sh put it there; otherwise the package manager's own remove command finishes the job.
use std::io::Write as _;

use serde_json::json;

use super::update::installed_by;
use crate::config::plugin_manage::sessions;
use crate::config::CONFIG_DIR;
use crate::core::paths::{self_exe, socket_path, DIR};
use crate::core::version::FROM_SOURCE;
use crate::integrations::{integration_status, uninstall_all};
use crate::platform::logos::{logo_status, uninstall_logos};
use crate::protocol::transport::connect_unix;

// prompt(): the question, then a line from stdin (None at its end)
async fn prompt(question: &str) -> Option<String> {
    let mut out = std::io::stdout();
    let _ = write!(out, "{question} ");
    let _ = out.flush();
    // waiting on the person at the keyboard, off the thread (nothing else runs meanwhile)
    let read = tokio::task::spawn_blocking(|| {
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).ok().filter(|&n| n > 0).map(|_| line)
    });
    read.await.ok().flatten()
}

pub async fn run_uninstall(purge: bool, yes: bool) -> i32 {
    if std::env::var("MODISA_SOCKET").is_ok_and(|s| !s.is_empty()) {
        errln!("run `modisa uninstall` from a terminal outside modisa: it stops every session, this one included");
        return 1;
    }
    let exe = self_exe();
    let how = installed_by(&exe, FROM_SOURCE);
    let running = sessions();
    let agents: Vec<String> = integration_status().into_iter().filter(|s| s.status != "none").map(|s| s.name).collect();
    let has_config = std::path::Path::new(&*CONFIG_DIR).is_dir();
    let logos = match logo_status() {
        Ok(l) => l,
        Err(e) => {
            errln!("modisa: {e}");
            return 1;
        }
    };
    let logos_there = logos.font.is_some() || logos.terminals.iter().any(|t| t.configured);

    if !yes {
        outln!("modisa uninstall will:");
        if !running.is_empty() {
            outln!("  stop running sessions: {}", running.join(", "));
        }
        if !agents.is_empty() {
            outln!("  remove modisa's hooks and skill from: {}", agents.join(", "));
        }
        if logos_there {
            let configured: Vec<&str> = logos.terminals.iter().filter(|t| t.configured).map(|t| t.name).collect();
            outln!("  remove the agent logo font and its settings in: {}", if configured.is_empty() { "no terminal".into() } else { configured.join(", ") });
        }
        outln!("  delete saved sessions and state in {}", *DIR);
        if has_config {
            if purge {
                outln!("  delete your config in {}", *CONFIG_DIR);
            } else {
                outln!("  keep your config in {} (--purge deletes it)", *CONFIG_DIR);
            }
        }
        if how.by == "script" {
            outln!("  delete {exe}");
        }
        let answer = prompt("Continue? [y/N]").await;
        let answer = answer.as_deref().map(str::trim).unwrap_or_default().to_lowercase();
        if answer != "y" && answer != "yes" {
            outln!("nothing removed");
            return 1;
        }
    }

    let (removed, failed) = uninstall_all();
    for line in &removed {
        outln!("{line}");
    }
    for line in &failed {
        errln!("{line}");
    }
    if logos_there {
        match uninstall_logos() {
            Ok(()) => outln!("removed the agent logo font and its terminal settings"),
            Err(e) => errln!("agent logos: {e}"),
        }
    }
    for name in &running {
        let Ok(c) = connect_unix(&socket_path(name), |_, _| {}).await else {
            continue; // a dead server's socket: the state directory goes below anyway
        };
        let _ = c.request("kill", json!({}), None).await;
        c.close();
        outln!("stopped session {name}");
    }
    let _ = std::fs::remove_dir_all(&*DIR);
    outln!("deleted {}", *DIR);
    if has_config && purge {
        let _ = std::fs::remove_dir_all(&*CONFIG_DIR);
        outln!("deleted {}", *CONFIG_DIR);
    } else if has_config {
        outln!("kept your config in {} (--purge deletes it)", *CONFIG_DIR);
    }

    if how.by == "script" {
        if std::fs::remove_file(&exe).is_ok() {
            outln!("deleted {exe}");
        } else {
            outln!("couldn't delete {exe}: remove it by hand");
        }
    } else if let Some(remove) = how.remove {
        outln!("modisa was installed with {}; finish with: {remove}", how.manager.unwrap_or_default());
    } else {
        outln!("modisa runs from source here: delete the checkout to finish");
    }
    if failed.is_empty() { 0 } else { 1 }
}
