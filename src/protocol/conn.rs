// One JSON-RPC 2.0 connection, newline-delimited, over any byte transport (unix socket or ssh stdio). Single-threaded:
// every connection lives on the LocalSet of the process that made it.
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::unix::OwnedWriteHalf;
use tokio::sync::{mpsc, oneshot};

use super::types::ERROR_CODES;

// An error carrying a stable code, on either side of the wire.
#[derive(Clone, Debug, PartialEq)]
pub struct RpcError {
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for RpcError {}

pub fn fail(code: &str, message: impl Into<String>) -> RpcError {
    RpcError { code: code.into(), message: message.into() }
}

pub fn error(message: impl Into<String>) -> RpcError {
    fail("error", message)
}

// Only modisa's own codes count; anything else is plain "error".
pub fn error_code(x: Option<&str>) -> String {
    match x {
        Some(c) if ERROR_CODES.contains(&c) => c.into(),
        _ => "error".into(),
    }
}

impl From<std::io::Error> for RpcError {
    fn from(e: std::io::Error) -> Self {
        error(e.to_string())
    }
}

pub type RpcResult<T = Value> = Result<T, RpcError>;

pub fn b64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

pub fn unb64(s: &str) -> Vec<u8> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(s).unwrap_or_default()
}

pub fn closed_error(cause: Option<&str>) -> RpcError {
    error(match cause {
        Some(c) => format!("connection closed: {c}"),
        None => "connection closed".into(),
    })
}

// A peer that stops reading can't make this side buffer without limit: past this many queued bytes the connection is
// closed. For a subscriber that's a gap in its history; it reconnects and takes a new snapshot.
pub static WRITE_QUEUE_LIMIT: LazyLock<usize> = LazyLock::new(|| std::env::var("MODISA_WRITE_QUEUE_LIMIT").ok().and_then(|v| v.parse().ok()).filter(|&n| n > 0).unwrap_or(16 * 1024 * 1024));

// who waits for a reply, and what runs with it the moment it's read (request_ordered)
type Pending = (oneshot::Sender<RpcResult>, Option<Box<dyn FnOnce(&RpcResult)>>);

struct Inner {
    out: RefCell<Option<mpsc::UnboundedSender<Vec<u8>>>>,
    queued: Rc<Cell<usize>>,
    seq: Cell<u64>,
    pending: RefCell<HashMap<u64, Pending>>,
    closed: Cell<bool>,
    close_reason: RefCell<Option<RpcError>>,
    on_close: RefCell<Vec<Box<dyn FnOnce()>>>, // every one runs, once, when it closes
    on_late_reply: RefCell<Option<Box<dyn Fn(Value)>>>,
    tasks: RefCell<Vec<tokio::task::AbortHandle>>, // [writer, reader]
    direct: Option<Rc<OwnedWriteHalf>>, // a socket: written to at once, as far as the kernel takes it
}

// Where a connection's lines go. A unix socket is written straight away and only what the kernel won't take yet is
// queued, like Bun's socket.write; any other stream (ssh's stdio) goes through the queue.
pub enum Writer {
    Socket(OwnedWriteHalf),
    Stream(Box<dyn AsyncWrite + Unpin>),
}

impl From<OwnedWriteHalf> for Writer {
    fn from(w: OwnedWriteHalf) -> Self {
        Writer::Socket(w)
    }
}

impl From<tokio::process::ChildStdin> for Writer {
    fn from(w: tokio::process::ChildStdin) -> Self {
        Writer::Stream(Box::new(w))
    }
}

