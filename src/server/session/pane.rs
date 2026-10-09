// Server-side pane: PTY + headless terminal screen. Lives as long as the server, clients come and go.
use std::rc::Rc;
use std::sync::LazyLock;

use indexmap::IndexMap;
use regex::bytes::Regex;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

use crate::protocol::types::PaneInfo;
use crate::server::pty::{self, Pty};
use crate::vt::{Screen, VtEvent};

// What a pane's process does that the server hears about: output, then (once) its exit.
pub enum PaneEvent {
    Output(Vec<u8>),
    Exited(i32),
    SyncTimeout, // a synchronized update outlived its deadline: show what it held back
}

// id, instance, event
pub type Sink = Rc<dyn Fn(&str, &str, PaneEvent)>;

pub struct PaneOpts {
    pub id: String,
    pub cwd: String,
    pub command: Option<String>,
    pub harness: Option<String>,
    pub name: Option<String>,
    pub created_by: String,
    pub cols: u16,
    pub rows: u16,
    pub env: Option<IndexMap<String, String>>,
}

// The payload of the last complete ESC ] 9 ; 4 … (BEL or ESC \) in a chunk of output.
static OSC_PROGRESS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1b\]9;(4;[^\x07\x1b]*)(?:\x07|\x1b\\)").unwrap());
fn progress(bytes: &[u8]) -> Option<String> {
    // most output has no OSC 9 at all: skip the regex
    if !bytes.windows(4).any(|w| w == b"\x1b]9;") {
        return None;
    }
    OSC_PROGRESS.captures_iter(bytes).last().map(|c| String::from_utf8_lossy(&c[1]).into_owned())
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub fn random_hex(bytes: usize) -> String {
    let mut b = vec![0u8; bytes];
    let _ = getrandom::fill(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub struct PtyPane {
    pub info: PaneInfo,
    pub screen: Screen,
    pub pid: i32,
    pub last_output: u64,
    pub osc_title: String, // the terminal title the program last set (OSC 0/2), e.g. an agent's spinner
    pub reported_title: String, // one reported for it (modisa report --title): over the program's own, under a name. Not saved
    pub default_title: String, // what it runs: its title when nothing else names it
    pub osc_progress: String, // its last OSC 9;4 progress report, e.g. "4;3" busy, "4;0" cleared
    pub disposed: bool, // its screen is gone: async work that held on to it must skip it
    pub closed_while_running: bool, // closed before its process exited, so the exit that follows was caused by the close
    pub env: Option<IndexMap<String, String>>, // what it was started with on top of the server's environment: saved
    pub ephemeral: bool, // closes when its process exits, command or not (a plugin's popup)
    pub generation: u64, // rises with every change to what's on its screen: output, a resize. What hasn't changed needn't be read again
    pty: Rc<Pty>,
    input: Option<mpsc::UnboundedSender<Vec<u8>>>,
    writer: AbortHandle, // the io task isn't held: it outlives a close, to report the exit the close causes
}

impl PtyPane {
    pub fn new(opts: PaneOpts, sink: Sink) -> std::io::Result<PtyPane> {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        let (cols, rows) = (opts.cols.max(2), opts.rows.max(1));
        let default_title = match &opts.command {
            Some(c) => c.split(' ').next().unwrap_or("").to_string(),
            None => shell.rsplit('/').next().unwrap_or("sh").to_string(),
        };
        let info = PaneInfo {
            id: opts.id.clone(),
            instance: random_hex(4),
            name: opts.name.clone(),
            title: opts.name.clone().unwrap_or_else(|| default_title.clone()),
            terminal_title: None,
            cwd: opts.cwd.clone(),
            command: opts.command.clone(),
            harness: opts.harness.clone(),
            created_by: opts.created_by.clone(),
            status: "running".into(),
            exit_code: None,
            agent: None,
            session: None,
            cols,
            rows,
            popup: None,
            takeover: None,
            muted: false,
        };
        let argv: Vec<String> = match &opts.command {
            Some(c) => vec![shell.clone(), "-lc".into(), c.clone()],
            None => vec![shell.clone(), "-l".into()],
        };
        let mut env = vec![("TERM".to_string(), "xterm-256color".to_string()), ("COLORTERM".into(), "truecolor".into()), ("PWD".into(), opts.cwd.clone())];
        env.extend(opts.env.iter().flatten().map(|(k, v)| (k.clone(), v.clone())));
        env.push(("MODISA_PANE_ID".into(), opts.id.clone())); // last: the pane's identity isn't the caller's to set
        let (pty, mut child) = pty::spawn(&argv, &opts.cwd, &env, cols, rows)?;
        let pid = child.id().map(|p| p as i32).unwrap_or(0);
        let pty = Rc::new(pty);

        // output, then its exit: what's already in the pty when the process exits is read first, so a reader that
        // waits for the exit sees all of its output
        let (id, instance) = (info.id.clone(), info.instance.clone());
        let reader = pty.clone();
        tokio::task::spawn_local(async move {
            let mut buf = vec![0u8; 64 * 1024];
            let mut exited = false;
            loop {
                tokio::select! {
                    r = reader.read(&mut buf) => match r {
                        Ok(n) if n > 0 => {
                            sink(&id, &instance, PaneEvent::Output(buf[..n].to_vec()));
                            tokio::task::yield_now().await; // one chunk at a time: everything else gets its turn between them
                        }
                        _ => break,
                    },
                    status = child.wait(), if !exited => {
                        exited = true;
                        while let Ok(Some(n @ 1..)) = reader.try_read(&mut buf) {
                            sink(&id, &instance, PaneEvent::Output(buf[..n].to_vec()));
                        }
                        sink(&id, &instance, PaneEvent::Exited(code(status)));
                    }
                }
            }
            if !exited {
                let status = child.wait().await;
                sink(&id, &instance, PaneEvent::Exited(code(status)));
            }
        });
        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let writer = pty.clone();
        let input = tokio::task::spawn_local(async move {
            while let Some(data) = rx.recv().await {
                if writer.write_all(&data).await.is_err() {
                    return;
                }
            }
        });
        Ok(PtyPane {
            info,
            screen: Screen::new(cols, rows, 10_000),
            pid,
            last_output: 0,
            osc_title: String::new(),
            reported_title: String::new(),
            default_title,
            osc_progress: String::new(),
            disposed: false,
            closed_while_running: false,
            env: opts.env,
            ephemeral: false,
            generation: 0,
            pty,
            input: Some(tx),
            writer: input.abort_handle(),
        })
    }

    pub fn id(&self) -> &str {
        &self.info.id
    }

    // Output from its process, into its screen. Whether its title changed.
    pub fn feed(&mut self, bytes: &[u8]) -> bool {
        self.last_output = now_ms();
        self.generation += 1;
        if let Some(p) = progress(bytes) {
            self.osc_progress = p;
        }
        let events = self.screen.write(bytes);
        self.handle(events)
    }

    pub fn flush_sync(&mut self) -> bool {
        self.generation += 1;
        let events = self.screen.flush_sync();
        self.handle(events)
    }

    // query replies (DA, cursor position…) come from here, not from clients, so they work while detached
    fn handle(&mut self, events: Vec<VtEvent>) -> bool {
        let mut title = None;
        for e in events {
            match e {
                VtEvent::PtyWrite(s) => self.write(s.as_bytes()),
                VtEvent::Title(t) => title = Some(t),
                VtEvent::ResetTitle => title = Some(String::new()),
                _ => {}
            }
        }
        let Some(t) = title else { return false };
        // read on request (list, a snapshot): not pushed to clients, which a spinning title would do many times a second
        self.info.terminal_title = (!t.is_empty()).then(|| t.clone());
        self.osc_title = t;
        self.refresh_title()
    }

    // Its title from what names it, first that's set: its @name, a reported title, its program's terminal title, what it
    // runs. Whether that changed it.
    pub fn refresh_title(&mut self) -> bool {
        let title = [self.info.name.as_deref().unwrap_or(""), &self.reported_title, &self.osc_title, &self.default_title].into_iter().find(|t| !t.is_empty()).unwrap_or("").to_string();
        if title == self.info.title {
            return false;
        }
        self.info.title = title;
        true
    }

    pub fn write(&self, data: &[u8]) {
        if self.info.running() {
            if let Some(tx) = &self.input {
                let _ = tx.send(data.to_vec());
            }
        }
    }

    // Text as typed input; bracketed when the app asked for it so newlines don't submit early.
    pub fn paste(&self, text: &str) {
        if self.screen.mode().contains(alacritty_terminal::term::TermMode::BRACKETED_PASTE) {
            self.write(format!("\x1b[200~{text}\x1b[201~").as_bytes());
        } else {
            self.write(text.as_bytes());
        }
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        let (cols, rows) = (cols.max(2), rows.max(1));
        if cols == self.info.cols && rows == self.info.rows {
            return;
        }
        self.info.cols = cols;
        self.info.rows = rows;
        self.generation += 1;
        self.screen.resize(cols, rows);
        if self.input.is_some() {
            let _ = self.pty.resize(cols, rows);
        }
    }

    pub fn replay(&self) -> String {
        self.screen.replay()
    }

    pub fn screen_text(&self) -> String {
        self.screen.screen_text()
    }

    pub fn text(&self) -> String {
        self.screen.text()
    }

    pub fn read(&self, source: &str, format: &str, lines: usize) -> String {
        self.screen.read(source, format, lines)
    }

    // the process group in the foreground of its terminal, while it runs
    pub fn foreground(&self) -> Option<i32> {
        self.input.as_ref().and(self.pty.foreground())
    }

    pub fn kill(&mut self) {
        if self.pid > 0 {
            unsafe { libc::kill(-self.pid, libc::SIGHUP) }; // whole process group, like closing a terminal
        }
        self.input = None;
    }

    pub fn dispose(&mut self) {
        self.closed_while_running = self.info.running(); // any exit from here on is the kill below, not its own
        self.disposed = true;
        self.kill();
        self.writer.abort();
    }
}

impl Drop for PtyPane {
    fn drop(&mut self) {
        self.writer.abort();
    }
}

// what the exit status says: its code, or 128 + the signal that killed it
fn code(status: std::io::Result<std::process::ExitStatus>) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    match status {
        Ok(s) => s.code().or_else(|| s.signal().map(|n| 128 + n)).unwrap_or(1),
        Err(_) => 1,
    }
}
