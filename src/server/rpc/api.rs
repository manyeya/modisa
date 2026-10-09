// The public API (CLI, plugins, integrations). Each handler checks its params as the protocol's schema says.
// `caller` is the pane id of the agent calling, if any.
use std::time::Duration;

use serde_json::{json, Value};

use super::dispatch::Handler;
use crate::async_handler;
use crate::core::layout::{side, Axis, Dir};
use crate::core::paths;
use crate::core::text::clean_text;
use crate::protocol::conn::{error, fail, RpcError, RpcResult};
use crate::protocol::schema::{invalid, Params, DESCRIBE, PROTOCOL};
use crate::protocol::types::AgentState;
use crate::server::session::{MoveTo, SpawnOpts};
use crate::server::{keys, permissions, shutdown, Server, Shared, Toast};

const STATES: &[&str] = &["working", "blocked", "done", "idle"];
const DIRECTIONS: &[&str] = &["left", "right", "up", "down"];
const TONES: &[&str] = &["fg", "dim", "accent", "warn", "working", "blocked", "done", "idle"];

pub fn route(method: &str) -> Option<Handler> {
    use Handler::Sync;
    Some(match method {
        "list" => Sync(list),
        "session.info" => Sync(session_info),
        "workspace.list" => Sync(workspace_list),
        "workspace.rename" => Sync(workspace_rename),
        "workspace.close" => Sync(workspace_close),
        "workspace.create" => Sync(workspace_create),
        "tab.create" => Sync(tab_create),
        "pane.split" => Sync(pane_split),
        "pane.run" => async_handler!(pane_run),
        "pane.read" => Sync(pane_read),
        "pane.keys" => async_handler!(pane_keys),
        "pane.close" => async_handler!(pane_close),
        "pane.rename" => Sync(pane_rename),
        "pane.meta.set" => Sync(pane_meta_set),
        "pane.meta.clear" => Sync(pane_meta_clear),
        "pane.focus" => Sync(pane_focus),
        "pane.move" => async_handler!(pane_move),
        "pane.swap" => Sync(pane_swap),
        "pane.resize" => Sync(pane_resize),
        "pane.zoom" => Sync(pane_zoom),
        "wait" => async_handler!(wait),
        "agent.spawn" => Sync(agent_spawn),
        "agent.list" => Sync(agent_list),
        "report" => Sync(report),
        "debug.detect" => async_handler!(debug_detect),
        "notify" => Sync(notify),
        "send" => Sync(send),
        "inbox" => Sync(inbox),
        "messages" => Sync(messages),
        "messaging.pause" => Sync(messaging_pause),
        "events.subscribe" => Sync(events_subscribe),
        "protocol.describe" => Sync(|_, _, _| Ok(serde_json::from_str(DESCRIBE).unwrap())),
        "integrations" => Sync(|_, _, _| Ok(json!(crate::integrations::integration_status()))),
        "integration" => Sync(integration),
        "restart" => Sync(|srv, _, _| lifecycle(srv, false, "restart")),
        "kill" => Sync(|srv, _, _| lifecycle(srv, true, "exit")),
        _ => return None,
    })
}

fn caller(p: &Params) -> RpcResult<Option<String>> {
    p.opt_str("caller")
}

fn dir_of(s: &str) -> Dir {
    serde_json::from_value(json!(s)).unwrap_or(Dir::Right)
}

fn axis(dir: &str) -> Axis {
    if dir == "down" { Axis::Col } else { Axis::Row }
}

fn listed(srv: &Server) -> Vec<Value> {
    let focused = srv.s.focused_id();
    srv.s
        .panes
        .values()
        .map(|p| {
            let mut v = serde_json::to_value(&p.info).unwrap();
            v["focused"] = json!(focused.as_deref() == Some(p.id()));
            if let Some((wi, ti)) = srv.s.locate(p.id()) {
                let ws = &srv.s.workspaces[wi];
                v["workspace"] = json!(ws.name);
                v["workspaceId"] = json!(ws.id);
                v["tabId"] = json!(ws.tabs[ti].id);
            }
            v
        })
        .collect()
}

// a pane just made, and where it is
fn created(srv: &Server, id: &str) -> RpcResult {
    let (wi, ti) = srv.s.place_of(id)?;
    let mut v = serde_json::to_value(&srv.s.panes[id].info).unwrap();
    v["workspaceId"] = json!(srv.s.workspaces[wi].id);
    v["tabId"] = json!(srv.s.workspaces[wi].tabs[ti].id);
    Ok(v)
}

