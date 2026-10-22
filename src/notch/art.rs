// The flock in woodcut, as modisa's emblem is drawn: flat shapes, a bold ink line, a flat cut of shade, gold. The sheep
// and the shepherd are drawn in their own units (feet on y = 0, facing right; a sheep about 76 wide and 54 tall, the
// shepherd 112 tall with his crook) and placed by the pasture's world; the night is drawn from the world's layout.
// Light comes from the moon, so what's lit and where shadows fall depends on where each thing stands.
use std::cell::{Cell, RefCell};
use std::f64::consts::{PI, TAU};
use std::sync::LazyLock;

use objc2::rc::Retained;
use objc2::AllocAnyThread;
use objc2_app_kit::{NSBitmapImageRep, NSCompositingOperation, NSDeviceRGBColorSpace, NSGraphicsContext, NSImage};
use objc2_foundation::{NSPoint, NSRect, NSSize};

use super::flock::State;
use super::pen::{fade, faded, fit, linear, radial, rgb, rotate, save, scale, text, translate, width, Path, Rgb};
use super::world::{Ent, Rng, World};

const INK: Rgb = rgb(0x120f0d);
const WOOL: [Rgb; 3] = [rgb(0xf6eedd), rgb(0xece0c8), rgb(0xd6c4a3)];
const FACE: [Rgb; 3] = [rgb(0x38332f), rgb(0x2a2623), rgb(0x1b1816)];
const INNER_EAR: Rgb = rgb(0xc98c7a);
const ROBE: [Rgb; 3] = [rgb(0x3d3a40), rgb(0x2e2b31), rgb(0x211f24)];
const SKIN: [Rgb; 3] = [rgb(0xefc9a6), rgb(0xe2b38c), rgb(0xc99671)];
const GOLD: [Rgb; 3] = [rgb(0xf8d76c), rgb(0xf5c542), rgb(0xc9961f)];
// the night
const SKY: [Rgb; 2] = [rgb(0x15141b), rgb(0x211f29)];
const HALO: Rgb = (245, 197, 66);
const CRATER: Rgb = (150, 120, 70);
const FAR: [Rgb; 3] = [rgb(0x33303b), rgb(0x2a2731), rgb(0x1f1d25)];
const NEAR: [Rgb; 3] = [rgb(0x272c27), rgb(0x20241f), rgb(0x181b17)];
const TREE: [Rgb; 3] = [rgb(0x2c322b), rgb(0x232822), rgb(0x1a1e19)];
const BARK: [Rgb; 3] = [rgb(0x4a3a30), rgb(0x35291f), rgb(0x211913)];
const FENCE: [Rgb; 3] = [rgb(0x6a5848), rgb(0x4b3e32), rgb(0x2e261f)];
const TUFT: [Rgb; 3] = [rgb(0x4a5444), rgb(0x353d31), rgb(0x262c22)];
const FRONT: [Rgb; 3] = [rgb(0x121411), rgb(0x0d0f0c), rgb(0x080907)];
const BLOOM: [Rgb; 3] = [rgb(0xf3e3b0), rgb(0xecb6c9), rgb(0xbcd2ff)];
const STAR: Rgb = (226, 232, 255);
const MIST: Rgb = (70, 66, 80);
const CLOUD: Rgb = (120, 115, 135);
const FLY: Rgb = (220, 255, 150);

type Box4 = (f64, f64, f64, f64);

fn ease(x: f64) -> f64 {
    x * x * (3.0 - 2.0 * x)
}

// ---------- light ----------
// Toward the light, in the units being drawn in (a sheep facing left is drawn mirrored, so its x flips), and its x on
// screen, for shadows.
thread_local! {
    static LIGHT: Cell<[f64; 3]> = const { Cell::new([-0.6, -0.8, -0.6]) };
}

fn set_light(sx: f64, sy: f64, flip: f64) {
    LIGHT.set([sx * flip, sy, sx]);
}

fn light_from(moon: (f64, f64, f64), x: f64, y: f64, flip: f64) {
    let (dx, dy) = (moon.0 - x, moon.1 - y);
    let m = dx.hypot(dy).max(1e-6);
    set_light(dx / m, dy / m, flip);
}

fn lx() -> f64 {
    LIGHT.get()[0]
}

fn ly() -> f64 {
    LIGHT.get()[1]
}

fn sx() -> f64 {
    LIGHT.get()[2]
}

// ---------- the woodcut's way with a shape ----------
// Flat, with one flat cut of shade across its lower side.
fn paint(p: &Path, c: [Rgb; 3], (x, y, w, h): Box4) {
    p.fill(c[1], 1.0);
    let _s = save();
    p.clip();
    Path::new()
        .move_to(x - 4.0, y + h * 0.62)
        .cubic(x + w * 0.35, y + h * 0.78, x + w * 0.65, y + h * 0.5, x + w + 4.0, y + h * 0.58)
        .line(x + w + 4.0, y + h + 4.0)
        .line(x - 4.0, y + h + 4.0)
        .close()
        .fill(c[2], 1.0);
}

// its ink line, drawn before it so the shape covers the inner half
fn ink(p: &Path, w: f64) {
    p.stroke(INK, 1.0, w);
}

// a round-ended bar, as a shape to fill
fn capsule(x1: f64, y1: f64, x2: f64, y2: f64, w: f64) -> Path {
    let (dx, dy) = (x2 - x1, y2 - y1);
    let l = dx.hypot(dy).max(1e-6);
    let (nx, ny, a, r) = (-dy / l * w / 2.0, dx / l * w / 2.0, dy.atan2(dx), w / 2.0);
    let p = Path::new();
    p.move_to(x1 + nx, y1 + ny).line(x2 + nx, y2 + ny).arc(x2, y2, r, a + PI / 2.0, a - PI / 2.0, true).line(x1 - nx, y1 - ny).arc(x1, y1, r, a - PI / 2.0, a + PI / 2.0, true).close();
    p
}

// a collar's colour, lit and shaded
pub fn shades(c: Rgb) -> [Rgb; 3] {
    let light = |v: u8| (v as u16 + (255 - v as u16) / 3) as u8;
    let dark = |v: u8| (v as u16 * 5 / 8) as u8;
    [(light(c.0), light(c.1), light(c.2)), c, (dark(c.0), dark(c.1), dark(c.2))]
}

// ---------- the sheep ----------
struct Bump {
    x: f64,
    y: f64,
    r: f64,
}

