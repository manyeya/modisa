// `modisa <noun> <verb>` — the socket API as shell commands, so any agent can drive panes with zero integration.
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use serde_json::{json, Map, Value};

use super::args::Args;
use super::help::HELP;
use crate::core::layout::{display_rects, neighbor, panes, rects, side, Dir, Node, Rect, Rects};
use crate::core::paths::abs_path;
use crate::protocol::conn::{error, error_code, fail, Conn, RpcError, RpcResult};
use crate::protocol::transport::connect_existing;
use crate::protocol::types::{find_pane, instance_target, PaneRef};

// Exit statuses scripts can branch on; every other failure exits 1. Listed in `modisa help`.
fn exit_status(code: &str) -> i32 {
    match code {
        "usage" | "invalid_params" => 2,
        "unreachable" => 3,
        "timeout" => 124,
        _ => 1,
    }
}

// Print a failure (as {"error":{code,message}} with --json) and return its exit status.
pub fn failed(code: &str, message: &str, json: bool) -> i32 {
    if json {
        errln!("{}", stringify(&json!({ "error": { "code": code, "message": message } }), false));
    } else {
        errln!("{message}");
    }
    exit_status(code)
}

// What a command prints with --json (snapshot, plugin ui and plugin run always): a string as it is, anything else as
// JSON.stringify(x, null, 2) writes it.
fn print(x: &Value) {
    match x {
        Value::String(s) => outln!("{s}"),
        _ => outln!("{}", stringify(x, true)),
    }
}

// console.log(x) of a field that may be missing.
fn log(x: Option<&Value>) {
    match x {
        // ponytail: console.log inspects objects; JSON reads the same to anyone but a parser
        Some(v @ (Value::Object(_) | Value::Array(_))) => outln!("{}", stringify(v, true)),
        _ => outln!("{}", tpl(x)),
    }
}

pub fn table(cols: &[&str], rows: Vec<Vec<String>>) {
    if rows.is_empty() {
        return outln!("(none)");
    }
    let len = |s: &str| s.encode_utf16().count(); // JavaScript's length, which padEnd counts in
    let pad = |s: &str, w: usize| format!("{s}{}", " ".repeat(w.saturating_sub(len(s))));
    let w: Vec<usize> = cols.iter().enumerate().map(|(i, c)| rows.iter().map(|r| len(&r[i])).max().unwrap_or(0).max(len(c))).collect();
    outln!("{}", cols.iter().enumerate().map(|(i, c)| pad(&c.to_uppercase(), w[i])).collect::<Vec<_>>().join("  "));
    for r in rows {
        outln!("{}", r.iter().enumerate().map(|(i, c)| pad(c, w[i])).collect::<Vec<_>>().join("  ").trim_end());
    }
}

const DIRS: [(&str, Dir); 4] = [("left", Dir::Left), ("right", Dir::Right), ("up", Dir::Up), ("down", Dir::Down)];

fn parse_dir(d: &str) -> Option<Dir> {
    DIRS.iter().find(|(name, _)| *name == d).map(|(_, dir)| *dir)
}

// Where a pane is, from a session.info snapshot: the server would take the same pane (named, else the calling pane,
// else the focused one) and refuse the same way.
struct Place<'a> {
    pane: &'a Value,
    ws: &'a Value,
    tab: &'a Value,
    tree: Node,
    area: Rect,
    rs: Rects,
}

fn place_in<'a>(snap: &'a Value, target: Option<&str>, caller: Option<&str>) -> RpcResult<Place<'a>> {
    let ps = arr(&snap["panes"]);
    let ws = index(&snap["workspaces"], &snap["active"]);
    let t = match (target, caller) {
        (Some(t), _) => Some(t.to_string()),
        (None, Some(c)) if !c.is_empty() && ps.iter().any(|p| p["id"] == c) => Some(c.to_string()),
        _ => ws.and_then(|w| index(&w["tabs"], &w["active"])).and_then(|t| t["focused"].as_str()).map(String::from),
    };
    let Some(pane) = t.as_deref().filter(|t| !t.is_empty()).and_then(|t| find_pane(ps, t)) else {
        return Err(match t.as_deref().and_then(instance_target) {
            Some((id, _)) => fail("pane_gone", format!("{id} has gone: that pane was closed or restarted since its message was sent")),
            None => fail("no_such_pane", format!("no such pane: {}", t.as_deref().unwrap_or("(none)"))),
        });
    };
    for w in arr(&snap["workspaces"]) {
        for tab in arr(&w["tabs"]) {
            let tree = tree_of(tab)?;
            if panes(&tree).iter().any(|p| p == pane.id()) {
                let area: Rect = serde_json::from_value(snap["area"].clone()).map_err(|_| error("server too old: modisa restart"))?;
                let rs = rects(&tree, area);
                return Ok(Place { pane, ws: w, tab, tree, area, rs });
            }
        }
    }
    Err(error(format!("{} is a popup: it has no place in a tab", pane.id())))
}

fn tree_of(tab: &Value) -> RpcResult<Node> {
    serde_json::from_value(tab["tree"].clone()).map_err(|e| error(format!("a tab's split tree doesn't read: {e}")))
}

// The callback a connection's messages go to, set once the command knows what it wants from them.
type Handler = Rc<RefCell<Option<Rc<dyn Fn(&Conn, Value)>>>>;

