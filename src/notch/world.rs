// The open notch's pasture as a little world, in the view's points. Depth z runs from 0 at the meadow's front edge to 1
// at the fence, and what's further back stands higher and smaller. A sheep does what its agent is doing: working, it
// walks to open grass and grazes there; needing you, it runs to the shepherd and hops in front of him; done, it strolls
// a little way and stands facing him; idle, it lies down at the back. Sheep steer round each other and the shepherd,
// and stride by the ground they cover, so their feet don't slide. Things that are only scenery (grass, fireflies,
// clouds, stars) are placed here too, so the picture is laid out in one place.
use std::f64::consts::TAU;

use super::art::{hop_lift, Body};
use super::flock::{Sheep, State};

pub const K0: f64 = 0.7; // a sheep's size at the front, against its drawing
const SEP_X: f64 = 60.0; // how close two sheep come, across
const SEP_Z: f64 = 0.5; // and in depth
const Z_PX: f64 = 64.0; // a unit of depth, as distance walked
const WALK: f64 = 13.0; // points a second
const RUN: f64 = 56.0;
const SHEP_X: f64 = 34.0;
const SHEP_Z: f64 = 0.5;
const CALL_AT: f64 = 52.0; // where a sheep that needs you stops, right of the shepherd
const X_MIN: f64 = 112.0; // where sheep graze from: the shepherd's corner is his

// The numbers the scenery is made from: the same ones every time for the same size, so it doesn't reshuffle.
pub struct Rng(pub u32);

impl Rng {
    pub fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        self.0 as f64 / 4294967296.0
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    Go,
    Graze,
    Hop,
    Stand,
    Lie,
}

pub struct Ent {
    pub sheep: Sheep,
    pub x: f64,
    pub z: f64,
    vx: f64,
    vz: f64,
    pub face: f64, // 1 facing right, -1 left, and between while it turns
    pub body: Body,
    pub mode: Mode,
    target: (f64, f64),
    timer: f64,
    go_t: f64,
    pub seen: f64, // since its state changed
    pub tag: f64,  // where its name tag is, along the bottom (NaN until it has one)
    id: usize,
}

pub struct Shep {
    pub x: f64,
    pub z: f64,
    pub raise: f64, // his crook, 0 down to 1 raised
    pub look: f64,  // his head, turned down (+) or up
}

pub struct Fly {
    x0: f64,
    z0: f64,
    a: f64,
    b: f64,
    ph: f64,
    pub x: f64,
    pub z: f64,
    pub h: f64,
    pub on: f64,
}

pub struct Tuft {
    pub x: f64,
    pub z: f64,
    pub n: usize,
    pub bloom: Option<usize>,
    pub s: f64,
}

pub struct Cloud {
    pub x: f64,
    pub y: f64,
    pub v: f64,
    pub a: f64,
    pub puffs: Vec<(f64, f64, f64, f64)>,
}

pub struct Star {
    pub x: f64,
    pub y: f64,
    pub s: f64,
    pub f: f64,
    pub p: f64,
}

pub struct World {
    pub w: f64,
    pub h: f64,
    pub top: f64, // the notch's height: the pasture is below it
    pub t: f64,
    pub tagged: f64, // when the name tags last moved
    pub sheep: Vec<Ent>,
    pub shep: Shep,
    pub moon: (f64, f64, f64),
    pub tree: (f64, f64, f64),
    pub tufts: Vec<Tuft>,
    pub flies: Vec<Fly>,
    pub front: Vec<(f64, f64, usize)>, // grass at the very front: x, size, blades
    pub flecks: Vec<(f64, f64)>,
    pub clouds: Vec<Cloud>,
    pub stars: Vec<Star>,
    rng: Rng,
    ids: usize,
}

fn sign(v: f64) -> f64 {
    if v > 0.0 {
        1.0
    } else if v < 0.0 {
        -1.0
    } else {
        0.0
    }
}