// the fleece's edge: locks over the back and round the rump and chest, flatter under the belly
static EDGE: LazyLock<Vec<Bump>> = LazyLock::new(|| {
    let mut r = Rng(5);
    (0..15)
        .map(|i| {
            let a = i as f64 / 15.0 * TAU + 0.2;
            let under = a.sin() > 0.55;
            Bump { x: a.cos() * 31.0 * 0.86, y: a.sin() * 20.0 * if under { 0.82 } else { 0.8 }, r: 20.0 * if under { 0.3 } else { 0.42 + r.next() * 0.1 } }
        })
        .collect()
});

// and the locks inside it
static LOCKS: LazyLock<Vec<Bump>> = LazyLock::new(|| {
    let mut r = Rng(21);
    let mut v: Vec<Bump> = (0..26)
        .map(|_| {
            let (a, d) = (r.next() * TAU, r.next().sqrt());
            Bump { x: a.cos() * d * 25.0, y: a.sin() * d * 14.0 - 2.0, r: 7.0 + r.next() * 3.5 }
        })
        .collect();
    v.sort_by(|a, b| a.y.total_cmp(&b.y));
    v
});

const HIPS: [f64; 4] = [-19.0, -12.0, 11.0, 18.0]; // back-far, back-near, front-far, front-near

fn fleece() -> Path {
    let p = Path::new();
    p.ellipse(0.0, 1.0, 28.0, 17.0, 0.0);
    for q in EDGE.iter() {
        p.circle(q.x, q.y, q.r);
    }
    p
}

// What a sheep's body is doing, as the pasture's world keeps it: where it is in its stride and how much it's striding
// (0 standing, 1 walking), running rather than walking, how far into its hop (when hopping), standing for good, lying
// (0 up, 1 down), its head (1 down grazing, below 0 raised), chewing, and its own phase so the flock doesn't blink as one.
#[derive(Clone, Default)]
pub struct Body {
    pub stride: f64,
    pub amp: f64,
    pub run: bool,
    pub hop: Option<f64>,
    pub stand: bool,
    pub lie: f64,
    pub head: f64,
    pub chew: f64,
    pub ph: f64,
}

// how high a hop has it, in its own units
pub fn hop_lift(b: &Body) -> f64 {
    let Some(h) = b.hop else { return 0.0 };
    let c = h % 1.2 / 1.2;
    if c > 0.17 && c < 0.8 { ((c - 0.17) / 0.63 * PI).sin() * 22.0 } else { 0.0 }
}

struct Pose {
    legs: [(f64, f64); 4], // each leg's swing and knee bend
    tuck: f64,
    bob: f64,
    pitch: f64,
    lie: f64,
    sx: f64,
    sy: f64,
    lift: f64,
    chew: f64,
    tail: f64,
    head: f64,
    lid: f64,
    ear: f64,
}

// The body this instant. Walking is four-beat (each foot down in turn, the knee bending as the leg swings forward);
// running is a bound (the back feet together, then the front). Both go by the stride, which goes by the ground covered.
fn pose(b: &Body, t: f64) -> Pose {
    let (blink, a, lie) = ((t + b.ph) % 3.6, b.amp, ease(b.lie));
    let mut p = Pose {
        legs: [(0.0, 0.0); 4],
        tuck: 0.0,
        bob: 0.0,
        pitch: 0.0,
        lie,
        sx: 1.0,
        sy: 1.0 + 0.025 * ((t * 1.8 + b.ph).sin() + 1.0) / 2.0,
        lift: hop_lift(b),
        chew: b.chew,
        tail: 0.0,
        head: b.head + 0.28 * lie,
        lid: if lie > 0.5 { 1.0 } else if blink < 0.16 { (blink / 0.16 * PI).sin() } else { 0.0 },
        ear: (t * 1.1 + b.ph).sin() * 0.12 + 0.5 * lie,
    };
    let (swing, knee, offs) = if b.run { (0.62, 0.9, [0.5, 0.6, 0.0, 0.1]) } else { (0.42, 0.7, [0.75, 0.25, 0.5, 0.0]) };
    for (leg, o) in p.legs.iter_mut().zip(offs) {
        let q = b.stride + o * TAU;
        *leg = (q.sin() * swing * a, q.cos().max(0.0) * knee * a);
    }
    if b.run {
        p.bob = 3.2 * a * (b.stride + 0.6).sin().max(0.0);
        p.pitch = 0.07 * a * (b.stride + 1.4).sin();
        p.ear += -0.4 * a + 0.2 * b.stride.sin();
        p.tail = -0.5 * a;
    } else {
        p.bob = 1.3 * a * b.stride.sin().abs();
        p.ear += 0.4 * a * (b.stride - 0.9).sin();
        p.tail = 0.4 * a * (b.stride - 0.5).sin();
        p.head += 0.06 * a * (b.stride * 2.0).sin();
    }
    if let Some(h) = b.hop {
        // crouch, up and over with the legs tucked, land
        let c = h % 1.2 / 1.2;
        if c < 0.17 {
            let s = (c / 0.17 * PI).sin();
            (p.sx, p.sy, p.ear, p.legs) = (1.0 + 0.12 * s, 1.0 - 0.15 * s, 0.6 * s, [(0.0, 0.5 * s); 4]);
        } else if c < 0.8 {
            let u = (c - 0.17) / 0.63;
            let st = (u * PI).cos().abs();
            (p.sx, p.sy, p.tuck, p.ear, p.tail, p.head) = (1.0 - 0.06 * st, 1.0 + 0.1 * st, (u * PI).sin(), -0.5 + u * 1.2, -0.6, -0.15);
        } else {
            let s = ((c - 0.8) / 0.2 * PI).sin();
            (p.sx, p.sy, p.ear, p.legs) = (1.0 + 0.14 * s, 1.0 - 0.14 * s, 0.8 * s, [(0.0, 0.45 * s); 4]);
        }
    } else if b.stand && a < 0.1 {
        let w = (t + b.ph) % 4.2;
        p.tail = if w < 0.7 { (t * 28.0).sin() * 0.7 } else { 0.0 }; // a flick now and then
    }
    p
}