// what a layout command acts on: a pane with a place in a tab (not a popup)
fn placed(srv: &Server, target: Option<&str>, caller: Option<&str>) -> RpcResult<String> {
    let id = srv.subject(target, caller)?;
    srv.s.place_of(&id)?;
    Ok(id)
}

// and to move it, not a plugin's overlay either
fn movable(srv: &Server, id: &str) -> RpcResult<String> {
    if !srv.movable(id) {
        return Err(error(format!("{id} is a plugin's overlay: it stays over the pane it opened on")));
    }
    Ok(id.to_string())
}

fn no_neighbor(id: &str, dir: Dir) -> RpcError {
    fail("no_such_pane", format!("no pane {} {id}", side(dir)))
}

// ---------- session & workspaces ----------

fn list(srv: &mut Server, p: &Value, _c: u64) -> RpcResult {
    Params::new(p).and_then(|p| caller(&p))?;
    Ok(json!(listed(srv)))
}

fn session_info(srv: &mut Server, p: &Value, _c: u64) -> RpcResult {
    let p = Params::new(p)?;
    caller(&p)?;
    let mut info = json!({ "session": srv.session, "clients": srv.attached().len(), "paused": srv.mail.paused, "version": paths::code_version() });
    if !p.flag("snapshot")? {
        info["panes"] = json!(srv.s.panes.len());
        info["workspaces"] = json!(srv.s.workspaces.len());
        return Ok(info);
    }
    // what clients draw, read without attaching: no area is set, no event is sent, nothing changes
    info["active"] = json!(srv.s.active);
    info["area"] = json!(srv.s.area);
    info["workspaces"] = serde_json::to_value(srv.s.view().workspaces).unwrap();
    info["panes"] = json!(listed(srv));
    Ok(info)
}

fn workspace_list(srv: &mut Server, p: &Value, _c: u64) -> RpcResult {
    Params::new(p).and_then(|p| caller(&p))?;
    Ok(json!(srv.s.workspaces.iter().enumerate().map(|(i, w)| json!({ "id": w.id, "name": w.name, "cwd": w.cwd, "tabs": w.tabs.len(), "active": i == srv.s.active })).collect::<Vec<_>>()))
}

fn workspace_rename(srv: &mut Server, p: &Value, _c: u64) -> RpcResult {
    let p = Params::new(p)?;
    caller(&p)?;
    let ws = p.len("workspace", 1, None)?;
    let name = p.str("name")?;
    if name.trim().is_empty() {
        return Err(crate::protocol::schema::invalid("name", "Too small: expected string to have >=1 characters"));
    }
    let i = srv.s.find_workspace(&ws)?;
    srv.s.rename_workspace(name.trim(), Some(i));
    Ok(json!(true))
}

fn workspace_close(srv: &mut Server, p: &Value, _c: u64) -> RpcResult {
    let p = Params::new(p)?;
    caller(&p)?;
    let i = srv.s.find_workspace(&p.len("workspace", 1, None)?)?;
    srv.s.close_workspace(Some(i))?;
    Ok(json!(true))
}

fn workspace_create(srv: &mut Server, p: &Value, _c: u64) -> RpcResult {
    let p = Params::new(p)?;
    let caller = caller(&p)?;
    let o = SpawnOpts { command: p.opt_str("command")?, created_by: caller, env: p.env("env")?, ..Default::default() };
    let name = p.opt_str("name")?;
    let cwd = p.opt_str("cwd")?.unwrap_or_else(paths::cwd);
    let id = srv.s.new_workspace(name, Some(cwd), o)?;
    created(srv, &id)
}

fn tab_create(srv: &mut Server, p: &Value, _c: u64) -> RpcResult {
    let p = Params::new(p)?;
    let caller = caller(&p)?;
    let o = SpawnOpts { command: p.opt_str("command")?, name: p.opt_str("paneName")?, cwd: p.opt_str("cwd")?, created_by: caller, env: p.env("env")?, ..Default::default() };
    let name = p.opt_str("name")?;
    if let Some(ws) = p.opt_str("workspace")? {
        let i = srv.s.workspaces.iter().position(|w| w.name == ws || w.id == ws).ok_or_else(|| error(format!("no such workspace: {ws}")))?;
        srv.s.select_workspace(i as i64);
    }
    let id = srv.s.new_tab(name, o)?;
    created(srv, &id)
}

