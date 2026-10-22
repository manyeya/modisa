// `modisa notch`: the flock in the MacBook's notch. A black shape grows out of the notch: the shepherd on its left, on
// its right the sheep that wants you most. Point at it and it opens into a pasture with a sheep for every agent in
// every session on this machine; click one to focus its pane and bring its terminal forward. When an agent needs you,
// is done, starts work or joins, a line drops below the notch to say so. A screen without a notch gets the same shape
// at the top of the screen.
//
// The flock is drawn in woodcut, like modisa's emblem (art.rs), and the open pasture is a little world of its own
// (world.rs): sheep that steer round each other, walk to the shepherd when they need you, sleep at the back.
//
// macOS only: AppKit through objc2, on one thread like the rest of modisa. A timer polls the sessions and the pointer and
// draws the next frame of the shape alone: sixty times a second while something moves, thirty while it's open, and
// closed only when something changes (or a sheep that needs you hops in its corner).
mod art;
mod flock;
mod pen;
mod world;

use std::cell::RefCell;
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationOptions, NSApplicationActivationPolicy, NSBackingStoreType, NSBezierPath, NSColor, NSEvent, NSMainMenuWindowLevel, NSPanel, NSResponder,
    NSRunningApplication, NSScreen, NSView, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSTimer};

use crate::config::themes::{find_theme, Theme};
use crate::core::paths::DIR;
use crate::platform::procs;
use art::Body;
use flock::{Flock, News, Sheep, State};
use pen::{fit, text, width, Rgb};
use world::World;

const EAR: f64 = 44.0; // what the closed shape adds each side of the notch
const FLARE: f64 = 7.0; // its top corners curve out into the screen's edge
const PEEK: f64 = 30.0; // how far a line of news drops below the notch
const PASTURE: f64 = 108.0; // how far the open pasture does
const SLOT: f64 = 104.0; // a sheep's room in the pasture
const FOLD: f64 = 70.0; // the shepherd's
const NEWS_FOR: Duration = Duration::from_secs(6);
const DWELL: Duration = Duration::from_millis(100); // pointing at the notch this long opens it
const LEAVE: Duration = Duration::from_millis(300); // and away from it this long closes it

pub fn run(verb: Option<&str>) -> i32 {
    match verb {
        None | Some("start") => start(),
        Some("stop") => stop(),
        Some("run") => app(),
        _ => {
            eprintln!("usage: modisa notch [start|stop|run]");
            2
        }
    }
}

fn pid_file() -> String {
    format!("{}/notch.pid", *DIR)
}

fn running() -> Option<i32> {
    let pid: i32 = std::fs::read_to_string(pid_file()).ok()?.trim().parse().ok()?;
    procs::info(pid).filter(|i| i.args.ends_with("notch run")).map(|_| pid)
}

// In the background, in a session of its own, so closing the terminal leaves it running.
fn start() -> i32 {
    use std::os::unix::process::CommandExt;
    if let Some(pid) = running() {
        println!("the flock is already in the notch (pid {pid})");
        return 0;
    }
    let _ = std::fs::create_dir_all(&*DIR);
    let log = std::fs::File::create(format!("{}/notch.log", *DIR));
    let mut cmd = std::process::Command::new(crate::core::paths::self_exe());
    cmd.args(["notch", "run"]).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null());
    if let Ok(log) = log {
        cmd.stderr(log);
    }
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    match cmd.spawn() {
        Ok(child) => {
            println!("the flock is in the notch (pid {}); `modisa notch stop` takes it out", child.id());
            0
        }
        Err(e) => {
            eprintln!("modisa notch: {e}");
            1
        }
    }
}

fn stop() -> i32 {
    match running() {
        Some(pid) => {
            unsafe { libc::kill(pid, libc::SIGTERM) };
            let _ = std::fs::remove_file(pid_file());
            println!("the flock has left the notch");
        }
        None => println!("the flock isn't in the notch"),
    }
    0
}