impl World {
    pub fn new() -> World {
        World {
            w: 0.0,
            h: 0.0,
            top: 0.0,
            t: 0.0,
            tagged: 0.0,
            sheep: vec![],
            shep: Shep { x: SHEP_X, z: SHEP_Z, raise: 0.0, look: 0.04 },
            moon: (0.0, 0.0, 0.0),
            tree: (0.0, 0.0, 0.0),
            tufts: vec![],
            flies: vec![],
            front: vec![],
            flecks: vec![],
            clouds: vec![],
            stars: vec![],
            rng: Rng(7),
            ids: 0,
        }
    }

    pub fn gy(&self, z: f64) -> f64 {
        let (front, back) = (self.h - 22.0, self.top + 50.0);
        front - (front - back) * z
    }

    pub fn gk(&self, z: f64) -> f64 {
        K0 * (1.0 - 0.38 * z)
    }

    pub fn fence_y(&self) -> f64 {
        self.gy(1.07)
    }

    // the scenery's scale: how tall the sky is against the one it was drawn for
    pub fn u(&self) -> f64 {
        (self.fence_y() - self.top) / 82.4
    }

    pub fn far_ridge(&self, x: f64) -> f64 {
        self.fence_y() - (26.0 + ((x * 0.0095 + 1.1).sin() + 1.0) * 7.0 + (x * 0.024 + 0.4).sin() * 3.0) * self.u()
    }

    pub fn near_ridge(&self, x: f64) -> f64 {
        self.fence_y() - (6.0 + ((x * 0.015 + 2.2).sin() + 1.0) * 4.0 + (x * 0.041).sin() * 1.2) * self.u()
    }

    // a gust crossing left to right every eight seconds: 0.25 calm, 1 at its height
    pub fn wind(&self, x: f64) -> f64 {
        0.25 + 0.75 * (self.t * 0.8 - x * 0.011).sin().max(0.0).powi(2)
    }

    fn x_max(&self) -> f64 {
        (self.w - 30.0).max(X_MIN)
    }

    // Lays the pasture out for its size: the moon, the tree, the grass, the stars.
    pub fn resize(&mut self, w: f64, h: f64, top: f64) {
        if (w, h, top) == (self.w, self.h, self.top) {
            return;
        }
        (self.w, self.h, self.top) = (w, h, top);
        let u = self.u();
        let mut r = Rng(11);
        self.moon = (w * 0.64, top + 26.0 * u, 10.0 * u);
        let tx = w * 0.9;
        self.tree = (tx, self.near_ridge(tx) + 2.0, u);
        self.tufts = (0..(w / 34.0) as usize)
            .map(|_| Tuft { x: SHEP_X - 30.0 + r.next() * (w - SHEP_X + 10.0), z: -0.08 + r.next() * 1.1, n: 3 + (r.next() * 3.0) as usize, bloom: (r.next() < 0.4).then(|| (r.next() * 3.0) as usize), s: 0.8 + r.next() * 0.5 })
            .collect();
        let (x0, x1) = (X_MIN, self.x_max());
        self.flies = (0..4).map(|_| Fly { x0: x0 + r.next() * (x1 - x0), z0: 0.25 + r.next() * 0.7, a: 0.2 + r.next() * 0.3, b: 0.3 + r.next() * 0.4, ph: r.next() * 9.0, x: 0.0, z: 0.0, h: 0.0, on: 0.0 }).collect();
        let n = (w / 90.0).max(3.0) as usize;
        self.front = (0..n).map(|i| ((i as f64 + 0.2 + r.next() * 0.6) / n as f64 * w, 0.85 + r.next() * 0.5, 4 + (r.next() * 3.0) as usize)).collect();
        self.flecks = (0..70).map(|_| (r.next() * w, r.next() * 1.05)).collect();
        self.clouds = [(10.0, 5.0, 0.12), (20.0, 3.2, 0.09)]
            .into_iter()
            .map(|(y, v, a)| {
                let x = r.next() * w;
                let puffs = (0..4).map(|j| (((j * 9) as f64 - 14.0 + r.next() * 4.0) * u, (r.next() * 3.0 - if j == 1 || j == 2 { 3.0 } else { 0.0 }) * u, (9.0 + r.next() * 6.0) * u, (3.2 + r.next() * 2.2) * u)).collect();
                Cloud { x, y: top + y * u, v, a, puffs }
            })
            .collect();
        self.stars.clear();
        let (mx, my, mr) = self.moon;
        let (tx, ty, ts) = self.tree;
        for _ in 0..600 {
            if self.stars.len() >= (w / 16.0) as usize {
                break;
            }
            let (x, y) = (r.next() * w, top + 4.0 + r.next() * (self.fence_y() - top - 10.0));
            let (s, f, p) = (if r.next() < 0.18 { 1.4 } else { 1.0 }, 0.5 + r.next() * 1.4, r.next() * 9.0);
            let clear = y < self.far_ridge(x) - 5.0 && (x - mx).hypot(y - my) > mr + 10.0 && !((x - tx).abs() < 22.0 * ts && y > ty - 42.0 * ts);
            if clear {
                self.stars.push(Star { x, y, s, f, p });
            }
        }
    }