// a leg: hip, knee, foot
fn leg(p: &Pose, i: usize, by: f64) -> [f64; 6] {
    let ((a, bend), hx) = (p.legs[i], HIPS[i]);
    let (front, tuck) = (i >= 2, p.tuck.max(p.lie));
    let hy = by + 14.0 + hx * p.pitch;
    let a1 = a - tuck * if front { 0.6 } else { -0.6 };
    let b1 = if front { -1.0 } else { 1.0 } * (bend + tuck * 1.4);
    let len = (16.0 + p.bob.max(0.0)) * 0.5 * (1.0 - p.tuck * 0.15);
    let (kx, ky) = (hx + a1.sin() * len, hy + a1.cos() * len);
    [hx, hy, kx, ky, kx + (a1 + b1).sin() * len, ky + (a1 + b1).cos() * len]
}

// Creases where the locks overlap, on the side away from the light, and hatching under the belly: they read as wool,
// never as faces.
fn wool_marks() {
    let s = if lx() < 0.0 { (-1.05, 0.15) } else { (-0.15, 1.05) };
    let p = Path::new();
    let crease = |x: f64, y: f64, r: f64, f: f64| {
        p.move_to(x + r * (f + s.0).cos(), y + r * (f + s.0).sin()).arc(x, y, r, f + s.0, f + s.1, false);
    };
    for q in EDGE.iter().filter(|q| q.y < 8.0) {
        crease(q.x, q.y, q.r - 0.6, (1.0 - q.y).atan2(-q.x));
    }
    for q in LOCKS.iter().step_by(3) {
        crease(q.x, q.y, q.r - 0.8, PI / 2.0);
    }
    p.stroke(INK, 1.0, 1.4);
    let mut r = Rng(4);
    let h = Path::new();
    for i in 0..9 {
        let (x, y) = (-22.0 + i as f64 * 5.5 + r.next() * 2.0, 9.0 + r.next() * 4.0);
        let (cy, ey) = (y + 3.0 + r.next() * 2.0, y + 2.0 + r.next() * 3.0);
        h.move_to(x, y).quad(x + 3.0, cy, x + 5.5, ey);
    }
    h.stroke(INK, 1.0, 1.5);
}

fn draw_sheep(p: &Pose, collar: [Rgb; 3]) {
    let by = -31.0 - p.bob + 14.0 * p.lie;
    let legs: Vec<(Path, Path, bool, Box4)> = if p.lie < 0.85 {
        (0..4)
            .map(|i| {
                let [hx, hy, kx, ky, fx, fy] = leg(p, i, by);
                let path = capsule(hx, hy, kx, ky, 6.2);
                path.add(&capsule(kx, ky, fx, fy, 5.2));
                let hoof = Path::new();
                hoof.ellipse(fx + 0.6, fy - 0.6, 3.6, 2.4, 0.0);
                (path, hoof, i % 2 == 0, (hx.min(fx) - 4.0, hy, (fx - hx).abs() + 8.0, (fy - hy).max(4.0)))
            })
            .collect()
    } else {
        vec![]
    };
    let draw_legs = |far: bool| {
        let _s = save();
        Path::new().rect(-90.0, -140.0, 180.0, 140.4).clip(); // folding to lie down, they go into the grass
        for (path, hoof, _, bx) in legs.iter().filter(|l| l.2 == far) {
            ink(path, 5.0);
            paint(path, if far { [FACE[1], FACE[2], FACE[2]] } else { FACE }, *bx);
            hoof.fill(FACE[2], 1.0);
        }
    };
    draw_legs(true);
    // the body, squashed and stretched about its middle, pitched as it bounds
    {
        let _s = save();
        translate(0.0, by);
        rotate(p.pitch);
        scale(p.sx, p.sy);
        let tail = Path::new();
        tail.circle(-32.0, -3.0 + p.tail * 3.0, 6.0);
        let fl = fleece();
        let whole = Path::new();
        whole.add(&fl).add(&tail);
        ink(&whole, 5.0);
        paint(&tail, WOOL, (-38.0, -9.0 + p.tail * 3.0, 12.0, 12.0));
        paint(&fl, WOOL, (-34.0, -20.0, 68.0, 40.0));
        let _c = save();
        fl.clip();
        wool_marks();
    }
    draw_legs(false);
    if p.lie > 0.6 {
        for x in [12.0, -10.0] {
            Path::new().ellipse(x, -1.5, 3.4, 2.0, 0.0).fill(FACE[2], ((p.lie - 0.6) / 0.3).min(1.0));
        }
    }
    // the collar, the near half of a ring round the neck; its bell hangs straight down from the lowest point
    let (cx, cy, rot) = (25.5, by + 1.0 + p.head * 7.0 + 24.0 * p.pitch, -0.45 + p.head * 0.45);
    let ring = Path::new();
    ring.ellipse_arc(cx, cy, 3.6, 7.5, rot, -PI / 2.0, PI / 2.0);
    ring.stroke(INK, 1.0, 6.5);
    ring.stroke(collar[2], 1.0, 3.8);
    ring.stroke(collar[1], 1.0, 2.6);
    {
        let _s = save();
        translate(lx() * 0.6, ly() * 0.6);
        ring.stroke(collar[0], 1.0, 0.9);
    }
    let (bx, bt) = (cx - 7.5 * rot.sin(), cy + 7.5 * rot.cos() - 0.6);
    let bell = Path::new();
    bell.move_to(bx - 3.8, bt + 6.0).cubic(bx - 3.6, bt + 1.0, bx - 2.4, bt + 0.2, bx, bt + 0.2).cubic(bx + 2.4, bt + 0.2, bx + 3.6, bt + 1.0, bx + 3.8, bt + 6.0).quad(bx, bt + 7.2, bx - 3.8, bt + 6.0).close();
    ink(&bell, 5.0);
    paint(&bell, GOLD, (bx - 3.8, bt, 7.6, 7.0));
    Path::new().ellipse(bx, bt + 6.2, 1.2, 0.8, 0.0).fill((60, 35, 10), 0.85);
    // the head on its neck: lowered to graze, laid down asleep, raised to call
    let _s = save();
    translate(30.0, by - 6.0 + p.head * 16.0 + 4.0 * p.lie + 30.0 * p.pitch);
    rotate(p.head * 0.8);
    let ear_a = -0.45 + p.ear * 0.45;
    let ear = |dx: f64, dy: f64, s: f64| {
        let e = Path::new();
        e.ellipse(dx, dy + p.ear * 1.6, 8.5 * s, 3.4 * s, ear_a);
        e
    };
    paint(&ear(-7.0, -7.0, 0.9), [FACE[1], FACE[2], FACE[2]], (-15.0, -11.0, 16.0, 8.0));
    let head = Path::new();
    head.ellipse(2.0, -0.5, 11.0, 9.8, 0.16).ellipse(10.5, 4.0 + p.chew * 0.8, 6.6, 5.4, 0.32);
    ink(&head, 5.0);
    paint(&head, FACE, (-9.0, -10.0, 26.0, 21.0));
    let lock = Path::new();
    for (x, y, r) in [(-3.0, -9.5, 4.4), (2.6, -10.5, 3.8), (-6.5, -5.5, 3.6)] {
        lock.circle(x, y, r);
    }
    ink(&lock, 5.0);
    paint(&lock, WOOL, (-10.0, -15.0, 16.0, 13.0));
    paint(&ear(-5.0, -4.0, 1.0), FACE, (-14.0, -9.0, 18.0, 9.0));
    Path::new().ellipse(-5.0, -4.0 + p.ear * 1.6, 5.4, 1.7, ear_a).fill(INNER_EAR, 1.0);
    // the eye, a glint on the side the light is on, the lid coming down to blink
    let (ex, ey) = (5.5, -1.5);
    let eye = Path::new();
    eye.ellipse(ex, ey, 3.2, 3.6, 0.0);
    eye.fill(rgb(0xfffaf3), 1.0);
    {
        let _e = save();
        eye.clip();
        Path::new().circle(ex + 0.9, ey + 0.2, 2.3).fill(rgb(0x1a1210), 1.0);
        Path::new().circle(ex + 0.9 + lx() * 1.1, ey + 0.2 + ly() * 1.2, 0.95).fill((255, 255, 255), 1.0);
        if p.lid > 0.0 {
            let y = ey - 4.0 + 7.6 * p.lid;
            Path::new().rect(ex - 4.0, ey - 4.0, 8.0, 7.6 * p.lid).fill(FACE[1], 1.0);
            Path::new().move_to(ex - 3.4, y).line(ex + 3.4, y).stroke(FACE[2], 1.0, 0.9);
        }
    }
    let m = p.chew;
    Path::new().ellipse(15.6, 3.4 + m * 0.8, 0.9, 0.6, 0.5).fill((0, 0, 0), 0.55);
    Path::new().move_to(12.5, 7.0 + m * 1.2).quad(14.5, 8.0 + m * 1.2, 16.0, 6.6 + m * 1.2).stroke((0, 0, 0), 0.45, 0.9);
    Path::new().ellipse(6.0, 4.6, 2.8, 1.6, 0.0).fill((255, 130, 150), 0.32);
}

