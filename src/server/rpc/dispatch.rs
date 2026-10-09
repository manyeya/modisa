// Route a JSON-RPC request to its handler, which validates its params (protocol/schema.rs `Params`), and send back the
// result or a JSON-RPC error. A handler that never waits runs right here, in the order requests arrive; one that waits
// (on a person, a process, a timer) runs as a task of its own.
use std::future::Future;
use std::pin::Pin;

use serde_json::{json, Value};

use crate::protocol::conn::RpcResult;
use crate::server::{Server, Shared};

pub type SyncHandler = fn(&mut Server, &Value, u64) -> RpcResult;
pub type AsyncHandler = fn(Shared, Value, u64) -> Pin<Box<dyn Future<Output = RpcResult>>>;

pub enum Handler {
    Sync(SyncHandler),
    Async(AsyncHandler),
}

fn route(method: &str) -> Option<Handler> {
    super::client::route(method)
        .or_else(|| super::api::route(method))
        .or_else(|| crate::server::attach::route(method))
        .or_else(|| crate::server::plugins::route(method))
        .or_else(|| crate::server::plugin_manager::route(method))
}

// A request made from inside the server, as if `client` had sent it: the plugin manager asks this session's plugin host
// (plugin.list, plugin.start, plugin.stop) this way.
pub async fn call(shared: &Shared, client: u64, method: &str, params: Value) -> RpcResult {
    match route(method) {
        None => Err(crate::protocol::conn::fail("unknown_method", format!("unknown method {method}"))),
        Some(Handler::Sync(f)) => {
            let mut srv = shared.borrow_mut();
            let r = f(&mut srv, &params, client);
            srv.settle();
            r
        }
        Some(Handler::Async(f)) => f(shared.clone(), params, client).await,
    }
}

pub fn dispatch(shared: &Shared, client: u64, m: Value) {
    let Some(method) = m.get("method").and_then(Value::as_str).map(String::from) else { return };
    let id = m.get("id").cloned();
    let params = m.get("params").cloned().unwrap_or(Value::Null);
    let Some(conn) = shared.borrow().clients.get(&client).map(|c| c.conn.clone()) else { return };
    let reply = move |r: RpcResult| {
        let Some(id) = &id else { return };
        match r {
            Ok(v) => conn.reply(id, Ok(v)),
            Err(e) if e.code == "invalid_params" => conn.reply_error(id, -32602, &e.message, &e.code),
            Err(e) => conn.reply(id, Err(e)),
        }
    };
    // Only a plugin's bound connection (after plugin.hello) is attributed to the plugin, and it can't claim to be a
    // pane: `caller` is how panes (agents) are told apart, and permission prompts depend on it. Any unbound connection,
    // a second one from the same plugin included, is trusted as the local user, caller claims and all. Nothing here is
    // authentication.
    let plugin = shared.borrow().clients.get(&client).and_then(|c| c.plugin.clone());
    if let (Some(plugin), Some(_)) = (plugin, params.get("caller")) {
        return reply(Err(crate::protocol::conn::fail("invalid_params", format!("invalid params: caller: plugin {plugin} acts as itself, not as a pane"))));
    }
    // a [[hook]] stands in front of this request: it runs first, then the request with what it left
    let hooked = crate::server::hooks::intercepted(&method).filter(|e| shared.borrow().cfg.hook.iter().any(|h| h.on == *e));
    if let Some(event) = hooked {
        let shared = shared.clone();
        tokio::task::spawn_local(async move {
            match crate::server::hooks::intercept(&shared, event, params).await {
                Ok(params) => reply(call(&shared, client, &method, params).await),
                Err(e) => reply(Err(e)),
            }
        });
        return;
    }
    match route(&method) {
        None => {
            if let Some(id) = m.get("id") {
                if let Some(c) = shared.borrow().clients.get(&client) {
                    c.conn.reply_error(id, -32601, &format!("unknown method {method}"), "unknown_method");
                }
            }
        }
        Some(Handler::Sync(f)) => {
            let r = {
                let mut srv = shared.borrow_mut();
                let r = f(&mut srv, &params, client);
                srv.settle();
                r
            };
            reply(r.map(|v| if v.is_null() { json!(null) } else { v }));
        }
        Some(Handler::Async(f)) => {
            let shared = shared.clone();
            tokio::task::spawn_local(async move {
                let r = f(shared.clone(), params, client).await;
                if let Ok(mut srv) = shared.try_borrow_mut() {
                    srv.settle();
                }
                reply(r);
            });
        }
    }
}

// Wraps an async fn as a handler.
#[macro_export]
macro_rules! async_handler {
    ($f:path) => {
        $crate::server::rpc::dispatch::Handler::Async(|s, p, c| Box::pin($f(s, p, c)))
    };
}
