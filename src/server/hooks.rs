// [[hook]]s: commands of the user's run on what happens in the session. One that observes runs in the background with
// the event as JSON on stdin (and in $MODISA_EVENT), its {fields} filled into its command line. One that intercepts
// (message.send, agent.spawn, pane.keys, pane.run, notify) runs before the request it's named for, and may stop it
// (printing {"allow": false, "reason": "…"}) or change its fields (printing them as an object). A hook that fails or
// runs out of time changes nothing.
use std::process::Stdio;
use std::time::Duration;

use serde_json::{json, Map, Value};
use tokio::io::AsyncWriteExt;

use super::{Server, Shared};
use crate::config::HookConfig;
use crate::protocol::conn::{fail, RpcResult};

// The requests a hook can stand in front of, by the event name it uses for them.
pub fn intercepted(method: &str) -> Option<&'static str> {
    Some(match method {
        "send" => "message.send",
        "agent.spawn" => "agent.spawn",
        "pane.keys" => "pane.keys",
        "pane.run" => "pane.run",
        "notify" => "notify",
        _ => return None,
    })
}

fn matching<'a>(hooks: &'a [HookConfig], ev: &Value) -> Vec<&'a HookConfig> {
    let kind = ev["type"].as_str().unwrap_or("");
    hooks.iter().filter(|h| h.on == kind && (h.when.trim().is_empty() || when(&h.when, ev).unwrap_or(false))).collect()
}

// `{field}` in a command line: the event's field, shell-quoted (a nested one by its dotted path).
fn fill(run: &str, ev: &Value) -> String {
    let mut out = String::new();
    let mut rest = run;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let Some(j) = rest[i..].find('}') else { break };
        let name = &rest[i + 1..i + j];
        match lookup(ev, name) {
            Some(v) if !name.is_empty() && !name.contains(char::is_whitespace) => out.push_str(&quote(&text(v))),
            _ => out.push_str(&rest[i..=i + j]),
        }
        rest = &rest[i + j + 1..];
    }
    out + rest
}

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        v => v.to_string(),
    }
}

fn lookup<'a>(ev: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(ev, |v, k| v.get(k))
}

async fn run(h: &HookConfig, ev: &Value) -> Option<Vec<u8>> {
    let payload = ev.to_string();
    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c").arg(fill(&h.run, ev)).env("MODISA_EVENT", &payload).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
    if let Some(p) = ev["pane"].as_str() {
        cmd.env("MODISA_PANE_ID", p);
    }
    let mut child = cmd.spawn().ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(payload.as_bytes()).await; // a hook that doesn't read it is fine
    }
    let out = tokio::time::timeout(Duration::from_millis(h.timeout.max(1)), child.wait_with_output()).await.ok()?.ok()?;
    out.status.success().then_some(out.stdout)
}

// An event happened: the hooks that observe it start, and nobody waits for them.
pub fn observe(srv: &Server, ev: &Value) {
    for h in matching(&srv.cfg.hook, ev) {
        let (h, ev) = (h.clone(), ev.clone());
        tokio::task::spawn_local(async move {
            let _ = run(&h, &ev).await;
        });
    }
}

// Before an intercepted request: its params as the hooks leave them (each sees what the one before it left), or why
// it's not going ahead.
pub async fn intercept(shared: &Shared, event: &str, params: Value) -> RpcResult<Value> {
    let hooks = shared.borrow().cfg.hook.clone();
    let mut params = params;
    let mut ev = json!({ "type": event });
    for h in hooks.iter().filter(|h| h.on == event) {
        if let (Some(e), Some(p)) = (ev.as_object_mut(), params.as_object()) {
            e.extend(p.clone());
        }
        if !h.when.trim().is_empty() && !when(&h.when, &ev).unwrap_or(false) {
            continue;
        }
        let Some(out) = run(h, &ev).await else { continue };
        let Ok(Value::Object(answer)) = serde_json::from_slice::<Value>(&out) else { continue };
        if answer.get("allow") == Some(&Value::Bool(false)) {
            let reason = answer.get("reason").and_then(Value::as_str).unwrap_or("no reason given");
            return Err(fail("refused", format!("a hook on {event} stopped it: {reason}")));
        }
        if let Value::Object(p) = &mut params {
            p.extend(answer.into_iter().filter(|(k, _)| k != "allow" && k != "type"));
        }
    }
    Ok(params)
}

// ---------- `when`: a condition on the event's fields ----------
// field == value, field != value, field contains value, !, && and || (|| weakest), parentheses; a field alone is true
// when it's there and not empty, 0 or false. Values are 'quoted', "quoted", numbers, true or false.

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Word(String),
    Str(String),
    Op(&'static str),
}