// ---------- the shepherd ----------
// breath 0..1, the wind at his back (about -0.4..1.4), his blink, his crook raised (0..1) and shaken, his head turned
struct Stance {
    breath: f64,
    wind: f64,
    lid: f64,
    raise: f64,
    wave: f64,
    look: f64,
}

fn draw_shepherd(s: &Stance) {
    let (w, b) = (s.wind, s.breath);
    for x in [-4.0, 8.0] {
        Path::new().ellipse(x, -0.5, 5.0, 2.4, 0.0).fill(FACE[2], 1.0);
    }
    // the robe, its hem pushed forward by the wind
    let robe = Path::new();
    robe.move_to(4.0, -66.0 - b)
        .cubic(-8.0, -67.0 - b, -14.0, -58.0, -16.0, -40.0)
        .cubic(-18.0, -24.0, -22.0 + w * 2.0, -10.0, -25.0 + w * 3.0, -1.0)
        .quad(-4.0, 1.5, 17.0 + w * 3.0, -0.5)
        .cubic(17.0 + w, -18.0, 15.0, -40.0, 14.0, -56.0)
        .quad(10.0, -64.0, 4.0, -66.0 - b)
        .close();
    ink(&robe, 5.0);
    paint(&robe, ROBE, (-25.0, -67.0, 45.0, 68.0));
    // the gold lining down its open front, and its folds
    {
        let _s = save();
        robe.clip();
        let lining = Path::new();
        lining.move_to(9.0, -54.0).cubic(6.0, -36.0, 3.0 + w, -16.0, 2.0 + w * 2.4, -0.5);
        lining.stroke(GOLD[1], 1.0, 3.2);
        {
            let _t = save();
            translate(lx() * 0.9, 0.0);
            lining.stroke(GOLD[0], 1.0, 1.0);
        }
        let folds = Path::new();
        for (x0, x1) in [(-8.0, -14.0), (-2.0, -6.0)] {
            folds.move_to(x0, -44.0).quad((x0 + x1) / 2.0 + w, -22.0, x1 + w * 1.5, -2.0);
        }
        folds.stroke(INK, 1.0, 1.4);
    }
    // the head in its hood, turned to whatever he watches
    {
        let _s = save();
        translate(4.0, -60.0 - b * 0.6);
        rotate(s.look);
        translate(-4.0, 60.0);
        let hood = Path::new();
        hood.move_to(-14.0, -56.0)
            .cubic(-18.0, -74.0, -10.0, -92.0, 0.0, -93.0)
            .cubic(10.0, -93.0, 15.0, -84.0, 15.5, -74.0)
            .cubic(15.5, -66.0, 13.0, -60.0, 8.0, -58.0)
            .cubic(0.0, -56.0, -8.0, -54.0, -14.0, -56.0)
            .close();
        ink(&hood, 5.0);
        paint(&hood, ROBE, (-18.0, -94.0, 34.0, 40.0));
        Path::new().ellipse(8.5, -73.0, 7.4, 10.0, 0.1).fill(ROBE[2], 1.0); // the hood's shadow
        let face = Path::new();
        face.ellipse(10.0, -74.0, 5.6, 7.2, 0.12).ellipse(14.2, -73.0, 2.4, 2.0, 0.4);
        paint(&face, SKIN, (4.0, -82.0, 13.0, 15.0));
        if s.lid > 0.5 {
            Path::new().rect(10.2, -76.4, 3.0, 0.9).fill(rgb(0x2a1d18), 1.0);
        } else {
            Path::new().ellipse(11.6, -76.0, 1.1, 1.3, 0.0).fill(rgb(0x2a1d18), 1.0);
        }
        Path::new().ellipse(11.0, -79.0, 3.0, 1.1, -0.15).fill(WOOL[0], 1.0); // his brow
        // the beard: long and white, swinging with the wind
        let beard = Path::new();
        beard.move_to(5.0, -70.0).cubic(9.0, -68.0, 14.0, -68.0, 16.5, -70.0).cubic(17.5, -60.0, 16.0 + w, -50.0, 12.0 + w * 2.2, -40.0).cubic(10.0 + w * 1.4, -46.0, 6.0, -54.0, 3.5, -62.0).close();
        ink(&beard, 5.0);
        paint(&beard, WOOL, (3.0, -71.0, 15.0, 31.0));
        {
            let _b = save();
            beard.clip();
            let strands = Path::new();
            for k in 0..4 {
                let k = k as f64;
                strands.move_to(7.0 + k * 2.6, -67.0).cubic(8.0 + k * 2.4, -58.0, 9.0 + k * 1.8 + w, -50.0, 11.0 + k * 0.8 + w * 1.8, -42.0);
            }
            strands.stroke(INK, 1.0, 0.8);
        }
        Path::new().move_to(9.0, -69.5).quad(13.0, -71.5, 17.0, -67.5).quad(13.0, -69.0, 9.0, -69.5).fill(WOOL[0], 1.0);
    }
    // the staff in his hand, its crook curling back over the top
    let (lift, sx, hy) = (-s.raise * 16.0, 27.0 + s.raise * 2.0, -48.0 - s.raise * 14.0);
    {
        let _s = save();
        translate(sx, hy);
        rotate(s.wave);
        translate(-sx, -hy);
        let staff = Path::new();
        staff.move_to(sx - 1.0, lift).line(sx, -98.0 + lift).cubic(sx + 1.0, -108.0 + lift, sx - 9.0, -112.0 + lift, sx - 11.0, -104.0 + lift).cubic(sx - 12.0, -100.0 + lift, sx - 9.0, -97.0 + lift, sx - 7.0, -99.0 + lift);
        staff.stroke(INK, 1.0, 6.0);
        staff.stroke(GOLD[2], 1.0, 3.8);
        staff.stroke(GOLD[1], 1.0, 2.8);
        let _t = save();
        translate(lx() * 0.8, ly() * 0.4);
        staff.stroke(GOLD[0], 1.0, 1.0);
    }
    // the arm: a wide sleeve out to his hand
    let sleeve = Path::new();
    sleeve.move_to(2.0, -60.0 - b * 0.5).cubic(10.0, -63.0, 18.0, -58.0 - s.raise * 10.0, sx - 4.0, hy - 3.0).line(sx - 2.0, hy + 4.0).cubic(17.0, hy + 6.0, 10.0, -46.0, 4.0, -45.0).close();
    ink(&sleeve, 5.0);
    paint(&sleeve, ROBE, (2.0, -63.0, sx - 2.0, 20.0));
    let hand = Path::new();
    hand.ellipse(sx, hy, 3.6, 4.0, 0.2);
    ink(&hand, 5.0);
    paint(&hand, SKIN, (sx - 4.0, hy - 4.0, 8.0, 8.0));
}