// Inside a pane, MODISA_SOCKET points at our own server unless a session is named explicitly (-s "" names none).
pub async fn connect_session(session: Option<&str>, on_message: impl Fn(&Conn, Value) + 'static) -> RpcResult<Conn> {
    let named = session.filter(|s| !s.is_empty() || std::env::var("MODISA_SOCKET").map_or(true, |v| v.is_empty()));
    connect_existing(named, on_message).await
}

// The connection dropped mid-request (the original's ConnectionClosedError): its server is unreachable now.
pub fn lost(conn: &Conn, e: &RpcError) -> bool {
    conn.closed() && e.message.starts_with("connection closed")
}

pub async fn run_cli(a: &Args) -> i32 {
    let noun = a.pos.first().map(String::as_str);
    let verb = a.pos.get(1).map(String::as_str);
    let json = a.on("json");
    let caller = std::env::var("MODISA_PANE_ID").ok();
    if noun == Some("report") && verb.is_none() && caller.as_deref().unwrap_or("").is_empty() {
        return 0; // an integration outside any modisa pane
    }
    if noun == Some("pane") && verb == Some("attach") {
        return super::pane_attach::pane_attach(a, a.pos.get(2).map(String::as_str)).await; // its own connection, maybe over ssh
    }
    let handler: Handler = Default::default();
    let h = handler.clone();
    let conn = match connect_session(a.str("session"), move |c, m| {
        let f = h.borrow().clone();
        if let Some(f) = f {
            f(c, m)
        }
    })
    .await
    {
        Ok(c) => c,
        Err(_) if noun == Some("report") => return 0, // hooks fire outside modisa too; stay quiet
        Err(e) => return failed("unreachable", &e.message, json),
    };
    let cli = Cli { a, conn: conn.clone(), caller, json, handler };
    let code = match cli.run(noun.unwrap_or("undefined"), verb).await {
        Ok(code) => code,
        Err(e) => failed(&if lost(&conn, &e) { "unreachable".into() } else { error_code(Some(&e.code)) }, &format!("modisa: {}", e.message), json),
    };
    if !a.on("follow") {
        conn.close();
    }
    code
}

struct Cli<'a> {
    a: &'a Args,
    conn: Conn,
    caller: Option<String>,
    json: bool,
    handler: Handler,
}

