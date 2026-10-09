// Single-pane attach (`modisa pane attach`): another terminal shows one pane full-screen and is its emulator, sent the
// pane's screen once and then its output as it comes. Takeover drives it: one connection per pane, the pane held at that
// terminal's size (Session.size_locks) and everyone else's typing dropped (rpc/client.rs `input`). Observe only watches,
// at the pane's own size. Watchers aren't attached clients: no views, prompts or focus. A watch ends when its connection
// closes, the pane's process exits, or the pane closes, and then a takeover gives the pane back its box.
use std::sync::LazyLock;

use regex::bytes::Regex;
use serde_json::{json, Value};

use crate::async_handler;
use crate::protocol::conn::{b64, error, fail, RpcResult};
use crate::protocol::schema::Params;
use crate::protocol::types::PaneInfo;
use crate::server::permissions;
use crate::server::rpc::dispatch::Handler;
use crate::server::session::pane::PtyPane;
use crate::server::{Server, Shared};

#[derive(Clone, Debug, PartialEq)]
pub struct Watch {
    pub pane: String,
    pub takeover: bool, // else observe
}

// A watcher's terminal stays on its alternate screen (its own is the user's shell), so it drops a program's screen
// switches; a chunk with one is sent as the pane's whole screen redrawn instead, as it is after that chunk.
static SWITCH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1b\[\?(?:1049|1047|47)[hl]").unwrap());

pub fn for_watchers(p: &PtyPane, bytes: &[u8], data: &str) -> String {
    if bytes.contains(&0x1b) && SWITCH.is_match(bytes) {
        b64(format!("\x1b[H\x1b[2J{}", p.replay()).as_bytes())
    } else {
        data.to_string()
    }
}

pub fn route(method: &str) -> Option<Handler> {
    Some(match method {
        "pane.attach" => async_handler!(pane_attach),
        "pane.attach.resize" => Handler::Sync(pane_attach_resize),
        _ => return None,
    })
}

// `end` says why to a watcher still there to hear it: (reason, exit code)
fn release(srv: &mut Server, c: u64, end: Option<(&str, Option<i32>)>) {
    let Some(w) = srv.watching.remove(&c) else { return };
    if let Some(set) = srv.watchers.get_mut(&w.pane) {
        set.retain(|x| *x != c);
        if set.is_empty() {
            srv.watchers.remove(&w.pane);
        }
    }
    if srv.takeovers.get(&w.pane) == Some(&c) {
        srv.takeovers.remove(&w.pane);
        srv.s.size_locks.remove(&w.pane);
        if let Some(p) = srv.s.panes.get_mut(&w.pane) {
            p.info.takeover = None;
        }
        srv.s.layout(); // back to its box, and clients drop the border's note
    }
    if let (Some((reason, code)), Some(client)) = (end, srv.clients.get(&c)) {
        let mut v = json!({ "pane": w.pane, "reason": reason });
        if let Some(code) = code {
            v["exitCode"] = json!(code);
        }
        client.conn.notify("attach.end", v);
    }
}

fn end_all(srv: &mut Server, id: &str, end: (&str, Option<i32>)) {
    for c in srv.watchers.get(id).cloned().unwrap_or_default() {
        release(srv, c, Some(end));
    }
}

pub fn disconnected(srv: &mut Server, c: u64) {
    release(srv, c, None);
}

pub fn pane_exited(srv: &mut Server, info: &PaneInfo) {
    end_all(srv, &info.id, ("exited", info.exit_code));
}

// a shell pane closes itself when its process exits: that's an exit, with its code
pub fn pane_closing(srv: &mut Server, id: &str, info: &PaneInfo) {
    if info.running() {
        end_all(srv, id, ("closed", None));
    } else {
        end_all(srv, id, ("exited", info.exit_code));
    }
}

fn busy(srv: &Server, id: &str) -> RpcResult<()> {
    if srv.takeovers.contains_key(id) {
        return Err(fail("ui_busy", format!("{id} is already taken over from another terminal; --observe watches it")));
    }
    Ok(())
}

async fn pane_attach(shared: Shared, p: Value, c: u64) -> RpcResult {
    let p = Params::new(&p)?;
    let caller = p.opt_str("caller")?;
    let target = p.opt_len("target", 1, None)?;
    let takeover = p.enum_or("mode", &["takeover", "observe"], "takeover")? == "takeover";
    let cols = p.opt_num("cols", Some(2.0), Some(1000.0), true)?.ok_or_else(|| crate::protocol::schema::invalid("cols", "Invalid input: expected number, received undefined"))? as u16;
    let rows = p.opt_num("rows", Some(1.0), Some(500.0), true)?.ok_or_else(|| crate::protocol::schema::invalid("rows", "Invalid input: expected number, received undefined"))? as u16;
    let (id, instance) = {
        let srv = shared.borrow();
        if let Some(w) = srv.watching.get(&c) {
            return Err(fail("usage", format!("this connection is already attached to {}", w.pane)));
        }
        let id = srv.subject(target.as_deref(), caller.as_deref())?;
        // its own output would come straight back to it
        if Some(&id) == caller.as_ref() {
            return Err(fail("usage", format!("{id} is your own pane: attach to another one")));
        }
        let pane = &srv.s.panes[&id];
        if !pane.info.running() {
            let code = pane.info.exit_code.map(|c| c.to_string()).unwrap_or_else(|| "?".into());
            return Err(error(format!("{id} has exited ({code}): modisa pane read {id} shows what it left")));
        }
        if takeover {
            if pane.info.popup == Some(true) {
                return Err(fail("usage", format!("{id} is a plugin's popup, sized by the client that opened it; --observe watches it")));
            }
            busy(&srv, &id)?;
        }
        (id, pane.info.instance.clone())
    };
    if takeover {
        permissions::permit(&shared, caller.as_deref(), "keys", &id, "take it over from another terminal (pane attach)").await?;
    }
    let mut srv = shared.borrow_mut();
    // the wait for an answer can outlast the pane, the connection, or someone else's takeover
    if takeover {
        if !srv.s.panes.get(&id).is_some_and(|p| p.info.instance == instance && p.info.running()) {
            return Err(fail("pane_gone", format!("{id} closed or exited before it could be attached")));
        }
        if srv.clients.get(&c).is_none_or(|c| c.conn.closed()) {
            return Err(error("the connection closed"));
        }
        busy(&srv, &id)?;
    }
    // One synchronous step from here to the replay: every byte after it reaches this connection, and none before it.
    srv.watching.insert(c, Watch { pane: id.clone(), takeover });
    srv.watchers.entry(id.clone()).or_default().push(c);
    if takeover {
        srv.takeovers.insert(id.clone(), c);
        srv.s.panes.get_mut(&id).unwrap().info.takeover = Some(true);
        srv.s.size_locks.insert(id.clone(), (cols, rows));
        srv.s.layout(); // resized before the replay, so it's drawn at the terminal's size
    }
    let pane = &srv.s.panes[&id];
    let v = json!({ "pane": id, "instance": instance, "mode": if takeover { "takeover" } else { "observe" }, "cols": pane.info.cols, "rows": pane.info.rows, "data": b64(pane.replay().as_bytes()) });
    srv.settle();
    Ok(v)
}

fn pane_attach_resize(srv: &mut Server, p: &Value, c: u64) -> RpcResult {
    let p = Params::new(p)?;
    p.opt_str("caller")?;
    let cols = p.opt_num("cols", Some(2.0), Some(1000.0), true)?.unwrap_or(80.0) as u16;
    let rows = p.opt_num("rows", Some(1.0), Some(500.0), true)?.unwrap_or(24.0) as u16;
    let Some(w) = srv.watching.get(&c).filter(|w| w.takeover).cloned() else {
        return Err(fail("usage", "only the connection that took a pane over (pane.attach) sizes it"));
    };
    srv.s.size_locks.insert(w.pane.clone(), (cols, rows));
    srv.s.layout();
    let pane = &srv.s.panes[&w.pane];
    Ok(json!({ "cols": pane.info.cols, "rows": pane.info.rows }))
}
