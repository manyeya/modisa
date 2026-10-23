// The flock, drawn the way `modisa notch` draws it (src/notch/art.rs): flat woodcut shapes, an ink line, one cut of
// light and one of shade, gold. Every page uses it: the home page's pasture, parallax and scrolled scenes, the strip of
// pasture over each docs page, and the stray on the 404. Each canvas says what it is with data-flock.
(() => {
const TAU = Math.PI * 2;
const reduce = matchMedia("(prefers-reduced-motion: reduce)").matches;
const fine = matchMedia("(pointer: fine)").matches;
const $ = (s) => document.querySelector(s);
const $$ = (s) => [...document.querySelectorAll(s)];
const INK = "#0e0d0b";
const WOOL = ["#f6eedd", "#ece0c8", "#d6c4a3"];
const FACE = ["#38332f", "#2a2623", "#1b1816"];
const GOLD = ["#f8d76c", "#f5c542", "#c9961f"];
const ROBE = ["#3d3a40", "#2e2b31", "#211f24"];
const SKIN = ["#efc9a6", "#e2b38c", "#c99671"];
const WOOD = ["#b39468", "#8f7149", "#6a5233"];
const COLLARS = ["#d97757", "#c0caf5", "#7aa2f7", "#e0af68", "#bb9af7", "#f7768e", "#7dcfff", "#ff9e64", "#73daca", "#f5c542"];
const rng = (seed) => () => (seed = (seed * 16807) % 2147483647) / 2147483647;
const ease = (a, b, k) => a + (b - a) * Math.min(1, k);
const clamp = (v, a = 0, b = 1) => Math.max(a, Math.min(b, v));
const lerp = (a, b, k) => a + (b - a) * k;
const hash = (s) => [...s].reduce((a, ch) => (a * 31 + ch.charCodeAt(0)) % 2147483629, 7) + 1;

// ---------- the woodcut's way with a shape: an ink line under it, flat colour, a cut of light on top and of shade below
function ink(c, p, w) { c.lineWidth = w; c.lineJoin = "round"; c.strokeStyle = INK; c.stroke(p); }
function paint(c, p, col, x, y, w, h) {
  c.fillStyle = col[1]; c.fill(p);
  c.save(); c.clip(p);
  c.fillStyle = col[0]; c.beginPath(); c.moveTo(x - 4, y - 4); c.lineTo(x + w + 4, y - 4); c.lineTo(x + w + 4, y + h * .26); c.lineTo(x - 4, y + h * .4); c.fill();
  c.fillStyle = col[2]; c.beginPath(); c.moveTo(x - 4, y + h * .74); c.lineTo(x + w + 4, y + h * .62); c.lineTo(x + w + 4, y + h + 4); c.lineTo(x - 4, y + h + 4); c.fill();
  c.restore();
}
function ellipse(p, x, y, rx, ry, rot) { p.moveTo(x + rx * Math.cos(rot), y + rx * Math.sin(rot)); p.ellipse(x, y, rx, ry, rot, 0, TAU); return p; }
function circle(p, x, y, r) { p.moveTo(x + r, y); p.arc(x, y, r, 0, TAU); return p; }

// ---------- a sheep: about 76 wide and 54 tall, feet on y = 0, facing right
const EDGE = [], LOCKS = [];
{
  const r = rng(5);
  for (let i = 0; i < 15; i++) { const a = i / 15 * TAU + .2, under = Math.sin(a) > .55; EDGE.push([Math.cos(a) * 31 * .86, Math.sin(a) * 20 * .8, 20 * (under ? .3 : .42 + r() * .1)]); }
  const q = rng(21);
  for (let i = 0; i < 16; i++) { const a = q() * TAU, d = Math.sqrt(q()); LOCKS.push([Math.cos(a) * d * 24, Math.sin(a) * d * 13 - 2, 6 + q() * 3]); }
}
const FLEECE = (() => { const p = ellipse(new Path2D(), 0, 1, 28, 17, 0); EDGE.forEach(([x, y, r]) => circle(p, x, y, r)); return p; })();
const HIPS = [-19, -12, 11, 18];

function sheep(c, x, gy, k, face, s, t, collar) {
  const lift = s.hop > 0 ? Math.sin(Math.min(1, s.hop) * Math.PI) * (s.hopH || 30) : 0;
  c.save(); c.translate(x, gy);
  c.fillStyle = "rgba(0,0,0,.32)"; c.beginPath(); c.ellipse(-4 * k * face, 1, 30 * k * (1 - lift / 120), 4 * k * (1 - lift / 120), 0, 0, TAU); c.fill();
  c.scale(face * k, k); c.translate(0, -lift);
  const lie = s.lie || 0, run = s.run || 0, walk = s.walk || 0;
  const by = -31 - walk * Math.abs(Math.sin(s.stride)) * (1.5 + run * 2.5) + 14 * lie;
  const pitch = run * walk * Math.sin(s.stride * 2) * .07;
  const tuck = lift > 0 ? 1 : 0;
  const legs = [];
  if (lie < .85) for (let i = 0; i < 4; i++) {
    const ph = s.stride + [0, Math.PI, Math.PI * .5, Math.PI * 1.5][i] * (run ? .6 : 1);
    const hx = HIPS[i], hy = by + 12 + (i >= 2 ? pitch * -40 : pitch * 40);
    const sw = Math.sin(ph) * walk * (9 + run * 6), up = Math.max(0, Math.cos(ph)) * walk * (4 + run * 3);
    const fx = hx + sw * (1 - tuck) + (i >= 2 ? -4 : 4) * tuck, fy = -up - tuck * 10;
    legs.push([i % 2 === 0, hx, hy, (hx + fx) / 2 + (i >= 2 ? -1.5 : 1.5) * (1 + up * .4), (hy + fy) / 2, fx, fy]);
  }
  const drawLegs = (far) => {
    c.save(); c.beginPath(); c.rect(-90, -160, 180, 160.4); c.clip(); // folding to lie down, they go into the grass
    for (const [f, hx, hy, kx, ky, fx, fy] of legs) if (f === far) {
      c.lineCap = "round"; c.lineJoin = "round";
      c.beginPath(); c.moveTo(hx, hy); c.lineTo(kx, ky); c.lineTo(fx, fy);
      c.strokeStyle = INK; c.lineWidth = 10.5; c.stroke();
      c.strokeStyle = far ? FACE[2] : FACE[1]; c.lineWidth = 6; c.stroke();
      c.fillStyle = FACE[2]; c.beginPath(); c.ellipse(fx + .6, fy - .6, 3.6, 2.4, 0, 0, TAU); c.fill();
    }
    c.restore();
  };
  drawLegs(true);
  c.save(); c.translate(0, by); c.rotate(pitch);
  const tail = circle(new Path2D(), -32, -3 + (s.tail || 0) * 3, 6);
  const whole = new Path2D(FLEECE); whole.addPath(tail);
  ink(c, whole, 5);
  paint(c, tail, WOOL, -38, -9, 12, 12);
  paint(c, FLEECE, WOOL, -34, -20, 68, 40);
  c.save(); c.clip(FLEECE);
  c.strokeStyle = WOOL[2]; c.lineWidth = 1.4; c.lineCap = "round";
  for (const [lx, ly, lr] of LOCKS) { c.beginPath(); c.arc(lx, ly, lr, .35, 1.9); c.stroke(); }
  c.restore(); c.restore();
  drawLegs(false);
  if (lie > .6) { c.fillStyle = FACE[2]; c.globalAlpha = Math.min(1, (lie - .6) / .3); for (const lx of [12, -10]) { c.beginPath(); c.ellipse(lx, -1.5, 3.4, 2, 0, 0, TAU); c.fill(); } c.globalAlpha = 1; }
  // the collar in its agent's colour, its bell hanging straight down
  const head = s.head || 0;
  const cx = 25.5, cy = by + 1 + head * 7 + 24 * pitch, rot = -.45 + head * .45;
  c.beginPath(); c.ellipse(cx, cy, 3.6, 7.5, rot, -Math.PI / 2, Math.PI / 2);
  c.lineCap = "round"; c.strokeStyle = INK; c.lineWidth = 6.5; c.stroke(); c.strokeStyle = collar; c.lineWidth = 3.6; c.stroke();
  const bx = cx - 7.5 * Math.sin(rot), bt = cy + 7.5 * Math.cos(rot) - .6;
  const bell = new Path2D(); bell.moveTo(bx - 3.8, bt + 6); bell.bezierCurveTo(bx - 3.6, bt + 1, bx - 2.4, bt + .2, bx, bt + .2); bell.bezierCurveTo(bx + 2.4, bt + .2, bx + 3.6, bt + 1, bx + 3.8, bt + 6); bell.quadraticCurveTo(bx, bt + 7.2, bx - 3.8, bt + 6); bell.closePath();
  ink(c, bell, 4); paint(c, bell, GOLD, bx - 3.8, bt, 7.6, 7);
  // the head: lowered to graze, laid down asleep, raised to call
  c.save(); c.translate(30, by - 6 + head * 16 + 4 * lie + 30 * pitch); c.rotate(head * .8);
  const ear = (dx, dy, sc) => ellipse(new Path2D(), dx, dy, 8.5 * sc, 3.4 * sc, -.45);
  c.fillStyle = FACE[2]; c.fill(ear(-7, -7, .9));
  const hd = ellipse(new Path2D(), 2, -.5, 11, 9.8, .16); ellipse(hd, 10.5, 4 + (s.chew || 0) * .8, 6.6, 5.4, .32);
  ink(c, hd, 5); paint(c, hd, FACE, -9, -10, 26, 21);
  const lock = new Path2D(); circle(lock, -3, -9.5, 4.4); circle(lock, 2.6, -10.5, 3.8); circle(lock, -6.5, -5.5, 3.6);
  ink(c, lock, 5); paint(c, lock, WOOL, -10, -15, 16, 13);
  const e2 = ear(-5, -4, 1); ink(c, e2, 3); paint(c, e2, FACE, -14, -9, 18, 9);
  c.fillStyle = "#c98c7a"; c.beginPath(); c.ellipse(-5, -4, 5.4, 1.7, -.45, 0, TAU); c.fill();
  if (lie > .6 || ((t + (s.seed || 0) * 3.7) % 4.3) < .14) { c.strokeStyle = "#fffaf3"; c.lineWidth = 1.3; c.beginPath(); c.arc(5.5, -2.4, 2.6, .3, Math.PI - .3); c.stroke(); }
  else {
    c.fillStyle = "#fffaf3"; c.beginPath(); c.ellipse(5.5, -1.5, 3.2, 3.6, 0, 0, TAU); c.fill();
    c.fillStyle = "#1a1210"; c.beginPath(); c.arc(6.4, -1.3, 2.3, 0, TAU); c.fill();
    c.fillStyle = "#fff"; c.beginPath(); c.arc(7.1, -2.1, .95, 0, TAU); c.fill();
  }
  c.fillStyle = "rgba(0,0,0,.55)"; c.beginPath(); c.ellipse(15.6, 3.4, .9, .6, .5, 0, TAU); c.fill();
  c.fillStyle = "rgba(255,130,150,.3)"; c.beginPath(); c.ellipse(6, 4.6, 2.8, 1.6, 0, 0, TAU); c.fill();
  c.restore();
  c.restore();
}

// ---------- the shepherd: 112 tall with his crook, feet on y = 0, facing right; he raises the crook when one calls
function shepherd(c, x, gy, k, face, st, t) {
  const wind = Math.sin(t * .9) * .5 + .4, breath = Math.sin(t * 1.6) * .6;
  const raise = st.raise || 0, shake = Math.sin(t * 16) * raise * .05;
  c.save(); c.translate(x, gy);
  c.fillStyle = "rgba(0,0,0,.34)"; c.beginPath(); c.ellipse(4 * k * face, 1, 30 * k, 4.2 * k, 0, 0, TAU); c.fill();
  c.scale(face * k, k);
  const robe = new Path2D();
  robe.moveTo(-20, 0); robe.quadraticCurveTo(-19, -40, -14, -62 + breath); robe.quadraticCurveTo(-14, -90, 1, -95 + breath);
  robe.quadraticCurveTo(16, -94, 18, -75 + breath); robe.lineTo(15, -63 + breath); robe.quadraticCurveTo(19, -34, 22 + wind * 3, 0); robe.closePath();
  ink(c, robe, 5); paint(c, robe, ROBE, -22, -95, 46, 95);
  const lining = new Path2D(); lining.moveTo(12, -60 + breath); lining.quadraticCurveTo(16, -30, 19 + wind * 3, 0); lining.lineTo(14 + wind * 2, 0); lining.quadraticCurveTo(12, -30, 9, -58 + breath); lining.closePath();
  paint(c, lining, GOLD, 9, -60, 12, 60);
  c.fillStyle = ROBE[2]; c.beginPath(); c.ellipse(9, -76 + breath, 7.4, 10, .1, 0, TAU); c.fill();
  c.fillStyle = SKIN[1]; c.beginPath(); c.ellipse(11.5, -78 + breath, 4.6, 5.6, .1, 0, TAU); c.fill();
  const beard = new Path2D(); beard.moveTo(7, -75 + breath); beard.quadraticCurveTo(9 - wind * 2, -60, 12 - wind * 3, -54 + breath); beard.quadraticCurveTo(15, -62, 16.5, -74 + breath); beard.closePath();
  ink(c, beard, 3); paint(c, beard, WOOL, 7, -75, 10, 21);
  c.fillStyle = WOOL[0]; c.beginPath(); c.ellipse(11.5, -81 + breath, 3, 1.1, -.15, 0, TAU); c.fill();
  c.fillStyle = "#1a1210"; c.beginPath(); c.arc(13, -79 + breath, .9, 0, TAU); c.fill();
  c.save(); c.translate(27, -48); c.rotate(-raise * .22 + shake); c.translate(0, -raise * 12);
  const staff = new Path2D(); staff.moveTo(0, 50); staff.lineTo(0, -52); staff.arc(7, -52, 7, Math.PI, 0); staff.lineTo(14, -46);
  c.lineCap = "round"; c.lineJoin = "round"; c.strokeStyle = INK; c.lineWidth = 8; c.stroke(staff); c.strokeStyle = GOLD[1]; c.lineWidth = 4; c.stroke(staff);
  c.restore();
  const hx = 27, hy = -48 - raise * 12;
  c.lineCap = "round"; c.beginPath(); c.moveTo(8, -60 + breath); c.lineTo(hx - 2, hy + 1);
  c.strokeStyle = INK; c.lineWidth = 13; c.stroke(); c.strokeStyle = ROBE[0]; c.lineWidth = 8.5; c.stroke();
  c.fillStyle = SKIN[1]; c.strokeStyle = INK; c.lineWidth = 2; c.beginPath(); c.arc(hx, hy, 3.6, 0, TAU); c.fill(); c.stroke();
  c.restore();
}

// a speech bubble over a head: "!" or "✓" in a tab of colour, or z's rising
function bubble(c, x, y, kind, col, t, sz = 1) {
  c.save();
  if (kind === "z") {
    c.fillStyle = "#dfe4f6";
    for (let i = 0; i < 3; i++) { const p = (t * .45 + i / 3) % 1; c.globalAlpha = Math.sin(p * Math.PI); c.font = `600 ${(9 + p * 6) * sz}px 'JetBrains Mono', monospace`; c.fillText("z", x + p * 10 * sz, y - p * 22 * sz); }
  } else {
    c.translate(x, y + Math.sin(t * 6) * 1.5); c.scale(sz, sz);
    c.fillStyle = col; c.strokeStyle = INK; c.lineWidth = 2.5;
    c.beginPath(); c.roundRect(-9, -20, 18, 18, 4); c.moveTo(-3, -2.5); c.lineTo(0, 3); c.lineTo(3, -2.5); c.fill(); c.stroke();
    c.fillStyle = INK; c.font = "700 12px 'JetBrains Mono', monospace"; c.textAlign = "center"; c.fillText(kind, 0, -6.5);
  }
  c.restore();
}

// ---------- the land
const STARS = (() => { const r = rng(9); return Array.from({ length: 150 }, () => [r(), r(), r() * 1.2 + .3, r() * TAU]); })();
function halo(c, x, y, r, rgb, a, spread = 6) { const h = c.createRadialGradient(x, y, r * .8, x, y, r * spread); h.addColorStop(0, `rgba(${rgb},${a})`); h.addColorStop(1, `rgba(${rgb},0)`); c.fillStyle = h; c.fillRect(x - r * spread, y - r * spread, r * spread * 2, r * spread * 2); }
function stars(c, W, horizon, t, a) { c.fillStyle = "#dfe4f6"; for (const [sx, sy, sr, sp] of STARS) { c.globalAlpha = a * (.35 + .35 * Math.sin(t * 1.3 + sp)); c.beginPath(); c.arc(sx * W, sy * horizon * .92, sr * .75, 0, TAU); c.fill(); } c.globalAlpha = 1; }
function moon(c, mx, my, mr) { halo(c, mx, my, mr, "223,228,246", .2, 7); c.fillStyle = "#eef0fa"; c.beginPath(); c.arc(mx, my, mr, 0, TAU); c.fill(); c.fillStyle = "#d1d6ec"; c.beginPath(); c.arc(mx - mr * .3, my - mr * .22, mr * .22, 0, TAU); c.arc(mx + mr * .36, my + mr * .28, mr * .14, 0, TAU); c.arc(mx + mr * .1, my + mr * .5, mr * .1, 0, TAU); c.fill(); }
function sun(c, sx, sy, sr) { halo(c, sx, sy, sr, "245,197,66", .5, 9); c.fillStyle = "#f8d76c"; c.beginPath(); c.arc(sx, sy, sr, 0, TAU); c.fill(); }
function sky(c, W, H, t, o) {
  const g = c.createLinearGradient(0, 0, 0, o.horizon);
  o.stops.forEach(([k, col]) => g.addColorStop(k, col));
  c.fillStyle = g; c.fillRect(0, 0, W, H);
  if (o.stars > 0) stars(c, W, o.horizon, t, o.stars);
  if (o.moon) moon(c, ...o.moon);
  if (o.sun) sun(c, ...o.sun);
}
// a hill line from a profile; the rim is the light catching its top edge
function ridge(c, W, H, fy, fill, rim) {
  const top = new Path2D(); top.moveTo(-10, fy(-10));
  for (let x = -2; x <= W + 10; x += 8) top.lineTo(x, fy(x));
  const body = new Path2D(top); body.lineTo(W + 10, H + 10); body.lineTo(-10, H + 10); body.closePath();
  if (rim) { c.save(); c.translate(0, -1.6); c.fillStyle = rim; c.fill(body); c.restore(); }
  c.fillStyle = fill; c.fill(body);
}
// flat-topped hills, the koppies across the valley
function mesas(W, seed, base, lo, hi, shift = 0) {
  const r = rng(seed), pts = []; let x = -300;
  pts.push([x, base]);
  while (x < W + 400) { const h = lo + r() * (hi - lo); x += 30 + r() * 80; pts.push([x, base - r() * 12]); x += 20 + r() * 16; pts.push([x, base - h]); x += 50 + r() * 150; pts.push([x, base - h - (r() - .5) * 6]); x += 20 + r() * 18; pts.push([x, base - r() * 10]); }
  return (xq) => { xq -= shift; for (let i = 1; i < pts.length; i++) if (pts[i][0] >= xq) { const [x0, y0] = pts[i - 1], [x1, y1] = pts[i]; return lerp(y0, y1, (xq - x0) / (x1 - x0 || 1)); } return base; };
}
function acacia(c, x, y, k, fill, rim) {
  c.save(); c.translate(x, y); c.scale(k, k);
  const draw = (col) => {
    c.strokeStyle = col; c.fillStyle = col; c.lineCap = "round";
    c.lineWidth = 7; c.beginPath(); c.moveTo(0, 2); c.quadraticCurveTo(3, -30, -6, -52); c.stroke();
    c.lineWidth = 5; c.beginPath(); c.moveTo(1, -24); c.quadraticCurveTo(15, -40, 24, -55); c.stroke();
    c.lineWidth = 3.5; c.beginPath(); c.moveTo(-3, -40); c.quadraticCurveTo(-20, -48, -32, -57); c.stroke();
    const can = new Path2D(); ellipse(can, -4, -61, 64, 11, 0); ellipse(can, -34, -58, 30, 8, -.05); ellipse(can, 30, -59, 34, 9, .04); ellipse(can, 2, -67, 40, 8, 0);
    c.fill(can);
  };
  if (rim) { c.save(); c.translate(0, -1.8); draw(rim); c.restore(); }
  draw(fill);
  c.restore();
}
function groundBand(c, W, H, top, t, cols = ["#1b2039", "#0f1224"], rim = "rgba(223,228,246,.16)", blade = "rgba(223,228,246,.07)") {
  const fy = (x) => top + Math.sin(x / W * 3.2 + .6) * 7 + Math.sin(x / W * 9) * 3;
  const g = c.createLinearGradient(0, top, 0, H); g.addColorStop(0, cols[0]); g.addColorStop(1, cols[1]);
  ridge(c, W, H, fy, g, rim);
  c.strokeStyle = blade; c.lineWidth = 1.2; c.lineCap = "round";
  const r = rng(3);
  for (let i = 0; i < W / 7; i++) {
    const gx = r() * W, gy = top + 10 + r() * (H - top - 6), h = 3 + (gy - top) / (H - top) * 7;
    const lean = Math.sin(t * 1.4 - gx / 120) * 2.4;
    c.beginPath(); c.moveTo(gx, gy); c.lineTo(gx + lean, gy - h); c.moveTo(gx + 2.5, gy); c.lineTo(gx + 2.5 + lean * 1.2, gy - h * .8); c.stroke();
  }
  c.strokeStyle = blade; c.lineWidth = 1.6;
  for (let i = 0; i < 3; i++) { const p = ((t * .07 + i * .37) % 1.3) - .15; const gx = p * W, gy = top + (i + 1) * (H - top) / 4.4; c.beginPath(); c.moveTo(gx, gy); c.quadraticCurveTo(gx + 70, gy - 7, gx + 160, gy - 2); c.stroke(); }
}

// ---------- scenes: each draws in its own units, scaled to its canvas, and only runs while it can be seen
const scenes = [];
function render(sc) { if (!sc.W) return; const { c } = sc; c.setTransform(sc.dpr * sc.s, 0, 0, sc.dpr * sc.s, 0, 0); c.clearRect(0, 0, sc.W, sc.h); sc.draw(c, sc.W, sc.h, sc.t); }
function scene(canvas, o) {
  const sc = Object.assign({ canvas, c: canvas.getContext("2d"), W: 0, s: 1, t: 0, on: true }, o);
  sc.fit = () => {
    const cw = canvas.clientWidth, dpr = Math.min(2, devicePixelRatio || 1);
    if (!cw) return;
    if (o.fit === "height") sc.s = (canvas.clientHeight || o.h) / o.h;
    else { sc.s = clamp(cw / 1000, o.minS ?? .5, o.maxS ?? 1); canvas.style.height = o.h * sc.s + "px"; }
    const old = sc.W; sc.W = cw / sc.s; sc.dpr = dpr;
    canvas.width = Math.round(cw * dpr); canvas.height = Math.round(o.h * sc.s * dpr);
    if (sc.resize) sc.resize(old, sc.W);
    if (sc.still || reduce) render(sc);
  };
  sc.fit();
  new ResizeObserver(() => sc.fit()).observe(canvas);
  new IntersectionObserver(([e]) => { sc.on = e.isIntersecting; }, { rootMargin: "120px" }).observe(canvas);
  scenes.push(sc);
  return sc;
}

// a sheep that walks where it's told
function makeSheep(o) { return Object.assign({ x: 0, d: .5, tx: null, face: 1, stride: 0, walk: 0, run: 0, head: 0, hop: 0, lie: 0, chew: 0, tail: 0, seed: Math.random() * 10, timer: 0, mood: "working" }, o); }
function steer(s, dt, speed, run) {
  if (s.tx == null) { s.walk = ease(s.walk, 0, dt * 6); return true; }
  const dx = s.tx - s.x;
  if (Math.abs(dx) < 1.5) { s.walk = ease(s.walk, 0, dt * 6); s.run = ease(s.run, 0, dt * 4); return true; }
  const v = Math.sign(dx) * Math.min(Math.abs(dx) / dt, speed);
  s.x += v * dt; s.face = Math.sign(dx); s.walk = ease(s.walk, 1, dt * 6); s.run = ease(s.run, run ? 1 : 0, dt * 4);
  s.stride += Math.abs(v) * dt * (run ? .11 : .16);
  return false;
}
const hopCycle = (s, dt) => { s.hopClock = (s.hopClock || 0) + dt; const p = s.hopClock % .95; s.hop = p < .62 ? p / .62 : 0; };
// a working sheep grazes, then wanders to new grass between lo and hi
function graze(s, dt, t, lo, hi, speed) {
  if (steer(s, dt, speed, false)) {
    s.head = ease(s.head, 1, dt * 2.5); s.chew = Math.max(0, Math.sin(t * 9 + s.seed));
    if ((s.timer -= dt) <= 0) { s.tx = lo + Math.random() * (hi - lo); s.timer = 3 + Math.random() * 4; }
    if (s.tx != null && Math.abs(s.tx - s.x) < 1.5) s.tx = null;
  } else s.head = ease(s.head, .1, dt * 4);
}
const rescale = (flock) => (old, W) => flock.forEach((s) => { const f = W / (old || 1000); s.x *= f; if (s.tx != null) s.tx *= f; });

// ---------- a pasture: the shepherd, a flock, and a script of who needs you when
function pasture(canvas, o) {
  const H = o.h, flock = o.flock.map(makeSheep), herd = { x: o.herdX, d: .6, raise: 0 };
  const top = H * o.top;
  let k0 = o.k || 1;
  const gy = (d) => top + 30 * k0 + d * (H - top - 56 * k0);
  const kAt = (d) => (.62 + d * .38) * k0;
  const byName = (n) => flock.find((s) => s.name === n);
  const toastEl = o.toast, toastText = toastEl?.querySelector(".t-text"), toastGlyph = toastEl?.querySelector(".t-glyph");
  let clock = 0, next = 0, toastTimer = 0;
  function setMood(s, mood) {
    s.mood = mood; s.timer = 0; s.hopClock = 0;
    const others = flock.filter((x) => x !== s && x.mood === mood).length;
    s.tx = mood === "needs" ? herd.x + 70 * k0 + others * 50 * k0 : mood === "done" ? herd.x + 140 * k0 + others * 58 * k0 : null;
  }
  function step(dt, W) {
    clock += dt;
    while (next < o.script.length && clock >= o.script[next][0]) {
      const [, who, mood, line] = o.script[next++];
      if (!who) { clock = 0; next = 0; break; }
      setMood(byName(who), mood);
      if (line && toastEl) { toastText.textContent = line; toastGlyph.textContent = mood === "done" ? "✓" : "!"; toastEl.classList.toggle("done", mood === "done"); toastEl.classList.add("show"); toastTimer = 3.6; }
    }
    if (toastTimer > 0 && (toastTimer -= dt) <= 0) toastEl.classList.remove("show");
    const g0 = W * o.grass[0], g1 = W * o.grass[1];
    for (const s of flock) {
      if (s.mood === "idle") { s.lie = ease(s.lie, 1, dt * 2); s.head = ease(s.head, .5, dt * 2); steer(s, dt, 0); continue; }
      s.lie = ease(s.lie, 0, dt * 3);
      if (s.mood === "working") {
        if (steer(s, dt, 34 * k0, false)) {
          s.head = ease(s.head, 1, dt * 2.5); s.chew = Math.max(0, Math.sin(clock * 9 + s.seed)) * s.head;
          if ((s.timer -= dt) <= 0) {
            let tx; for (let i = 0; i < 8; i++) { tx = g0 + Math.random() * (g1 - g0); if (flock.every((x) => x === s || Math.abs((x.tx ?? x.x) - tx) > 70 * k0 || Math.abs(x.d - s.d) > .3)) break; }
            s.tx = tx; s.timer = 3 + Math.random() * 4;
          }
          if (s.tx != null && Math.abs(s.tx - s.x) < 1.5) s.tx = null;
        } else s.head = ease(s.head, .1, dt * 4);
      } else if (s.mood === "needs") {
        if (steer(s, dt, 150 * k0, true)) { s.face = -1; s.head = ease(s.head, -.4, dt * 5); hopCycle(s, dt); }
        else { s.hop = 0; s.head = ease(s.head, -.1, dt * 5); }
      } else if (s.mood === "done") {
        if (steer(s, dt, 40 * k0, false)) { s.face = -1; s.head = ease(s.head, -.05, dt * 3); }
        else s.head = ease(s.head, .1, dt * 4);
      }
      if (s.mood !== "needs") s.hop = 0;
      s.tail = Math.sin(clock * 20 + s.seed) * (Math.sin(clock * .7 + s.seed) > .8 ? 1 : 0);
    }
    herd.raise = ease(herd.raise, flock.some((s) => s.mood === "needs" && s.hop > 0) ? 1 : 0, dt * 5);
  }
  function draw(c, W, Hh, t) {
    o.backdrop(c, W, H, t, top);
    const tag = (text, x, y, col) => { c.font = `500 ${10 * k0 + 1}px 'JetBrains Mono', monospace`; c.textAlign = "center"; c.fillStyle = col; c.globalAlpha = .9; c.fillText(text, x, y + 14 * k0 + 2); c.globalAlpha = 1; };
    const items = flock.map((s) => ({ d: s.d, draw: () => {
      const y = gy(s.d), k = kAt(s.d);
      sheep(c, s.x, y, k, s.face, s, t, s.collar);
      const lift = s.hop > 0 ? Math.sin(s.hop * Math.PI) * 30 : 0;
      if (s.mood === "needs" || s.mood === "done") bubble(c, s.x + s.face * 24 * k, y - (60 + lift) * k, s.mood === "needs" ? "!" : "✓", s.mood === "needs" ? "#f5c542" : "#dfe4f6", t, k0);
      if (s.mood === "idle") bubble(c, s.x + 22 * k, y - 46 * k, "z", "", t, k0);
      tag(s.name, s.x, y, s.collar);
    } }));
    items.push({ d: herd.d, draw: () => { const y = gy(herd.d); shepherd(c, herd.x, y, 1.12 * k0, 1, herd, t); tag("modisa", herd.x + 4, y, "#f5c542"); } });
    items.sort((a, b) => a.d - b.d).forEach((i) => i.draw());
  }
  const sc = scene(canvas, { h: H, fit: "height", step, draw });
  const spread = rescale(flock);
  sc.resize = (old, W) => { spread(old, W); k0 = (o.k || 1) * clamp(W / 900, .62, 1); herd.x = o.herdX * Math.max(k0, .8); };
  sc.resize(1000, sc.W);
  if (reduce) { for (let i = 0; i < 330; i++) { sc.t += 1 / 60; step(1 / 60, sc.W); } render(sc); }
  return sc;
}

// ===== the home page's pasture: sky, the koppies, the hill with its acacia, the flock, the grass in front
const hero = $(".hero");
if (hero) {
  scene($(".ly-sky"), { h: 640, fit: "height", draw: (c, W, H, t) => sky(c, W, H, t, { stops: [[0, "#06070f"], [.55, "#10142b"], [1, "#272c4d"]], horizon: H * .8, stars: 1, moon: W / H > .9 ? [W * .8, H * .27, 24] : null }) });
  scene($(".ly-far"), { h: 240, fit: "height", still: true, draw: (c, W, H) => {
    ridge(c, W, H, mesas(W, 4, 200, 40, 110), "#20263f", "rgba(223,228,246,.14)");
    ridge(c, W, H, mesas(W, 14, 236, 20, 60, 140), "#1b2038", "rgba(223,228,246,.1)");
  } });
  scene($(".ly-mid"), { h: 230, fit: "height", still: true, draw: (c, W, H) => {
    const fy = (x) => 150 - 70 * Math.exp(-(((x - W * .92) / (W * .13)) ** 2)) - 12 * Math.sin(x / W * 6 + 1);
    ridge(c, W, H, fy, "#161a2f", "rgba(223,228,246,.17)");
    acacia(c, W * .92, fy(W * .92) + 5, 1.25, "#161a2f", "rgba(223,228,246,.22)");
    acacia(c, W * .07, fy(W * .07) + 4, .6, "#161a2f", "rgba(223,228,246,.18)");
  } });
  pasture($("#pasture"), {
    h: 300, top: .2, herdX: 120, grass: [.36, .93], toast: $("#toast"),
    backdrop: (c, W, H, t, top) => groundBand(c, W, H, top, t),
    flock: [
      { name: "@coder", collar: "#d97757", x: 520, d: .55 },
      { name: "@reviewer", collar: "#c0caf5", x: 700, d: .8 },
      { name: "@docs", collar: "#bb9af7", x: 860, d: .35 },
      { name: "@tests", collar: "#7aa2f7", x: 400, d: .95 },
      { name: "@notes", collar: "#e0af68", x: 935, d: .08, mood: "idle", lie: 1, head: .5 },
    ],
    script: [
      [2.5, "@reviewer", "needs", "@reviewer needs you · codex wants to run git push"],
      [9.5, "@reviewer", "working"],
      [11, "@docs", "done", "@docs is done · migration notes written"],
      [16.5, "@tests", "needs", "@tests needs you · gemini has a question"],
      [21.5, "@tests", "working"],
      [23, "@docs", "working"],
      [26, null],
    ],
  });
  scene($(".ly-fg"), { h: 96, fit: "height", still: true, draw: (c, W, H) => {
    const r = rng(17);
    c.fillStyle = "#070912";
    for (let i = 0; i < W / 4; i++) {
      const x = r() * W, h = 18 + r() * r() * 70, lean = (r() - .5) * 26;
      c.beginPath(); c.moveTo(x - 3, H); c.quadraticCurveTo(x + lean * .3, H - h * .6, x + lean, H - h); c.quadraticCurveTo(x + lean * .2 + 2, H - h * .5, x + 3, H); c.fill();
    }
    for (const px of [W * .035, W * .975]) {
      c.fillStyle = "rgba(223,228,246,.12)"; c.fillRect(px - 8, H - 92, 16, 92);
      c.fillStyle = "#070912"; c.fillRect(px - 8, H - 90.5, 16, 92);
    }
    c.strokeStyle = "#070912"; c.lineWidth = 2;
    c.beginPath(); c.moveTo(W * .035, H - 70); c.quadraticCurveTo(W * .035 + 80, H - 60, W * .035 + 170, H - 66); c.stroke();
    c.beginPath(); c.moveTo(W * .975, H - 58); c.quadraticCurveTo(W * .975 - 90, H - 48, W * .975 - 180, H - 54); c.stroke();
    c.fillRect(0, H - 10, W, 10);
  } });
}

// ===== counting sheep over a fence, one per agent modisa knows
const counter = $("#count");
if (counter) {
  const names = counter.dataset.agents.split("|"), H = 150, GY = 128, nEl = $("#count-n"), nameEl = $("#count-name");
  const jumpers = []; let spawn = 0, n = 0;
  const sc = scene(counter, { h: H, minS: 1, maxS: 1.2, step: (dt, W) => {
    if ((spawn -= dt) <= 0) { jumpers.push(makeSheep({ hopH: 58, x: -50, name: names[n % names.length], collar: COLLARS[n % COLLARS.length], i: n % names.length })); n++; spawn = 1.7; }
    const fence = W / 2;
    for (const s of jumpers) {
      s.x += 95 * dt; s.walk = 1; s.run = .5; s.stride += 95 * dt * .14;
      const p = (s.x - (fence - 58)) / 116; s.hop = p > 0 && p < 1 ? p : 0;
      if (!s.counted && s.x > fence) { s.counted = true; nEl.textContent = String(s.i + 1).padStart(2, "0"); nameEl.textContent = s.name; }
    }
    while (jumpers.length && jumpers[0].x > W + 60) jumpers.shift();
  }, draw: (c, W, Hh, t) => {
    const fence = W / 2;
    c.strokeStyle = "rgba(241,232,212,.35)"; c.lineWidth = 1.5; c.beginPath(); c.moveTo(0, GY + .5); c.lineTo(W, GY + .5); c.stroke();
    const r = rng(11); c.strokeStyle = "rgba(241,232,212,.18)"; c.lineWidth = 1;
    for (let i = 0; i < W / 14; i++) { const x = r() * W; c.beginPath(); c.moveTo(x, GY); c.lineTo(x - 2, GY - 4 - r() * 3); c.moveTo(x + 2, GY); c.lineTo(x + 3, GY - 3 - r() * 3); c.stroke(); }
    for (const px of [fence - 26, fence + 26]) { const post = new Path2D(); post.rect(px - 4, GY - 38, 8, 38); ink(c, post, 4); paint(c, post, WOOD, px - 4, GY - 38, 8, 38); }
    for (const ry of [GY - 33, GY - 21, GY - 9]) { const rail = new Path2D(); rail.rect(fence - 40, ry, 80, 6); ink(c, rail, 4); paint(c, rail, WOOD, fence - 40, ry, 80, 6); }
    for (const s of jumpers) {
      sheep(c, s.x, GY, .82, 1, s, t, s.collar);
      const lift = s.hop > 0 ? Math.sin(s.hop * Math.PI) * s.hopH * .82 : 0;
      c.font = "500 11px 'JetBrains Mono', monospace"; c.textAlign = "center"; c.fillStyle = "#f1e8d4"; c.globalAlpha = s.hop > 0 ? 1 : .5;
      c.fillText(s.name, s.x, GY - 62 - lift); c.globalAlpha = 1;
    }
  } });
  if (reduce) { for (let i = 0; i < 260; i++) sc.step(1 / 60, sc.W); render(sc); }
}

// ===== one little scene per agent state
for (const cv of $$("canvas[data-mini]")) {
  const mood = cv.dataset.mini, s = makeSheep({ face: mood === "working" ? 1 : -1, lie: mood === "idle" ? 1 : 0, head: mood === "idle" ? .5 : 0 });
  const herd = { raise: 0 }, GY = 132;
  const sc = scene(cv, { h: 156, fit: "height", step: (dt, W) => {
    if (mood === "needs") { s.x = W * .62; hopCycle(s, dt); s.head = -.4; herd.raise = ease(herd.raise, s.hop > 0 ? 1 : 0, dt * 5); }
    if (mood === "working") { if (!s.x) s.x = W * .4; graze(s, dt, sc.t, W * .3, W * .7, 26); }
    if (mood === "done") { s.x = W * .62; s.head = -.05; }
    if (mood === "idle") s.x = W * .5;
  }, draw: (c, W, H, t) => {
    c.strokeStyle = "rgba(223,228,246,.22)"; c.lineWidth = 1.2; c.beginPath(); c.moveTo(0, GY + .5); c.lineTo(W, GY + .5); c.stroke();
    if (mood === "needs" || mood === "done") shepherd(c, W * .22, GY, .92, 1, herd, t);
    sheep(c, s.x, GY, .92, s.face, s, t, { needs: "#c0caf5", working: "#d97757", done: "#bb9af7", idle: "#e0af68" }[mood]);
    const lift = s.hop > 0 ? Math.sin(s.hop * Math.PI) * 30 : 0;
    if (mood === "needs") bubble(c, s.x - 22, GY - (60 + lift) * .92, "!", "#f5c542", t);
    if (mood === "done") bubble(c, s.x - 22, GY - 55, "✓", "#dfe4f6", t);
    if (mood === "idle") bubble(c, s.x + 22, GY - 42, "z", "", t);
  } });
  if (reduce) { for (let i = 0; i < 40; i++) { sc.t += 1 / 60; sc.step(1 / 60, sc.W); } render(sc); }
}
if ($("#deco-hills")) scene($("#deco-hills"), { h: 320, fit: "height", still: true, draw: (c, W, H) => {
  ridge(c, W, H, mesas(W, 31, 210, 30, 90), "#121630", "rgba(223,228,246,.07)");
  ridge(c, W, H, (x) => 270 - 26 * Math.sin(x / W * 4 + 2), "#0f1224", "rgba(223,228,246,.08)");
} });
if ($("#deco-acacia")) scene($("#deco-acacia"), { h: 420, fit: "height", still: true, draw: (c, W, H) => acacia(c, W * .55, H, 3.2, "#131730", "rgba(223,228,246,.08)") });

// the scroll positions the scrubbed scenes read
let nightP = .5, gateP = reduce ? .62 : 0, dawnP = reduce ? .85 : 0;

// ===== through the night: the moon crosses as you scroll; the flock grazes on
const nightCanvas = $("#night-canvas");
if (nightCanvas) {
  const H = 380, flock = [
    makeSheep({ x: 230, d: .3, collar: "#d97757", head: 1 }), makeSheep({ x: 520, d: .85, collar: "#c0caf5", head: 1, face: -1 }),
    makeSheep({ x: 760, d: .55, collar: "#bb9af7", head: 1 }), makeSheep({ x: 900, d: .15, collar: "#e0af68", lie: 1, head: .5, mood: "idle" }),
  ];
  const sc = scene(nightCanvas, { h: H, minS: .6, maxS: 1.5, step: (dt) => {
    for (const s of flock) if (s.mood !== "idle") { s.timer = s.timer ?? 0; graze(s, dt, sc.t, 120, sc.W - 80, 22); }
  }, draw: (c, W, Hh, t) => {
    const p = nightP;
    sky(c, W, H, t, { stops: [[0, "#0f1224"], [.5, "#10142b"], [1, "#2b2f55"]], horizon: H * .58, stars: 1, moon: [W * lerp(.88, .12, p), H * (.34 - Math.sin(p * Math.PI) * .16), 22] });
    ridge(c, W, H, mesas(W, 6, H * .55, 20, 70, (p - .5) * 120), "#1e2440", "rgba(223,228,246,.13)");
    groundBand(c, W, H, H * .62, t);
    [...flock].sort((a, b) => a.d - b.d).forEach((s) => {
      const y = H * .7 + s.d * H * .22, k = .75 + s.d * .35;
      sheep(c, s.x, y, k, s.face, s, t, s.collar);
      if (s.mood === "idle") bubble(c, s.x + 22 * k, y - 46 * k, "z", "", t);
    });
  } });
  sc.resize = rescale(flock);
  sc.resize(1000, sc.W);
}

// ===== the notch: its ears, and the pasture it opens into
const notch = $("#notch-btn");
if (notch) {
  const pet = makeSheep({ face: -1 });
  scene($("#ear-l"), { h: 30, fit: "height", draw: (c, W, H, t) => shepherd(c, W * .5, H - 3, .22, 1, { raise: pet.hop > 0 ? 1 : 0 }, t) });
  scene($("#ear-r"), { h: 30, fit: "height", step: (dt) => hopCycle(pet, dt), draw: (c, W, H, t) => sheep(c, W * .55, H - 3, .3, -1, pet, t, "#c0caf5") });
  const inside = pasture($("#notch-pasture"), {
    h: 226, top: .42, herdX: 70, grass: [.38, .9], k: .72,
    backdrop: (c, W, H, t, top) => {
      sky(c, W, H, t, { stops: [[0, "#000"], [.5, "#0b0e20"], [1, "#22284a"]], horizon: top + 8, stars: 1, moon: [W * .84, H * .2, 12] });
      ridge(c, W, H, mesas(W, 8, top + 4, 10, 34), "#1b2039", "rgba(223,228,246,.12)");
      groundBand(c, W, H, top, t);
    },
    flock: [
      { name: "@api", collar: "#d97757", x: 560, d: .5 },
      { name: "@web", collar: "#7aa2f7", x: 760, d: .85 },
      { name: "@infra", collar: "#bb9af7", x: 880, d: .2 },
    ],
    script: [[1.4, "@api", "needs"], [6.5, "@api", "working"], [7.5, "@web", "done"], [12.5, "@web", "working"], [13.5, "@infra", "needs"], [18, "@infra", "working"], [20, null]],
  });
  let pinned = false;
  const set = (open) => { inside.held = open; notch.classList.toggle("open", open); notch.setAttribute("aria-expanded", open); notch.setAttribute("aria-label", open ? "Close the flock in the notch" : "Open the flock in the notch"); };
  notch.addEventListener("click", () => { pinned = !notch.classList.contains("open"); set(pinned); });
  if (fine) { notch.addEventListener("pointerenter", () => set(true)); notch.addEventListener("pointerleave", () => { if (!pinned) set(false); }); }
  new IntersectionObserver(([e], io) => { if (e.isIntersecting) { io.disconnect(); setTimeout(() => { set(true); setTimeout(() => { if (!pinned && !notch.matches(":hover")) set(false); }, 6500); }, 1200); } }, { threshold: .6 }).observe(notch);
}

// ===== first light: the gate swings open as you scroll and the flock walks out of the kraal
const gateCanvas = $("#gate-canvas");
if (gateCanvas) {
  const H = 340, flock = [0, 1, 2, 3, 4, 5].map((i) => makeSheep({ i, collar: COLLARS[i], seed: i * 1.7, dy: [6, -10, 12, -4, 2, -12][i] }));
  const herd = { raise: 0 };
  scene(gateCanvas, { h: H, minS: .55, maxS: 1.35, draw: (c, W, Hh, t) => {
    const p = gateP, open = clamp((p - .15) / .2);
    sky(c, W, H, t, { stops: [[0, "#0f1224"], [.5, "#262450"], [1, "#7c4f60"]], horizon: H * .64, stars: .45 });
    halo(c, W * .78, H * .64, 40, "240,163,94", .32, 8);
    ridge(c, W, H, mesas(W, 8, H * .62, 16, 56, (p - .5) * -90), "#16183a", "rgba(240,163,94,.3)");
    groundBand(c, W, H, H * .66, t, ["#191b3a", "#0f1224"]);
    const gx = W * .5, GY = H * .8, hinge = [gx - 30, GY + 22], back = [gx + 30, GY - 18];
    const post = (x, y, h, w) => { const pp = new Path2D(); pp.rect(x - w / 2, y - h, w, h); ink(c, pp, 3.5); paint(c, pp, WOOD, x - w / 2, y - h, w, h); };
    const rails = (a, b, hA, hB, w) => { for (const f of [.3, .7]) { c.beginPath(); c.moveTo(a[0], a[1] - hA * f); c.lineTo(b[0], b[1] - hB * f); c.lineCap = "round"; c.strokeStyle = INK; c.lineWidth = w + 3; c.stroke(); c.strokeStyle = WOOD[1]; c.lineWidth = w; c.stroke(); } };
    const far = [gx - W * .42, GY - 34];
    rails(far, back, 34, 44, 2.5); post(far[0], far[1], 34, 5); post((far[0] + back[0]) / 2, (far[1] + back[1]) / 2, 39, 6); post(back[0], back[1], 46, 7);
    flock.map((s) => {
      const q = clamp((p - .3 - s.i * .05) / .3), x = lerp(gx - 120 - s.i * 64, W + 70, q), moving = q > 0 && q < 1;
      s.walk = moving ? 1 : 0; s.stride = x * .16; s.head = moving ? .1 : (Math.sin(t * .6 + s.i) > .2 ? 1 : .2); s.chew = moving ? 0 : Math.max(0, Math.sin(t * 9 + s.i));
      return { y: GY + s.dy * (1 - q * .5), x, s };
    }).sort((a, b) => a.y - b.y).forEach(({ x, y, s }) => sheep(c, x, y, .9, 1, s, t, s.collar));
    // the gate, hung on the front post, swinging out toward you
    const va = Math.atan2(back[1] - hinge[1], back[0] - hinge[0]), vb = Math.atan2(6, 118), la = Math.hypot(back[0] - hinge[0], back[1] - hinge[1]);
    const a = lerp(va, vb, open), len = lerp(la, 118, open), end = [hinge[0] + Math.cos(a) * len, hinge[1] + Math.sin(a) * len];
    const hH = 56, eH = lerp(44, 58, open);
    const bar = (x1, y1, x2, y2, w) => { c.beginPath(); c.moveTo(x1, y1); c.lineTo(x2, y2); c.lineCap = "round"; c.strokeStyle = INK; c.lineWidth = w + 3.5; c.stroke(); c.strokeStyle = WOOD[0]; c.lineWidth = w; c.stroke(); };
    for (const f of [.12, .4, .68, .94]) bar(hinge[0], hinge[1] - 4 - hH * f, end[0], end[1] - 4 - eH * f, 4);
    bar(hinge[0], hinge[1] - 6, end[0], end[1] - eH * .9, 3.5);
    bar(end[0], end[1] - 2, end[0], end[1] - eH - 4, 4.5);
    post(hinge[0], hinge[1] + 4, 72, 11);
    rails(hinge, [hinge[0] - 150, hinge[1] + 90], 66, 96, 3.5);
    post(hinge[0] - 110, hinge[1] + 66, 88, 12);
    herd.raise = clamp((p - .3) / .08) * (1 - clamp((p - .72) / .1));
    shepherd(c, gx + 175, GY + 42, 1.15, -1, herd, t);
  } });
}

// ===== dawn: the sun comes up as you reach the bottom, and the flock walks the ridge
const dawnCanvas = $("#dawn-canvas");
if (dawnCanvas) {
  const H = 440;
  scene(dawnCanvas, { h: H, minS: .7, maxS: 1.6, draw: (c, W, Hh, t) => {
    const q = dawnP;
    sky(c, W, H, t, { stops: [[0, "#0f1224"], [.42, "#3d3262"], [.78, "#b9655a"], [1, "#f0a35e"]], horizon: H * .78, stars: .4 * (1 - q), sun: [W * .5, lerp(H * .9, H * .6, q), 36] });
    ridge(c, W, H, mesas(W, 12, H * .78, 24, 70, (q - .5) * -60), "#3a2440", "rgba(245,197,66,.45)");
    const mid = (x) => H * .84 - 30 * Math.exp(-(((x - W * .22) / (W * .2)) ** 2)) - 8 * Math.sin(x / W * 5 + (q - .5) * 1.4);
    ridge(c, W, H, mid, "#22172e", "rgba(245,197,66,.4)");
    acacia(c, W * .22, mid(W * .22) + 6, .95, "#22172e", "rgba(245,197,66,.45)");
    const near = (x) => H * .92 + Math.sin(x / W * 4) * 6;
    ridge(c, W, H, near, "#0f1224", "rgba(245,197,66,.35)");
    const span = W + 900, at = (d) => (((t * 44 - d) % span) + span) % span - 300;
    for (let i = 0; i < 7; i++) { const x = at(i * 88 + (i % 2) * 16); sheep(c, x, near(x) + 2, .85, 1, { stride: t * 44 * .16 + i * 1.3, walk: 1, head: Math.sin(t * .8 + i) > .85 ? .7 : .05, seed: i }, t, COLLARS[i]); }
    const hx = at(7 * 88 + 50); shepherd(c, hx, near(hx) + 2 - Math.abs(Math.sin(t * 3.2)) * 1.2, 1, 1, { raise: 0 }, t);
  } });
}

// ===== the docs: a strip of pasture over every page, in the colours of the page's theme
const look = { night: true, hill: "#1a1f36", rim: "rgba(223,228,246,.16)", blade: "rgba(223,228,246,.08)", ground: "#0f1224" };
const readLook = () => {
  const css = getComputedStyle(document.documentElement), v = (n) => css.getPropertyValue(n).trim();
  Object.assign(look, { night: document.documentElement.dataset.theme !== "day", hill: v("--hill") || look.hill, rim: v("--hill-rim") || look.rim, blade: v("--hill-blade") || look.blade, ground: v("--bg") || look.ground });
};
for (const cv of $$('canvas[data-flock="band"]')) {
  readLook();
  const seed = hash(cv.dataset.seed || "modisa"), r = rng(seed), H = 120;
  const flock = [0, 1, 2].map((i) => makeSheep({ x: 380 + i * 210 + r() * 90, d: r(), collar: COLLARS[(seed + i * 3) % COLLARS.length], head: 1, timer: r() * 4 }));
  const herd = { raise: 0 };
  const sc = scene(cv, { h: H, minS: .8, maxS: 1.15, step: (dt, W) => { for (const s of flock) graze(s, dt, sc.t, W * .3, W * .82, 18); }, draw: (c, W, Hh, t) => {
    if (look.night) { stars(c, W, 70, t, .7); moon(c, W * .88, 30, 13); } else sun(c, W * .88, 34, 15);
    ridge(c, W, H, mesas(W, seed % 997 + 3, 80, 12, 40), look.hill, look.rim);
    groundBand(c, W, H, 92, t, [look.ground, look.ground], look.rim, look.blade);
    shepherd(c, W * .14, 108, .55, 1, herd, t);
    [...flock].sort((a, b) => a.d - b.d).forEach((s) => sheep(c, s.x, 104 + s.d * 10, .48 + s.d * .1, s.face, s, t, s.collar));
  } });
  sc.resize = rescale(flock);
  sc.resize(1000, sc.W);
}
new MutationObserver(() => { readLook(); scenes.forEach((sc) => (sc.still || reduce) && render(sc)); }).observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });

// ===== the 404: one that wandered off, alone in the pasture, looking about
const strayCanvas = $('canvas[data-flock="stray"]');
if (strayCanvas) {
  const H = 260, s = makeSheep({ x: 300, collar: "#7aa2f7" });
  let pause = 1.5;
  const sc = scene(strayCanvas, { h: H, minS: .75, maxS: 1.2, step: (dt) => {
    if (steer(s, dt, 30, false)) {
      s.tx = null; pause -= dt;
      s.head = ease(s.head, pause > 1.6 ? -.35 : 1, dt * 3);
      if (pause > 1.6 && Math.sin(sc.t * 1.3) > .6) s.face = Math.sign(Math.sin(sc.t * .7)) || 1;
      s.chew = Math.max(0, Math.sin(sc.t * 9)) * (s.head > .5 ? 1 : 0);
      if (pause <= 0) { s.tx = 90 + Math.random() * (sc.W - 180); pause = 3.5 + Math.random() * 2; }
    } else s.head = ease(s.head, .1, dt * 3);
  }, draw: (c, W, Hh, t) => {
    sky(c, W, H, t, { stops: [[0, "#06070f"], [1, "#272c4d"]], horizon: H * .55, stars: 1, moon: [W * .8, H * .2, 16] });
    ridge(c, W, H, mesas(W, 21, H * .55, 14, 46), "#1e2440", "rgba(223,228,246,.13)");
    groundBand(c, W, H, H * .6, t);
    sheep(c, s.x, H * .84, 1.05, s.face, s, t, s.collar);
  } });
  sc.resize = rescale([s]);
  sc.resize(1000, sc.W);
}