    // The flock as the sessions last said: new sheep join (walking in from the right, once there's a flock), gone ones
    // leave, and a sheep whose agent changed state starts doing what the new one asks.
    pub fn sync(&mut self, flock: &[Sheep]) {
        let same = |a: &Sheep, b: &Sheep| a.session == b.session && a.pane == b.pane;
        let first = self.sheep.is_empty();
        self.sheep.retain(|e| flock.iter().any(|s| same(s, &e.sheep)));
        let mut next = vec![];
        for (i, s) in flock.iter().enumerate() {
            match self.sheep.iter().position(|e| same(&e.sheep, s)) {
                Some(j) => {
                    let mut e = self.sheep.swap_remove(j);
                    let changed = e.sheep.state != s.state;
                    e.sheep = s.clone();
                    next.push((e, changed));
                }
                None => {
                    self.ids += 1;
                    let x = if first { X_MIN + (i as f64 + 0.5) / flock.len() as f64 * (self.x_max() - X_MIN) } else { self.w + 30.0 };
                    let z = 0.1 + self.rng.next() * 0.8;
                    let body = Body { ph: self.ids as f64 * 1.37 + 0.4, stride: self.rng.next() * 6.0, ..Body::default() };
                    let e = Ent { sheep: s.clone(), x, z, vx: 0.0, vz: 0.0, face: -1.0, body, mode: Mode::Stand, target: (x, z), timer: 0.0, go_t: 0.0, seen: 1.0, tag: f64::NAN, id: self.ids };
                    next.push((e, true));
                }
            }
        }
        let entered: Vec<usize> = next.iter().enumerate().filter(|(_, (_, c))| *c).map(|(i, _)| i).collect();
        self.sheep = next.into_iter().map(|(e, _)| e).collect();
        for j in entered {
            self.enter(j);
            if first {
                // there from the start: already settled in
                let timer = 1.0 + self.rng.next() * 3.0;
                let e = &mut self.sheep[j];
                match e.sheep.state {
                    State::Idle => (e.mode, e.target, e.body.lie) = (Mode::Lie, (e.x, e.z), 1.0),
                    State::Done => e.mode = Mode::Stand,
                    State::Working => (e.mode, e.timer, e.body.head) = (Mode::Graze, timer, 1.0),
                    State::Blocked => {}
                }
                e.seen = 1.0;
            }
        }
    }