impl Cli<'_> {
    async fn call(&self, method: &str, params: Value) -> RpcResult {
        self.conn.request(method, with_caller(&self.caller, params), None).await
    }

    fn on_message(&self, f: impl Fn(&Conn, Value) + 'static) {
        *self.handler.borrow_mut() = Some(Rc::new(f));
    }

    fn target(&self, t: Option<&str>) -> Option<String> {
        t.or(self.a.str("target")).map(String::from)
    }

    // --cwd: ~ and relative paths are from here
    fn cwd_opt(&self) -> Option<String> {
        self.a.str("cwd").map(abs_path)
    }

    // --env NAME=value, once per variable (the server checks the names)
    fn env(&self) -> RpcResult<Option<Value>> {
        let Some(list) = self.a.lists.get("env") else { return Ok(None) };
        let mut vars = Map::new();
        for kv in list {
            match kv.find('=') {
                Some(eq) if eq >= 1 => vars.insert(kv[..eq].to_string(), kv[eq + 1..].into()),
                _ => return Err(fail("usage", format!("--env takes NAME=value, not \"{kv}\""))),
            };
        }
        Ok(Some(Value::Object(vars)))
    }

    // what session.info { snapshot } describes: a server from before it sends counts
    async fn snapshot(&self) -> RpcResult {
        let snap = self.call("session.info", json!({ "snapshot": true })).await?;
        if !snap["workspaces"].is_array() {
            return Err(fail("error", "server too old: modisa restart"));
        }
        Ok(snap)
    }

    async fn run(&self, noun: &str, verb: Option<&str>) -> RpcResult<i32> {
        let (a, json) = (self.a, self.json);
        let rest: Vec<&str> = a.pos.iter().skip(2).map(String::as_str).collect();
        let r = |i: usize| rest.get(i).copied();
        let caller = self.caller.as_deref();
        // The switch is on "noun verb"; a few cases take any verb ("wait <target>"), the way a template literal spells
        // it (a missing one is "undefined").
        let key = format!("{noun} {}", verb.unwrap_or("")).trim().to_string();
        let any = |noun: &str| format!("{noun} {}", verb.unwrap_or("undefined"));
        match key.as_str() {
            "pane list" | "list" => {
                let ps = self.call("list", json!({})).await?;
                if json {
                    print(&ps);
                } else {
                    let rows = arr(&ps).iter().map(|p| {
                        let agent = p.get("agent").filter(|x| truthy(Some(x)));
                        vec![
                            cell(p.get("id")),
                            at_name(p.get("name")),
                            cell(p.get("title")),
                            agent.map_or(String::new(), |x| format!("{}:{}", tpl(x.get("harness")), tpl(x.get("state")))),
                            if p["status"] == "exited" { format!("exited {}", tpl(p.get("exitCode"))) } else { "running".into() },
                            cell(p.get("workspace")),
                            cell(p.get("cwd")),
                        ]
                    });
                    table(&["id", "name", "title", "agent", "status", "workspace", "cwd"], rows.collect());
                }
            }
            "pane split" => {
                let joined = rest.join(" ");
                let command = if joined.is_empty() { s(a.str("command")) } else { s(Some(&joined)) };
                let p = self.call("pane.split", obj(vec![("target", s(self.target(None))), ("dir", s(Some(if a.on("down") { "down" } else { "right" }))), ("ratio", n(a.num("ratio"))), ("name", s(a.str("name"))), ("cwd", s(self.cwd_opt())), ("command", command), ("focus", b(a.on("focus"))), ("env", self.env()?)])).await?;
                if json { print(&p) } else { log(p.get("id")) }
            }
            "pane run" => {
                self.call("pane.run", obj(vec![("target", s(r(0))), ("command", s(Some(&rest.iter().skip(1).copied().collect::<Vec<_>>().join(" "))))])).await?;
            }
            "pane read" => {
                if a.on("screen") && a.has("source") && a.str("source") != Some("visible") {
                    return Err(fail("usage", "--screen is --source visible: use one of them"));
                }
                let source = if a.on("screen") { Some("visible") } else { a.str("source") };
                let snap = self.call("pane.read", obj(vec![("target", s(self.target(r(0)))), ("lines", n(Some(a.num("lines").unwrap_or(50.0)))), ("source", s(source)), ("format", s(a.str("format")))])).await?;
                if json {
                    print(&snap);
                } else if let Some(content) = snap.get("content") {
                    log(Some(content));
                } else {
                    // a server from before source and format: it sends the plain screen and recent lines, and nothing else
                    let old = if a.has("format") && a.str("format") != Some("text") {
                        None
                    } else if source == Some("visible") {
                        snap.get("screen")
                    } else if source.is_none() || source == Some("recent") {
                        snap.get("recentOutput")
                    } else {
                        None
                    };
                    let Some(old) = old else { return Err(fail("error", "server too old: modisa restart")) };
                    log(Some(old));
                }
            }
            "pane keys" => {
                self.call("pane.keys", obj(vec![("target", s(r(0))), ("keys", Some(json!(rest.iter().skip(1).collect::<Vec<_>>())))])).await?;
            }
            "pane close" => {
                self.call("pane.close", obj(vec![("target", s(self.target(r(0))))])).await?;
            }
            "pane rename" => {
                let p = if rest.len() > 1 { obj(vec![("target", s(r(0))), ("name", s(r(1)))]) } else { obj(vec![("name", s(r(0)))]) };
                self.call("pane.rename", p).await?;
            }
            "pane meta" => {
                // pane meta set key=value… | clear [key…]   (--pane p, --ttl seconds)
                let target = s(a.str("pane").map(String::from));
                match r(0) {
                    Some("set") => {
                        let values: Map<String, Value> = rest.iter().skip(1).filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.to_string(), json!(v))).collect();
                        if values.is_empty() {
                            return Err(fail("usage", "usage: modisa pane meta set <key>=<value>… [--pane p] [--ttl seconds]"));
                        }
                        let res = self.call("pane.meta.set", obj(vec![("target", target), ("values", Some(Value::Object(values))), ("ttl", n(a.num("ttl")))])).await?;
                        if json {
                            print(&res);
                        }
                    }
                    Some("clear") => {
                        self.call("pane.meta.clear", obj(vec![("target", target), ("keys", Some(json!(rest.iter().skip(1).collect::<Vec<_>>())))])).await?;
                    }
                    _ => return Err(fail("usage", "usage: modisa pane meta set <key>=<value>… | clear [key…]   [--pane p] [--ttl seconds]")),
                }
            }
            "pane focus" => {
                self.call("pane.focus", obj(vec![("target", s(r(0))), ("dir", s(a.str("direction")))])).await?;
            }
            "pane move" => {
                // --target is where it goes: the pane moved is the positional one
                let res = self.call("pane.move", obj(vec![("target", s(r(0))), ("tab", s(a.str("tab"))), ("beside", s(a.str("target"))), ("newTab", yes(a.switch("new-tab"))), ("workspace", s(a.str("workspace"))), ("newWorkspace", yes(a.switch("new-workspace"))), ("name", s(a.str("name"))), ("dir", s(a.str("split"))), ("ratio", n(a.num("ratio"))), ("focus", b(a.on("focus")))])).await?;
                if json {
                    print(&res);
                }
            }
            "pane swap" => {
                // pane swap [a] (<b> | --direction d): one positional without a direction is b
                let dir = a.str("direction");
                let (x, y) = if dir.is_some_and(|d| !d.is_empty()) || rest.len() > 1 { (r(0), r(1)) } else { (None, r(0)) };
                self.call("pane.swap", obj(vec![("target", s(x)), ("with", s(y)), ("dir", s(dir))])).await?;
            }
            "pane resize" => {
                let res = self.call("pane.resize", obj(vec![("target", s(r(0))), ("dir", s(a.str("direction"))), ("amount", n(a.num("amount")))])).await?;
                if json { print(&res) } else { outln!("{}", if truthy(res.get("changed")) { "changed" } else { "unchanged" }) }
            }
            // worked out here from a snapshot, with the layout math the server and the TUI use
            "pane layout" => {
                let snap = self.snapshot().await?;
                let at = place_in(&snap, r(0), caller)?;
                let focused = at.tab["focused"].as_str().unwrap_or("");
                let shown = display_rects(&at.tree, at.area, focused, truthy(at.tab.get("zoomed")));
                let name = |id: &str| arr(&snap["panes"]).iter().find(|p| p["id"] == id).and_then(|p| p.get("name")).cloned();
                let list: Vec<Value> = at.rs.iter().map(|(id, r)| obj(vec![("id", Some(json!(id))), ("name", name(id)), ("x", Some(json!(r.x))), ("y", Some(json!(r.y))), ("w", Some(json!(r.w))), ("h", Some(json!(r.h))), ("shown", b(shown.contains_key(id)))])).collect();
                if json {
                    print(&obj(vec![("pane", at.pane.get("id").cloned()), ("workspaceId", at.ws.get("id").cloned()), ("tabId", at.tab.get("id").cloned()), ("area", snap.get("area").cloned()), ("focused", at.tab.get("focused").cloned()), ("zoomed", at.tab.get("zoomed").cloned()), ("panes", Some(Value::Array(list)))]));
                } else {
                    let rows = list.iter().map(|p| vec![cell(p.get("id")), at_name(p.get("name")), cell(p.get("x")), cell(p.get("y")), cell(p.get("w")), cell(p.get("h")), (if p["shown"] == true { "yes" } else { "no" }).into(), (if p["id"] == focused { "*" } else { "" }).into()]);
                    table(&["id", "name", "x", "y", "w", "h", "shown", "focused"], rows.collect());
                }
            }
            "pane neighbor" => {
                let Some(d) = a.str("direction").and_then(parse_dir) else { return Err(fail("usage", "pane neighbor needs --direction left|right|up|down")) };
                let snap = self.snapshot().await?;
                let at = place_in(&snap, r(0), caller)?;
                let Some(n) = neighbor(&at.rs, at.pane.id(), d) else { return Err(fail("no_such_pane", format!("no pane {} {}", side(d), at.pane.id()))) };
                if json { log(arr(&snap["panes"]).iter().find(|p| p["id"] == n.as_str())) } else { outln!("{n}") }
            }
            "pane edges" => {
                let snap = self.snapshot().await?;
                let at = place_in(&snap, r(0), caller)?;
                let sides: Vec<(&str, Option<String>)> = DIRS.iter().map(|(name, d)| (*name, neighbor(&at.rs, at.pane.id(), *d))).collect(); // none: the tab's edge
                if json {
                    let mut r = Map::new();
                    r.insert("pane".into(), json!(at.pane.id()));
                    for (name, n) in &sides {
                        r.insert(name.to_string(), n.as_deref().map_or(Value::Null, Value::from));
                    }
                    print(&Value::Object(r));
                } else {
                    table(&["side", "pane"], sides.into_iter().map(|(name, n)| vec![name.to_string(), n.unwrap_or_else(|| "(edge)".into())]).collect());
                }
            }
            "pane process-info" => {
                let d = self.call("debug.detect", obj(vec![("target", s(r(0)))])).await?;
                let Some(process) = d.get("process").filter(|p| truthy(Some(p))) else { return Err(fail("error", "server too old: modisa restart")) };
                let mut res = Map::new();
                if let Some(p) = d.get("pane") {
                    res.insert("pane".into(), p.clone());
                }
                for (k, v) in process.as_object().into_iter().flatten() {
                    res.insert(k.clone(), v.clone());
                }
                if json {
                    print(&Value::Object(res));
                } else {
                    let fg = res.get("foreground").filter(|f| truthy(Some(f))).map(|f| format!("foreground {} {}", tpl(f.get("pid")), tpl(f.get("args"))));
                    let cwd = res.get("cwd").filter(|c| truthy(Some(c))).map(|c| format!("cwd {}", js_str(c)));
                    outln!("{}", [Some(format!("pid {}", tpl(res.get("pid")))), fg, cwd].into_iter().flatten().collect::<Vec<_>>().join("\n"));
                }
            }
            "pane zoom" => {
                let modes: Vec<&str> = ["on", "off", "toggle"].into_iter().filter(|m| a.on(m)).collect();
                if modes.len() > 1 {
                    return Err(fail("usage", "use one of --on, --off and --toggle"));
                }
                let res = self.call("pane.zoom", obj(vec![("target", s(r(0))), ("mode", s(modes.first().copied()))])).await?;
                if json { print(&res) } else { outln!("{}", if truthy(res.get("zoomed")) { "zoomed" } else { "unzoomed" }) }
            }
            "agent spawn" => {
                let p = self.call("agent.spawn", obj(vec![("harness", s(r(0))), ("name", s(a.str("name"))), ("prompt", s(a.str("prompt"))), ("dir", s(Some(if a.on("down") { "down" } else { "right" }))), ("tab", b(a.on("tab"))), ("target", s(self.target(None))), ("focus", b(a.on("focus"))), ("env", self.env()?)])).await?;
                if json { print(&p) } else { log(p.get("id")) }
            }
            "agent list" => {
                let agents = self.call("agent.list", json!({})).await?;
                if json {
                    print(&agents);
                } else {
                    let rows = arr(&agents).iter().map(|x| vec![cell(x.get("id")), at_name(x.get("name")), cell(x.get("harness")), cell(x.get("state")), cell(x.get("source")), cell(x.get("workspace"))]);
                    table(&["id", "name", "harness", "state", "source", "workspace"], rows.collect());
                }
            }
            k if k == any("wait") => {
                let state = if a.on("idle") { Some("idle") } else { a.str("state") };
                let res = self.call("wait", obj(vec![("target", s(verb)), ("exited", b(a.on("exited"))), ("state", s(state)), ("match", s(a.str("match"))), ("timeout", n(a.num("timeout")))])).await?;
                let exit = res.get("exitCode");
                if json {
                    print(&res);
                } else if let Some(code) = exit {
                    outln!("exited {}", tpl(Some(code)));
                } else {
                    log(res.get("state").filter(|s| !s.is_null()).or(res.get("match")));
                }
                if let Some(code) = exit {
                    return Ok(code.as_f64().map_or(0, |c| c as i32)); // the child's status, in either output format
                }
            }
            k if k == any("send") => {
                let to = verb.unwrap_or("undefined");
                let res = self.call("send", obj(vec![("to", s(verb)), ("body", s(Some(&rest.join(" "))))])).await?;
                if json {
                    print(&res);
                } else {
                    let state = res.get("recipientState").filter(|x| truthy(Some(x))).map_or(String::new(), |x| format!(" ({})", js_str(x)));
                    outln!("queued for {to}{state}: message {}, not delivered yet", tpl(res.get("id")));
                }
            }
            "inbox" => {
                let ms = self.call("inbox", json!({})).await?;
                if json {
                    print(&ms);
                } else if arr(&ms).is_empty() {
                    outln!("(no messages)");
                } else {
                    for m in arr(&ms) {
                        let reply = m.get("replyTo").filter(|x| truthy(Some(x))).map_or(String::new(), |x| format!(" (reply: modisa send {} \"...\")", js_str(x)));
                        outln!("from @{}{reply}:\n{}\n", tpl(m.get("from")), tpl(m.get("body")));
                    }
                }
            }
            "messages" => {
                for m in arr(&self.call("messages", json!({})).await?) {
                    show_message(m);
                }
                if a.on("follow") {
                    let caller = self.caller.clone();
                    self.on_message(move |conn, m| {
                        if m["method"] == "event" && m["params"]["type"] == "message.sent" {
                            let (conn, caller) = (conn.clone(), caller.clone());
                            tokio::task::spawn_local(async move {
                                if let Ok(all) = conn.request("messages", with_caller(&caller, json!({})), None).await {
                                    if let Some(last) = arr(&all).last() {
                                        show_message(last);
                                    }
                                }
                            });
                        }
                    });
                    self.call("events.subscribe", json!({})).await?;
                    std::future::pending::<()>().await;
                }
            }
            "pause" => {
                let res = self.call("messaging.pause", json!({})).await?;
                outln!("{}", if truthy(res.get("paused")) { "messaging paused" } else { "messaging resumed" });
            }
            k if k == "report" || k == any("report") => {
                self.call("report", obj(vec![("pane", s(verb)), ("state", s(a.str("state"))), ("source", s(a.str("source"))), ("agent", s(a.str("agent"))), ("seq", n(a.num("seq"))), ("session", s(a.str("session-id"))), ("release", yes(a.switch("release"))), ("title", s(a.str("title")))])).await?;
            }
            "notify" => return Err(fail("usage", "notify needs a title: modisa notify <title> [--body text]")),
            k if k == any("notify") => {
                let title = std::iter::once(verb.unwrap_or("")).chain(rest.iter().copied()).collect::<Vec<_>>().join(" ");
                let res = self.call("notify", obj(vec![("title", s(Some(&title))), ("body", s(a.str("body"))), ("tone", s(a.str("tone"))), ("system", yes(a.switch("system"))), ("sound", yes(a.switch("sound")))])).await?;
                if json {
                    print(&res);
                }
                if !truthy(res.get("clients")) {
                    errln!("no client attached"); // nobody to show it to, which isn't a failure
                }
            }
            // what clients draw, read without attaching: always JSON
            "snapshot" => print(&self.snapshot().await?),
            "tab list" => {
                let snap = self.snapshot().await?;
                let mut tabs = vec![];
                for (wi, w) in arr(&snap["workspaces"]).iter().enumerate() {
                    for (ti, t) in arr(&w["tabs"]).iter().enumerate() {
                        let shown = w["active"].as_f64() == Some(ti as f64);
                        let name = t.get("name").filter(|x| truthy(Some(x))).cloned();
                        tabs.push(obj(vec![("id", t.get("id").cloned()), ("name", name), ("workspaceId", w.get("id").cloned()), ("workspace", w.get("name").cloned()), ("panes", Some(json!(panes(&tree_of(t)?)))), ("focused", t.get("focused").cloned()), ("zoomed", t.get("zoomed").cloned()), ("active", b(shown)), ("current", b(snap["active"].as_f64() == Some(wi as f64) && shown))]));
                    }
                }
                if json {
                    print(&Value::Array(tabs));
                } else {
                    let rows = tabs.iter().map(|t| vec![cell(t.get("id")), cell(t.get("name")), cell(t.get("workspace")), js_str(&t["panes"]), cell(t.get("focused")), (if truthy(t.get("zoomed")) { "yes" } else { "" }).into(), (if t["current"] == true { "*" } else { "" }).into()]);
                    table(&["id", "name", "workspace", "panes", "focused", "zoomed", "current"], rows.collect());
                }
            }
            "plugin list" => {
                let ps = self.call("plugin.list", json!({})).await?;
                if json {
                    print(&ps);
                } else {
                    let rows = arr(&ps).iter().map(|p| {
                        let status = if p.get("exitCode").is_some() && p["status"] != "running" { format!("{} {}", tpl(p.get("status")), tpl(p.get("exitCode"))) } else { cell(p.get("status")) };
                        let source = match p.get("install").filter(|x| truthy(Some(x))) {
                            Some(i) => {
                                let r#ref = i.get("ref").filter(|x| truthy(Some(x))).map_or(String::new(), |x| format!(" {}", js_str(x)));
                                format!("{}{} @{}", tpl(i.get("source")), r#ref, tpl(i.get("commit")).chars().take(7).collect::<String>())
                            }
                            None => cell(p.get("dir")),
                        };
                        vec![cell(p.get("name")), status, (if truthy(p.get("connected")) { "yes" } else { "no" }).into(), cell(p.get("actions")), source, cell(p.get("error")), cell(p.get("log"))]
                    });
                    table(&["name", "status", "connected", "actions", "source", "error", "log"], rows.collect());
                    for p in arr(&ps) {
                        for k in arr(&p["keys"]) {
                            if k["state"] == "disabled" {
                                let key = k.get("key").filter(|x| truthy(Some(x))).map_or("(none)".into(), js_str);
                                let what = k.get("action").filter(|x| !x.is_null()).or(k.get("pane"));
                                outln!("{}: key {key} ({}) is off in the server's config: {}", tpl(p.get("name")), tpl(what), tpl(k.get("reason")));
                            }
                        }
                        // where it draws instead of modisa, and where it asked to but config.toml's [slots] gives it to modisa or
                        // another plugin
                        let names = |k: &str| arr(&p["slots"][k]).iter().map(js_str).collect::<Vec<_>>().join(", ");
                        if !names("holds").is_empty() {
                            outln!("{}: draws instead of modisa in {}", tpl(p.get("name")), names("holds"));
                        }
                        if !names("asks").is_empty() {
                            outln!("{}: asked to draw instead of modisa in {}; [slots] or a plugin before it by name has that", tpl(p.get("name")), names("asks"));
                        }
                    }
                }
            }
            "plugin stop" | "plugin start" => {
                let p = self.call(&format!("plugin.{}", verb.unwrap_or_default()), obj(vec![("name", s(r(0)))])).await?;
                if json {
                    print(&p);
                } else {
                    let err = p.get("error").filter(|x| truthy(Some(x))).map_or(String::new(), |x| format!(" ({})", js_str(x)));
                    outln!("{}: {}{err}", tpl(p.get("name")), tpl(p.get("status")));
                }
            }
            "plugin ui" => print(&self.call("ui.state", obj(vec![("plugin", s(r(0)))])).await?),
            "plugin pane" => {
                let params = json_param(r(2))?;
                let opened = self.call("plugin.pane.open", obj(vec![("plugin", s(r(0))), ("pane", s(r(1))), ("params", params)])).await?;
                if json { print(&opened) } else { outln!("{} ({})", tpl(opened.get("pane")), tpl(opened.get("placement"))) }
            }
            "plugin logs" => {
                let list = self.call("plugin.list", json!({})).await?;
                let wanted = r(0).map(Value::from);
                let Some(p) = arr(&list).iter().find(|x| x.get("name") == wanted.as_ref()) else {
                    return Err(fail("no_such_plugin", format!("no plugin named {} (see modisa plugin list)", r(0).unwrap_or("undefined"))));
                };
                let text = match p["log"].as_str() {
                    Some(path) => tokio::fs::read(path).await.map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default(),
                    None => String::new(),
                };
                let lines: Vec<&str> = text.trim_end().split('\n').collect();
                let from = js_slice_start(lines.len(), -a.num("lines").unwrap_or(50.0));
                outln!("{}", lines[from..].join("\n"));
            }
            "plugin run" => {
                let params = json_param(r(2))?;
                print(&self.call("plugin.invoke", obj(vec![("plugin", s(r(0))), ("action", s(r(1))), ("params", params)])).await?);
            }
            "workspace create" => {
                let p = self.call("workspace.create", obj(vec![("name", s(r(0))), ("cwd", s(self.cwd_opt())), ("command", s(a.str("command"))), ("env", self.env()?)])).await?;
                if json { print(&p) } else { log(p.get("id")) }
            }
            "workspace rename" => {
                self.call("workspace.rename", obj(vec![("workspace", s(r(0))), ("name", s(Some(&rest.iter().skip(1).copied().collect::<Vec<_>>().join(" "))))])).await?;
            }
            "workspace close" => {
                self.call("workspace.close", obj(vec![("workspace", s(r(0)))])).await?;
            }
            "workspace list" => {
                let ws = self.call("workspace.list", json!({})).await?;
                if json {
                    print(&ws);
                } else {
                    let rows = arr(&ws).iter().map(|w| vec![cell(w.get("id")), cell(w.get("name")), cell(w.get("tabs")), (if truthy(w.get("active")) { "*" } else { "" }).into(), cell(w.get("cwd"))]);
                    table(&["id", "name", "tabs", "active", "cwd"], rows.collect());
                }
            }
            "tab create" => {
                let p = self.call("tab.create", obj(vec![("name", s(r(0))), ("command", s(a.str("command"))), ("workspace", s(a.str("workspace"))), ("cwd", s(self.cwd_opt())), ("paneName", s(a.str("pane-name"))), ("env", self.env()?)])).await?;
                if json { print(&p) } else { log(p.get("id")) }
            }
            "events" => {
                self.on_message(|_, m| {
                    if m["method"] == "event" {
                        outln!("{}", m.get("params").map_or("undefined".into(), |p| stringify(p, false)));
                    }
                });
                self.call("events.subscribe", obj(vec![("output", b(a.on("output")))])).await?;
                if a.on("follow") {
                    std::future::pending::<()>().await;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            "debug detect" => print(&self.call("debug.detect", obj(vec![("target", s(r(0)))])).await?),
            "detach" => {
                self.call("detach-all", json!({})).await?;
            }
            _ => {
                let help = if json { String::new() } else { format!("\n\n{HELP}") };
                return Ok(failed("usage", &format!("unknown command: {}{help}", a.pos.join(" ")), json));
            }
        }
        Ok(0)
    }
}