// ---------- in the notch's corners and under it ----------

// The shepherd standing still, `height` points tall with his crook, his feet at (x, ground).
pub fn shepherd_still(x: f64, ground: f64, height: f64) {
    let k = height / 112.0;
    set_light(0.5, -0.85, 1.0);
    let _s = save();
    translate(x, ground);
    scale(k, k);
    draw_shepherd(&Stance { breath: 0.5, wind: 0.0, lid: 0.0, raise: 0.0, wave: 0.0, look: 0.04 });
}

// A sheep with its feet at (x, ground), k its size, doing what `body` says; `hop` is how much of a hop's height it
// keeps (little, in a corner too short for a whole one).
pub fn sheep_small(x: f64, ground: f64, k: f64, left: bool, body: &Body, t: f64, collar: Rgb, hop: f64) {
    let mut p = pose(body, t);
    p.lift *= hop;
    let f = if left { -1.0 } else { 1.0 };
    set_light(-0.6, -0.8, f);
    let _s = save();
    translate(x, ground - p.lift * k);
    scale(f * k, k);
    draw_sheep(&p, shades(collar));
}

// A speech bubble over a head: "!" or "✓" in a tab of colour, or z's rising.
pub fn bubble(x: f64, y: f64, kind: &str, c: Rgb, t: f64, a: f64) {
    let y = y + (t * 5.0).sin();
    if kind == "z" {
        for i in 0..3 {
            let f = (t * 0.55 + i as f64 / 3.0) % 1.0;
            let size = 7.0 + f * 4.0;
            text("z", x + 3.0 + f * 9.0, y - f * 15.0 + 4.0 - size * 0.6, size, 0.4, c, a * (1.0 - f) * (f * 5.0).min(1.0));
        }
        return;
    }
    let p = Path::new();
    p.round_rect(x - 7.5, y - 7.5, 15.0, 13.0, 5.0).move_to(x - 2.5, y + 5.2).line(x + 0.5, y + 9.0).line(x + 2.5, y + 5.2).close();
    p.stroke(INK, a, 2.0);
    p.fill(c, a);
    text(kind, x - width(kind, 10.0, 0.62) / 2.0, y - 7.0, 10.0, 0.62, INK, a);
}

// ---------- the pasture ----------