fn lex(s: &str) -> Result<Vec<Tok>, String> {
    let mut out = Vec::new();
    let cs: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '\'' || c == '"' {
            let end = cs[i + 1..].iter().position(|&d| d == c).ok_or("a quote isn't closed")? + i + 1;
            out.push(Tok::Str(cs[i + 1..end].iter().collect()));
            i = end + 1;
        } else if let Some(op) = ["==", "!=", "&&", "||"].into_iter().find(|op| cs[i..].starts_with(&op.chars().collect::<Vec<_>>())) {
            out.push(Tok::Op(op));
            i += 2;
        } else if let Some(op) = ["!", "(", ")"].into_iter().find(|op| op.starts_with(c)) {
            out.push(Tok::Op(op));
            i += 1;
        } else {
            let end = cs[i..].iter().position(|d| d.is_whitespace() || "()!=&|'\"".contains(*d)).map_or(cs.len(), |n| n + i);
            out.push(Tok::Word(cs[i..end].iter().collect()));
            i = end;
        }
    }
    Ok(out)
}

struct Parser<'a> {
    toks: Vec<Tok>,
    at: usize,
    ev: &'a Value,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.at)
    }
    fn eat(&mut self, op: &str) -> bool {
        let yes = matches!(self.peek(), Some(Tok::Op(o)) if *o == op) || matches!(self.peek(), Some(Tok::Word(w)) if w == op);
        self.at += yes as usize;
        yes
    }
    fn or(&mut self) -> Result<bool, String> {
        let mut v = self.and()?;
        while self.eat("||") {
            v |= self.and()?;
        }
        Ok(v)
    }
    fn and(&mut self) -> Result<bool, String> {
        let mut v = self.not()?;
        while self.eat("&&") {
            v &= self.not()?;
        }
        Ok(v)
    }
    fn not(&mut self) -> Result<bool, String> {
        if self.eat("!") {
            return Ok(!self.not()?);
        }
        if self.eat("(") {
            let v = self.or()?;
            return if self.eat(")") { Ok(v) } else { Err("a ( isn't closed".into()) };
        }
        let left = self.value()?;
        if self.eat("==") {
            return Ok(same(&left, &self.value()?));
        }
        if self.eat("!=") {
            return Ok(!same(&left, &self.value()?));
        }
        if self.eat("contains") {
            return Ok(text(&left).contains(&text(&self.value()?)));
        }
        Ok(truthy(&left))
    }
    fn value(&mut self) -> Result<Value, String> {
        let t = self.toks.get(self.at).cloned().ok_or("a condition ends too soon")?;
        self.at += 1;
        Ok(match t {
            Tok::Str(s) => Value::String(s),
            Tok::Word(w) if w == "true" || w == "false" => Value::Bool(w == "true"),
            Tok::Word(w) => match w.parse::<f64>() {
                Ok(n) => json!(n),
                Err(_) => lookup(self.ev, &w).cloned().unwrap_or(Value::Null),
            },
            Tok::Op(o) => return Err(format!("{o} where a value should be")),
        })
    }
}

fn same(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => x == y,
        _ => text(a) == text(b),
    }
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

pub fn when(condition: &str, ev: &Value) -> Result<bool, String> {
    let mut p = Parser { toks: lex(condition)?, at: 0, ev };
    let v = p.or()?;
    if p.at < p.toks.len() {
        return Err(format!("{:?} after the end", p.toks[p.at]));
    }
    Ok(v)
}

// What `config check` says about a hook's condition (None: it reads).
pub fn check_when(condition: &str) -> Option<String> {
    when(condition, &Value::Object(Map::new())).err()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conditions_read_the_events_fields() {
        let ev = json!({ "type": "agent.state", "to": "blocked", "from": "working", "name": "reviewer", "n": 3, "agent": { "harness": "codex" } });
        for (c, want) in [
            ("to == 'blocked'", true),
            ("to == \"done\"", false),
            ("to != 'done' && name contains 'view'", true),
            ("!(to == 'blocked') || n == 3", true),
            ("agent.harness == 'codex'", true),
            ("agent.harness == codex", false), // a bare word is a field: there's no codex field, so it's null
            ("agent.harness == 'codex' && missing", false),
            ("n", true),
        ] {
            assert_eq!(when(c, &ev), Ok(want), "{c}");
        }
        assert!(check_when("to == ").is_some() && check_when("(a").is_some() && check_when("to == 'x' x").is_some());
        assert_eq!(check_when("to == 'blocked' || !n"), None);
    }

    #[test]
    fn a_hooks_command_line_gets_the_fields_quoted() {
        let ev = json!({ "name": "it's", "pane": "p3", "agent": { "harness": "codex" } });
        assert_eq!(fill("say {name} in {pane} {agent.harness} {nope} {}", &ev), r"say 'it'\''s' in 'p3' 'codex' {nope} {}");
    }
}
