// The plugin manager's requests: plugin.resolve, install, update, unlink, logs and catalog, and marketplace.*. They do
// what the CLI does (config/plugin_manage.rs, config/marketplaces.rs) on the server's machine, so a --remote TUI
// manages plugins where they run. Only the user can: an agent in a pane (a caller) or a plugin's own connection is
// refused, since these fetch code that then runs as the user. Git runs async, so a slow clone holds nothing else up.
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{json, Value};

use super::rpc::dispatch::{call, Handler};
use super::Shared;
use crate::async_handler;
use crate::cli::plugin_search::find;
use crate::config::marketplaces::{add_marketplace, js_space, list_marketplaces, marketplace_plugins, remove_marketplace, update_marketplaces};
use crate::config::plugin_manage::{install_plugin, resolve_plugin, start_with, unlink_plugin, update_plugin, Here, InstallFrom};
use crate::config::plugins::installs;
use crate::core::text::clean_text;
use crate::protocol::conn::{error, fail, RpcResult};
use crate::protocol::schema::{invalid, Params};

pub fn route(method: &str) -> Option<Handler> {
    Some(match method {
        "plugin.resolve" => async_handler!(resolve),
        "plugin.install" => async_handler!(install),
        "plugin.update" => async_handler!(update),
        "plugin.unlink" => async_handler!(unlink),
        "plugin.logs" => async_handler!(logs),
        "plugin.catalog" => async_handler!(catalog),
        "marketplace.list" => async_handler!(marketplace_list),
        "marketplace.add" => async_handler!(marketplace_add),
        "marketplace.update" => async_handler!(marketplace_update),
        "marketplace.remove" => async_handler!(marketplace_remove),
        _ => return None,
    })
}

// this session, through its own plugin host
fn here(shared: &Shared, c: u64) -> Here {
    let session = shared.borrow().session.clone();
    let shared = shared.clone();
    Here { session, ask: Rc::new(move |method, params| {
        let shared = shared.clone();
        Box::pin(async move { call(&shared, c, &method, params).await })
    }) }
}

// Checked after the params, as the original's wrapper ran after their schema: not agents in panes, not plugins.
fn only_user(shared: &Shared, p: &Params, c: u64) -> RpcResult<()> {
    let caller = p.opt_str("caller")?.is_some_and(|s| !s.is_empty());
    let plugin = shared.borrow().clients.get(&c).is_some_and(|c| c.plugin.as_deref().is_some_and(|p| !p.is_empty()));
    if caller || plugin {
        return Err(error("only the user can manage plugins and marketplaces"));
    }
    Ok(())
}

fn to_json(x: &impl serde::Serialize) -> RpcResult {
    serde_json::to_value(x).map_err(|e| error(e.to_string()))
}

// plugin.resolve and plugin.install: a git URL, or a marketplace entry, which has its own ref and subdir
const ONE_SOURCE: &str = "exactly one of source (with ref and subdir if needed) or marketplacePlugin (whose entry has its own)";
static FULL_COMMIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9a-f]{40}$").unwrap());