fn show_message(m: &Value) {
    let at = m["at"].as_f64().map_or("Invalid Date".into(), locale_time);
    let hops = m.get("hops").filter(|x| truthy(Some(x))).map_or(String::new(), |x| format!(" (hop {})", js_str(x)));
    let queued = if truthy(m.get("delivered")) { "" } else { "  [queued]" };
    outln!("{at}  @{} → @{}{hops}{queued}\n  {}", tpl(m.get("fromName")), tpl(m.get("toName")), tpl(m.get("body")).replace('\n', "\n  "));
}

// a plugin action's or pane's params: JSON on the command line
fn json_param(text: Option<&str>) -> RpcResult<Option<Value>> {
    match text {
        None => Ok(None),
        Some(t) => serde_json::from_str(t).map(Some).map_err(|_| fail("usage", format!("params must be JSON, like '{{\"key\":\"value\"}}': {t}"))),
    }
}

// `{ caller, ...params }`
fn with_caller(caller: &Option<String>, params: Value) -> Value {
    let mut m = Map::new();
    if let Some(c) = caller {
        m.insert("caller".into(), json!(c));
    }
    if let Value::Object(p) = params {
        m.extend(p);
    }
    Value::Object(m)
}

// ---------- JSON as JavaScript has it ----------

// An object as JSON.stringify writes it: what's undefined (None) is left out.
fn obj(pairs: Vec<(&str, Option<Value>)>) -> Value {
    Value::Object(pairs.into_iter().filter_map(|(k, v)| Some((k.to_string(), v?))).collect())
}
// a field from an optional string, number (as Number() read it), boolean, or `x === true || undefined`
fn s(x: Option<impl AsRef<str>>) -> Option<Value> {
    x.map(|x| Value::from(x.as_ref()))
}
fn n(x: Option<f64>) -> Option<Value> {
    x.map(num_json)
}
fn b(x: bool) -> Option<Value> {
    Some(Value::Bool(x))
}
fn yes(x: bool) -> Option<Value> {
    x.then_some(Value::Bool(true))
}

