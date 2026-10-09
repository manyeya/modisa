// Images: base64 PNG, JPEG or GIF (its first frame), drawn with the terminal's graphics protocol (kitty, iTerm2, sixel)
// where it has one, else in half blocks. Decoding and encoding run on a thread of their own, so a big picture never holds
// up a frame; an element keeps its encoding until its data, its fit or its size changes. Where it can't be drawn (yet,
// or at all), its alt text shows.
use std::rc::Rc;
use std::sync::{mpsc, OnceLock};
use std::time::{Duration, Instant};

use ratatui::layout::{Rect, Size};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Paragraph, Widget, Wrap};
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::{Resize, ResizeEncodeRender};
use serde_json::Value;

use super::build::{hash_of, Ctx, Draw, Elems};
use crate::client::draw::Canvas;
use crate::client::{App, Shared};

static PICKER: OnceLock<Picker> = OnceLock::new();
static WORKER: OnceLock<mpsc::Sender<Job>> = OnceLock::new();

// An image element's encoding, for its data and fit (`of`); `gen` names the request it waits for.
#[derive(Default)]
pub struct Slot {
    of: u64,
    proto: Option<StatefulProtocol>,
    busy: bool,
    failed: bool,
    gen: u64,
}

// Where a result goes: a view's element, as it was when it asked (a newer request makes it stale).
struct Target {
    view: String,
    key: String,
    gen: u64,
}

enum Src {
    Data(String),
    Proto(StatefulProtocol),
}

struct Job {
    to: Target,
    src: Src,
    resize: Resize,
    size: Size,
}

fn picker() -> &'static Picker {
    PICKER.get_or_init(Picker::halfblocks)
}

// Whether the terminal answers a query at all (Primary Device Attributes) in a moment. ratatui-image's own query reads
// until it's answered, on a thread that would go on reading the keyboard; a terminal that never answers (a bare pty)
// gets half blocks instead.
fn answers() -> bool {
    use std::io::Write;
    let mut out = std::io::stdout();
    if out.write_all(b"\x1b[c").and_then(|_| out.flush()).is_err() {
        return false;
    }
    let end = Instant::now() + Duration::from_millis(400);
    let mut got = vec![];
    // its reply is ESC [ ? … c
    while !got.windows(3).position(|w| w == b"\x1b[?").is_some_and(|i| got[i..].contains(&b'c')) {
        let left = end.saturating_duration_since(Instant::now()).as_millis() as i32;
        let mut fd = libc::pollfd { fd: 0, events: libc::POLLIN, revents: 0 };
        let mut b = [0u8; 64];
        // the descriptor, not io::stdin(): its buffer would keep what comes after
        if left <= 0 || unsafe { libc::poll(&mut fd, 1, left) } <= 0 {
            return false;
        }
        let n = unsafe { libc::read(0, b.as_mut_ptr().cast(), b.len()) };
        if n <= 0 {
            return false;
        }
        got.extend_from_slice(&b[..n as usize]);
    }
    true
}

// Ask the terminal which graphics protocol it speaks (once, before anything else reads it), and start the thread that
// decodes and encodes; what it makes comes back to the client's thread.
pub fn start(app: &Shared) {
    let picker = if answers() { Picker::from_query_stdio().ok() } else { None };
    let _ = PICKER.set(picker.unwrap_or_else(Picker::halfblocks));
    let (jobs, todo) = mpsc::channel::<Job>();
    let (done, mut made) = tokio::sync::mpsc::unbounded_channel();
    std::thread::spawn(move || {
        for job in todo {
            let p = make(job.src, &job.resize, job.size);
            if done.send((job.to, p)).is_err() {
                return;
            }
        }
    });
    let _ = WORKER.set(jobs);
    let me = Rc::downgrade(app);
    tokio::task::spawn_local(async move {
        while let Some((to, p)) = made.recv().await {
            let Some(app) = me.upgrade() else { return };
            let mut app = app.borrow_mut();
            settle(&mut app, to, p);
            app.dirty();
        }
    });
}

fn settle(app: &mut App, to: Target, p: Option<StatefulProtocol>) {
    let Some(v) = app.views.iter_mut().find(|v| super::id_of(&v.state) == to.view) else { return };
    let Some(slot) = v.elems.get_mut(&to.key).and_then(|s| s.image.as_mut()).filter(|s| s.gen == to.gen) else { return };
    (slot.busy, slot.failed, slot.proto) = (false, p.is_none(), p);
}

// An image decoded (when it's data) and encoded for `size` cells; None when it can't be.
fn make(src: Src, resize: &Resize, size: Size) -> Option<StatefulProtocol> {
    let mut p = match src {
        Src::Data(data) => picker().new_resize_protocol(image::load_from_memory(&crate::protocol::conn::unb64(&data)).ok()?),
        Src::Proto(p) => p,
    };
    if let Some(at) = p.needs_resize(resize, size) {
        p.resize_encode(resize, at);
        p.last_encoding_result()?.ok()?;
    }
    Some(p)
}

// Off to the thread; without one (`modisa view render`, tests), done here and now.
fn ask(slot: &mut Slot, job: Job) {
    match WORKER.get() {
        Some(w) => {
            slot.busy = w.send(job).is_ok();
            slot.failed = !slot.busy;
        }
        None => {
            let p = make(job.src, &job.resize, job.size);
            (slot.failed, slot.proto) = (p.is_none(), p);
        }
    }
}

pub fn draw(ctx: &Ctx, c: &mut Canvas, n: &Value, key: &str, area: Rect, elems: &mut Elems, d: &Draw) {
    let data = n["data"].as_str().unwrap_or("");
    let resize = || match n["resize"].as_str() {
        Some("crop") => Resize::Crop(None),
        Some("scale") => Resize::Scale(None),
        _ => Resize::Fit(None),
    };
    let of = hash_of((data, n["resize"].as_str()));
    let slot = elems.entry(key.to_string()).or_default().image.get_or_insert_with(Slot::default);
    if slot.of != of {
        *slot = Slot { of, gen: slot.gen + 1, ..Default::default() };
    }
    let to = |slot: &Slot| Target { view: d.view.to_string(), key: key.to_string(), gen: slot.gen };
    let size = area.as_size();
    if !slot.busy && !slot.failed {
        match slot.proto.take() {
            None => ask(slot, Job { to: to(slot), src: Src::Data(data.to_string()), resize: resize(), size }),
            // a new size, encoded off the frame; until then nothing's drawn here
            Some(p) if p.needs_resize(&resize(), size).is_some() => {
                slot.gen += 1;
                ask(slot, Job { to: to(slot), src: Src::Proto(p), resize: resize(), size });
            }
            Some(p) => slot.proto = Some(p),
        }
    }
    match slot.proto.as_mut().filter(|_| !slot.busy) {
        Some(p) => p.render(area, c.buf),
        None => {
            let alt = n["alt"].as_str().unwrap_or("[image]");
            Paragraph::new(alt.to_string()).wrap(Wrap { trim: true }).style(Style::new().fg(ctx.tone("dim")).add_modifier(Modifier::ITALIC)).render(area, c.buf);
        }
    }
}
