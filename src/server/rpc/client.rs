// Methods only the TUI client uses: attaching, screen replay, input, and the UI commands behind keys, menus and mouse
// gestures.
use serde_json::{json, Value};

use super::dispatch::Handler;
use crate::core::layout::{Axis, Dir, Rect};
use crate::protocol::conn::{b64, error, unb64, RpcResult};
use crate::protocol::schema::Params;
use crate::server::session::SpawnOpts;
use crate::server::{understands_plugins, understands_slots, understands_views, Server};

pub fn route(method: &str) -> Option<Handler> {
    Some(Handler::Sync(match method {
        "attach" => attach,
        "replay" => replay,
        "detach" => detach,
        "detach-all" => detach_all,
        "area" => area,
        "input" => input,
        "promptReply" => prompt_reply,
        "cmd" => cmd,
        "search" => search,
        "adapters" => adapters,
        _ => return None,
    }))
}

fn rect(v: &Value) -> Option<Rect> {
    serde_json::from_value(v.clone()).ok()
}

fn attach(srv: &mut Server, p: &Value, c: u64) -> RpcResult {
    let client = srv.clients.get_mut(&c).unwrap();
    client.attached = true;
    client.ui = p.get("ui").and_then(Value::as_u64).unwrap_or(0) as u32; // the plugin UI it can draw; a client from before plugin UI sends none
    if let Some(a) = p.get("area").and_then(rect) {
        srv.s.set_area(a);
    }
    srv.emit("client.attached", json!({}));
    let client = &srv.clients[&c];
    let (plugins, views, slots) = (understands_plugins(client), understands_views(client), understands_slots(client));
    let mut v = serde_json::to_value(srv.s.view()).unwrap();
    v["paused"] = json!(srv.mail.paused);
    if plugins {
        v["plugins"] = srv.plugin_ui(!slots);
    } else if let Some(o) = v.as_object_mut() {
        o.remove("plugins");
    }
    if views {
        v["views"] = srv.plugin_views();
    }
    if slots {
        v["slots"] = srv.slots();
    }
    v["session"] = json!(srv.session);
    v["prompts"] = json!(srv.prompts.keys().collect::<Vec<_>>());
    v["version"] = json!(srv.version);
    Ok(v)
}

// Current screen of every pane as a VT stream; the client asks once its terminals exist.
fn replay(srv: &mut Server, _p: &Value, _c: u64) -> RpcResult {
    Ok(json!(srv.s.panes.values().map(|p| json!({ "pane": p.info.id, "data": b64(p.replay().as_bytes()) })).collect::<Vec<_>>()))
}

fn detach(srv: &mut Server, _p: &Value, c: u64) -> RpcResult {
    if let Some(client) = srv.clients.get_mut(&c) {
        client.attached = false;
    }
    srv.changed();
    Ok(Value::Null)
}

fn detach_all(srv: &mut Server, _p: &Value, _c: u64) -> RpcResult {
    srv.broadcast("detach", json!({}));
    Ok(Value::Null)
}

fn area(srv: &mut Server, p: &Value, _c: u64) -> RpcResult {
    if let Some(a) = p.get("area").and_then(rect) {
        srv.s.set_area(a);
    }
    Ok(Value::Null)
}

// a pane taken over from another terminal (pane attach) types only what that terminal does
fn input(srv: &mut Server, p: &Value, c: u64) -> RpcResult {
    let pane = p["pane"].as_str().unwrap_or("");
    let owner = srv.takeovers.get(pane);
    if owner.is_none() || owner == Some(&c) {
        if let Some(pane) = srv.s.panes.get(pane) {
            pane.write(&unb64(p["data"].as_str().unwrap_or("")));
        }
    }
    Ok(Value::Null)
}

fn prompt_reply(srv: &mut Server, p: &Value, _c: u64) -> RpcResult {
    if let Some(tx) = p["id"].as_u64().and_then(|id| srv.prompts.shift_remove(&id)) {
        let _ = tx.send(p["answer"].as_str().unwrap_or("deny").to_string());
    }
    Ok(Value::Null)
}

fn dir_of(v: &Value) -> Option<Dir> {
    serde_json::from_value(v.clone()).ok()
}