fn install_from(p: &Params) -> RpcResult<InstallFrom> {
    p.opt_str("caller")?;
    let o = InstallFrom { source: p.opt_len("source", 1, None)?, marketplace_plugin: p.opt_len("marketplacePlugin", 1, None)?, r#ref: p.opt_len("ref", 1, None)?, subdir: p.opt_len("subdir", 1, None)? };
    Ok(o)
}

fn one_source(p: &Params, o: &InstallFrom) -> RpcResult<()> {
    p.refine(o.source.is_none() != o.marketplace_plugin.is_none() && !(o.marketplace_plugin.is_some() && (o.r#ref.is_some() || o.subdir.is_some())), ONE_SOURCE)
}

async fn resolve(shared: Shared, p: Value, c: u64) -> RpcResult {
    let p = Params::new(&p)?;
    let o = install_from(&p)?;
    one_source(&p, &o)?;
    only_user(&shared, &p, c)?;
    to_json(&resolve_plugin(&o).await?)
}

async fn install(shared: Shared, p: Value, c: u64) -> RpcResult {
    let p = Params::new(&p)?;
    let o = install_from(&p)?;
    let commit = p.opt_str("commit")?;
    if commit.as_deref().is_some_and(|c| !FULL_COMMIT.is_match(c)) {
        return Err(invalid("commit", "a full commit id"));
    }
    one_source(&p, &o)?;
    only_user(&shared, &p, c)?;
    let mut result = install_plugin(&o, commit.as_deref(), None).await?;
    if !result.installed && result.already_installed != Some(true) {
        return to_json(&result);
    }
    let h = here(&shared, c);
    result.start = Some(start_with(&h.ask, &h.session, result.name.as_deref().unwrap_or_default()).await?);
    to_json(&result)
}

async fn update(shared: Shared, p: Value, c: u64) -> RpcResult {
    let p = Params::new(&p)?;
    p.opt_str("caller")?;
    let name = p.len("name", 1, None)?;
    only_user(&shared, &p, c)?;
    to_json(&update_plugin(&name, Some(&here(&shared, c))).await?)
}

// and, here, out of the plugin list once stopped (the CLI's unlink leaves it listed as stopped)
async fn unlink(shared: Shared, p: Value, c: u64) -> RpcResult {
    let p = Params::new(&p)?;
    p.opt_str("caller")?;
    let name = p.len("name", 1, None)?;
    only_user(&shared, &p, c)?;
    let result = unlink_plugin(&name, None, Some(&here(&shared, c))).await?;
    crate::server::plugins::forget(&mut shared.borrow_mut(), &name);
    to_json(&result)
}

// the last lines of its log here, cleaned to show in a terminal
async fn logs(shared: Shared, p: Value, c: u64) -> RpcResult {
    let p = Params::new(&p)?;
    p.opt_str("caller")?;
    let name = p.len("name", 1, None)?;
    let lines = p.opt_num("lines", None, None, true)?.unwrap_or(200.0);
    if lines <= 0.0 {
        return Err(invalid("lines", "Too small: expected number to be >0"));
    }
    if lines > 10_000.0 {
        return Err(invalid("lines", "Too big: expected number to be <=10000"));
    }
    only_user(&shared, &p, c)?;
    let h = here(&shared, c);
    let list = (h.ask)("plugin.list".into(), json!({})).await?;
    let Some(pl) = list.as_array().and_then(|ps| ps.iter().find(|x| x["name"] == name.as_str())).cloned() else {
        return Err(fail("no_such_plugin", format!("no plugin named {name} (see modisa plugin list)")));
    };
    let text = pl["log"].as_str().and_then(|f| std::fs::read(f).ok()).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    let all: Vec<&str> = text.trim_end_matches(js_space).split('\n').collect();
    let tail = &all[all.len().saturating_sub(lines as usize)..];
    let text = tail.iter().map(|l| clean_text(&l.replace('\t', "  "), 1000)).collect::<Vec<_>>().join("\n");
    let mut out = serde_json::Map::new();
    out.insert("name".into(), pl["name"].clone());
    if let Some(log) = pl.get("log").filter(|l| !l.is_null()) {
        out.insert("log".into(), log.clone());
    }
    out.insert("text".into(), json!(text));
    Ok(Value::Object(out))
}

// the index's plugins (as `plugin search` finds them) and the marketplaces', each saying whether it's here
async fn catalog(shared: Shared, p: Value, c: u64) -> RpcResult {
    let p = Params::new(&p)?;
    p.opt_str("caller")?;
    let query = p.opt_str("query")?.unwrap_or_default();
    only_user(&shared, &p, c)?;
    let words: Vec<String> = query.split(js_space).filter(|w| !w.is_empty()).map(String::from).collect();
    let index = find(&words, 100).await;
    let listed = marketplace_plugins(&words);
    let sources: HashSet<String> = installs().into_iter().map(|r| r.source).collect();
    let (total, found, index_error) = match index {
        Ok(s) => (s.total, s.results, None),
        Err(e) => (json!(0), vec![], Some(e)),
    };
    let results: Vec<Value> = found
        .into_iter()
        .map(|r| {
            let source = r.install.strip_prefix("modisa plugin install ").unwrap_or(&r.install).to_string(); // its clone URL
            let mut v = to_json(&r).unwrap_or_default();
            if let Some(m) = v.as_object_mut() {
                m.insert("installed".into(), json!(sources.contains(&source)));
                m.insert("source".into(), json!(source));
                let installed = m.shift_remove("installed").unwrap_or_default();
                m.insert("installed".into(), installed); // after source, as { ...r, source, installed }
            }
            v
        })
        .collect();
    let mut index = json!({ "total": total, "results": results });
    if let Some(e) = index_error {
        index["error"] = json!(e);
    }
    Ok(json!({ "query": words.join(" "), "index": index, "marketplaces": to_json(&listed)? }))
}

async fn marketplace_list(shared: Shared, p: Value, c: u64) -> RpcResult {
    let p = Params::new(&p)?;
    only_user(&shared, &p, c)?;
    to_json(&list_marketplaces())
}

async fn marketplace_add(shared: Shared, p: Value, c: u64) -> RpcResult {
    let p = Params::new(&p)?;
    p.opt_str("caller")?;
    let source = p.len("source", 1, None)?;
    let r#ref = p.opt_len("ref", 1, None)?;
    only_user(&shared, &p, c)?;
    to_json(&add_marketplace(&source, r#ref.as_deref()).await?)
}

async fn marketplace_update(shared: Shared, p: Value, c: u64) -> RpcResult {
    let p = Params::new(&p)?;
    p.opt_str("caller")?;
    let name = p.opt_len("name", 1, None)?; // none: every one
    only_user(&shared, &p, c)?;
    to_json(&update_marketplaces(name.as_deref()).await?)
}

async fn marketplace_remove(shared: Shared, p: Value, c: u64) -> RpcResult {
    let p = Params::new(&p)?;
    p.opt_str("caller")?;
    let name = p.len("name", 1, None)?;
    only_user(&shared, &p, c)?;
    to_json(&remove_marketplace(&name)?)
}
