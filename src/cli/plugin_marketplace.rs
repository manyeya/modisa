// `modisa plugin marketplace add|list|update|remove`: config/marketplaces.rs does each, and this prints what it did.
// No session is needed: marketplaces live in the state directory, shared by every session.
use serde_json::json;

use super::args::Args;
use super::commands::{stringify, table};
use crate::config::marketplaces::{add_marketplace, list_marketplaces, remove_marketplace, update_marketplaces};
use crate::protocol::conn::RpcResult;

const USAGE: &str = "usage: modisa plugin marketplace add <owner/repo | git-url> [--ref r] | list | update [name] | remove <name>   [--json]";

fn short(commit: &str) -> String {
    commit.chars().take(7).collect()
}

fn print(x: &impl serde::Serialize) {
    outln!("{}", stringify(&serde_json::to_value(x).unwrap_or_default(), true));
}

pub async fn marketplace(args: &[&str], a: &Args) -> i32 {
    let (verb, arg) = (args.first().copied(), args.get(1).copied());
    let json = a.on("json");
    match run(verb, arg, a, json).await {
        Ok(Some(code)) => code,
        Ok(None) => {
            errln!("{USAGE}");
            2
        }
        Err(e) => {
            if json {
                errln!("{}", stringify(&json!({ "error": { "code": "error", "message": e.message } }), false));
            } else {
                errln!("modisa: {}", e.message);
            }
            1
        }
    }
}

// the exit status, or None for usage
async fn run(verb: Option<&str>, arg: Option<&str>, a: &Args, json: bool) -> RpcResult<Option<i32>> {
    match (verb, arg) {
        (Some("add"), Some(arg)) => {
            let r = add_marketplace(arg, a.str("ref")).await?;
            if json {
                print(&r);
            } else if let Some(stage) = r.stage {
                errln!("modisa: marketplace not added ({stage}): {}", r.reason.as_deref().unwrap_or_default());
            } else {
                let name = r.name.as_deref().unwrap_or_default();
                let what = if r.already_added == Some(true) { format!("marketplace {name} is already added") } else { format!("added marketplace {name}") };
                let r#ref = r.r#ref.as_deref().filter(|r| !r.is_empty()).map_or(String::new(), |r| format!("ref {r}, "));
                let description = r.description.as_deref().filter(|d| !d.is_empty()).map_or(String::new(), |d| format!(": {d}"));
                outln!("{what} from {} ({}commit {}){description}", r.source, r#ref, short(r.commit.as_deref().unwrap_or_default()));
                let plugins = r.plugins.clone().unwrap_or_default();
                outln!("  {} plugin{}{}", plugins.len(), if plugins.len() == 1 { "" } else { "s" }, if plugins.is_empty() { String::new() } else { format!(": {}", plugins.join(", ")) });
                if r.already_added == Some(true) {
                    outln!("  modisa plugin marketplace update {name} fetches its latest");
                } else {
                    outln!("  install one with modisa plugin install <plugin>@{name}. A marketplace is a list, not a review: each plugin runs as you.");
                }
            }
            Ok(Some(if r.stage.is_some() { 1 } else { 0 }))
        }
        (Some("list"), _) => {
            let list = list_marketplaces();
            if json {
                print(&list);
            } else {
                let rows = list
                    .iter()
                    .map(|m| {
                        let plugins = m.error.as_ref().map_or(m.plugins.to_string(), |e| format!("({e})"));
                        vec![m.m.name.clone(), plugins, m.m.source.clone(), m.m.r#ref.clone().unwrap_or_default(), short(&m.m.commit), m.description.clone().unwrap_or_default()]
                    })
                    .collect();
                table(&["name", "plugins", "source", "ref", "commit", "description"], rows);
            }
            Ok(Some(0))
        }
        (Some("update"), arg) => {
            let results = update_marketplaces(arg).await?;
            if json {
                print(&results);
            } else {
                if results.is_empty() {
                    outln!("no marketplaces added (modisa plugin marketplace add <owner/repo>)");
                }
                for r in &results {
                    match &r.reason {
                        Some(reason) => errln!("modisa: marketplace {} not updated: {reason}", r.name),
                        None if r.updated => outln!("updated marketplace {}: {} → {}, {} plugins", r.name, short(&r.from), short(&r.commit), r.plugins.unwrap_or(0)),
                        None => outln!("marketplace {} is up to date ({})", r.name, short(&r.commit)),
                    }
                }
            }
            Ok(Some(if results.iter().any(|r| r.reason.is_some()) { 1 } else { 0 }))
        }
        (Some("remove"), Some(arg)) => {
            let r = remove_marketplace(arg)?;
            if json {
                print(&r);
            } else {
                outln!("removed marketplace {} (deleted {})", r.name, r.dir);
                if !r.installed.is_empty() {
                    outln!("still installed from it: {} (modisa plugin unlink <name> removes one; plugin update can't fetch one that came from inside it)", r.installed.join(", "));
                }
            }
            Ok(Some(0))
        }
        _ => Ok(None),
    }
}
