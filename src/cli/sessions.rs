// Session commands: attach (locally or over ssh), the ssh proxy, ls, restart, kill, config.
use std::collections::HashSet;
use std::future::Future;
use std::io::{Read, Write};
use std::pin::Pin;
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;
use serde_json::{json, Value};

use super::args::Args;
use super::commands::{locale_date_time, stringify, tpl, truthy};
use crate::config::{ensure_config_file, load_config, with_default_keys, CONFIG_PATH};
use crate::core::paths::{cwd, socket_path, DIR};
use crate::protocol::conn::{Conn, RpcError, RpcResult};
use crate::protocol::transport::{connect_stdio, connect_unix, ensure_server, running_pid};
use crate::server::persist::store;

// Where a client's connection's messages go.
pub type OnMessage = Rc<dyn Fn(&Conn, Value)>;
// connect(spawn, on_message): spawn = start the server if it isn't running (the first attach, or whoever asked for a
// restart)
pub type Connect = Rc<dyn Fn(bool, OnMessage) -> Pin<Box<dyn Future<Output = RpcResult<Conn>>>>>;

// What the TUI client is started with (src/client/context.ts).
#[derive(Clone)]
pub struct ClientOptions {
    pub session: String,
    pub connect: Connect,
    pub remote: bool,
}

pub async fn run_client(opts: ClientOptions) -> i32 {
    crate::client::run_client(opts).await
}

pub async fn attach(name: &str, dir: &str, remote: Option<&str>) -> i32 {
    let session = name.to_string();
    let Some(remote) = remote.filter(|r| !r.is_empty()) else {
        let (name, dir) = (name.to_string(), dir.to_string());
        let connect: Connect = Rc::new(move |spawn, on_message| {
            let (name, dir) = (name.clone(), dir.clone());
            Box::pin(async move {
                let on_message = move |c: &Conn, m: Value| on_message(c, m);
                if spawn {
                    ensure_server(&name, &dir, on_message).await
                } else {
                    Ok(connect_unix(&socket_path(&name), on_message).await?)
                }
            })
        });
        return run_client(ClientOptions { session, connect, remote: false }).await;
    };
    let argv = ssh_proxy(remote, &load_config().remote_command, name);
    let connect: Connect = Rc::new(move |_, on_message| {
        let argv = argv.clone();
        Box::pin(async move { Ok(connect_stdio(&argv, move |c, m| on_message(c, m))?) })
    });
    run_client(ClientOptions { session, connect, remote: true }).await
}

static SSH_URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^ssh://([^/:]+)(?::([0-9]+))?").unwrap());

// The command that reaches a session over ssh: remote is ssh://user@host:port or an ~/.ssh/config alias, and the far
// side runs `modisa proxy` (remote_command is how modisa is started there).
pub fn ssh_proxy(remote: &str, remote_command: &str, session: &str) -> Vec<String> {
    let u = SSH_URL.captures(remote);
    let host = u.as_ref().and_then(|u| u.get(1)).map_or(remote, |h| h.as_str());
    let mut argv = vec![std::env::var("MODISA_SSH").unwrap_or_else(|_| "ssh".into()), "-T".into()];
    if let Some(port) = u.as_ref().and_then(|u| u.get(2)) {
        argv.extend(["-p".into(), port.as_str().into()]);
    }
    argv.extend([host.into(), remote_command.into(), "proxy".into(), "-s".into(), session.into()]);
    argv
}

// Bridge stdio to the local socket byte-for-byte (used over ssh by --remote).
// ponytail: done once the server closes the socket, not when stdin ends after that as well.
pub async fn proxy(name: &str) -> i32 {
    match ensure_server(name, &cwd(), |_, _| {}).await {
        Ok(c) => c.close(),
        Err(e) => return fail(&e),
    }
    // tokio's connect handles a path too long for a socket address; the copies below block, so it's made blocking
    let sock = match crate::protocol::transport::connect_stream(&socket_path(name)).await.and_then(|s| s.into_std()).and_then(|s| s.set_nonblocking(false).map(|_| s)) {
        Ok(s) => s,
        Err(e) => return fail(&e.into()),
    };
    let Ok(mut up) = sock.try_clone() else { return 1 };
    // blocking copies on threads of their own: stdin to the socket, the socket to stdout
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut std::io::stdin().lock(), &mut up);
        let _ = up.shutdown(std::net::Shutdown::Write);
    });
    let down = tokio::task::spawn_blocking(move || {
        let (mut sock, mut buf, mut out) = (sock, vec![0u8; 65536], std::io::stdout());
        loop {
            match sock.read(&mut buf) {
                Ok(0) => break,
                Ok(n) if out.write_all(&buf[..n]).and_then(|_| out.flush()).is_ok() => {}
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                _ => break,
            }
        }
    });
    let _ = down.await;
    0
}

