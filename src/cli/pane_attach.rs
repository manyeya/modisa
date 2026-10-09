// `modisa pane attach [pane] [--takeover | --observe] [--remote ssh://host]`: one pane full-screen in this terminal,
// which is its emulator: the server's replay redraws the pane's screen (its modes too), then its output streams in.
// --takeover (the default) drives it, at this terminal's size, and only this terminal's typing reaches it; --observe
// watches it at its own size and types nothing. The prefix then d detaches (the prefix twice types it once); observing,
// q or Ctrl-C does too. Exit status: 0 detached (or the pane was closed), the pane's own when its process exits, 3 when
// the connection is lost.
use std::borrow::Cow;
use std::io::{Read, Write};
use std::sync::LazyLock;

use regex::bytes::Regex;
use serde_json::{json, Value};
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::mpsc;

use super::args::Args;
use super::commands::{connect_session, failed, lost, tpl};
use super::sessions::ssh_proxy;
use crate::config::{load_config, parse_prefix};
use crate::core::paths::socket_path;
use crate::protocol::conn::{b64, error_code, unb64, Conn};
use crate::protocol::transport::connect_stdio;

// Everything a program in the pane may have turned on, off again: style, a hidden cursor and its shape, mouse reporting
// (1000/1002/1003 and SGR 1006), bracketed paste, focus events, application cursor keys and keypad, the kitty keyboard
// flags it pushed, the scroll region. Kitty's flags are kept per screen, so popping them on the alternate one leaves the
// shell's alone.
macro_rules! modes_off {
    () => {
        "\x1b[0m\x1b[?25h\x1b[0 q\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?2004l\x1b[?1004l\x1b[?1l\x1b>\x1b[<99u\x1b[r"
    };
}
const ENTER: &str = concat!("\x1b[?1049h", modes_off!(), "\x1b[H\x1b[2J"); // the alternate screen, as a program's would start
const LEAVE: &str = concat!(modes_off!(), "\x1b[?1049l"); // and this terminal's own screen back as it was
// What the pane sends can't reach this terminal's own screen: its screen switches stay out (the server redraws the
// pane instead, attach.rs), and so does erasing the scrollback, which is the user's.
static OWN_SCREEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1b\[(?:\?(?:1049|1047|47)[hl]|3J)").unwrap());

fn kept(b: &[u8]) -> Cow<'_, [u8]> {
    if b.contains(&0x1b) { OWN_SCREEN.replace_all(b, &b""[..]) } else { Cow::Borrowed(b) }
}