// what the pasture put where, in the view: each sheep (by its index in the world) and the shepherd
pub struct Hits {
    pub sheep: Vec<(NSRect, usize)>,
    pub shepherd: NSRect,
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

fn sheep_box(w: &World, e: &Ent) -> NSRect {
    let (k, y) = (w.gk(e.z), w.gy(e.z));
    let lift = hop_lift(&e.body) * k;
    rect(e.x - 38.0 * k, y - 58.0 * k - lift, 76.0 * k, 60.0 * k + lift)
}

fn ground_shadow(x: f64, y: f64, rx: f64, k: f64, lift: f64) {
    let s = 1.0 - (lift / 50.0).min(0.5);
    Path::new().ellipse(x - sx() * 6.0 * k, y + 0.5, rx * s * (1.0 + sx().abs() * 0.25), 3.4 * k * s, 0.0).fill((0, 0, 0), 0.42 * s); // cast away from the moon
}

fn sheep_at(w: &World, e: &Ent, glow: Option<Rgb>) {
    let (k, y, p) = (w.gk(e.z), w.gy(e.z), pose(&e.body, w.t));
    let fs = if e.face < 0.0 { -1.0 } else { 1.0 };
    light_from(w.moon, e.x, y - 26.0 * k, fs);
    if let Some(c) = glow {
        Path::new().ellipse(e.x, y + 0.5, 32.0 * k, 5.0 * k, 0.0).fill(c, 0.35);
    }
    ground_shadow(e.x, y, 27.0 * k, k, p.lift);
    let _s = save();
    translate(e.x, y - p.lift * k);
    scale(fs * e.face.abs().max(0.06) * k, k); // turning, it passes through edge-on
    draw_sheep(&p, shades(super::collar(&e.sheep)));
}

fn shepherd_at(w: &World) {
    let (s, t) = (&w.shep, w.t);
    let (k, y, blink) = (w.gk(s.z), w.gy(s.z), t % 4.1);
    light_from(w.moon, s.x, y - 50.0 * k, 1.0);
    ground_shadow(s.x + 2.0 * k, y, 24.0 * k, k, 0.0);
    let _s = save();
    translate(s.x, y);
    scale(k, k);
    draw_shepherd(&Stance {
        breath: ((t * 1.6).sin() + 1.0) / 2.0,
        wind: w.wind(s.x) * 1.6 - 0.4 + (t * 6.1).sin() * 0.08,
        lid: if blink < 0.16 { (blink / 0.16 * PI).sin() } else { 0.0 },
        raise: ease(s.raise),
        wave: s.raise * (t * 8.0).sin() * 0.05,
        look: s.look,
    });
}

fn hill(w: &World, far: bool) -> (Path, Path) {
    let ridge = |x: f64| if far { w.far_ridge(x) } else { w.near_ridge(x) };
    let (body, top) = (Path::new(), Path::new());
    body.move_to(-12.0, w.h + 10.0);
    top.move_to(-12.0, ridge(-12.0));
    let mut x = -12.0;
    while x <= w.w + 12.0 {
        body.line(x, ridge(x));
        top.line(x, ridge(x));
        x += 4.0;
    }
    body.line(w.w + 12.0, w.h + 10.0).close();
    (body, top)
}

// What never moves: the sky, the moon, the hills and their tree, the meadow's rows, the fence.
fn backdrop(w: &World) {
    let (ww, h, top, fy, u) = (w.w, w.h, w.top, w.fence_y(), w.u());
    let iu = u.max(0.7);
    let all = Path::new();
    all.rect(-2.0, top, ww + 4.0, h - top + 2.0);
    linear(&all, (0.0, top), (0.0, fy), &[((0, 0, 0), 1.0), (SKY[0], 1.0), (SKY[1], 1.0)]);
    let (mx, my, mr) = w.moon;
    radial(&all, (mx, my), mr * 0.8, mr * 8.0, &[(HALO, 0.22), (HALO, 0.0)]);
    set_light(-0.55, -0.4, 1.0); // the moon is lit by the sun
    let disc = Path::new();
    disc.circle(mx, my, mr);
    ink(&disc, 2.2 * iu);
    paint(&disc, [GOLD[0], GOLD[1], GOLD[2]], (mx - mr, my - mr, 2.0 * mr, 2.0 * mr));
    for (dx, dy, cr) in [(-3.0, -2.0, 2.4), (3.0, 3.0, 1.6), (2.5, -4.0, 1.1)] {
        Path::new().circle(mx + dx * mr / 10.0, my + dy * mr / 10.0, cr * mr / 10.0).fill(CRATER, 0.28);
    }
    for (far, c, bx) in [(true, FAR, (0.0, fy - 44.0 * u, ww, 60.0 * u)), (false, NEAR, (0.0, fy - 18.0 * u, ww, h - fy + 18.0 * u))] {
        light_from(w.moon, ww / 2.0, bx.1, 1.0);
        let (body, ridge) = hill(w, far);
        paint(&body, c, bx);
        ink(&ridge, 2.4 * iu);
    }
    let (tx, ty, s) = w.tree;
    light_from(w.moon, tx, ty - 20.0 * s, 1.0);
    let trunk = capsule(tx, ty, tx + 1.5 * s, ty - 15.0 * s, 3.4 * s);
    ink(&trunk, 2.2 * iu);
    paint(&trunk, BARK, (tx - 3.0 * s, ty - 15.0 * s, 6.0 * s, 15.0 * s));
    let crown = Path::new();
    for (dx, dy, cr) in [(-6.0, -19.0, 7.5), (5.5, -21.0, 8.5), (0.0, -28.0, 8.5), (8.0, -14.0, 5.5), (-9.0, -13.0, 5.0)] {
        crown.circle(tx + dx * s, ty + dy * s, cr * s);
    }
    ink(&crown, 2.2 * iu);
    paint(&crown, TREE, (tx - 15.0 * s, ty - 37.0 * s, 30.0 * s, 30.0 * s));
    // the meadow: darker toward you, gouged in rows that shrink with distance
    let meadow = Path::new();
    meadow.rect(-2.0, fy, ww + 4.0, h - fy + 2.0);
    linear(&meadow, (0.0, fy), (0.0, h), &[((0, 0, 0), 0.0), ((0, 0, 0), 0.3)]);
    let mut r = Rng(17);
    for z in [0.05, 0.25, 0.47, 0.7, 0.92] {
        let (y, k) = (w.gy(z), w.gk(z));
        let rows = Path::new();
        let mut x = 4.0 + r.next() * 30.0;
        while x < ww {
            let l = (10.0 + r.next() * 22.0) * k;
            rows.move_to(x, y).quad(x + l / 2.0, y - 1.2 * k, x + l, y + 0.4);
            x += 30.0 + r.next() * 50.0;
        }
        rows.stroke(INK, 1.0, 1.3 * k);
    }
    // the fence along the back
    let fk = w.gk(1.07);
    let rails = Path::new();
    for hh in [15.0, 8.0] {
        rails.move_to(0.0, fy - hh * fk).line(ww, fy - hh * fk);
    }
    rails.stroke(INK, 1.0, 2.8 * fk + 1.0);
    rails.stroke(FENCE[2], 1.0, 2.2 * fk);
    {
        let _s = save();
        translate(0.0, -0.5 * fk);
        rails.stroke(FENCE[0], 1.0, 0.7 * fk);
    }
    let mut x = 10.0 + r.next() * 10.0;
    while x < ww {
        let hh = 20.0 * fk + r.next() * 2.0;
        let bx = (x - 1.8 * fk, fy - hh, 3.6 * fk, hh + 1.0);
        let post = Path::new();
        post.round_rect(bx.0, bx.1, bx.2, bx.3, 0.8);
        light_from(w.moon, bx.0, bx.1, 1.0);
        ink(&post, 1.8);
        paint(&post, FENCE, bx);
        x += 34.0 + r.next() * 6.0;
    }
}

thread_local! {
    static BACKDROP: RefCell<Option<([u64; 4], Retained<NSImage>)>> = const { RefCell::new(None) };
}

// The backdrop never moves, so it's drawn once for each size, into an image at the screen's resolution (`px` pixels a
// point), and copied every frame after.
fn backdrop_cached(w: &World, px: f64) {
    let key = [w.w.to_bits(), w.h.to_bits(), w.top.to_bits(), px.to_bits()];
    let img = BACKDROP.with_borrow_mut(|c| {
        if c.as_ref().is_none_or(|(k, _)| *k != key) {
            *c = backdrop_image(w, px).map(|img| (key, img));
        }
        c.as_ref().map(|(_, img)| img.clone())
    });
    match img {
        Some(img) => unsafe { img.drawInRect_fromRect_operation_fraction_respectFlipped_hints(rect(0.0, 0.0, w.w, w.h), NSRect::ZERO, NSCompositingOperation::SourceOver, faded(), true, None) },
        None => backdrop(w),
    }
}

fn backdrop_image(w: &World, px: f64) -> Option<Retained<NSImage>> {
    let (pw, ph) = ((w.w * px).ceil() as isize, (w.h * px).ceil() as isize);
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            pw,
            ph,
            8,
            4,
            true,
            false,
            NSDeviceRGBColorSpace,
            0,
            0,
        )
    }?;
    rep.setSize(NSSize::new(w.w, w.h));
    let ctx = NSGraphicsContext::graphicsContextWithBitmapImageRep(&rep)?;
    let a = faded();
    NSGraphicsContext::saveGraphicsState_class();
    NSGraphicsContext::setCurrentContext(Some(&ctx));
    fade(1.0);
    translate(0.0, w.h); // an image's y runs up; the pasture's down
    scale(1.0, -1.0);
    backdrop(w);
    ctx.flushGraphics();
    NSGraphicsContext::restoreGraphicsState_class();
    fade(a);
    let img = NSImage::initWithSize(NSImage::alloc(), NSSize::new(w.w, w.h));
    img.addRepresentation(&rep);
    Some(img)
}