async fn write_socket(w: &OwnedWriteHalf, mut data: &[u8]) -> std::io::Result<()> {
    while !data.is_empty() {
        w.writable().await?;
        match w.try_write(data) {
            Ok(n) => data = &data[n..],
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[derive(Clone)]
pub struct Conn(Rc<Inner>);

impl PartialEq for Conn {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for Conn {}
impl std::hash::Hash for Conn {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (Rc::as_ptr(&self.0) as usize).hash(state)
    }
}

impl Conn {
    // Runs the connection on the current LocalSet: messages that aren't replies go to `on_message`.
    pub fn spawn<R>(read: R, write: impl Into<Writer>, on_message: impl Fn(&Conn, Value) + 'static) -> Conn
    where
        R: AsyncRead + Unpin + 'static,
    {
        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let queued = Rc::new(Cell::new(0usize));
        let (direct, mut stream) = match write.into() {
            Writer::Socket(w) => (Some(Rc::new(w)), None),
            Writer::Stream(w) => (None, Some(w)),
        };
        let conn = Conn(Rc::new(Inner {
            out: RefCell::new(Some(tx)),
            queued: queued.clone(),
            seq: Cell::new(0),
            pending: RefCell::new(HashMap::new()),
            closed: Cell::new(false),
            close_reason: RefCell::new(None),
            on_close: RefCell::new(vec![]),
            on_late_reply: RefCell::new(None),
            tasks: RefCell::new(vec![]),
            direct: direct.clone(),
        }));
        let writer = conn.clone();
        let w = tokio::task::spawn_local(async move {
            while let Some(chunk) = rx.recv().await {
                let n = chunk.len();
                let written = match (&direct, &mut stream) {
                    (Some(sock), _) => write_socket(sock, &chunk).await,
                    (None, Some(stream)) => stream.write_all(&chunk).await,
                    _ => Ok(()),
                };
                if let Err(e) = written {
                    writer.closed_by_peer(Some(&e.to_string()));
                    return;
                }
                queued.set(queued.get().saturating_sub(n));
            }
            if let Some(stream) = stream.as_mut() {
                let _ = stream.shutdown().await;
            }
            // a socket's write half shuts down when the last reference to it goes
        });
        let reader = conn.clone();
        let r = tokio::task::spawn_local(async move {
            let mut lines = BufReader::new(read);
            let mut line = Vec::new();
            loop {
                line.clear();
                match lines.read_until(b'\n', &mut line).await {
                    Ok(0) => break,
                    Ok(_) => reader.feed(&line, &on_message),
                    Err(e) => {
                        reader.closed_by_peer(Some(&e.to_string()));
                        return;
                    }
                }
                if reader.closed() {
                    return;
                }
            }
            reader.closed_by_peer(None);
        });
        *conn.0.tasks.borrow_mut() = vec![w.abort_handle(), r.abort_handle()];
        conn
    }

    pub fn on_close(&self, f: impl FnOnce() + 'static) {
        if self.closed() {
            f();
        } else {
            self.0.on_close.borrow_mut().push(Box::new(f));
        }
    }

    // a reply to a request that timed out (or was never made): reported, never taken for a request
    pub fn on_late_reply(&self, f: impl Fn(Value) + 'static) {
        *self.0.on_late_reply.borrow_mut() = Some(Box::new(f));
    }

    fn feed(&self, line: &[u8], on_message: &impl Fn(&Conn, Value)) {
        let line = String::from_utf8_lossy(line);
        let line = line.trim_end_matches(['\n', '\r']);
        if line.is_empty() {
            return;
        }
        let m: Value = match serde_json::from_str(line) {
            Ok(m) => m,
            Err(_) => return self.send(&json!({ "jsonrpc": "2.0", "error": { "code": -32700, "message": "parse error" } })),
        };
        let reply_id = if m.get("method").is_none() { m.get("id").and_then(Value::as_u64) } else { None };
        match reply_id {
            Some(id) => {
                let pending = self.0.pending.borrow_mut().remove(&id);
                match pending {
                    None => {
                        if let Some(f) = &*self.0.on_late_reply.borrow() {
                            f(m)
                        }
                    }
                    Some((tx, first)) => {
                        let r = match m.get("error") {
                            Some(e) => Err(fail(&error_code(e.pointer("/data/code").and_then(Value::as_str)), e.get("message").and_then(Value::as_str).unwrap_or("error"))),
                            None => Ok(m.get("result").cloned().unwrap_or(Value::Null)),
                        };
                        if let Some(f) = first {
                            f(&r);
                        }
                        let _ = tx.send(r);
                    }
                }
            }
            None => on_message(self, m),
        }
    }

    pub fn send(&self, m: &Value) {
        if self.closed() {
            return;
        }
        let mut bytes = serde_json::to_vec(m).unwrap_or_default();
        bytes.push(b'\n');
        if self.0.queued.get() + bytes.len() > *WRITE_QUEUE_LIMIT {
            eprintln!("modisa: closing a connection: more than {} bytes waiting to be written: the peer isn't reading", *WRITE_QUEUE_LIMIT);
            // abruptly: a graceful end waits on a peer that isn't reading, and it would never see the close
            if let Some(w) = self.0.tasks.borrow().first() {
                w.abort();
            }
            self.close();
            return;
        }
        // nothing waiting: hand the kernel what it takes now, and queue only the rest
        let mut bytes = bytes;
        if let (Some(sock), 0) = (&self.0.direct, self.0.queued.get()) {
            match sock.try_write(&bytes) {
                Ok(n) if n == bytes.len() => return,
                Ok(n) => bytes = bytes.split_off(n),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => return self.closed_by_peer(Some(&e.to_string())),
            }
        }
        self.0.queued.set(self.0.queued.get() + bytes.len());
        let sent = self.0.out.borrow().as_ref().map(|tx| tx.send(bytes).is_ok()).unwrap_or(false);
        if !sent {
            self.closed_by_peer(None);
        }
    }

    pub fn notify(&self, method: &str, params: Value) {
        self.send(&json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    pub fn reply(&self, id: &Value, r: RpcResult) {
        match r {
            Ok(result) => self.send(&json!({ "jsonrpc": "2.0", "id": id, "result": result })),
            Err(e) => self.reply_error(id, -32000, &e.message, &e.code),
        }
    }

    pub fn reply_error(&self, id: &Value, code: i32, message: &str, data_code: &str) {
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message, "data": { "code": data_code } } }));
    }

    // timeout: stop waiting and forget the request, so a peer that never answers can't pile up pending requests. The
    // request was still sent, so its outcome is unknown; a reply that comes after goes to on_late_reply.
    pub async fn request(&self, method: &str, params: Value, timeout: Option<Duration>) -> RpcResult {
        self.request_with(method, params, timeout, None).await
    }

    // As request, but `first` runs with the reply as soon as it's read, before anything the peer sent after it: what
    // the reply holds is applied before a notification that came after it can overtake it (attach's view, a replay's
    // screens, then the output since).
    pub async fn request_ordered(&self, method: &str, params: Value, first: impl FnOnce(&RpcResult) + 'static) -> RpcResult {
        self.request_with(method, params, None, Some(Box::new(first))).await
    }

    async fn request_with(&self, method: &str, params: Value, timeout: Option<Duration>, first: Option<Box<dyn FnOnce(&RpcResult)>>) -> RpcResult {
        if self.closed() {
            return Err(self.0.close_reason.borrow().clone().unwrap_or_else(|| closed_error(None)));
        }
        let id = self.0.seq.get() + 1;
        self.0.seq.set(id);
        let (tx, rx) = oneshot::channel();
        self.0.pending.borrow_mut().insert(id, (tx, first));
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        let closed = || self.0.close_reason.borrow().clone().unwrap_or_else(|| closed_error(None));
        match timeout {
            None => rx.await.unwrap_or_else(|_| Err(closed())),
            Some(t) => match tokio::time::timeout(t, rx).await {
                Ok(r) => r.unwrap_or_else(|_| Err(closed())),
                Err(_) => {
                    self.0.pending.borrow_mut().remove(&id);
                    Err(fail("timeout", format!("no reply to {method} within {}s", t.as_secs_f64())))
                }
            },
        }
    }

    // requests sent and not yet answered, failed or timed out
    pub fn in_flight(&self) -> usize {
        self.0.pending.borrow().len()
    }

    pub fn closed(&self) -> bool {
        self.0.closed.get()
    }

    pub fn closed_by_peer(&self, cause: Option<&str>) {
        if self.closed() {
            return;
        }
        self.0.closed.set(true);
        let reason = closed_error(cause);
        *self.0.close_reason.borrow_mut() = Some(reason.clone());
        for (_, (tx, _)) in self.0.pending.borrow_mut().drain() {
            let _ = tx.send(Err(reason.clone()));
        }
        self.0.out.borrow_mut().take(); // the writer flushes what's queued, then shuts the transport down
        if let Some(r) = self.0.tasks.borrow().get(1) {
            r.abort(); // nothing more is read
        }
        let fs = std::mem::take(&mut *self.0.on_close.borrow_mut());
        for f in fs {
            f();
        }
    }

    pub fn close(&self) {
        self.closed_by_peer(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // A reply and the notification after it, read in one go: the reply's `first` runs before the notification is handled.
    #[test]
    fn an_ordered_reply_is_applied_before_what_follows_it() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        tokio::task::LocalSet::new().block_on(&rt, async {
            let (ours, theirs) = std::os::unix::net::UnixStream::pair().unwrap();
            ours.set_nonblocking(true).unwrap();
            let (read, write) = tokio::net::UnixStream::from_std(ours).unwrap().into_split();
            let seen = Rc::new(RefCell::new(Vec::<String>::new()));
            let log = seen.clone();
            let conn = Conn::spawn(read, write, move |_, m| log.borrow_mut().push(m["method"].as_str().unwrap_or("").into()));
            let log = seen.clone();
            let peer = std::thread::spawn(move || {
                let mut theirs = theirs;
                let mut buf = [0u8; 256];
                let _ = std::io::Read::read(&mut theirs, &mut buf); // the request
                theirs.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":\"screens\"}\n{\"jsonrpc\":\"2.0\",\"method\":\"output\"}\n").unwrap();
                theirs
            });
            let r = conn.request_ordered("replay", json!({}), move |r| log.borrow_mut().push(r.clone().unwrap().as_str().unwrap().into())).await;
            assert_eq!(r.unwrap(), json!("screens"));
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert_eq!(*seen.borrow(), ["screens", "output"]);
            drop(peer.join());
        });
    }
}