fn app() -> i32 {
    let Some(mtm) = MainThreadMarker::new() else { return 1 };
    let _ = std::fs::write(pid_file(), std::process::id().to_string());
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory); // no Dock icon, no menu bar
    let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
        NSPanel::alloc(mtm),
        NSRect::ZERO,
        NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
        NSBackingStoreType::Buffered,
        false,
    );
    panel.setLevel(NSMainMenuWindowLevel + 3); // over the menu bar
    panel.setOpaque(false);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    panel.setHasShadow(false);
    panel.setHidesOnDeactivate(false);
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces | NSWindowCollectionBehavior::Stationary | NSWindowCollectionBehavior::FullScreenAuxiliary | NSWindowCollectionBehavior::IgnoresCycle,
    );
    let view = NotchView::new(mtm);
    view.setWantsLayer(true);
    panel.setContentView(Some(&view));
    view.place();
    panel.orderFrontRegardless();
    view.schedule(4.0);
    app.run();
    0
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Closed,
    Peek,
    Open,
}

// A value that follows its target with a little bounce, settling in about a third of a second.
#[derive(Default)]
struct Spring {
    at: f64,
    vel: f64,
}

impl Spring {
    // Says whether it's still moving.
    fn step(&mut self, to: f64, dt: f64) -> bool {
        const K: f64 = 260.0;
        const D: f64 = 22.0; // damping ratio D / 2√K ≈ 0.68
        if (to - self.at).abs() < 0.25 && self.vel.abs() < 2.0 {
            self.at = to;
            self.vel = 0.0;
            return false;
        }
        let n = (dt / 0.004).ceil().max(1.0);
        for _ in 0..n as usize {
            self.vel += (K * (to - self.at) - D * self.vel) * dt / n;
            self.at += self.vel * dt / n;
        }
        true
    }
}

#[derive(Clone)]
enum Hit {
    Sheep(Sheep),
    Shepherd,
}

struct Notch {
    flock: Flock,
    sheep: Vec<Sheep>,
    news: Option<(News, Instant)>,
    mode: Mode,
    since: Option<Instant>, // when the pointer came (closed) or left (open)
    hush: bool,             // after a click: stays closed until the pointer leaves
    w: Spring,
    h: Spring,
    r: Spring,
    notch: NSRect,  // the notch, in the view
    screen: NSRect, // the screen it's on
    at: f64,        // the notch's middle, from the screen's left edge
    pointer: Option<NSPoint>,
    hits: Vec<(NSRect, Hit)>, // what the last frame put where
    world: World,
    drawn: f64, // the width the last frame drew
    want: f64,  // and the width the window last needed
    start: Instant,
    last: Instant,
    placed: Instant,
    timer: Option<Retained<NSTimer>>,
    rate: f64, // the timer's, a second
}

define_class!(
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = RefCell<Notch>]
    struct NotchView;

    unsafe impl NSObjectProtocol for NotchView {}

    impl NotchView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _e: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            self.draw();
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, e: &NSEvent) {
            self.click(self.convertPoint_fromView(e.locationInWindow(), None));
        }

        #[unsafe(method(tick:))]
        fn tick(&self, _t: &NSTimer) {
            self.step();
        }
    }
);