fn sky(w: &World) {
    for s in &w.stars {
        let a = 0.22 + 0.6 * (((w.t * s.f + s.p).sin() + 1.0) / 2.0).powi(2);
        Path::new().rect(s.x, s.y, s.s, s.s).fill(STAR, a);
    }
    for c in &w.clouds {
        // drifting with the wind, over the moon
        let x = (c.x + w.t * c.v).rem_euclid(w.w + 120.0) - 60.0;
        let p = Path::new();
        for &(dx, dy, rx, ry) in &c.puffs {
            p.ellipse(x + dx, c.y + dy, rx, ry, 0.0);
        }
        p.fill(CLOUD, c.a);
    }
}

// blades of grass from (x, y), bending with the wind there; and where their tips are
fn blades(w: &World, x: f64, y: f64, k: f64, n: usize) -> (Path, Vec<(f64, f64)>) {
    let bend = ((w.wind(x) - 0.25) * 5.0 + (w.t * 2.1 + x).sin() * 0.5) * k;
    let (p, mut tips) = (Path::new(), vec![]);
    for j in 0..n {
        let o = j as f64 - (n as f64 - 1.0) / 2.0;
        let (bx, h) = (x + o * 2.4 * k, (8.0 + ((j * 7) % 3) as f64 * 2.5) * k);
        let (tx, ty) = (bx + o * 1.6 * k + bend * h / (10.0 * k), y - h);
        p.move_to(bx - 1.1 * k, y).quad(bx - 0.2 * k, y - h * 0.55, tx, ty).quad(bx + 0.6 * k, y - h * 0.5, bx + 1.1 * k, y).close();
        tips.push((tx, ty));
    }
    (p, tips)
}

// the bubbles over the flock's heads: kept inside the pasture, below the notch, and off each other
fn bubbles(w: &World) {
    let th = super::ion();
    let mut list: Vec<(f64, f64, &str, Rgb, f64)> = vec![];
    for e in &w.sheep {
        let (k, y) = (w.gk(e.z), w.gy(e.z));
        let fs = if e.face < 0.0 { -1.0 } else { 1.0 };
        let head = y - (hop_lift(&e.body) + 50.0 - 16.0 * e.body.lie) * k;
        let tone = super::tone(th, e.sheep.state);
        match e.sheep.state {
            State::Blocked => list.push((e.x + fs * 27.0 * k, head - 13.0, "!", tone, 1.0)),
            State::Done => list.push((e.x + fs * 27.0 * k, head - 11.0, "✓", tone, (e.seen * 2.0).min(1.0))),
            State::Idle if e.body.lie > 0.7 => list.push((e.x + fs * 16.0 * k, y - 30.0 * k, "z", tone, (e.body.lie - 0.7) / 0.3)),
            _ => {}
        }
    }
    list.sort_by(|a, b| a.0.total_cmp(&b.0));
    let top = w.top + 9.0;
    for i in 0..list.len() {
        list[i].0 = list[i].0.clamp(12.0, w.w - 12.0);
        list[i].1 = list[i].1.max(top);
        for j in 0..i {
            let o = list[j];
            if (o.0 - list[i].0).abs() < 17.0 && (o.1 - list[i].1).abs() < 15.0 {
                if o.1 - 16.0 >= top { list[i].1 = o.1 - 16.0 } else { list[i].0 = o.0 + 18.0 }
            }
        }
    }
    for (x, y, kind, c, a) in list {
        bubble(x, y, kind, c, w.t, a);
    }
}

fn singular(s: State) -> &'static str {
    match s {
        State::Blocked => "needs you",
        State::Working => "working",
        State::Done => "done",
        State::Idle => "idle",
    }
}