// ---------- panes ----------

fn pane_split(srv: &mut Server, p: &Value, _c: u64) -> RpcResult {
    let p = Params::new(p)?;
    let caller = caller(&p)?;
    let target = p.opt_len("target", 1, None)?;
    let dir = p.enum_or("dir", &["right", "down"], "right")?;
    let ratio = p.num_or("ratio", 0.5, Some(0.1), Some(0.9), false)?;
    let o = SpawnOpts { command: p.opt_str("command")?, name: p.opt_str("name")?, cwd: p.opt_str("cwd")?, created_by: caller.clone(), env: p.env("env")?, ..Default::default() };
    let focus = p.flag("focus")?;
    let subject = srv.subject(target.as_deref(), caller.as_deref())?;
    let id = srv.s.split(axis(&dir), o, Some(&subject), focus, ratio)?.ok_or_else(|| error("nothing to split"))?;
    created(srv, &id)
}

async fn pane_run(shared: Shared, p: Value, _c: u64) -> RpcResult {
    let p = Params::new(&p)?;
    let caller = caller(&p)?;
    let target = p.len("target", 1, None)?;
    let command = p.str("command")?;
    let (id, instance) = {
        let srv = shared.borrow();
        let id = srv.need(Some(&target), None)?;
        let instance = srv.s.panes[&id].info.instance.clone();
        (id, instance)
    };
    permissions::permit(&shared, caller.as_deref(), "run", &id, &command).await?;
    let srv = shared.borrow();
    if let Some(pane) = srv.s.panes.get(&id).filter(|p| p.info.instance == instance) {
        pane.write(format!("{command}\r").as_bytes());
    }
    Ok(json!(true))
}

fn pane_read(srv: &mut Server, p: &Value, _c: u64) -> RpcResult {
    let p = Params::new(p)?;
    let caller = caller(&p)?;
    let target = p.opt_len("target", 1, None)?;
    let lines = p.num_or("lines", 50.0, Some(1.0), Some(10_000.0), true)? as usize;
    let source = p.enum_or("source", &["visible", "recent", "recent-unwrapped"], "recent")?;
    let format = p.enum_or("format", &["text", "ansi"], "text")?;
    let id = srv.need(target.as_deref(), caller.as_deref())?;
    srv.snapshot(&id, lines, &source, &format)
}

async fn pane_keys(shared: Shared, p: Value, _c: u64) -> RpcResult {
    let p = Params::new(&p)?;
    let caller = caller(&p)?;
    let target = p.len("target", 1, None)?;
    let keys = p.str_list("keys", 1, 0)?.ok_or_else(|| crate::protocol::schema::invalid("keys", "Invalid input: expected array, received undefined"))?;
    let (id, instance) = {
        let srv = shared.borrow();
        let id = srv.need(Some(&target), None)?;
        let instance = srv.s.panes[&id].info.instance.clone();
        (id, instance)
    };
    permissions::permit(&shared, caller.as_deref(), "keys", &id, &keys.join(" ")).await?;
    let srv = shared.borrow();
    if let Some(pane) = srv.s.panes.get(&id).filter(|p| p.info.instance == instance) {
        pane.write(keys.iter().map(|k| keys::key_bytes(k)).collect::<String>().as_bytes());
    }
    Ok(json!(true))
}

async fn pane_close(shared: Shared, p: Value, _c: u64) -> RpcResult {
    let p = Params::new(&p)?;
    let caller = caller(&p)?;
    let target = p.opt_len("target", 1, None)?;
    let (id, instance) = {
        let srv = shared.borrow();
        let id = srv.need(target.as_deref(), caller.as_deref())?;
        let instance = srv.s.panes[&id].info.instance.clone();
        (id, instance)
    };
    permissions::permit(&shared, caller.as_deref(), "close", &id, "").await?;
    let mut srv = shared.borrow_mut();
    if srv.s.panes.get(&id).is_some_and(|p| p.info.instance == instance) {
        srv.s.close(Some(&id));
    }
    srv.settle();
    Ok(json!(true))
}

fn pane_rename(srv: &mut Server, p: &Value, _c: u64) -> RpcResult {
    let p = Params::new(p)?;
    let caller = caller(&p)?;
    let target = p.opt_len("target", 1, None)?;
    let name = p.str("name")?;
    let id = srv.need(target.as_deref(), caller.as_deref())?;
    srv.s.rename_pane(&id, &name);
    Ok(json!(true))
}

