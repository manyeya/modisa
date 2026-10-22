// The flock: every agent in every session on this machine, asked of each session's socket twice a second (`list`, as
// often as agent detection ticks), and what changed since the last answer. Sockets are non-blocking and read a frame
// at a time, so the notch stays on one thread like the rest of modisa.
use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::core::paths::DIR;

const ASK: Duration = Duration::from_millis(500);
const SCAN: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Blocked,
    Working,
    Done,
    Idle,
}

impl State {
    fn of(s: &str) -> State {
        match s {
            "blocked" => State::Blocked,
            "working" => State::Working,
            "done" => State::Done,
            _ => State::Idle,
        }
    }

    // which one the notch's corner shows: the one that wants you most
    pub fn urgency(self) -> u8 {
        match self {
            State::Blocked => 3,
            State::Done => 2,
            State::Working => 1,
            State::Idle => 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sheep {
    pub session: String,
    pub pane: String,
    pub label: String,
    pub harness: String,
    pub state: State,
    pub focused: bool,
}

// What the notch says for a moment about one sheep.
#[derive(Clone, Debug, PartialEq)]
pub struct News {
    pub sheep: Sheep,
    pub what: &'static str,
}

struct Link {
    session: String,
    stream: UnixStream,
    buf: Vec<u8>,
    sheep: Option<Vec<Sheep>>, // None until its first answer
    dead: bool,
}

impl Link {
    fn send(&mut self, method: &str, params: Value) {
        let line = format!("{}\n", json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }));
        match self.stream.write_all(line.as_bytes()) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::WouldBlock => {} // ponytail: dropped; it's asked again in half a second
            Err(_) => self.dead = true,
        }
    }

    // Every whole line that has arrived.
    fn read(&mut self) -> Vec<Value> {
        let mut chunk = [0u8; 65536];
        loop {
            match self.stream.read(&mut chunk) {
                Ok(0) => {
                    self.dead = true;
                    break;
                }
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(_) => {
                    self.dead = true;
                    break;
                }
            }
        }
        let Some(end) = self.buf.iter().rposition(|&b| b == b'\n') else { return vec![] };
        let lines: Vec<u8> = self.buf.drain(..=end).collect();
        lines.split(|&b| b == b'\n').filter_map(|l| serde_json::from_slice(l).ok()).collect()
    }
}

#[derive(Default)]
pub struct Flock {
    links: Vec<Link>,
    scanned: Option<Instant>,
    asked: Option<Instant>,
}

impl Flock {
    pub fn sheep(&self) -> Vec<Sheep> {
        self.links.iter().flat_map(|l| l.sheep.iter().flatten().cloned()).collect()
    }

    // Called every frame: looks for new sessions now and then, asks every session for its panes twice a second, and
    // reads what has arrived. Says whether the flock changed, and what's news.
    pub fn tick(&mut self, now: Instant) -> (bool, Vec<News>) {
        if self.scanned.is_none_or(|t| now - t >= SCAN) {
            self.scanned = Some(now);
            self.scan();
        }
        if self.asked.is_none_or(|t| now - t >= ASK) {
            self.asked = Some(now);
            self.links.iter_mut().for_each(|l| l.send("list", json!({})));
        }
        let (mut changed, mut news) = (false, vec![]);
        for l in &mut self.links {
            for msg in l.read() {
                let Some(panes) = msg["result"].as_array() else { continue }; // pane.focus's `true`
                let next = sheep_of(&l.session, panes);
                if let Some(prev) = &l.sheep {
                    news.extend(news_of(prev, &next));
                }
                changed |= l.sheep.as_ref() != Some(&next);
                l.sheep = Some(next);
            }
        }
        let before = self.links.len();
        self.links.retain(|l| !l.dead);
        (changed || self.links.len() != before, news)
    }

    pub fn focus(&mut self, s: &Sheep) {
        if let Some(l) = self.links.iter_mut().find(|l| l.session == s.session) {
            l.send("pane.focus", json!({ "target": s.pane }));
        }
    }

    // Every session's socket that answers: a dead server's refuses, and is tried again at the next scan.
    // ponytail: a socket path past sun_path's 104 bytes (a very long MODISA_DIR) can't be reached; transport.rs's
    // symlink trick would fix it
    fn scan(&mut self) {
        let Ok(dir) = std::fs::read_dir(&*DIR) else { return };
        for e in dir.flatten() {
            let path = e.path();
            let Some(session) = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(".sock")) else { continue };
            if self.links.iter().any(|l| l.session == session) {
                continue;
            }
            let Ok(stream) = UnixStream::connect(&path) else { continue };
            if stream.set_nonblocking(true).is_ok() {
                self.links.push(Link { session: session.into(), stream, buf: vec![], sheep: None, dead: false });
            }
        }
        self.links.sort_by(|a, b| a.session.cmp(&b.session));
    }
}

fn sheep_of(session: &str, panes: &[Value]) -> Vec<Sheep> {
    let str_of = |v: &Value| v.as_str().unwrap_or_default().to_string();
    panes
        .iter()
        .filter(|p| p["status"] == "running" && p["agent"].is_object())
        .map(|p| Sheep {
            session: session.into(),
            pane: str_of(&p["id"]),
            label: label(p),
            harness: str_of(&p["agent"]["harness"]),
            state: State::of(p["agent"]["state"].as_str().unwrap_or_default()),
            focused: p["focused"] == true,
        })
        .collect()
}

// Its @name, else its title without the spinner the agent puts in front, else the agent.
fn label(p: &Value) -> String {
    let title = p["title"].as_str().map(|t| t.trim_start_matches(|c: char| !c.is_alphanumeric()).trim());
    [p["name"].as_str(), title, p["agent"]["harness"].as_str()].into_iter().flatten().find(|s| !s.is_empty()).unwrap_or("agent").to_string()
}

// A new agent, one that needs you or is done, and one that started work you didn't just give it (not the pane you're in).
fn news_of(prev: &[Sheep], next: &[Sheep]) -> Vec<News> {
    next.iter()
        .filter_map(|s| {
            let what = match prev.iter().find(|p| p.pane == s.pane) {
                None => "joined the flock",
                Some(p) if p.state == s.state => return None,
                Some(p) => match s.state {
                    State::Blocked => "needs you",
                    State::Done => "is done",
                    State::Working if !s.focused && p.state != State::Blocked => "started",
                    _ => return None,
                },
            };
            Some(News { sheep: s.clone(), what })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_flock_and_its_news() {
        let pane = |id: &str, state: &str, focused: bool| json!({ "id": id, "title": "◐ Fix the parser", "status": "running", "focused": focused, "agent": { "harness": "codex", "state": state, "source": "screen" } });
        let shell = json!({ "id": "p9", "title": "zsh", "status": "running", "focused": false });
        let before = sheep_of("default", &[pane("p1", "working", true), pane("p2", "idle", false), shell.clone()]);
        assert_eq!(before.len(), 2);
        assert_eq!(before[0].label, "Fix the parser");
        let after = sheep_of("default", &[pane("p1", "blocked", true), pane("p2", "working", false), pane("p3", "idle", false), shell]);
        let news: Vec<_> = news_of(&before, &after).into_iter().map(|n| (n.sheep.pane, n.what)).collect();
        assert_eq!(news, [("p1".into(), "needs you"), ("p2".into(), "started"), ("p3".into(), "joined the flock")]);
        // working again after you answered it, in the pane you're in: not news
        assert!(news_of(&after, &sheep_of("default", &[pane("p1", "working", true)])).is_empty());
    }
}