impl NotchView {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let now = Instant::now();
        let s = Notch {
            flock: Flock::default(),
            sheep: vec![],
            news: None,
            mode: Mode::Closed,
            since: None,
            hush: false,
            w: Spring::default(),
            h: Spring::default(),
            r: Spring::default(),
            notch: NSRect::ZERO,
            screen: NSRect::ZERO,
            at: 0.0,
            pointer: None,
            hits: vec![],
            world: World::new(),
            drawn: 0.0,
            want: 0.0,
            start: now,
            last: now,
            placed: now,
            timer: None,
            rate: 0.0,
        };
        let this = Self::alloc(mtm).set_ivars(RefCell::new(s));
        unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] }
    }

    fn schedule(&self, rate: f64) {
        let mut s = self.ivars().borrow_mut();
        if let Some(t) = s.timer.take() {
            t.invalidate();
        }
        let t = unsafe { NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(1.0 / rate, self, sel!(tick:), None, true) };
        t.setTolerance(0.1 / rate);
        s.timer = Some(t);
        s.rate = rate;
    }

    // Over the notch of the screen that has one (else the main screen's top middle). The window ignores the pointer
    // outside the shape, so the menu bar under it still works.
    fn place(&self) {
        let mtm = self.mtm();
        let Some(screen) = NSScreen::screens(mtm).iter().find(|s| s.safeAreaInsets().top > 0.0).or_else(|| NSScreen::mainScreen(mtm)) else { return };
        let f = screen.frame();
        let top = screen.safeAreaInsets().top;
        let (at, size) = if top > 0.0 {
            let (l, r) = (screen.auxiliaryTopLeftArea(), screen.auxiliaryTopRightArea());
            let w = f.size.width - l.size.width - r.size.width;
            (l.size.width + w / 2.0, NSSize::new(w, top))
        } else {
            let v = screen.visibleFrame();
            (f.size.width / 2.0, NSSize::new(0.0, (f.origin.y + f.size.height - v.origin.y - v.size.height).max(24.0)))
        };
        let want = {
            let mut s = self.ivars().borrow_mut();
            if s.notch.size == size && s.at == at && s.screen == f {
                return; // the same display as before
            }
            (s.screen, s.at, s.notch.size) = (f, at, size);
            (s.w.at, s.h.at) = (size.width + 2.0 * EAR, size.height);
            s.want
        };
        self.size(want, true);
    }

    // The window is only as wide as what it shows (a wide one costs the compositor every frame), in steps so it isn't
    // resized while the shape springs about.
    fn size(&self, want: f64, force: bool) {
        let Some(window) = self.window() else { return };
        let mut s = self.ivars().borrow_mut();
        let have = window.frame().size.width;
        let need = ((want.max(s.notch.size.width + 2.0 * EAR + 2.0 * FLARE) + 32.0) / 64.0).ceil() * 64.0;
        let need = need.min(s.screen.size.width);
        if !force && need <= have && need > have - 128.0 {
            return;
        }
        let h = s.notch.size.height + PASTURE + 24.0; // room for the spring's overshoot
        let x = (s.screen.origin.x + s.at - need / 2.0).clamp(s.screen.origin.x, s.screen.origin.x + s.screen.size.width - need);
        window.setFrame_display(NSRect::new(NSPoint::new(x, s.screen.origin.y + s.screen.size.height - h), NSSize::new(need, h)), true);
        s.notch.origin = NSPoint::new(s.screen.origin.x + s.at - x - s.notch.size.width / 2.0, 0.0);
    }

    fn pointer(&self) -> Option<NSPoint> {
        let m = NSEvent::mouseLocation();
        let f = self.window()?.frame();
        Some(NSPoint::new(m.x - f.origin.x, f.origin.y + f.size.height - m.y))
    }

    fn step(&self) {
        let now = Instant::now();
        if now - self.ivars().borrow().placed > Duration::from_secs(2) {
            self.ivars().borrow_mut().placed = now;
            self.place(); // a display came or went
        }
        let pointer = self.pointer();
        let (dirty, rate, inside, want) = {
            let mut s = self.ivars().borrow_mut();
            let s = &mut *s;
            let dt = (now - s.last).as_secs_f64().min(0.05);
            s.last = now;
            let (changed, news) = s.flock.tick(now);
            if changed {
                s.sheep = s.flock.sheep();
            }
            let l = self.layout(s);
            s.world.resize(l.width, l.height, s.notch.size.height);
            if changed {
                s.world.sync(&s.sheep);
            }
            if s.h.at > s.notch.size.height + PEEK {
                s.world.step(dt); // the pasture lives while it shows
            }
            if let Some(n) = news.into_iter().max_by_key(|n| n.sheep.state.urgency()) {
                s.news = Some((n, now));
            }
            let mode = s.mode;
            let over_shape = pointer.is_some_and(|p| inside(s.shape().inset(-8.0), p));
            let over_notch = pointer.is_some_and(|p| inside(s.closed().inset(-6.0), p));
            if s.hush && !over_shape {
                s.hush = false;
            }
            if s.news.as_ref().is_some_and(|(_, at)| now - *at > NEWS_FOR) && !(s.mode == Mode::Peek && over_shape) {
                s.news = None;
            }
            if s.mode == Mode::Open {
                if over_shape {
                    s.since = None;
                } else if now - *s.since.get_or_insert(now) > LEAVE {
                    (s.mode, s.since) = (Mode::Closed, None);
                }
            } else if over_notch && !s.hush {
                if now - *s.since.get_or_insert(now) > DWELL {
                    (s.mode, s.since) = (Mode::Open, None);
                }
            } else {
                s.since = None;
            }
            if s.mode != Mode::Open {
                s.mode = if s.news.is_some() { Mode::Peek } else { Mode::Closed };
            }
            let (tw, th, tr) = self.target(s);
            let moving = s.w.step(tw, dt) | s.h.step(th, dt) | s.r.step(tr, dt);
            s.pointer = pointer;
            // closed, only a sheep that needs you moves (it hops in the corner); the rest of the time the timer just
            // watches the pointer and the sessions
            let hops = s.sheep.iter().any(|sh| sh.state == State::Blocked);
            let rate = if moving { 60.0 } else if s.mode != Mode::Closed { 30.0 } else if hops { 12.0 } else { 8.0 };
            let all = moving || changed || mode != s.mode || s.mode != Mode::Closed;
            let cx = s.notch.origin.x + s.notch.size.width / 2.0;
            let dirty = if all {
                Some(self.frame())
            } else {
                // closed, only the sheep hopping in the corner moves
                hops.then(|| NSRect::new(NSPoint::new(cx + s.notch.size.width / 2.0, 0.0), NSSize::new(EAR + 8.0, s.notch.size.height)))
            };
            // as wide as the open pasture even while closed: a window resized as it opens shows its old picture
            // stretched for a frame
            let open = (FOLD + s.sheep.len() as f64 * SLOT + 40.0).max(s.notch.size.width + 300.0);
            let want = s.w.at.max(tw).max(s.drawn).max(open) + 2.0 * FLARE + 16.0;
            s.want = want;
            (dirty, (rate != s.rate).then_some(rate), over_shape, want)
        };
        self.size(want, false);
        if let Some(w) = self.window() {
            w.setIgnoresMouseEvents(!inside);
        }
        if let Some(r) = dirty {
            self.setNeedsDisplayInRect(r);
        }
        if let Some(rate) = rate {
            self.schedule(rate);
        }
    }

    fn target(&self, s: &Notch) -> (f64, f64, f64) {
        let (nw, nh) = (s.notch.size.width, s.notch.size.height);
        let closed = nw + 2.0 * EAR;
        match s.mode {
            Mode::Closed => (closed, nh, 10.0),
            Mode::Peek => (closed.max(s.news.as_ref().map_or(0.0, |(n, _)| peek_width(n) + 48.0)), nh + PEEK, 16.0),
            Mode::Open => {
                let max = self.frame().size.width - 40.0;
                ((FOLD + s.sheep.len() as f64 * SLOT + 40.0).max(nw + 300.0).min(max), nh + PASTURE, 26.0)
            }
        }
    }

    // The open pasture in the view: its left edge, its width and its height.
    fn layout(&self, s: &Notch) -> Layout {
        let (nw, nh) = (s.notch.size.width, s.notch.size.height);
        let tw = (FOLD + s.sheep.len() as f64 * SLOT + 40.0).max(nw + 300.0).min(self.frame().size.width - 40.0);
        Layout { left: s.notch.origin.x + nw / 2.0 - tw / 2.0, width: tw, height: nh + PASTURE }
    }

    fn click(&self, p: NSPoint) {
        let hit = {
            let mut s = self.ivars().borrow_mut();
            if s.mode != Mode::Open && p.y < s.notch.size.height {
                (s.mode, s.since) = (Mode::Open, None);
                None
            } else {
                let hit = s.hits.iter().find(|(r, _)| inside(*r, p)).map(|(_, h)| h.clone());
                if hit.is_some() {
                    (s.mode, s.hush, s.news) = (Mode::Closed, true, None);
                }
                if let Some(Hit::Sheep(sh)) = &hit {
                    s.flock.focus(sh);
                }
                hit
            }
        };
        match hit {
            Some(Hit::Sheep(sh)) => bring_terminal(self.mtm(), Some(&sh.session)),
            Some(Hit::Shepherd) => bring_terminal(self.mtm(), None),
            None => {}
        }
        self.schedule(60.0);
    }

    fn draw(&self) {
        let mut s = self.ivars().borrow_mut();
        let s = &mut *s;
        s.hits.clear();
        pen::fade(1.0);
        let shape = s.shape();
        s.drawn = shape.size.width;
        let path = outline(shape, s.r.at);
        NSColor::blackColor().setFill();
        path.fill();
        path.addClip();
        let t = ion();
        let clock = (Instant::now() - s.start).as_secs_f64();
        let (nw, nh) = (s.notch.size.width, s.notch.size.height);
        let cx = s.notch.origin.x + nw / 2.0;
        // opening, the ears fade out as the shape grows past a line of news, then the pasture fades in; closing, the
        // other way round, so neither blinks out while the shape is still moving
        let grown = shape.size.height - nh;
        let a = ((grown / PASTURE - 0.45) / 0.55).clamp(0.0, 1.0);
        if a > 0.0 {
            pen::fade(a);
            self.pasture(s, t);
        }
        let ears = (1.0 - (grown - PEEK - 4.0) / (0.45 * PASTURE - PEEK - 4.0)).clamp(0.0, 1.0); // past a line of news and its bounce
        pen::fade(ears);
        if ears > 0.0 {
            // the ears: the shepherd, and the sheep that wants you most (turned to him) with how many are like it
            art::shepherd_still(cx - nw / 2.0 - EAR / 2.0, nh - 3.0, nh - 6.0);
            if let Some(lead) = s.sheep.iter().max_by_key(|sh| sh.state.urgency()).cloned() {
                let n = s.sheep.iter().filter(|sh| sh.state == lead.state).count();
                // facing the notch, its head reaches 50 of its units forward and its tail 40 back: all of it clears
                // the camera's edge; small enough to leave the count room, and as high as the count
                let (x, k) = (cx + nw / 2.0 + 2.0, 0.31);
                art::sheep_small(x + 50.0 * k, (nh + 54.0 * k) / 2.0 + 1.0, k, true, &still(&lead, clock), clock, collar(&lead), 0.3);
                text(&n.to_string(), x + 90.0 * k + 3.0, (nh - 13.0) / 2.0, 10.0, 0.4, tone(t, lead.state), 1.0);
            }
        }
        if let (Mode::Peek | Mode::Open, Some((news, _))) = (s.mode, s.news.clone()) {
            pen::fade(((shape.size.height - nh) / PEEK).clamp(0.0, 1.0).min(ears));
            let w = peek_width(&news);
            let (x, y) = (cx - w / 2.0, nh + (PEEK - 18.0) / 2.0 - 2.0);
            art::sheep_small(x + 11.0, nh + PEEK - 3.0, 0.4, false, &still(&news.sheep, clock), clock, collar(&news.sheep), 0.3);
            let label = fit(&news.sheep.label, 220.0, 12.0, 0.4);
            text(&label, x + 30.0, y + 1.0, 12.0, 0.4, hex(t.fg), 1.0);
            let what = if news.what == "joined the flock" { hex(t.accent) } else { tone(t, news.sheep.state) };
            text(news.what, x + 30.0 + width(&label, 12.0, 0.4) + 5.0, y + 1.0, 12.0, 0.0, what, 1.0);
            s.hits.push((NSRect::new(NSPoint::new(shape.origin.x, nh), NSSize::new(shape.size.width, PEEK)), Hit::Sheep(news.sheep.clone())));
        }
        pen::fade(1.0);
    }

    // The open notch: what's beside it, then the pasture (art.rs), and what it put where.
    fn pasture(&self, s: &mut Notch, t: &Theme) {
        let nh = s.notch.size.height;
        let l = self.layout(s);
        let dim = hex(t.dim);
        text("modisa", l.left + 22.0, (nh - 14.0) / 2.0, 11.0, 0.4, dim, 1.0);
        let count = match s.sheep.len() {
            1 => "1 agent".to_string(),
            n => format!("{n} agents"),
        };
        text(&count, l.left + l.width - 22.0 - width(&count, 11.0, 0.0), (nh - 14.0) / 2.0, 11.0, 0.0, dim, 1.0);
        let sessions = s.sheep.iter().map(|sh| &sh.session).collect::<std::collections::HashSet<_>>().len();
        let px = self.window().map_or(2.0, |w| w.backingScaleFactor());
        let mut hits = art::pasture(&mut s.world, l.left, s.pointer, sessions, px);
        hits.sheep.sort_by(|a, b| s.world.sheep[a.1].z.total_cmp(&s.world.sheep[b.1].z)); // the nearest first, for clicks
        for (r, j) in hits.sheep {
            s.hits.push((r, Hit::Sheep(s.world.sheep[j].sheep.clone())));
        }
        s.hits.push((hits.shepherd, Hit::Shepherd));
    }
}