// Values for a pane that formats and plugins show (CUSTOMIZE.md, Metadata): `values` a record of key → value (strings,
// numbers or booleans), for `ttl` seconds if given.
fn pane_meta_set(srv: &mut Server, p: &Value, _c: u64) -> RpcResult {
    use crate::server::session::pane::META_KEYS;
    let p = Params::new(p)?;
    let caller = caller(&p)?;
    let target = p.opt_len("target", 1, None)?;
    let values = p.record("values")?.unwrap_or_default();
    let ttl = p.opt_num("ttl", Some(0.0), Some(86400.0 * 30.0), false)?.map(|s| (s * 1000.0) as u64);
    let mut set = Vec::new();
    for (k, v) in &values {
        let key = k.to_lowercase();
        if key.is_empty() || key.len() > 32 || !key.chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c)) {
            return Err(invalid(&format!("values.{k}"), "a key is 1 to 32 of a-z, 0-9, _ . -"));
        }
        let text = match v {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            _ => return Err(invalid(&format!("values.{k}"), "a value is a string, a number or a boolean")),
        };
        set.push((key, text));
    }
    let id = srv.subject(target.as_deref(), caller.as_deref())?;
    let pane = srv.s.panes.get_mut(&id).ok_or_else(|| fail("not_found", format!("no pane {id}")))?;
    if set.iter().filter(|(k, _)| !pane.info.meta.contains_key(k)).count() + pane.info.meta.len() > META_KEYS {
        return Err(fail("limit", format!("a pane has at most {META_KEYS} values")));
    }
    let changed = set.into_iter().fold(false, |any, (k, v)| pane.set_meta(k, v, ttl) || any);
    if changed {
        srv.s.changed();
    }
    Ok(json!(srv.s.panes[&id].info.meta))
}

fn pane_meta_clear(srv: &mut Server, p: &Value, _c: u64) -> RpcResult {
    let p = Params::new(p)?;
    let caller = caller(&p)?;
    let target = p.opt_len("target", 1, None)?;
    let keys: Vec<String> = p.str_list("keys", 0, 1)?.unwrap_or_default().iter().map(|k| k.to_lowercase()).collect();
    let id = srv.subject(target.as_deref(), caller.as_deref())?;
    if srv.s.panes.get_mut(&id).is_some_and(|pane| pane.clear_meta(&keys)) {
        srv.s.changed();
    }
    Ok(json!(true))
}

fn pane_focus(srv: &mut Server, p: &Value, _c: u64) -> RpcResult {
    let p = Params::new(p)?;
    let caller = caller(&p)?;
    let target = p.opt_len("target", 1, None)?;
    let dir = p.opt_enum("dir", DIRECTIONS)?;
    let id = srv.subject(target.as_deref(), caller.as_deref())?;
    match dir {
        None => srv.s.focus_pane(&id),
        Some(d) => {
            if srv.s.focus_dir(dir_of(&d), Some(&id)).is_none() {
                return Err(no_neighbor(&id, dir_of(&d)));
            }
        }
    }
    Ok(json!(true))
}