// A JavaScript number as JSON has it: whole numbers without a fraction, and NaN or Infinity as null.
pub fn num_json(x: f64) -> Value {
    if !x.is_finite() {
        Value::Null
    } else if x.fract() == 0.0 && x.abs() < 9007199254740992.0 {
        Value::from(x as i64)
    } else {
        Value::from(x)
    }
}

fn arr(v: &Value) -> &[Value] {
    v.as_array().map_or(&[], Vec::as_slice)
}

// array[i], with i a JSON number
fn index<'a>(list: &'a Value, i: &Value) -> Option<&'a Value> {
    list.get(i.as_u64()? as usize)
}

pub fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

// String(x)
pub fn js_str(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => js_num(n.as_f64().unwrap_or(f64::NAN)),
        Value::String(s) => s.clone(),
        Value::Array(xs) => xs.iter().map(|x| if x.is_null() { String::new() } else { js_str(x) }).collect::<Vec<_>>().join(","),
        Value::Object(_) => "[object Object]".into(),
    }
}

// `${x}` of a field that may be missing
pub fn tpl(v: Option<&Value>) -> String {
    v.map_or("undefined".into(), js_str)
}

// a table's cell: String(x ?? "")
fn cell(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(v) => js_str(v),
    }
}

// a name column: @name, or nothing
fn at_name(v: Option<&Value>) -> String {
    if truthy(v) { format!("@{}", tpl(v)) } else { String::new() }
}

