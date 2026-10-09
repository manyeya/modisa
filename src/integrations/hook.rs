// `modisa hook <agent> <action>`: what an agent's hook runs. It reads the hook's JSON on stdin,
// keeps only the events and fields that mean something for that agent, and reports to the pane's
// server. It never prints (some agents feed hook output back into the conversation) and always exits
// 0, so a hook can't break the agent — outside modisa it does nothing.
use std::collections::HashMap;
use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};

use crate::protocol::transport::connect_unix;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HookReport {
    pub state: Option<String>, // "working" | "blocked" | "idle"
    pub session: Option<String>,
}

type Input = Map<String, Value>;

fn text<'a>(o: &'a Input, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|k| o.get(*k).and_then(Value::as_str).filter(|s| !s.is_empty()))
}
fn squash(s: &str) -> String {
    s.chars().filter(|c| *c != '_' && *c != '-').collect::<String>().to_lowercase()
}
// JavaScript truthiness, for fields that only need to be there
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64() != Some(0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

// Pure: the report a hook call amounts to, if any.
pub fn interpret(agent: &str, action: &str, input: &Input, env: &HashMap<String, String>) -> Option<HookReport> {
    let var = |k: &str| env.get(k).map(String::as_str).filter(|v| !v.is_empty());
    let event = text(input, &["hook_event_name", "hookEventName"]);
    let session = match agent {
        "claude-code" => {
            // subagents fire their own hooks, and Cursor runs Claude-compatible hooks: neither is this pane's session
            if event.is_some_and(|e| e != "SessionStart") || truthy(input.get("agent_id")) || var("CURSOR_VERSION").is_some() || truthy(input.get("cursor_version")) {
                return None;
            }
            text(input, &["session_id"])
        }
        "codex" => {
            if event.is_some_and(|e| e != "SessionStart") || text(input, &["transcript_path"]).is_none() {
                return None;
            }
            let session = text(input, &["session_id"]);
            if var("CODEX_THREAD_ID").is_some_and(|thread| Some(thread) != session) {
                return None; // a child thread
            }
            session
        }
        "copilot" => {
            let other = match event {
                Some(e) => squash(e) != "sessionstart",
                None => input.contains_key("prompt") || text(input, &["tool_name", "toolName", "notification_type", "notificationType", "stop_reason", "stopReason", "reason"]).is_some(),
            };
            if other {
                return None;
            }
            text(input, &["session_id", "sessionId"])
        }
        "cursor-agent" => {
            if event.is_some_and(|e| e != "sessionStart") {
                return None;
            }
            text(input, &["session_id", "sessionId", "conversation_id", "conversationId"])
        }
        "grok" => {
            if event.is_some_and(|e| squash(e) != "sessionstart") {
                return None;
            }
            var("GROK_SESSION_ID").or_else(|| text(input, &["session_id", "sessionId"]))
        }
        "antigravity" => text(input, &["conversationId"]),
        _ => text(input, &["session_id", "sessionId"]), // devin, droid, qodercli, qwen, kimi, mastracode
    };
    let session = session.map(String::from);
    match action {
        "session" => session.map(|s| HookReport { state: None, session: Some(s) }),
        "working" | "blocked" | "idle" => Some(HookReport { state: Some(action.into()), session }),
        _ => None,
    }
}

// Hooks pipe their JSON in; run by hand in a terminal, the timeout stops it waiting forever. (A thread
// of its own reads it, so one left waiting doesn't hold the process open.)
async fn stdin() -> Input {
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let mut raw = vec![];
        let _ = std::io::stdin().read_to_end(&mut raw);
        let _ = tx.send(String::from_utf8_lossy(&raw).into_owned());
    });
    let raw = tokio::time::timeout(Duration::from_secs(2), rx).await.ok().and_then(Result::ok).unwrap_or_default();
    match serde_json::from_str(&raw) {
        Ok(Value::Object(value)) => value,
        _ => Map::new(),
    }
}