// ponytail: the original let these throw (Bun printed the stack and exited 1); this says the message.
fn fail(e: &RpcError) -> i32 {
    errln!("modisa: {}", e.message);
    1
}

pub async fn list_sessions() -> i32 {
    let mut names = HashSet::new();
    // what `ls -1` lists: no dotfiles
    let mut socks: Vec<String> = std::fs::read_dir(&*DIR)
        .map(|d| d.filter_map(|e| e.ok()?.file_name().into_string().ok()).filter(|f| !f.starts_with('.')).filter_map(|f| f.strip_suffix(".sock").map(String::from)).collect())
        .unwrap_or_default();
    socks.sort();
    let mut rows = vec![];
    let mut blocked = false;
    for n in socks {
        let path = socket_path(&n);
        let info = async {
            let c = connect_unix(&path, |_, _| {}).await?;
            let i = c.request("session.info", json!({}), None).await?;
            c.close();
            Ok::<_, RpcError>(i)
        };
        match info.await {
            Ok(i) => {
                names.insert(n.clone());
                let attached = if truthy(i.get("clients")) { format!(" ({} attached)", tpl(i.get("clients"))) } else { String::new() };
                rows.push(format!("{n}\t{} panes, {} workspaces{attached}", tpl(i.get("panes")), tpl(i.get("workspaces"))));
            }
            Err(_) => match running_pid(&path) {
                Some(pid) => {
                    names.insert(n.clone());
                    blocked = true;
                    rows.push(format!("{n}\trunning (pid {pid}), but this process can't connect to it: a sandbox is blocking it"));
                }
                None => drop(std::fs::remove_file(&path)), // dead server
            },
        }
    }
    for (name, saved_at) in store::saved() {
        if !names.contains(&name) {
            rows.push(format!("{name}\tsaved {} — attach to restore", locale_date_time(saved_at as f64)));
        }
    }
    outln!("{}", if rows.is_empty() { "no sessions".into() } else { rows.join("\n") });
    if blocked { 3 } else { 0 } // same status as any other command that can't reach its server
}

async fn output(program: &str, args: &[&str]) -> String {
    tokio::process::Command::new(program).args(args).output().await.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
}