// Number.prototype.toString: the shortest digits that read back the same, with an exponent past 1e21 and below 1e-6.
pub fn js_num(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if x == 0.0 {
        return "0".into(); // -0 too
    }
    if x.abs() >= 1e21 || x.abs() < 1e-6 {
        let e = format!("{x:e}");
        return match e.split_once('e') {
            Some((m, exp)) if !exp.starts_with('-') => format!("{m}e+{exp}"),
            _ => e,
        };
    }
    format!("{x}")
}

// JSON.stringify(x) (compact) or JSON.stringify(x, null, 2) (pretty), numbers written as JavaScript writes them.
pub fn stringify(v: &Value, pretty: bool) -> String {
    let mut out = String::new();
    write_json(&mut out, v, pretty, 0);
    out
}

fn write_json(out: &mut String, v: &Value, pretty: bool, depth: usize) {
    let newline = |out: &mut String, depth: usize| {
        if pretty {
            out.push('\n');
            out.push_str(&"  ".repeat(depth));
        }
    };
    let quote = |s: &str| serde_json::to_string(s).unwrap_or_default();
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&js_num(n.as_f64().unwrap_or(0.0))),
        Value::String(s) => out.push_str(&quote(s)),
        Value::Array(xs) if xs.is_empty() => out.push_str("[]"),
        Value::Object(m) if m.is_empty() => out.push_str("{}"),
        Value::Array(xs) => {
            out.push('[');
            for (i, x) in xs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(out, depth + 1);
                write_json(out, x, pretty, depth + 1);
            }
            newline(out, depth);
            out.push(']');
        }
        Value::Object(m) => {
            out.push('{');
            for (i, (k, x)) in m.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(out, depth + 1);
                out.push_str(&quote(k));
                out.push_str(if pretty { ": " } else { ":" });
                write_json(out, x, pretty, depth + 1);
            }
            newline(out, depth);
            out.push('}');
        }
    }
}