static SEQ: AtomicU64 = AtomicU64::new(0);

pub async fn run_hook(agent: Option<&str>, action: Option<&str>) -> i32 {
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let (Some(agent), Some(action), Some(pane), Some(socket)) = (agent.filter(|a| !a.is_empty()), action.filter(|a| !a.is_empty()), var("MODISA_PANE_ID"), var("MODISA_SOCKET")) else {
        return 0;
    };
    let env: HashMap<String, String> = std::env::vars_os().filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?))).collect();
    let Some(report) = interpret(agent, action, &stdin().await, &env) else { return 0 };
    if let Ok(conn) = connect_unix(&socket, |_, _| {}).await {
        let mut params = json!({ "pane": pane, "source": format!("modisa:{agent}"), "agent": agent });
        if let Some(state) = report.state {
            params["state"] = state.into();
        }
        if let Some(session) = report.session {
            params["session"] = session.into();
        }
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        params["seq"] = (now * 1000 + SEQ.fetch_add(1, Ordering::Relaxed) + 1).into();
        let _ = conn.request("report", params, Some(Duration::from_millis(1500))).await;
        conn.close();
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(v: Value) -> Input {
        serde_json::from_value(v).unwrap()
    }
    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }
    fn session(s: &str) -> Option<HookReport> {
        Some(HookReport { state: None, session: Some(s.into()) })
    }

    #[test]
    fn hooks_keep_only_the_events_and_sessions_that_belong_to_the_panes_agent() {
        let none = env(&[]);
        let start = input(json!({ "hook_event_name": "SessionStart", "session_id": "s1", "transcript_path": "/t" }));
        let with = |k: &str, v: Value| {
            let mut m = start.clone();
            m.insert(k.into(), v);
            m
        };
        assert_eq!(interpret("claude-code", "session", &start, &none), session("s1"));
        assert_eq!(interpret("claude-code", "session", &with("agent_id", json!("sub")), &none), None); // a subagent
        assert_eq!(interpret("claude-code", "session", &start, &env(&[("CURSOR_VERSION", "1")])), None); // Cursor's Claude-compatible hooks
        assert_eq!(interpret("claude-code", "session", &with("hook_event_name", json!("Stop")), &none), None);
        assert_eq!(interpret("codex", "session", &start, &none), session("s1"));
        let mut untracked = start.clone();
        untracked.remove("transcript_path");
        assert_eq!(interpret("codex", "session", &untracked, &none), None);
        assert_eq!(interpret("codex", "session", &start, &env(&[("CODEX_THREAD_ID", "other")])), None); // a child thread
        assert_eq!(interpret("copilot", "session", &input(json!({ "hookEventName": "session_start", "sessionId": "c1" })), &none), session("c1"));
        assert_eq!(interpret("copilot", "session", &input(json!({ "prompt": "hi", "sessionId": "c1" })), &none), None);
        assert_eq!(interpret("cursor-agent", "session", &input(json!({ "hook_event_name": "sessionStart", "conversation_id": "k1" })), &none), session("k1"));
        assert_eq!(interpret("grok", "session", &Input::new(), &env(&[("GROK_SESSION_ID", "g1")])), session("g1"));
        assert_eq!(interpret("antigravity", "session", &input(json!({ "conversationId": "a1" })), &none), session("a1"));
        assert_eq!(interpret("kimi", "blocked", &input(json!({ "session_id": "m1" })), &none), Some(HookReport { state: Some("blocked".into()), session: Some("m1".into()) }));
        assert_eq!(interpret("mastracode", "idle", &Input::new(), &none), Some(HookReport { state: Some("idle".into()), session: None }));
        assert_eq!(interpret("droid", "session", &Input::new(), &none), None);
        assert_eq!(interpret("droid", "bogus", &input(json!({ "session_id": "x" })), &none), None);
    }
}