// Save the session, stop its server, start a fresh one on the current code (it restores everything).
pub async fn restart_session(name: &str) -> i32 {
    let path = socket_path(name);
    let Ok(c) = connect_unix(&path, |_, _| {}).await else {
        outln!("no running session \"{name}\"");
        return 0;
    };
    let supported = c.request("restart", json!({}), None).await.is_ok();
    c.close();
    if !supported {
        // servers from before `restart` existed: their state is already saved (within 1s of any change), so stop them
        tokio::time::sleep(Duration::from_millis(1200)).await;
        // only the process listening on this session's socket (not same-named sessions elsewhere)
        for pid in output("lsof", &["-t", &path]).await.split('\n').filter(|p| !p.is_empty()) {
            if output("ps", &["-o", "args=", "-p", pid]).await.contains(&format!("server -s {name}")) {
                output("kill", &[pid]).await;
            }
        }
    }
    for _ in 0..50 {
        let Ok(x) = connect_unix(&path, |_, _| {}).await else { break };
        x.close();
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    match ensure_server(name, &cwd(), |_, _| {}).await {
        Ok(c) => c.close(),
        Err(e) => return fail(&e),
    }
    outln!("restarted {name}");
    0
}

pub async fn kill_session(name: &str) -> i32 {
    let killed = async {
        let c = connect_unix(&socket_path(name), |_, _| {}).await?;
        c.request("kill", json!({}), None).await?;
        c.close();
        Ok::<_, RpcError>(())
    };
    if killed.await.is_err() {
        store::forget(name);
    }
    outln!("killed {name}");
    0
}

// `modisa config [path|edit|check|reset-keys]`: on this machine's config.toml, no session needed. The exit status.
pub async fn config_command(sub: Option<&str>, a: &Args) -> i32 {
    let path = CONFIG_PATH.as_str();
    match sub {
        Some("edit") => {
            let file = match ensure_config_file() {
                Ok(f) => f,
                Err(e) => return fail(&e.into()),
            };
            let editor = std::env::var("EDITOR").ok().filter(|e| !e.is_empty()).unwrap_or_else(|| "vi".into());
            match tokio::process::Command::new(editor).arg(file).status().await {
                Ok(_) => 0,
                Err(e) => fail(&e.into()),
            }
        }
        Some("path") => {
            outln!("{path}");
            0
        }
        None => match ensure_config_file().and_then(std::fs::read_to_string) {
            Ok(text) => {
                outln!("{text}");
                0
            }
            Err(e) => fail(&e.into()),
        },
        Some("check") => {
            let report = serde_json::to_value(crate::config::check::check(path)).unwrap_or_default();
            let problems = report["problems"].as_array().cloned().unwrap_or_default();
            if a.on("json") {
                outln!("{}", stringify(&report, true));
            } else if report["exists"] != true {
                outln!("no {path}: modisa uses its defaults");
            } else {
                for p in &problems {
                    let column = p.get("column").filter(|c| truthy(Some(c))).map_or(String::new(), |c| format!(":{}", tpl(Some(c))));
                    let at = p.get("line").filter(|l| truthy(Some(l))).map_or(String::new(), |l| format!(":{}{column}", tpl(Some(l))));
                    let key = p.get("key").filter(|k| truthy(Some(k))).map_or(String::new(), |k| format!("{}: ", tpl(Some(k))));
                    outln!("{path}{at}: {}: {key}{}", tpl(p.get("level")), tpl(p.get("message")));
                }
                let errors = problems.iter().filter(|p| p["level"] == "error").count();
                let warnings = problems.len() - errors;
                let plural = |n: usize, what: &str| format!("{n} {what}{}", if n == 1 { "" } else { "s" });
                let counts = [(errors > 0).then(|| plural(errors, "error")), (warnings > 0).then(|| plural(warnings, "warning"))];
                outln!("{}", if problems.is_empty() { format!("{path}: no problems") } else { counts.into_iter().flatten().collect::<Vec<_>>().join(", ") });
            }
            if report["ok"] == true { 0 } else { 1 }
        }
        // modisa's own keys back, after a copy of the file as it was
        Some("reset-keys") => {
            let Ok(source) = std::fs::read_to_string(path) else {
                outln!("no config.toml: the keys are modisa's already");
                return 0;
            };
            let reset = match with_default_keys(&source) {
                Ok(r) => r,
                Err(e) => {
                    errln!("modisa: {e}");
                    return 1;
                }
            };
            if reset.changes.is_empty() {
                outln!("the keys are modisa's already: nothing changed");
                return 0;
            }
            if let Err(e) = std::fs::write(format!("{path}.bak"), &source).and_then(|_| std::fs::write(path, &reset.result)) {
                return fail(&e.into());
            }
            outln!("{}\nthe file as it was: {path}.bak", reset.changes.join("\n"));
            0
        }
        Some(other) => {
            errln!("modisa: no config command {other}: path, edit, check [--json] or reset-keys");
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_proxy_reaches_the_far_side() {
        assert_eq!(ssh_proxy("ssh://me@devbox:2222/x", "modisa", "s")[1..], ["-T", "-p", "2222", "me@devbox", "modisa", "proxy", "-s", "s"]);
        assert_eq!(ssh_proxy("devbox", "bun main.ts", "default")[1..], ["-T", "devbox", "bun main.ts", "proxy", "-s", "default"]);
    }
}