// Where array.slice(start) starts, start counted from the end when negative (and NaN as 0).
fn js_slice_start(len: usize, start: f64) -> usize {
    let s = if start.is_nan() { 0.0 } else { start.trunc() };
    if s < 0.0 { (len as f64 + s).max(0.0) as usize } else { s.min(len as f64) as usize }
}

// ---------- dates as Bun's en-US toLocale…String has them, in local time ----------

extern "C" {
    fn tzset(); // in every libc, though not in the libc crate's list
}

fn local(ms: f64) -> Option<libc::tm> {
    if !ms.is_finite() || ms.abs() > 8.64e15 {
        return None;
    }
    let secs = (ms / 1000.0).floor() as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        tzset();
        (!libc::localtime_r(&secs, &mut tm).is_null()).then_some(tm)
    }
}

// new Date(ms).toLocaleTimeString(): 3:04:05 PM
pub fn locale_time(ms: f64) -> String {
    let Some(t) = local(ms) else { return "Invalid Date".into() };
    let h = if t.tm_hour % 12 == 0 { 12 } else { t.tm_hour % 12 };
    format!("{h}:{:02}:{:02} {}", t.tm_min, t.tm_sec, if t.tm_hour < 12 { "AM" } else { "PM" })
}

// new Date(ms).toLocaleString(): 10/8/2026, 3:04:05 PM
pub fn locale_date_time(ms: f64) -> String {
    let Some(t) = local(ms) else { return "Invalid Date".into() };
    format!("{}/{}/{}, {}", t.tm_mon + 1, t.tm_mday, t.tm_year + 1900, locale_time(ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_reads_like_javascripts() {
        let v: Value = serde_json::from_str(r#"{"a":1.0,"b":0.30000000000000004,"c":1e21,"d":1e-7,"e":[],"f":{},"g":[1,{"h":"x\u001b\n"}],"i":0.5,"j":-0.0}"#).unwrap();
        assert_eq!(stringify(&v, false), r#"{"a":1,"b":0.30000000000000004,"c":1e+21,"d":1e-7,"e":[],"f":{},"g":[1,{"h":"x\u001b\n"}],"i":0.5,"j":0}"#);
        assert_eq!(stringify(&json!({ "a": [1, 2], "b": {}, "c": { "d": null } }), true), "{\n  \"a\": [\n    1,\n    2\n  ],\n  \"b\": {},\n  \"c\": {\n    \"d\": null\n  }\n}");
        assert_eq!(js_num(1.5e-7), "1.5e-7");
        assert_eq!(js_num(1e-6), "0.000001");
        assert_eq!(js_num(123.0), "123");
        assert_eq!(num_json(50.0), json!(50));
        assert_eq!(num_json(f64::NAN), Value::Null);
        assert_eq!(js_str(&json!(["a", null, 3])), "a,,3");
    }

    #[test]
    fn undefined_is_left_out_and_caller_goes_first() {
        let p = obj(vec![("target", s(None::<&str>)), ("dir", s(Some("right"))), ("ratio", n(Some(f64::NAN))), ("new", yes(false)), ("focus", b(false))]);
        assert_eq!(stringify(&with_caller(&Some("p1".into()), p), false), r#"{"caller":"p1","dir":"right","ratio":null,"focus":false}"#);
    }

    #[test]
    fn exit_statuses() {
        assert_eq!([exit_status("usage"), exit_status("invalid_params"), exit_status("unreachable"), exit_status("timeout"), exit_status("no_such_pane")], [2, 2, 3, 124, 1]);
    }

    #[test]
    fn slices_like_javascript() {
        assert_eq!(js_slice_start(10, -50.0), 0);
        assert_eq!(js_slice_start(10, -3.0), 7);
        assert_eq!(js_slice_start(10, -0.0), 0);
        assert_eq!(js_slice_start(10, f64::NAN), 0);
        assert_eq!(js_slice_start(10, 5.0), 5);
    }

    #[test]
    fn places_a_pane_in_its_tab() {
        let snap = json!({
            "active": 0, "area": { "x": 0, "y": 0, "w": 120, "h": 38 },
            "workspaces": [{ "id": "w1", "active": 0, "tabs": [{ "id": "t1", "focused": "p2", "zoomed": false, "tree": { "dir": "row", "ratio": 0.5, "a": { "pane": "p1" }, "b": { "pane": "p2" } } }] }],
            "panes": [{ "id": "p1", "instance": "aaaa" }, { "id": "p2", "instance": "bbbb", "name": "two" }, { "id": "p3", "instance": "cccc" }],
        });
        assert_eq!(place_in(&snap, None, None).unwrap().pane["id"], "p2"); // the focused one
        assert_eq!(place_in(&snap, None, Some("p1")).unwrap().pane["id"], "p1"); // the caller
        let at = place_in(&snap, Some("@two"), Some("p1")).unwrap();
        assert_eq!((at.tab["id"].as_str(), at.rs.len()), (Some("t1"), 2));
        assert_eq!(neighbor(&at.rs, "p2", Dir::Left).as_deref(), Some("p1"));
        let err = |t| place_in(&snap, Some(t), None).err().unwrap();
        assert_eq!(err("p9"), fail("no_such_pane", "no such pane: p9"));
        assert_eq!(err("p2:zzzz"), fail("pane_gone", "p2 has gone: that pane was closed or restarted since its message was sent"));
        assert_eq!(err("p3").message, "p3 is a popup: it has no place in a tab");
    }
}