// One key at the start of `s`: a kitty keyboard report (CSI code[:alternates] [;mods[:event]] [;text] u, sent once the
// program in the pane asks for them) or one byte. ctrl: with Ctrl and nothing else; plain: no modifier; up: a key's
// release (reported when asked for).
static CSI_U: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\x1b\[([0-9]+)(?::[0-9:]*)?(?:;([0-9]*)(?::([0-9]+))?)?(?:;[0-9:]*)?u").unwrap());

#[derive(Debug, PartialEq)]
struct Key {
    len: usize,
    ch: char,
    ctrl: bool,
    plain: bool,
    up: bool,
}

fn key(s: &[u8]) -> Key {
    if let Some(m) = CSI_U.captures(s) {
        let number = |i: usize| m.get(i).filter(|x| !x.is_empty()).map(|x| String::from_utf8_lossy(x.as_bytes()).parse::<f64>().unwrap_or(f64::INFINITY));
        let mods = to_int32(number(2).unwrap_or(1.0).max(1.0) - 1.0) & !(64 | 128); // caps and num lock don't count
        let code = number(1).unwrap_or(0.0).min(0x10ffff as f64) as u32;
        let up = m.get(3).is_some_and(|x| x.as_bytes() == b"3");
        return Key { len: m[0].len(), ch: char::from_u32(code).unwrap_or('\u{fffd}'), ctrl: mods == 4, plain: mods == 0, up };
    }
    match s[0] {
        b if b < 0x20 => Key { len: 1, ch: char::from(b | 0x60), ctrl: true, plain: false, up: false },
        b => Key { len: 1, ch: char::from(b), ctrl: false, plain: true, up: false }, // as latin1
    }
}

// JavaScript's ToInt32, which `&` applies
fn to_int32(x: f64) -> i32 {
    if x.is_finite() { x.trunc().rem_euclid(4294967296.0) as u32 as i32 } else { 0 }
}

// This terminal's size, as process.stdout has it (ponytail: an ioctl on stdout, not crossterm's /dev/tty-first size).
fn size() -> (u16, u16) {
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    let (cols, rows) = if unsafe { libc::ioctl(1, libc::TIOCGWINSZ, &mut ws) } == 0 { (ws.ws_col, ws.ws_row) } else { (0, 0) };
    ((if cols == 0 { 80 } else { cols }).max(2), (if rows == 0 { 24 } else { rows }).max(1))
}

// Raw input the way Bun's setRawMode(true) has it (libuv's): no echo, no line editing, no signals from keys, but output
// processing left on. crossterm's raw mode (cfmakeraw) turns that off too. Returns the mode to put back.
fn raw_mode() -> Option<libc::termios> {
    unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(0, &mut t) != 0 {
            return None;
        }
        let saved = t;
        t.c_iflag &= !(libc::BRKINT | libc::ICRNL | libc::INPCK | libc::ISTRIP | libc::IXON);
        t.c_oflag |= libc::ONLCR;
        t.c_cflag |= libc::CS8;
        t.c_lflag &= !(libc::ECHO | libc::ICANON | libc::IEXTEN | libc::ISIG);
        t.c_cc[libc::VMIN] = 1;
        t.c_cc[libc::VTIME] = 0;
        libc::tcsetattr(0, libc::TCSADRAIN, &t);
        Some(saved)
    }
}

// What's typed, as it arrives: a thread of its own blocks on stdin so the runtime never does.
pub(crate) fn stdin_chunks() -> mpsc::UnboundedReceiver<Vec<u8>> {
    let (tx, rx) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        let mut buf = vec![0u8; 65536];
        let mut stdin = std::io::stdin();
        loop {
            match stdin.read(&mut buf) {
                Ok(0) => break,
                Ok(n) if tx.send(buf[..n].to_vec()).is_ok() => {}
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                _ => break,
            }
        }
    });
    rx
}

fn write(b: &[u8]) {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(b).and_then(|_| out.flush()); // a terminal that's gone (SIGHUP)
}

