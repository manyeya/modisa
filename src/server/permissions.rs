// Agents acting on panes they didn't create: allowed, denied, or asked of whoever is attached.
use std::time::Duration;

use serde_json::json;
use tokio::sync::oneshot;

use crate::config::Policy;
use crate::protocol::conn::{error, RpcResult};
use crate::server::Shared;

pub async fn permit(shared: &Shared, caller: Option<&str>, action: &str, target: &str, detail: &str) -> RpcResult<()> {
    let (rx, id, instead, name) = {
        let mut srv = shared.borrow_mut();
        let Some(caller) = caller.filter(|c| srv.s.panes.contains_key(*c)) else { return Ok(()) };
        let Some(t) = srv.s.panes.get(target) else { return Ok(()) };
        if target == caller || t.info.created_by == caller {
            return Ok(());
        }
        let policy = match action {
            "keys" => srv.cfg.permissions.keys_foreign,
            "close" => srv.cfg.permissions.close_foreign,
            _ => srv.cfg.permissions.run_foreign,
        };
        let key = format!("{caller}:{action}:{target}");
        if policy == Policy::Allow || srv.always.contains(&key) {
            return Ok(());
        }
        let name = srv.name(target);
        // Typing at another agent is what the mailbox is for: say so, or every exchange interrupts the user.
        let instead = if action != "close" && (t.info.agent.is_some() || t.info.harness.is_some()) { format!("; to talk to it use: modisa send @{name} \"…\"") } else { String::new() };
        if policy == Policy::Deny {
            return Err(error(format!("permission denied: {action} on {name}{instead}")));
        }
        let to = srv.attached();
        if to.is_empty() {
            return Err(error(format!("permission needed for {action} on {name}, but no one is attached to approve it{instead}")));
        }
        srv.prompt_seq += 1;
        let id = srv.prompt_seq;
        let (tx, rx) = oneshot::channel();
        srv.prompts.insert(id, tx);
        let detail = if detail.is_empty() { String::new() } else { format!(":\n\n  {detail}") };
        let text = format!("@{} wants to {action} pane \"{name}\"{detail}", srv.name(caller));
        srv.broadcast_to("prompt", json!({ "id": id, "text": text }), &to);
        (rx, id, instead, (name, key))
    };
    let answer = tokio::time::timeout(Duration::from_secs(60), rx).await.ok().and_then(Result::ok).unwrap_or_else(|| "deny".into());
    let mut srv = shared.borrow_mut();
    srv.prompts.shift_remove(&id);
    srv.broadcast("prompt.done", json!({ "id": id }));
    match answer.as_str() {
        "always" => {
            srv.always.insert(name.1);
            Ok(())
        }
        "allow" => Ok(()),
        _ => Err(error(format!("denied by user: {action} on {}{instead}", name.0))),
    }
}