// ---------- parallax on the home page: layers by scroll and pointer, sections by where they sit in the view
const layers = $$("[data-f]").map((el) => ({ el, f: +el.dataset.f, px: +(el.dataset.px || 0) }));
const depths = $$("[data-depth]").map((el) => ({ el, f: +el.dataset.depth, host: el.closest("section, footer") }));
const heroCopy = $("#hero-copy"), topbar = $(".flock-top"), dawnEl = $("#dawn");
let mx = 0, my = 0, tmx = 0, tmy = 0;
if (hero && fine) {
  hero.addEventListener("pointermove", (e) => { const r = hero.getBoundingClientRect(); tmx = (e.clientX - r.left) / r.width - .5; tmy = (e.clientY - r.top) / r.height - .5; });
  hero.addEventListener("pointerleave", () => { tmx = tmy = 0; });
}
const through = (el) => { const r = el.getBoundingClientRect(), vh = innerHeight; return clamp((vh - r.top) / (vh + r.height)); };
function parallax(dt) {
  if (!hero) return;
  mx = ease(mx, tmx, dt * 3); my = ease(my, tmy, dt * 3);
  const y = Math.min(scrollY, hero.offsetHeight), vh = innerHeight;
  for (const l of layers) l.el.style.transform = `translate3d(${(-mx * l.px).toFixed(2)}px, ${(y * l.f - my * l.px * .4).toFixed(2)}px, 0)`;
  if (heroCopy) heroCopy.style.opacity = clamp(1 - y / (hero.offsetHeight * .6));
  for (const d of depths) { const r = d.host.getBoundingClientRect(); if (r.bottom < -200 || r.top > vh + 200) continue; d.el.style.transform = `translate3d(0, ${((r.top + r.height / 2 - vh / 2) * d.f).toFixed(2)}px, 0)`; }
  if (nightCanvas) nightP = through(nightCanvas);
  if (gateCanvas) gateP = through(gateCanvas);
  if (dawnEl) { const fr = dawnEl.getBoundingClientRect(); dawnP = clamp((vh - fr.top) / fr.height); }
}
if (topbar) { const solid = () => topbar.classList.toggle("solid", scrollY > 40); solid(); addEventListener("scroll", solid, { passive: true }); }

// ---------- one clock for every scene
if (reduce) { const still = () => scenes.forEach(render); still(); document.fonts?.ready.then(still); }
else if (scenes.length) {
  let last = performance.now();
  const frame = (now) => {
    const dt = Math.min(.05, (now - last) / 1000); last = now;
    parallax(dt);
    for (const sc of scenes) if ((sc.on || sc.held) && !sc.still) { sc.t += dt; sc.step?.(dt, sc.W); render(sc); }
    requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
}
})();