// The pasture in the view's points.
struct Layout {
    left: f64,
    width: f64,
    height: f64,
}

impl Notch {
    // where the shape is now, and where it is closed
    fn shape(&self) -> NSRect {
        let cx = self.notch.origin.x + self.notch.size.width / 2.0;
        NSRect::new(NSPoint::new(cx - self.w.at / 2.0, 0.0), NSSize::new(self.w.at, self.h.at))
    }

    fn closed(&self) -> NSRect {
        let w = self.notch.size.width + 2.0 * EAR;
        NSRect::new(NSPoint::new(self.notch.origin.x + self.notch.size.width / 2.0 - w / 2.0, 0.0), NSSize::new(w, self.notch.size.height))
    }
}

fn hex(s: &str) -> Rgb {
    let v = u32::from_str_radix(s.trim_start_matches('#'), 16).unwrap_or(0x8295ad);
    ((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

// A sheep in a corner, posed for its state: hopping when it needs you, grazing at work, standing done, asleep idle.
fn still(sh: &Sheep, clock: f64) -> Body {
    let ph = phase(sh) as f64 * 1.7;
    match sh.state {
        State::Blocked => Body { hop: Some(clock + ph * 0.1), ph, ..Body::default() },
        State::Working => Body { head: 1.0, ph, ..Body::default() },
        State::Done => Body { stand: true, head: -0.06, ph, ..Body::default() },
        State::Idle => Body { lie: 1.0, ph, ..Body::default() },
    }
}

fn collar(sh: &Sheep) -> Rgb {
    hex(crate::config::agents::brands::brand(&sh.harness).color.unwrap_or("#8295ad"))
}

trait Inset {
    fn inset(self, d: f64) -> NSRect;
}

impl Inset for NSRect {
    fn inset(self, d: f64) -> NSRect {
        NSRect::new(NSPoint::new(self.origin.x + d, self.origin.y + d), NSSize::new(self.size.width - 2.0 * d, self.size.height - 2.0 * d))
    }
}

fn inside(r: NSRect, p: NSPoint) -> bool {
    p.x >= r.origin.x && p.x < r.origin.x + r.size.width && p.y >= r.origin.y && p.y < r.origin.y + r.size.height
}

// The notch's shape: straight down from the top, round at the bottom, its top corners curving out into the screen's
// edge as the real notch's do.
fn outline(r: NSRect, radius: f64) -> Retained<NSBezierPath> {
    let (x0, x1, h) = (r.origin.x, r.origin.x + r.size.width, r.size.height);
    let rad = radius.min(h / 2.0).min(r.size.width / 4.0).max(0.0);
    let k = 0.45; // how far a cubic's controls sit along a quarter circle's tangents (≈ 1 - 0.5523)
    let p = NSBezierPath::bezierPath();
    p.moveToPoint(NSPoint::new(x0 - FLARE, 0.0));
    p.curveToPoint_controlPoint1_controlPoint2(NSPoint::new(x0, FLARE), NSPoint::new(x0 - FLARE * k, 0.0), NSPoint::new(x0, FLARE * (1.0 - k)));
    p.lineToPoint(NSPoint::new(x0, h - rad));
    p.curveToPoint_controlPoint1_controlPoint2(NSPoint::new(x0 + rad, h), NSPoint::new(x0, h - rad * k), NSPoint::new(x0 + rad * k, h));
    p.lineToPoint(NSPoint::new(x1 - rad, h));
    p.curveToPoint_controlPoint1_controlPoint2(NSPoint::new(x1, h - rad), NSPoint::new(x1 - rad * k, h), NSPoint::new(x1, h - rad * k));
    p.lineToPoint(NSPoint::new(x1, FLARE));
    p.curveToPoint_controlPoint1_controlPoint2(NSPoint::new(x1 + FLARE, 0.0), NSPoint::new(x1, FLARE * (1.0 - k)), NSPoint::new(x1 + FLARE * k, 0.0));
    p.closePath();
    p
}

fn phase(sh: &Sheep) -> u64 {
    sh.pane.trim_start_matches('p').parse().unwrap_or(0)
}

fn ion() -> &'static Theme {
    find_theme("ion").expect("the ion theme") // the notch is black whatever theme the TUI wears
}

fn tone(t: &Theme, s: State) -> Rgb {
    hex(match s {
        State::Blocked => t.blocked,
        State::Working => t.working,
        State::Done => t.done,
        State::Idle => t.idle,
    })
}

fn peek_width(n: &News) -> f64 {
    let label = fit(&n.sheep.label, 220.0, 12.0, 0.4);
    35.0 + width(&label, 12.0, 0.4) + 5.0 + width(n.what, 12.0, 0.0)
}

// The terminal app a session's TUI is attached in: up from its client's process to the first regular app. Activating
// it is cooperative since macOS 14, so this app takes the focus first, to hand it on.
// ponytail: brings the app forward, not the one window the TUI is in when it has several
fn bring_terminal(mtm: MainThreadMarker, session: Option<&str>) {
    let clients = procs::all().into_iter().filter(|&pid| procs::info(pid).and_then(|i| client_of(&i.args)).is_some_and(|s| session.is_none_or(|want| want == s)));
    for pid in clients {
        let mut p = pid;
        while p > 1 {
            if let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(p) {
                if app.activationPolicy() == NSApplicationActivationPolicy::Regular {
                    NSApplication::sharedApplication(mtm).activate();
                    app.activateWithOptions(NSApplicationActivationOptions::empty());
                    return;
                }
            }
            p = procs::info(p).map_or(0, |i| i.ppid);
        }
    }
}

// The session a TUI client is attached to, from its command line: `modisa`, `modisa -s api`, `modisa attach api`,
// `modisa new api --cwd x`. None for anything else modisa runs (the server, a hook, a command), and for a client of a
// remote session.
fn client_of(args: &str) -> Option<String> {
    let mut words = args.split_whitespace();
    if words.next()?.rsplit('/').next() != Some("modisa") {
        return None;
    }
    let words: Vec<&str> = words.collect();
    if words.contains(&"--remote") {
        return None;
    }
    let flag = words.windows(2).find(|w| w[0] == "-s" || w[0] == "--session").map(|w| w[1]);
    let mut positional = words.iter().enumerate().filter(|(i, w)| !w.starts_with('-') && !(*i > 0 && ["-s", "--session", "--cwd"].contains(&words[i - 1]))).map(|(_, w)| *w);
    let session = match positional.next() {
        None => flag,
        Some("attach" | "a" | "new") => positional.next().or(flag),
        Some(_) => return None,
    };
    Some(session.unwrap_or("default").into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_a_client_by_its_command_line() {
        assert_eq!(client_of("modisa").as_deref(), Some("default"));
        assert_eq!(client_of("/Users/me/.local/bin/modisa -s api").as_deref(), Some("api"));
        assert_eq!(client_of("modisa attach api").as_deref(), Some("api"));
        assert_eq!(client_of("modisa new web --cwd /tmp").as_deref(), Some("web"));
        assert_eq!(client_of("modisa server -s default"), None);
        assert_eq!(client_of("modisa notch run"), None);
        assert_eq!(client_of("modisa --remote ssh://devbox"), None);
        assert_eq!(client_of("zsh"), None);
    }
}