    fn enter(&mut self, j: usize) {
        let state = self.sheep[j].sheep.state;
        let target = match state {
            State::Working => Some(self.spot(j, 0.05, 0.95, f64::INFINITY)),
            State::Done => Some(self.spot(j, 0.05, 0.95, 120.0)),
            State::Idle => Some(self.spot(j, 0.62, 1.0, f64::INFINITY)),
            State::Blocked => None,
        };
        let e = &mut self.sheep[j];
        (e.mode, e.seen, e.go_t) = (Mode::Go, 0.0, 0.0);
        if let Some(t) = target {
            e.target = t;
        }
    }

    // 1: just touching
    fn gap(a: (f64, f64), b: (f64, f64)) -> f64 {
        ((a.0 - b.0) / SEP_X).hypot((a.1 - b.1) / SEP_Z)
    }

    // Open grass for sheep j: the best of a dozen tries, as far from the others (and from where they're heading) as can
    // be, and not too far off.
    fn spot(&mut self, j: usize, z0: f64, z1: f64, range: f64) -> (f64, f64) {
        let (ex, ez) = (self.sheep[j].x.min(self.w), self.sheep[j].z);
        let x0 = X_MIN.max(ex - range);
        let x1 = self.x_max().min(ex + range).max(x0);
        let tries: Vec<(f64, f64)> = (0..12).map(|_| (x0 + self.rng.next() * (x1 - x0), z0 + self.rng.next() * (z1 - z0))).collect();
        let score = |c: (f64, f64)| {
            let d = self.sheep.iter().enumerate().filter(|(i, _)| *i != j).fold(4.0f64, |d, (_, o)| {
                let d = d.min(Self::gap(c, (o.x, o.z)));
                if o.mode == Mode::Go { d.min(Self::gap(c, o.target)) } else { d }
            });
            d - (c.0 - ex).abs() / 500.0
        };
        tries.into_iter().max_by(|a, b| score(*a).total_cmp(&score(*b))).unwrap_or((ex, ez))
    }

    pub fn step(&mut self, dt: f64) {
        self.t += dt;
        let callers: Vec<usize> = (0..self.sheep.len()).filter(|&j| self.sheep[j].sheep.state == State::Blocked).collect();
        for j in 0..self.sheep.len() {
            self.think(j, dt, &callers);
        }
        self.advance(dt);
        self.tend(dt, &callers);
        let t = self.t;
        for f in &mut self.flies {
            f.z = (f.z0 + (t * f.a + f.ph).sin() * 0.15).clamp(0.0, 1.0);
            f.x = f.x0 + (t * f.b + f.ph * 2.0).sin() * 40.0;
            f.h = 14.0 + (t * 1.1 + f.ph).sin() * 7.0;
            f.on = (t * 1.6 + f.ph * 3.0).sin().max(0.0).powi(3);
        }
    }

    fn think(&mut self, j: usize, dt: f64, callers: &[usize]) {
        let state = self.sheep[j].sheep.state;
        let (shx, shz) = (self.shep.x, self.shep.z);
        let e = &mut self.sheep[j];
        e.timer -= dt;
        e.seen += dt;
        if e.mode == Mode::Go {
            e.go_t += dt;
        }
        let arrived = e.mode == Mode::Go && ((e.x - e.target.0).hypot((e.z - e.target.1) * Z_PX) < 4.0 || e.go_t > 14.0);
        match state {
            State::Working if arrived => {
                let timer = 2.5 + self.rng.next() * 4.0;
                let e = &mut self.sheep[j];
                (e.mode, e.timer) = (Mode::Graze, timer);
            }
            State::Working if e.mode == Mode::Graze && e.timer <= 0.0 => {
                let t = self.spot(j, 0.05, 0.95, f64::INFINITY);
                let e = &mut self.sheep[j];
                (e.mode, e.go_t, e.target) = (Mode::Go, 0.0, t);
            }
            State::Blocked => {
                // callers line up in front of the shepherd
                let k = callers.iter().position(|&c| c == j).unwrap_or(0);
                e.target = (shx + CALL_AT + k as f64 * 21.0, (shz + [0.0, -0.3, 0.3][k % 3]).clamp(0.0, 1.0));
                if e.mode == Mode::Go && (e.x - e.target.0).hypot((e.z - e.target.1) * Z_PX) < 6.0 {
                    (e.mode, e.body.hop) = (Mode::Hop, Some(0.0));
                }
            }
            State::Idle if arrived => e.mode = Mode::Lie,
            State::Done if arrived => e.mode = Mode::Stand,
            _ => {}
        }
        let e = &mut self.sheep[j];
        e.body.run = state == State::Blocked && e.mode == Mode::Go;
        e.body.stand = e.mode == Mode::Stand;
    }