fn cmd(srv: &mut Server, p: &Value, c: u64) -> RpcResult {
    let a = p.get("args").cloned().unwrap_or(json!({}));
    let text = |k: &str| a.get(k).and_then(Value::as_str).map(String::from);
    let int = |k: &str| a.get(k).and_then(Value::as_i64).unwrap_or(0);
    let s = &mut srv.s;
    match p["name"].as_str().unwrap_or("") {
        "split" => {
            let dir = if a["dir"].as_str() == Some("col") { Axis::Col } else { Axis::Row };
            s.split(dir, SpawnOpts::default(), None, true, 0.5)?;
        }
        "close" => s.close(text("pane").as_deref()),
        "closeTab" => s.close_tab(),
        "mutePane" => {
            if let Some(p) = text("pane").and_then(|id| s.panes.get_mut(&id)) {
                p.info.muted = !p.info.muted;
                s.changed();
            }
        }
        "focusDir" => {
            if let Some(d) = dir_of(&a["dir"]) {
                s.focus_dir(d, None);
            }
        }
        "focusPane" => s.focus_pane(&text("pane").unwrap_or_default()),
        "zoom" => {
            s.zoom(None, "toggle");
        }
        "resize" => {
            if let Some(d) = dir_of(&a["dir"]) {
                s.resize_pane(d, a.get("cells").and_then(Value::as_i64).unwrap_or(2) as i32, None);
            }
        }
        "selectTab" => s.select_tab(int("index")),
        "cycleTab" => s.cycle_tab(int("step")),
        "newTab" => {
            let o = match text("command") {
                Some(command) => SpawnOpts { command: Some(command), ephemeral: a["ephemeral"].as_bool().unwrap_or(false), ..Default::default() },
                None => SpawnOpts::default(),
            };
            s.new_tab(text("name"), o)?;
        }
        "selectWorkspace" => s.select_workspace(int("index")),
        "newWorkspace" => {
            let cwd = text("cwd").unwrap_or_else(|| s.ws().cwd.clone());
            s.new_workspace(text("name"), Some(cwd), SpawnOpts::default())?;
        }
        "renameTab" => s.rename_tab(&text("name").unwrap_or_default()),
        "renameWorkspace" => s.rename_workspace(&text("name").unwrap_or_default(), a.get("index").and_then(Value::as_u64).map(|i| i as usize)),
        "closeWorkspace" => s.close_workspace(a.get("index").and_then(Value::as_u64).map(|i| i as usize))?,
        "renamePane" => {
            let id = text("pane").or_else(|| s.focused_id()).unwrap_or_default();
            s.rename_pane(&id, &text("name").unwrap_or_default());
        }
        "dragStart" => {
            s.drag_start(c, int("x") as i32, int("y") as i32);
        }
        "dragMove" => s.drag_move(c, int("x") as i32, int("y") as i32),
        "dragEnd" => s.drag_end(c),
        "spawnAgent" => {
            let dir = if a["dir"].as_str() == Some("col") { Axis::Col } else { Axis::Row };
            let o = srv.agent_opts(&text("harness").unwrap_or_default(), None, text("name"), None);
            srv.s.split(dir, o, None, true, 0.5)?;
        }
        "pause" => {
            srv.mail.paused = !srv.mail.paused;
            srv.changed();
        }
        other => return Err(error(format!("unknown command {other}"))),
    }
    Ok(json!(true))
}

fn search(srv: &mut Server, p: &Value, _c: u64) -> RpcResult {
    let pr = Params::new(p)?;
    let id = srv.need(pr.opt_str("pane")?.as_deref(), None)?;
    let text = srv.s.panes[&id].text();
    let q = pr.opt_str("query")?.unwrap_or_default().to_lowercase();
    let lines: Vec<&str> = text.split('\n').collect();
    let matches: Vec<usize> = lines.iter().enumerate().filter(|(_, l)| l.to_lowercase().contains(&q)).map(|(i, _)| i).collect();
    Ok(json!({ "total": lines.len(), "matches": matches }))
}

fn adapters(srv: &mut Server, _p: &Value, _c: u64) -> RpcResult {
    Ok(json!(srv.adapters.iter().filter(|a| a.id != "generic").map(|a| json!({ "id": a.id, "name": a.name })).collect::<Vec<_>>()))
}