// no permission asked: like focus, these rearrange panes and touch nothing running in them
async fn pane_move(shared: Shared, p: Value, _c: u64) -> RpcResult {
    let p = Params::new(&p)?;
    let caller = caller(&p)?;
    let target = p.opt_len("target", 1, None)?;
    let tab = p.opt_len("tab", 1, None)?;
    let beside = p.opt_len("beside", 1, None)?;
    let new_tab = p.flag("newTab")?;
    let workspace = p.opt_len("workspace", 1, None)?;
    let new_workspace = p.flag("newWorkspace")?;
    let name = p.opt_str("name")?;
    let dir = p.enum_or("dir", &["right", "down"], "right")?;
    let ratio = p.num_or("ratio", 0.5, Some(0.1), Some(0.9), false)?;
    let focus = p.flag("focus")?;
    p.refine([tab.is_some() || beside.is_some(), new_tab, new_workspace].iter().filter(|x| **x).count() == 1, "exactly one destination: tab and/or beside, newTab, or newWorkspace")?;
    let (id, pid, running, cwd) = {
        let srv = shared.borrow();
        let id = movable(&srv, &placed(&srv, target.as_deref(), caller.as_deref())?)?;
        let pane = &srv.s.panes[&id];
        (id.clone(), pane.pid, pane.info.running(), pane.info.cwd.clone())
    };
    // a new space starts where the pane is now: its shell may have cd'd since it started
    let here = if new_workspace && running { crate::platform::procs::cwd(pid) } else { None };
    let mut srv = shared.borrow_mut();
    if !srv.s.panes.contains_key(&id) {
        return Err(fail("pane_gone", format!("{id} closed before it could be moved")));
    }
    let to = if new_workspace {
        MoveTo::NewSpace { name, cwd: here.unwrap_or(cwd) }
    } else if new_tab {
        let ws = match workspace {
            Some(w) => {
                let i = srv.s.find_workspace(&w)?;
                srv.s.workspaces[i].id.clone()
            }
            None => srv.s.workspaces[srv.s.place_of(&id)?.0].id.clone(),
        };
        MoveTo::NewTab { ws, name }
    } else {
        let tab = match &tab {
            Some(t) => Some(srv.s.find_tab(t)?),
            None => None,
        };
        let beside = match &beside {
            Some(b) => srv.need(Some(b), None)?,
            None => {
                let (wi, ti) = tab.unwrap();
                srv.s.workspaces[wi].tabs[ti].focused.clone()
            }
        };
        if let Some(t) = tab {
            if srv.s.locate(&beside) != Some(t) {
                return Err(error(format!("{beside} isn't in tab {}", p.opt_str("tab")?.unwrap_or_default())));
            }
        }
        MoveTo::Beside { beside, dir: axis(&dir), share: ratio }
    };
    let (ws, tab) = srv.s.move_pane(&id, to, focus)?;
    let instance = srv.s.panes[&id].info.instance.clone();
    srv.settle();
    Ok(json!({ "pane": id, "instance": instance, "workspaceId": ws, "tabId": tab }))
}

fn pane_swap(srv: &mut Server, p: &Value, _c: u64) -> RpcResult {
    let p = Params::new(p)?;
    let caller = caller(&p)?;
    let target = p.opt_len("target", 1, None)?;
    let with = p.opt_len("with", 1, None)?;
    let dir = p.opt_enum("dir", DIRECTIONS)?;
    p.refine(with.is_none() != dir.is_none(), "exactly one of with or dir")?;
    let a = placed(srv, target.as_deref(), caller.as_deref())?;
    let b = match &with {
        Some(w) => Some(srv.need(Some(w), None)?),
        None => srv.s.neighbor_of(&a, dir_of(dir.as_deref().unwrap())),
    };
    let b = b.ok_or_else(|| no_neighbor(&a, dir_of(dir.as_deref().unwrap_or("right"))))?;
    let (a, b) = (movable(srv, &a)?, movable(srv, &b)?);
    srv.s.swap(&a, &b)?;
    Ok(json!(true))
}

fn pane_resize(srv: &mut Server, p: &Value, _c: u64) -> RpcResult {
    let p = Params::new(p)?;
    let caller = caller(&p)?;
    let target = p.opt_len("target", 1, None)?;
    let dir = p.opt_enum("dir", DIRECTIONS)?.ok_or_else(|| crate::protocol::schema::invalid("dir", "Invalid option: expected one of \"left\"|\"right\"|\"up\"|\"down\""))?;
    let amount = p.num_or("amount", 2.0, Some(1.0), Some(1000.0), true)? as i32;
    let id = placed(srv, target.as_deref(), caller.as_deref())?;
    Ok(json!({ "changed": srv.s.resize_pane(dir_of(&dir), amount, Some(&id)) }))
}

fn pane_zoom(srv: &mut Server, p: &Value, _c: u64) -> RpcResult {
    let p = Params::new(p)?;
    let caller = caller(&p)?;
    let target = p.opt_len("target", 1, None)?;
    let mode = p.enum_or("mode", &["on", "off", "toggle"], "toggle")?;
    let id = placed(srv, target.as_deref(), caller.as_deref())?;
    Ok(json!({ "zoomed": srv.s.zoom(Some(&id), &mode) }))
}

