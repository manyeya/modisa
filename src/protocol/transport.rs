// Getting a connection: to a local session server (starting it if needed), or over ssh.
use std::os::unix::process::CommandExt;
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use tokio::net::UnixStream;

use super::conn::{error, Conn, RpcError, RpcResult};
use crate::core::paths::{self, socket_path, DIR};

// The longest path a unix socket address holds (sun_path, its NUL included): 104 bytes on macOS, 108 on Linux.
const SUN_PATH: usize = if cfg!(target_os = "macos") { 104 } else { 108 };

// A short name of our own in the temp directory, for a socket whose real path is too long to use.
fn short_path() -> String {
    let mut b = [0u8; 6];
    let _ = getrandom::fill(&mut b);
    format!("/tmp/modisa-{}-{}.sock", std::process::id(), b.iter().map(|x| format!("{x:02x}")).collect::<String>())
}

// Connecting through a symlink reaches the socket it points to, so a long path is reached by a short one.
pub async fn connect_stream(path: &str) -> std::io::Result<UnixStream> {
    if path.len() < SUN_PATH {
        return UnixStream::connect(path).await;
    }
    let link = short_path();
    std::os::unix::fs::symlink(path, &link)?;
    let stream = UnixStream::connect(&link).await;
    let _ = std::fs::remove_file(&link);
    stream
}

// A socket bound at a short path and then renamed stays bound under its new name, so a long path is listened on too.
pub fn bind(path: &str) -> std::io::Result<tokio::net::UnixListener> {
    if path.len() < SUN_PATH {
        return tokio::net::UnixListener::bind(path);
    }
    let short = short_path();
    let listener = tokio::net::UnixListener::bind(&short)?;
    std::fs::rename(&short, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&short);
    })?;
    Ok(listener)
}

pub async fn connect_unix(path: &str, on_message: impl Fn(&Conn, Value) + 'static) -> std::io::Result<Conn> {
    let stream = connect_stream(path).await?;
    let (r, w) = stream.into_split();
    Ok(Conn::spawn(r, w, on_message))
}

// Remote transport: `modisa proxy` on the far side of ssh, same protocol over stdio.
pub fn connect_stdio(cmd: &[String], on_message: impl Fn(&Conn, Value) + 'static) -> std::io::Result<Conn> {
    let mut child = tokio::process::Command::new(&cmd[0]).args(&cmd[1..]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true).spawn()?;
    let (r, w) = (child.stdout.take().unwrap(), child.stdin.take().unwrap());
    let conn = Conn::spawn(r, w, on_message);
    conn.on_close(move || drop(child));
    Ok(conn)
}

fn alive(pid: i32) -> bool {
    // no such process: a dead server; EPERM: it exists, a sandbox just can't signal it
    unsafe { libc::kill(pid, 0) == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM) }
}

// A server that's running but can't be reached, from the pid file next to its socket. Inside an agent's sandbox
// (Codex's on macOS) connecting fails with ENOENT, just like a dead server's socket, but signalling the process still
// says it exists (EPERM). So its socket must stay and no second server start. Outside a sandbox, ps also checks the pid
// still belongs to a modisa server.
pub fn running_pid(sock: &str) -> Option<i32> {
    let pid: i32 = std::fs::read_to_string(sock.trim_end_matches(".sock").to_string() + ".pid").ok()?.trim().parse().ok()?;
    if pid <= 0 || !alive(pid) {
        return None;
    }
    if let Ok(ps) = std::process::Command::new("ps").args(["-p", &pid.to_string(), "-o", "args="]).output() {
        if ps.status.success() && !String::from_utf8_lossy(&ps.stdout).contains(" server ") {
            return None; // the pid now belongs to something else
        }
    }
    Some(pid)
}

pub fn unreachable(pid: i32) -> String {
    format!("modisa's server is running (pid {pid}) but this process can't connect to its socket: a sandbox is blocking it. See https://manyeya.github.io/modisa/docs/troubleshooting/#sandbox")
}

// Start a session's server in the background: its own session (setsid), its output to <session>.log.
pub fn spawn_server(session: &str, dir: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(&*DIR)?;
    let log = std::fs::File::create(format!("{}/{session}.log", *DIR))?; // truncated: a shorter run would keep the last one's tail
    let mut cmd = std::process::Command::new(paths::self_exe());
    cmd.args(["server", "-s", session]).current_dir(dir).env("PWD", dir).stdin(Stdio::null()).stdout(log.try_clone()?).stderr(log);
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    cmd.spawn()?;
    Ok(())
}

// Connect to a session's server, starting it if needed.
pub async fn ensure_server(session: &str, dir: &str, on_message: impl Fn(&Conn, Value) + Clone + 'static) -> RpcResult<Conn> {
    let path = socket_path(session);
    match connect_unix(&path, on_message.clone()).await {
        Ok(c) => return Ok(c),
        Err(_) => {
            if let Some(pid) = running_pid(&path) {
                return Err(error(unreachable(pid)));
            }
            let _ = std::fs::remove_file(&path); // stale socket from a dead server
        }
    }
    spawn_server(session, dir)?;
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if let Ok(c) = connect_unix(&path, on_message.clone()).await {
            return Ok(c);
        }
    }
    Err(error(format!("server for session \"{session}\" did not start; see {}/{session}.log", *DIR)))
}

// Inside a pane, MODISA_SOCKET points at our own server unless a session is named explicitly.
pub async fn connect_existing(session: Option<&str>, on_message: impl Fn(&Conn, Value) + 'static) -> Result<Conn, RpcError> {
    let path = match (session, std::env::var("MODISA_SOCKET")) {
        (None, Ok(s)) if !s.is_empty() => s,
        _ => socket_path(session.unwrap_or("default")),
    };
    connect_unix(&path, on_message).await.map_err(|_| {
        error(match running_pid(&path) {
            Some(pid) => unreachable(pid),
            None => format!("no modisa server for session \"{}\"", session.unwrap_or("default")),
        })
    })
}