// Name tags along the bottom, each under its sheep and nudged along so none overlap. The one you point at says more.
fn tags(w: &mut World, hover: Option<usize>, sessions: usize) {
    let n = w.sheep.len();
    if n == 0 {
        return;
    }
    let dt = (w.t - w.tagged).clamp(0.0, 0.1);
    w.tagged = w.t;
    let (th, y, size) = (super::ion(), w.h - 9.0, 10.5);
    let (fg, dim) = (super::hex(th.fg), super::hex(th.dim));
    let budget = ((w.w - 40.0) / n as f64 - 26.0).clamp(40.0, 150.0);
    let mut tags: Vec<(usize, Vec<(String, Rgb, f64)>, f64, f64)> = w
        .sheep
        .iter()
        .enumerate()
        .map(|(j, e)| {
            let (sh, tone) = (&e.sheep, super::tone(th, e.sheep.state));
            let parts = if hover == Some(j) {
                let place = if sessions > 1 { format!("{} · {}", sh.session, sh.pane) } else { sh.pane.clone() };
                vec![(fit(&sh.label, 180.0, size, 0.4), fg, 0.4), (format!("  {} · {place} · ", sh.harness), dim, 0.0), (singular(sh.state).into(), tone, 0.23)]
            } else {
                vec![(fit(&sh.label, budget, size, 0.23), if sh.state == State::Blocked { tone } else { dim }, 0.23)]
            };
            let wd = 19.0 + parts.iter().map(|(s, _, wt)| width(s, size, *wt)).sum::<f64>();
            (j, parts, wd, e.x - wd / 2.0)
        })
        .collect();
    tags.sort_by(|a, b| a.3.total_cmp(&b.3));
    for i in 1..n {
        tags[i].3 = tags[i].3.max(tags[i - 1].3 + tags[i - 1].2 + 6.0);
    }
    let mut right = w.w - 16.0;
    for t in tags.iter_mut().rev() {
        t.3 = t.3.min(right - t.2);
        right = t.3 - 6.0;
    }
    for (j, parts, wd, x) in tags {
        let e = &mut w.sheep[j];
        let x = x.max(16.0);
        e.tag = if e.tag.is_nan() { x } else { e.tag + (x - e.tag) * (dt * 8.0).min(1.0) };
        let x = e.tag;
        Path::new().round_rect(x, y - 7.5, wd, 15.0, 7.5).fill((0, 0, 0), 0.84);
        Path::new().circle(x + 7.5, y, 2.3).fill(super::tone(th, e.sheep.state), 1.0);
        let mut tx = x + 13.0;
        for (s, c, wt) in parts {
            text(&s, tx, y - 7.0, size, wt, c, 1.0);
            tx += width(&s, size, wt);
        }
    }
}

// The open notch, its left edge at `left` in the view: the night, the flock and its shepherd back to front, what each
// sheep wants, their names. `pointer` is in the view too; `px` is how many pixels the screen has to a point.
pub fn pasture(w: &mut World, left: f64, pointer: Option<NSPoint>, sessions: usize, px: f64) -> Hits {
    let _s = save();
    translate(left, 0.0);
    backdrop_cached(w, px);
    sky(w);
    let boxes: Vec<NSRect> = w.sheep.iter().map(|e| sheep_box(w, e)).collect();
    let hover = pointer.map(|p| NSPoint::new(p.x - left, p.y)).and_then(|p| (0..boxes.len()).filter(|&j| super::inside(boxes[j], p)).min_by(|&a, &b| w.sheep[a].z.total_cmp(&w.sheep[b].z)));
    enum Item {
        Tuft(usize),
        Fly(usize),
        Sheep(usize),
        Shepherd,
        Mist,
    }
    let mut items: Vec<(f64, Item)> = w.tufts.iter().enumerate().map(|(i, q)| (q.z, Item::Tuft(i))).collect();
    items.extend(w.flies.iter().enumerate().map(|(i, f)| (f.z, Item::Fly(i))));
    items.extend(w.sheep.iter().enumerate().map(|(i, e)| (e.z, Item::Sheep(i))));
    items.extend([(w.shep.z, Item::Shepherd), (0.62, Item::Mist)]);
    items.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (_, item) in items {
        match item {
            Item::Tuft(i) => {
                let q = &w.tufts[i];
                let (k, y) = (w.gk(q.z) * q.s, w.gy(q.z));
                let (p, tips) = blades(w, q.x, y, k, q.n);
                light_from(w.moon, q.x, y - 6.0 * k, 1.0);
                paint(&p, TUFT, (q.x - 7.0 * k, y - 13.0 * k, 14.0 * k, 13.0 * k));
                if let Some(b) = q.bloom {
                    let (tx, ty) = tips[tips.len() / 2];
                    Path::new().circle(tx, ty, 1.4 * k).fill(BLOOM[b], 1.0);
                    Path::new().circle(tx, ty, 0.5 * k).fill((255, 240, 170), 0.9);
                }
            }
            Item::Fly(i) => {
                let f = &w.flies[i];
                if f.on >= 0.03 {
                    let k = w.gk(f.z);
                    let (x, y) = (f.x, w.gy(f.z) - f.h * k);
                    let glow = Path::new();
                    glow.circle(x, y, 7.0 * k);
                    radial(&glow, (x, y), 0.0, 7.0 * k, &[(FLY, 0.55 * f.on), (FLY, 0.0)]);
                    Path::new().circle(x, y, 1.1 * k).fill((250, 255, 210), f.on);
                }
            }
            Item::Sheep(j) => {
                let e = &w.sheep[j];
                sheep_at(w, e, (hover == Some(j)).then(|| super::tone(super::ion(), e.sheep.state)));
            }
            Item::Shepherd => shepherd_at(w),
            Item::Mist => {
                // a band of mist lying mid-field: what's behind it fades a little
                let (y0, y1) = (w.gy(1.1), w.gy(0.3));
                let band = Path::new();
                band.rect(-2.0, y0, w.w + 4.0, y1 - y0);
                linear(&band, (0.0, y0), (0.0, y1), &[(MIST, 0.0), (MIST, 0.13 + 0.04 * (w.t * 0.3).sin()), (MIST, 0.0)]);
            }
        }
    }
    for &(x, s, n) in &w.front {
        // grass at the very front, over everyone's feet
        let (k, y) = (w.gk(-0.3) * s * 1.4, w.gy(-0.3));
        light_from(w.moon, x, y - 8.0 * k, 1.0);
        paint(&blades(w, x, y, k, n).0, FRONT, (x - 9.0 * k, y - 14.0 * k, 18.0 * k, 14.0 * k));
    }
    bubbles(w);
    tags(w, hover, sessions);
    let shift = |r: NSRect| NSRect::new(NSPoint::new(r.origin.x + left, r.origin.y), r.size);
    let (k, y) = (w.gk(w.shep.z), w.gy(w.shep.z));
    let shepherd = shift(rect(w.shep.x - 26.0 * k, y - 112.0 * k, 56.0 * k, 112.0 * k));
    Hits { sheep: boxes.into_iter().enumerate().map(|(j, r)| (shift(r), j)).collect(), shepherd }
}