async fn wait(shared: Shared, p: Value, _c: u64) -> RpcResult {
    let p = Params::new(&p)?;
    caller(&p)?;
    let target = p.len("target", 1, None)?;
    let exited = p.flag("exited")?;
    let want = p.opt_enum("state", STATES)?.and_then(|s| AgentState::parse(&s));
    let re = match p.opt_str("match")? {
        Some(m) => Some(regex::RegexBuilder::new(&m).multi_line(true).build().map_err(|e| error(format!("invalid regular expression: {e}")))?),
        None => None,
    };
    let timeout = p.opt_num("timeout", None, None, false)?;
    if timeout.is_some_and(|t| t <= 0.0) {
        return Err(crate::protocol::schema::invalid("timeout", "Too small: expected number to be >0"));
    }
    let (id, instance) = {
        let srv = shared.borrow();
        let id = srv.need(Some(&target), None)?;
        let instance = srv.s.panes[&id].info.instance.clone();
        (id, instance)
    };
    let deadline = timeout.map(|t| tokio::time::Instant::now() + Duration::from_secs_f64(t));
    loop {
        {
            let srv = shared.borrow();
            let pane = srv.s.panes.get(&id).filter(|p| p.info.instance == instance);
            // exited-then-closed (a shell pane closes itself on exit) still counts as exited; an exit caused by the
            // close (its SIGHUP) doesn't, so that fails like any other close
            if exited {
                let info = pane.map(|p| (&p.info, p.closed_while_running)).or_else(|| srv.s.retired_exit(&instance));
                if let Some((info, false)) = info.filter(|(i, _)| !i.running()) {
                    return Ok(json!({ "exitCode": info.exit_code }));
                }
            }
            let Some(pane) = pane else { return Err(fail("pane_gone", format!("{id} closed before the wait was met"))) };
            let st = pane.info.agent.as_ref().map(|a| a.state);
            if let (Some(want), Some(st)) = (want, st) {
                if st == want || (want == AgentState::Idle && st == AgentState::Done) {
                    return Ok(json!({ "state": st }));
                }
            }
            if let Some(re) = &re {
                let text = pane.text();
                let lines: Vec<&str> = text.split('\n').collect();
                if let Some(m) = re.find(&lines[lines.len().saturating_sub(500)..].join("\n")) {
                    return Ok(json!({ "match": m.as_str() }));
                }
            }
        }
        if deadline.is_some_and(|d| tokio::time::Instant::now() > d) {
            return Err(fail("timeout", "timeout"));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

// ---------- agents ----------

fn agent_spawn(srv: &mut Server, p: &Value, _c: u64) -> RpcResult {
    let p = Params::new(p)?;
    let caller = caller(&p)?;
    let harness = p.str("harness")?;
    let name = p.opt_str("name")?;
    let prompt = p.opt_str("prompt")?;
    let target = p.opt_len("target", 1, None)?;
    let dir = p.enum_or("dir", &["right", "down"], "right")?;
    let tab = p.flag("tab")?;
    let focus = p.flag("focus")?;
    let env = p.env("env")?;
    let o = SpawnOpts { env, ..srv.agent_opts(&harness, prompt.as_deref(), name.clone(), caller.clone()) };
    let id = if tab {
        srv.s.new_tab(name, o)?
    } else {
        let subject = srv.subject(target.as_deref(), caller.as_deref())?;
        srv.s.split(axis(&dir), o, Some(&subject), focus, 0.5)?.ok_or_else(|| error("nothing to split"))?
    };
    created(srv, &id)
}

fn agent_list(srv: &mut Server, p: &Value, _c: u64) -> RpcResult {
    Params::new(p).and_then(|p| caller(&p))?;
    Ok(json!(srv
        .s
        .panes
        .values()
        .filter_map(|p| {
            let a = p.info.agent.as_ref()?;
            let mut v = json!({ "id": p.id(), "name": p.info.name, "title": p.info.title, "harness": a.harness, "state": a.state, "source": a.source });
            if p.info.name.is_none() {
                v.as_object_mut().unwrap().remove("name");
            }
            if let Some((wi, _)) = srv.s.locate(p.id()) {
                v["workspace"] = json!(srv.s.workspaces[wi].name);
            }
            Some(v)
        })
        .collect::<Vec<_>>()))
}

fn report(srv: &mut Server, p: &Value, _c: u64) -> RpcResult {
    let p = Params::new(p)?;
    let caller = caller(&p)?;
    let pane = p.opt_str("pane")?;
    let source = p.opt_len("source", 1, None)?;
    let agent = p.opt_str("agent")?;
    let state = p.opt_enum("state", STATES)?.and_then(|s| AgentState::parse(&s));
    let seq = p.opt_num("seq", None, None, false)?;
    let session = p.opt_len("session", 1, None)?;
    let release = p.flag("release")?;
    let title = p.opt_str("title")?;
    let id = srv.need(pane.as_deref().or(caller.as_deref()), None)?;
    let src = source.clone().unwrap_or_else(|| "custom".into());
    if release {
        srv.detector.release(&id, &src);
    }
    if let Some(session) = &session {
        let pane = srv.s.panes.get_mut(&id).unwrap();
        let agent = agent.clone().or_else(|| pane.info.agent.as_ref().map(|a| a.harness.clone())).or_else(|| pane.info.harness.clone());
        if let Some(agent) = agent {
            pane.info.session = Some(crate::protocol::types::SessionRef { agent, id: session.clone(), source: src.clone() });
        }
        srv.changed(); // saved, so a restart resumes this exact session
    }
    if let Some(title) = &title {
        let pane = srv.s.panes.get_mut(&id).unwrap();
        pane.reported_title = clean_text(title, 200);
        if pane.refresh_title() {
            srv.changed();
        }
    }
    // state needs a named source: hooks from older modisa versions sent none and are ignored
    if let (Some(state), Some(source)) = (state, &source) {
        srv.detector.report(&id, source, agent.as_deref(), state, seq);
    }
    crate::server::agents::monitor::tick_soon(srv);
    Ok(json!(true))
}

async fn debug_detect(shared: Shared, p: Value, _c: u64) -> RpcResult {
    let p = Params::new(&p)?;
    let caller = caller(&p)?;
    let target = p.opt_len("target", 1, None)?;
    // read before waiting on the process table: the pane can close meanwhile, and its screen with it
    let seen = {
        let srv = shared.borrow();
        let id = srv.subject(target.as_deref(), caller.as_deref())?;
        let pane = &srv.s.panes[&id];
        let mut seen = json!({ "pane": id });
        let o = seen.as_object_mut().unwrap();
        if let Some(a) = &pane.info.agent {
            o.insert("agent".into(), json!(a));
        }
        if let Some(s) = &pane.info.session {
            o.insert("session".into(), json!(s));
        }
        if let Some(d) = srv.detector.last.get(&id) {
            o.insert("detection".into(), json!(d));
        }
        if let Some(a) = srv.detector.authority.get(&id) {
            o.insert("authority".into(), json!(a));
        }
        o.insert("title".into(), json!(pane.osc_title));
        o.insert("progress".into(), json!(pane.osc_progress));
        o.insert("screen".into(), json!(pane.screen_text()));
        o.insert("process".into(), process_of(pane.pid, pane.info.running(), pane.foreground()));
        seen
    };
    Ok(seen)
}

// Its process, the job in the foreground of its terminal (the process leading the group its pty has in the
// foreground), and where the shell is now (it may have cd'd since it started); an exited pane has only the pid it had.
pub fn process_of(pid: i32, running: bool, foreground: Option<i32>) -> Value {
    use crate::platform::procs;
    let mut v = json!({ "pid": pid });
    if !running {
        return v;
    }
    if let Some(fg) = foreground.and_then(procs::info) {
        v["foreground"] = json!({ "pid": fg.pid, "args": fg.args });
    }
    if let Some(d) = procs::cwd(pid) {
        v["cwd"] = json!(d);
    }
    v
}

// A toast, titled with the pane that sent it (its @name, else its id), else "notify"; a plugin's connection is the
// plugin, within its budget. How many clients it reached: none attached is 0, not a failure.
fn notify(srv: &mut Server, p: &Value, c: u64) -> RpcResult {
    let p = Params::new(p)?;
    let caller = caller(&p)?;
    let title = p.len("title", 1, None)?;
    let body = p.opt_str("body")?;
    let tone = p.enum_or("tone", TONES, "fg")?;
    let system = p.flag("system")?;
    let sound = p.flag("sound")?;
    let plugin = srv.clients.get(&c).and_then(|c| c.plugin.clone());
    let pane = caller.as_deref().and_then(|c| srv.s.panes.get(c));
    let from = plugin.clone().unwrap_or_else(|| match pane {
        Some(p) => p.info.name.as_ref().map(|n| format!("@{n}")).unwrap_or_else(|| p.id().into()),
        None => "notify".into(),
    });
    let source = match (&plugin, pane) {
        (Some(pl), _) => format!("plugin:{pl}"),
        (None, Some(p)) => format!("pane:{}", p.id()),
        _ => "user".into(),
    };
    let text = match body {
        Some(b) if !b.is_empty() => format!("{title}: {b}"),
        _ => title,
    };
    let text = regex::Regex::new(r"\s*\n\s*").unwrap().replace_all(&text, " ").into_owned(); // one line
    let clients = srv.toast(Toast { from, source, plugin: plugin.is_some(), text, tone, system, sound })?;
    Ok(json!({ "clients": clients }))
}

// ---------- messaging ----------

fn send(srv: &mut Server, p: &Value, _c: u64) -> RpcResult {
    let p = Params::new(p)?;
    let caller = caller(&p)?;
    let to = p.len("to", 1, None)?;
    let body = p.len("body", 1, None)?;
    let from = caller.filter(|c| srv.s.panes.contains_key(c)).unwrap_or_else(|| "user".into());
    let to = srv.need(Some(&to), None)?;
    let recipient = &srv.s.panes[&to].info;
    if recipient.agent.is_none() && recipient.harness.is_none() {
        return Err(error(format!("{} is not an agent pane", srv.name(&to))));
    }
    let state = recipient.agent.as_ref().map(|a| a.state);
    let reply_to = (from != "user").then(|| format!("{from}:{}", srv.s.panes[&from].info.instance));
    let (max_hops, per_minute) = (srv.cfg.messaging.max_hops as u32, srv.cfg.messaging.per_minute as usize);
    let (from_name, to_name) = (srv.name(&from), srv.name(&to));
    let m = srv.mail.send(&from, &from_name, &to, &to_name, &body, reply_to, max_hops, per_minute)?;
    srv.emit("message.sent", json!({ "id": m.id, "from": m.from_name, "to": m.to_name, "hops": m.hops }));
    srv.changed();
    // queued, not delivered: it's typed in when the recipient is idle (see `messages`)
    let mut v = json!({ "id": m.id, "queued": true, "delivered": false });
    if let Some(s) = state {
        v["recipientState"] = json!(s);
    }
    Ok(v)
}

fn inbox(srv: &mut Server, p: &Value, _c: u64) -> RpcResult {
    let p = Params::new(p)?;
    let caller = caller(&p)?;
    let id = srv.need(None, caller.as_deref())?;
    Ok(json!(srv
        .mail
        .take(&id)
        .into_iter()
        .map(|m| {
            let mut v = json!({ "id": m.id, "from": m.from_name, "replyTo": m.reply_to, "body": m.body, "at": m.at });
            if m.reply_to.is_none() {
                v.as_object_mut().unwrap().remove("replyTo");
            }
            v
        })
        .collect::<Vec<_>>()))
}

fn messages(srv: &mut Server, _p: &Value, _c: u64) -> RpcResult {
    Ok(json!(srv.mail.log))
}

fn messaging_pause(srv: &mut Server, p: &Value, _c: u64) -> RpcResult {
    let p = Params::new(p)?;
    srv.mail.paused = p.opt_bool("paused")?.unwrap_or(!srv.mail.paused);
    srv.changed();
    Ok(json!({ "paused": srv.mail.paused }))
}

// ---------- events & lifecycle ----------

// The snapshot is taken in the same synchronous step that turns events on, so every change after it is an event with
// a higher seq and nothing is in both. An event can reach the client before this reply does: order by seq.
fn events_subscribe(srv: &mut Server, p: &Value, c: u64) -> RpcResult {
    let p = Params::new(p)?;
    caller(&p)?;
    let output = p.flag("output")?;
    let snapshot = p.flag("snapshot")?;
    if let Some(client) = srv.clients.get_mut(&c) {
        client.events = true;
        client.output = output;
    }
    let mut v = json!({ "protocol": PROTOCOL, "epoch": srv.epoch, "seq": srv.seq });
    if snapshot {
        v["panes"] = json!(listed(srv));
    }
    Ok(v)
}

fn integration(_srv: &mut Server, p: &Value, _c: u64) -> RpcResult {
    let p = Params::new(p)?;
    let caller = caller(&p)?;
    let id = p.len("id", 1, None)?;
    let install = p.opt_bool("install")?.ok_or_else(|| crate::protocol::schema::invalid("install", "Invalid input: expected boolean, received undefined"))?;
    if caller.is_some() {
        return Err(error("only the user can change integrations")); // not agents in panes
    }
    crate::integrations::set_integration(&id, install)
}

// Save, stop, and let the caller start a fresh server on the current code; it restores the session.
fn lifecycle(srv: &mut Server, empty: bool, why: &'static str) -> RpcResult {
    let shared = srv.shared();
    tokio::task::spawn_local(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        shutdown(shared, empty, why).await;
    });
    Ok(json!(true))
}