pub async fn pane_attach(a: &Args, target: Option<&str>) -> i32 {
    if a.on("takeover") && a.on("observe") {
        return failed("usage", "modisa: use one of --takeover and --observe", false);
    }
    if unsafe { libc::isatty(0) == 0 || libc::isatty(1) == 0 } {
        return failed("usage", "modisa: pane attach needs a terminal: its input and output can't be redirected", false);
    }
    let observe = a.on("observe");
    let (remote, session) = (a.str("remote"), a.str("session"));
    let cfg = load_config();
    // $MODISA_PANE_ID is a pane of this pane's own server: over ssh, or in another session, the caller is no pane at all
    let own_socket = std::env::var("MODISA_SOCKET").ok();
    let caller = if remote.is_none() && session.is_none_or(|s| s.is_empty() || own_socket.as_deref() == Some(&socket_path(s))) { std::env::var("MODISA_PANE_ID").ok() } else { None };

    // What arrives with the reply, or before this side is ready for it, waits its turn: the replay is drawn first. The
    // channel ends with the connection: its reader drops the sender then. (Not on_close: that's the transport's, which
    // keeps the ssh child alive in it.)
    let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
    let on_message = move |_: &Conn, m: Value| drop(tx.send(m));
    let conn = match remote {
        Some(remote) => connect_stdio(&ssh_proxy(remote, &cfg.remote_command, session.unwrap_or("default")), on_message).map_err(|e| e.to_string()),
        None => connect_session(session, on_message).await.map_err(|e| e.message),
    };
    let conn = match conn {
        Ok(c) => c,
        Err(message) => return failed("unreachable", &message, false),
    };

    let (cols, rows) = size();
    let mode = if observe { "observe" } else { "takeover" };
    let mut p = json!({ "caller": caller, "target": target, "mode": mode, "cols": cols, "rows": rows });
    p.as_object_mut().unwrap().retain(|_, v| !v.is_null()); // undefined ones aren't sent
    let r = match conn.request("pane.attach", p, None).await {
        Ok(r) => r,
        Err(e) => {
            let code = if lost(&conn, &e) { "unreachable".into() } else { error_code(Some(&e.code)) };
            conn.close();
            return failed(&code, &format!("modisa: {}", e.message), false);
        }
    };
    let pane = r["pane"].as_str().unwrap_or_default().to_string();
    let (pane_cols, pane_rows) = (r["cols"].as_u64().unwrap_or(0), r["rows"].as_u64().unwrap_or(0));
    let prefix = parse_prefix(&cfg.prefix).name; // the letter: C-b → b

    let finish = |code: i32, note: &str, error: bool, saved: Option<libc::termios>| {
        write(LEAVE.as_bytes());
        if let Some(t) = saved {
            unsafe { libc::tcsetattr(0, libc::TCSADRAIN, &t) };
        }
        conn.close();
        if !note.is_empty() {
            if error { errln!("{note}") } else { outln!("{note}") }
        }
        code
    };

    // Observing, the pane keeps its own size: one bigger than this terminal is cut off at its edge, so say so (over the
    // bottom row, where the pane's next redraw of it goes over it).
    let warn = || {
        let (cols, rows) = size();
        if !observe || (cols as u64 >= pane_cols && rows as u64 >= pane_rows) {
            return;
        }
        let text = format!(" {pane} is {pane_cols}×{pane_rows}, bigger than this terminal ({cols}×{rows}): past its edge is cut off ");
        write(format!("\x1b7\x1b[{rows};1H\x1b[0;7m{}\x1b[0m\x1b8", text.chars().take(cols as usize).collect::<String>()).as_bytes());
    };
    let resized = || {
        if observe {
            return warn();
        }
        let (conn, (cols, rows)) = (conn.clone(), size());
        tokio::task::spawn_local(async move { drop(conn.request("pane.attach.resize", json!({ "cols": cols, "rows": rows }), None).await) });
    };

    let mut lost_note = "connection lost";
    // a message from the server (None: the connection closed): Some(the exit status and what to say) once it ends the
    // attach
    let mut handle = |m: Option<Value>| -> Option<(i32, String, bool)> {
        let Some(m) = m else { return Some((3, format!("[{lost_note}]"), true)) };
        let d = &m["params"];
        match m["method"].as_str() {
            Some("output") => write(&kept(&unb64(d["data"].as_str().unwrap_or_default()))),
            Some("attach.end") if d["reason"] == "exited" => {
                let code = d.get("exitCode").filter(|c| !c.is_null());
                return Some((code.and_then(Value::as_f64).map_or(1, |c| c as i32), format!("[{pane} exited {}]", code.map_or("?".into(), |c| tpl(Some(c)))), false));
            }
            Some("attach.end") => return Some((0, format!("[{pane} closed]"), false)),
            Some("restart") => lost_note = "the session's server is restarting: attach again once it's back",
            Some("exit") => lost_note = "the session ended",
            _ => {}
        }
        None
    };

    let (Ok(mut int), Ok(mut term), Ok(mut hup), Ok(mut winch)) = (signal(SignalKind::interrupt()), signal(SignalKind::terminate()), signal(SignalKind::hangup()), signal(SignalKind::window_change())) else {
        return finish(1, "modisa: can't listen for signals", true, None);
    };

    write(ENTER.as_bytes());
    write(&kept(&unb64(r["data"].as_str().unwrap_or_default())));
    warn();
    loop {
        let m = match rx.try_recv() {
            Ok(m) => Some(m),
            Err(mpsc::error::TryRecvError::Empty) => break,
            Err(mpsc::error::TryRecvError::Disconnected) => None,
        };
        if let Some((code, note, error)) = handle(m) {
            return finish(code, &note, error, None);
        }
    }
    let saved_mode = raw_mode();
    let mut typing = stdin_chunks();
    let mut typing_open = true;
    let mut armed = false; // the prefix was pressed: the next key is for modisa

    loop {
        tokio::select! {
            m = rx.recv() => {
                if let Some((code, note, error)) = handle(m) {
                    return finish(code, &note, error, saved_mode);
                }
            }
            chunk = typing.recv(), if typing_open => {
                let Some(chunk) = chunk else {
                    typing_open = false;
                    continue;
                };
                let mut send: Vec<u8> = vec![];
                let flush = |send: &mut Vec<u8>| {
                    if !send.is_empty() && !observe {
                        conn.notify("input", json!({ "pane": pane, "data": b64(send) }));
                    }
                    send.clear();
                };
                let mut i = 0;
                while i < chunk.len() {
                    let k = key(&chunk[i..]);
                    let raw = &chunk[i..i + k.len];
                    i += k.len;
                    if armed {
                        if k.up {
                            continue; // the prefix's own release
                        }
                        armed = false;
                        if k.plain && k.ch == 'd' {
                            flush(&mut send); // what came before it in this chunk
                            return finish(0, &format!("[detached from {pane}]"), false, saved_mode);
                        }
                        // the prefix twice types it once; after any other key the prefix is dropped and the key goes on
                    } else if k.ctrl && k.ch.to_string() == prefix && !k.up {
                        armed = true;
                        continue;
                    } else if observe && !k.up && ((k.plain && k.ch == 'q') || (k.ctrl && k.ch == 'c')) {
                        return finish(0, &format!("[stopped watching {pane}]"), false, saved_mode);
                    }
                    send.extend_from_slice(raw);
                }
                flush(&mut send);
            }
            _ = winch.recv() => resized(),
            _ = int.recv() => return finish(128 + 2, "", false, saved_mode),
            _ = term.recv() => return finish(128 + 15, "", false, saved_mode),
            _ = hup.recv() => return finish(128 + 1, "", false, saved_mode),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_keys_plain_and_kitty() {
        assert_eq!(key(b"\x02d"), Key { len: 1, ch: 'b', ctrl: true, plain: false, up: false });
        assert_eq!(key(b"d"), Key { len: 1, ch: 'd', ctrl: false, plain: true, up: false });
        assert_eq!(key(b"\x1b[98;5u"), Key { len: 7, ch: 'b', ctrl: true, plain: false, up: false });
        assert_eq!(key(b"\x1b[98;5:3u"), Key { len: 9, ch: 'b', ctrl: true, plain: false, up: true });
        assert_eq!(key(b"\x1b[100u"), Key { len: 6, ch: 'd', ctrl: false, plain: true, up: false });
        assert_eq!(key(b"\x1b[99;69u").ctrl, true); // Ctrl with caps lock (64) still counts as Ctrl alone
        assert_eq!(key(b"\x1b[99;u").plain, true);
        assert_eq!(key(b"\x1b[A").len, 1); // not a kitty report: the escape byte alone
    }

    #[test]
    fn keeps_screen_switches_out() {
        assert_eq!(&*kept(b"a\x1b[?1049hb\x1b[3Jc\x1b[?47ld\x1b[31me"), b"abcd\x1b[31me");
        assert!(matches!(kept(b"plain"), Cow::Borrowed(_)));
    }
}