    // where sheep j is pulled: to its target, and away from anyone it's too close to
    fn steer(&self, j: usize) -> (f64, f64) {
        let e = &self.sheep[j];
        let (mut ax, mut az) = (0.0, 0.0);
        if e.mode == Mode::Go && e.body.lie < 0.05 {
            // a sleeper stands up before it walks
            let (dx, dz) = (e.target.0 - e.x, (e.target.1 - e.z) * Z_PX);
            let d = dx.hypot(dz).max(1e-6);
            let v = if e.body.run { RUN * (d / 30.0).min(1.0) } else { WALK * (d / 14.0).min(1.0) }; // slowing over the last stride
            (ax, az) = (dx / d * v, dz / d * v);
        }
        if e.mode == Mode::Lie || e.mode == Mode::Hop {
            return (ax, az);
        }
        let others = self.sheep.iter().filter(|o| o.id != e.id).map(|o| (o.x, o.z, SEP_X, SEP_Z, o.id as f64)).chain([(self.shep.x, self.shep.z, 40.0, 0.3, -1.0)]);
        for (ox, oz, sx, sz, oid) in others {
            let (dx, dz) = (e.x - ox, e.z - oz);
            let n = (dx / sx).hypot(dz / sz);
            if n >= 1.0 {
                continue;
            }
            // head on, one steps in front and the other behind
            let (ux, uz) = (dx / sx, if dz.abs() < 0.02 { if (e.id as f64) < oid { -0.4 } else { 0.4 } } else { dz / sz });
            let m = ux.hypot(uz).max(1e-6);
            let f = (1.0 - n) * if e.mode == Mode::Go { 50.0 } else { 22.0 };
            ax += ux / m * f;
            az += uz / m * f;
        }
        (ax, az)
    }

    fn advance(&mut self, dt: f64) {
        let pulls: Vec<(f64, f64)> = (0..self.sheep.len()).map(|j| self.steer(j)).collect();
        let (shx, w, t) = (self.shep.x, self.w, self.t);
        for (j, (ax, az)) in pulls.into_iter().enumerate() {
            let k = self.gk(self.sheep[j].z);
            let e = &mut self.sheep[j];
            let a = (dt * 5.0).min(1.0);
            e.vx += (ax - e.vx) * a;
            e.vz += (az - e.vz) * a;
            // it faces where it's going, or the shepherd when it's calling him or done; the flip passes through edge-on
            let mut dir = if e.face < 0.0 { -1.0 } else { 1.0 };
            if e.mode == Mode::Go && e.vx.abs() > 5.0 {
                dir = sign(e.vx);
            } else if e.mode == Mode::Hop || e.mode == Mode::Stand {
                dir = if shx < e.x { -1.0 } else { 1.0 };
            }
            e.face += (dir - e.face) * (dt * 7.0).min(1.0);
            let turning = e.face * sign(e.vx) < 0.3;
            let x = e.x + e.vx * dt * if turning { 0.25 } else { 1.0 };
            e.x = x.max(shx + 30.0).min((w - 30.0).max(e.x)); // one walking in from outside may; none walks out
            e.z = (e.z + e.vz / Z_PX * dt).clamp(-0.02, 1.0);
            let v = e.vx.hypot(e.vz * 0.5);
            let b = &mut e.body;
            b.amp += (if v > 3.0 { 1.0 } else { 0.0 } - b.amp) * (dt * 6.0).min(1.0);
            b.stride += v / k / if b.run { 22.0 } else { 13.0 } * TAU * dt; // a stride for every stride's length covered
            b.hop = if e.mode == Mode::Hop { Some(b.hop.unwrap_or(0.0) + dt) } else { None };
            b.lie = (b.lie + if e.mode == Mode::Lie { 1.0 } else { -1.0 } * dt * 1.3).clamp(0.0, 1.0);
            let head = match e.mode {
                Mode::Graze => 1.0,
                Mode::Stand => -0.06,
                _ => 0.0,
            };
            b.head += (head - b.head) * (dt * 3.0).min(1.0);
            b.chew = if b.head > 0.7 { ((t * 11.0 + b.ph).sin() + 1.0) / 2.0 * (b.head - 0.7) / 0.3 } else { 0.0 };
        }
    }

