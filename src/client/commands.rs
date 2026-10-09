// The user's [[command]]s: their prompts asked, their {variables} filled in, then run where `in` says: in a new pane
// (beside the focused one, below it, in a tab, zoomed) or in the background, where only a failure says anything.
use serde_json::{json, Value};

use super::modals::{pick, prompt, ListItem};
use super::Shared;
use crate::config::CommandConfig;

pub fn run(shared: &Shared, c: CommandConfig) {
    let s = shared.clone();
    tokio::task::spawn_local(async move {
        if let Err(e) = go(&s, &c).await {
            toast(&s, &format!("{}: {e}", c.name));
        }
    });
}

fn toast(shared: &Shared, text: &str) {
    let mut app = shared.borrow_mut();
    let b = app.th.blocked;
    app.toast(text, b);
}

// What {variables} mean, from the pane focused now.
fn context(shared: &Shared) -> Vec<(String, String)> {
    let app = shared.borrow();
    let mut vars = vec![("session".to_string(), app.opts.session.clone())];
    if app.ready() {
        let (ws, tab) = (app.ws(), app.tab());
        vars.push(("space".into(), ws.name.clone()));
        vars.push(("tab".into(), tab.name.clone().unwrap_or_default()));
        vars.push(("pane".into(), tab.focused.clone()));
        let info = app.info(&tab.focused);
        vars.push(("cwd".into(), info.map(|i| i.cwd.clone()).unwrap_or_else(|| ws.cwd.clone())));
        vars.push(("name".into(), info.map(|i| i.name.clone().unwrap_or_else(|| i.title.clone())).unwrap_or_default()));
    }
    vars
}

// A shell word that stays one word whatever it holds.
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

// `{name}` replaced by its value (shell-quoted in a command line); a name that isn't one stays as it was.
pub fn fill(template: &str, vars: &[(String, String)], shell: bool) -> String {
    let mut out = template.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("{{{k}}}"), &if shell { quote(v) } else { v.clone() });
    }
    out
}

async fn lines_of(command: &str, cwd: &str) -> Result<Vec<String>, String> {
    let out = tokio::process::Command::new("sh").arg("-c").arg(command).current_dir(cwd).output().await.map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(last_line(&out.stderr).unwrap_or_else(|| format!("{command} failed")));
    }
    Ok(String::from_utf8_lossy(&out.stdout).lines().filter(|l| !l.trim().is_empty()).map(String::from).collect())
}

fn last_line(bytes: &[u8]) -> Option<String> {
    String::from_utf8_lossy(bytes).lines().rev().find(|l| !l.trim().is_empty()).map(|l| l.trim().to_string())
}

async fn request(shared: &Shared, method: &str, params: Value) -> Result<Value, String> {
    let conn = shared.borrow().conn.clone().ok_or("not connected")?;
    conn.request(method, params, None).await.map_err(|e| e.message)
}

async fn go(shared: &Shared, c: &CommandConfig) -> Result<(), String> {
    let mut vars = context(shared);
    let here = vars.iter().find(|(k, _)| k == "cwd").map(|(_, v)| v.clone()).unwrap_or_else(|| ".".into());
    for p in &c.prompts {
        let title = if p.title.is_empty() { &p.name } else { &p.title };
        let answer = if p.pick.is_empty() {
            prompt(shared, title, &p.default, "").await
        } else {
            let lines = lines_of(&p.pick, &here).await?;
            if lines.is_empty() {
                return Err(format!("{} printed nothing to pick from", p.pick));
            }
            pick(shared, title, lines.into_iter().map(|l| ListItem::new(l.clone(), "", l)).collect(), None).await
        };
        let Some(answer) = answer else { return Ok(()) }; // escape: nothing runs
        vars.push((p.name.clone(), answer));
    }
    let run = fill(&c.run, &vars, true);
    let cwd = if c.cwd.is_empty() { here } else { fill(&c.cwd, &vars, false) };
    match c.place.as_str() {
        "background" => background_in(shared, &run, &cwd),
        "tab" => drop(request(shared, "tab.create", json!({ "name": c.name, "command": run, "cwd": cwd })).await?),
        place => {
            let dir = if place == "split-down" { "down" } else { "right" };
            let pane = request(shared, "pane.split", json!({ "dir": dir, "command": run, "cwd": cwd, "focus": true })).await?;
            if place == "zoomed" {
                request(shared, "pane.zoom", json!({ "target": pane["id"], "mode": "on" })).await?;
            }
        }
    }
    Ok(())
}

// sh:<command>, run where the focused pane is, with no pane of its own.
pub fn background(shared: &Shared, command: &str) {
    let vars = context(shared);
    let cwd = vars.iter().find(|(k, _)| k == "cwd").map(|(_, v)| v.clone()).unwrap_or_else(|| ".".into());
    background_in(shared, command, &cwd);
}

fn background_in(shared: &Shared, command: &str, cwd: &str) {
    let (s, command, cwd) = (shared.clone(), command.to_string(), cwd.to_string());
    tokio::task::spawn_local(async move {
        let out = tokio::process::Command::new("sh").arg("-c").arg(&command).current_dir(&cwd).stdin(std::process::Stdio::null()).output().await;
        match out {
            Ok(o) if o.status.success() => {}
            Ok(o) => toast(&s, &format!("{command}: {}", last_line(&o.stderr).unwrap_or_else(|| format!("exited {}", o.status.code().unwrap_or(-1))))),
            Err(e) => toast(&s, &format!("{command}: {e}")),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_variables_quoted_for_the_shell() {
        let vars = vec![("filter".to_string(), "it's a b".to_string()), ("cwd".to_string(), "/tmp/x y".to_string())];
        assert_eq!(fill("bun test {filter} {nope}", &vars, true), r"bun test 'it'\''s a b' {nope}");
        assert_eq!(fill("{cwd}/sub", &vars, false), "/tmp/x y/sub");
    }
}