    // The shepherd watches a sheep that needs you, else whichever is on the move nearest him, and raises his crook
    // once a caller has reached him.
    fn tend(&mut self, dt: f64, callers: &[usize]) {
        let here = callers.iter().any(|&j| self.sheep[j].mode == Mode::Hop);
        self.shep.raise += (if here { 1.0 } else { 0.0 } - self.shep.raise) * (dt * 4.0).min(1.0);
        let at = callers.first().copied().or_else(|| (0..self.sheep.len()).filter(|&j| self.sheep[j].body.amp > 0.5).min_by(|&a, &b| self.sheep[a].x.total_cmp(&self.sheep[b].x)));
        let mut want = 0.04;
        if let Some(j) = at {
            let e = &self.sheep[j];
            let eye = self.gy(self.shep.z) - 76.0 * self.gk(self.shep.z);
            let head = self.gy(e.z) - (hop_lift(&e.body) + 38.0) * self.gk(e.z);
            want = (head - eye).atan2((e.x - self.shep.x).max(30.0)).clamp(-0.18, 0.32);
        }
        self.shep.look += (want - self.shep.look) * (dt * 2.5).min(1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheep(pane: &str, state: State) -> Sheep {
        Sheep { session: "default".into(), pane: pane.into(), label: pane.into(), harness: "codex".into(), state, focused: false }
    }

    #[test]
    fn a_sheep_that_needs_you_runs_to_the_shepherd_and_the_rest_keep_apart() {
        let mut w = World::new();
        w.resize(520.0, 140.0, 32.0);
        let mut flock: Vec<Sheep> = ["p1", "p2", "p3", "p4"].iter().map(|p| sheep(p, State::Working)).collect();
        w.sync(&flock);
        for _ in 0..600 {
            w.step(1.0 / 30.0);
        }
        flock[2].state = State::Blocked;
        flock[3].state = State::Idle;
        w.sync(&flock);
        for _ in 0..450 {
            w.step(1.0 / 30.0);
        }
        let caller = &w.sheep[2];
        assert_eq!(caller.mode, Mode::Hop);
        assert!((caller.x - (SHEP_X + CALL_AT)).abs() < 8.0, "{}", caller.x);
        assert!(w.shep.raise > 0.9);
        assert_eq!(w.sheep[3].mode, Mode::Lie);
        assert!(w.sheep[3].z > 0.55);
        // the two still working stand clear of each other and of the caller
        for (a, b) in [(0, 1), (0, 2), (1, 2)] {
            let (p, q) = (&w.sheep[a], &w.sheep[b]);
            assert!(World::gap((p.x, p.z), (q.x, q.z)) > 0.7, "{a} and {b} overlap");
        }
        // a new sheep walks in from the right
        flock.push(sheep("p5", State::Working));
        w.sync(&flock);
        assert!(w.sheep[4].x > w.w);
        for _ in 0..300 {
            w.step(1.0 / 30.0);
        }
        assert!(w.sheep[4].x < w.w - 29.0);
    }
}
