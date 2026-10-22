// Drawing the way a canvas does, over AppKit: paths of lines, curves, ellipses and arcs, filled and stroked under the
// current transform, which is saved and restored with the graphics state. The notch's view is flipped, so y runs down
// and angles turn clockwise, as on a canvas.
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::AllocAnyThread;
use objc2_app_kit::{
    NSAffineTransformNSAppKitAdditions, NSBezierPath, NSColor, NSFont, NSFontAttributeName, NSForegroundColorAttributeName, NSGradient, NSGradientDrawingOptions,
    NSGraphicsContext, NSLineCapStyle, NSLineJoinStyle, NSStringDrawing,
};
use objc2_foundation::{NSAffineTransform, NSArray, NSDictionary, NSPoint, NSRect, NSSize, NSString};

pub type Rgb = (u8, u8, u8);

pub const fn rgb(v: u32) -> Rgb {
    ((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

thread_local! {
    static FADE: Cell<f64> = const { Cell::new(1.0) };
    // text measured, and labels cut to fit: the same few, every frame
    static WIDTHS: RefCell<HashMap<(String, u64, u64), f64>> = RefCell::new(HashMap::new());
    static FITS: RefCell<HashMap<(String, u64, u64, u64), String>> = RefCell::new(HashMap::new());
}

// remembered, forgetting everything now and then so it doesn't grow without end
fn memo<K: std::hash::Hash + Eq, V: Clone>(cache: &'static std::thread::LocalKey<RefCell<HashMap<K, V>>>, key: K, f: impl FnOnce() -> V) -> V {
    if let Some(v) = cache.with_borrow(|m| m.get(&key).cloned()) {
        return v;
    }
    let v = f();
    cache.with_borrow_mut(|m| {
        if m.len() > 2000 {
            m.clear();
        }
        m.insert(key, v.clone());
    });
    v
}

// How opaque everything drawn from here on is: the pasture fades in and out as the notch opens and closes.
pub fn fade(a: f64) {
    FADE.set(a);
}

pub fn faded() -> f64 {
    FADE.get()
}

pub fn color(c: Rgb, a: f64) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(c.0 as f64 / 255.0, c.1 as f64 / 255.0, c.2 as f64 / 255.0, a * FADE.get())
}

fn pt(x: f64, y: f64) -> NSPoint {
    NSPoint::new(x, y)
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(pt(x, y), NSSize::new(w, h))
}

pub struct Path(Retained<NSBezierPath>);

impl Path {
    pub fn new() -> Path {
        Path(NSBezierPath::bezierPath())
    }

    pub fn move_to(&self, x: f64, y: f64) -> &Path {
        self.0.moveToPoint(pt(x, y));
        self
    }

    pub fn line(&self, x: f64, y: f64) -> &Path {
        self.0.lineToPoint(pt(x, y));
        self
    }

    pub fn cubic(&self, ax: f64, ay: f64, bx: f64, by: f64, x: f64, y: f64) -> &Path {
        self.0.curveToPoint_controlPoint1_controlPoint2(pt(x, y), pt(ax, ay), pt(bx, by));
        self
    }

    // a quadratic curve, as the cubic that traces it
    pub fn quad(&self, cx: f64, cy: f64, x: f64, y: f64) -> &Path {
        let p = self.0.currentPoint();
        let k = 2.0 / 3.0;
        self.cubic(p.x + k * (cx - p.x), p.y + k * (cy - p.y), x + k * (cx - x), y + k * (cy - y), x, y)
    }

    pub fn close(&self) -> &Path {
        self.0.closePath();
        self
    }

    pub fn circle(&self, cx: f64, cy: f64, r: f64) -> &Path {
        self.0.appendBezierPathWithOvalInRect(rect(cx - r, cy - r, 2.0 * r, 2.0 * r));
        self
    }

    // turned `rot` radians about its middle
    pub fn ellipse(&self, cx: f64, cy: f64, rx: f64, ry: f64, rot: f64) -> &Path {
        self.shaped(NSBezierPath::bezierPathWithOvalInRect(rect(-1.0, -1.0, 2.0, 2.0)), cx, cy, rx, ry, rot)
    }

    // part of a circle, from angle a to b (radians; backwards when `back`), joined to the path by a line
    pub fn arc(&self, cx: f64, cy: f64, r: f64, a: f64, b: f64, back: bool) -> &Path {
        self.0.appendBezierPathWithArcWithCenter_radius_startAngle_endAngle_clockwise(pt(cx, cy), r, a.to_degrees(), b.to_degrees(), back);
        self
    }

    // part of an ellipse, as a path of its own
    pub fn ellipse_arc(&self, cx: f64, cy: f64, rx: f64, ry: f64, rot: f64, a: f64, b: f64) -> &Path {
        let p = NSBezierPath::bezierPath();
        p.appendBezierPathWithArcWithCenter_radius_startAngle_endAngle_clockwise(NSPoint::ZERO, 1.0, a.to_degrees(), b.to_degrees(), false);
        self.shaped(p, cx, cy, rx, ry, rot)
    }

    fn shaped(&self, p: Retained<NSBezierPath>, cx: f64, cy: f64, sx: f64, sy: f64, rot: f64) -> &Path {
        let t = NSAffineTransform::transform();
        t.translateXBy_yBy(cx, cy);
        t.rotateByRadians(rot);
        t.scaleXBy_yBy(sx, sy);
        p.transformUsingAffineTransform(&t);
        self.0.appendBezierPath(&p);
        self
    }

    pub fn rect(&self, x: f64, y: f64, w: f64, h: f64) -> &Path {
        self.0.appendBezierPathWithRect(rect(x, y, w, h));
        self
    }

    pub fn round_rect(&self, x: f64, y: f64, w: f64, h: f64, r: f64) -> &Path {
        self.0.appendBezierPathWithRoundedRect_xRadius_yRadius(rect(x, y, w, h), r, r);
        self
    }

    pub fn add(&self, o: &Path) -> &Path {
        self.0.appendBezierPath(&o.0);
        self
    }

    pub fn fill(&self, c: Rgb, a: f64) {
        color(c, a).setFill();
        self.0.fill();
    }

    // round ends and corners
    pub fn stroke(&self, c: Rgb, a: f64, width: f64) {
        color(c, a).setStroke();
        self.0.setLineWidth(width);
        self.0.setLineCapStyle(NSLineCapStyle::Round);
        self.0.setLineJoinStyle(NSLineJoinStyle::Round);
        self.0.stroke();
    }

    // until the graphics state is restored
    pub fn clip(&self) {
        self.0.addClip();
    }
}

// The graphics state, restored when this goes.
#[must_use]
pub struct Saved(());

impl Drop for Saved {
    fn drop(&mut self) {
        NSGraphicsContext::restoreGraphicsState_class();
    }
}

pub fn save() -> Saved {
    NSGraphicsContext::saveGraphicsState_class();
    Saved(())
}

fn concat(f: impl FnOnce(&NSAffineTransform)) {
    let t = NSAffineTransform::transform();
    f(&t);
    t.concat();
}

pub fn translate(x: f64, y: f64) {
    concat(|t| t.translateXBy_yBy(x, y));
}

pub fn rotate(r: f64) {
    concat(|t| t.rotateByRadians(r));
}

pub fn scale(sx: f64, sy: f64) {
    concat(|t| t.scaleXBy_yBy(sx, sy));
}

// A gradient over a path, evenly through its colours from one point to another, its end colours beyond.
pub fn linear(p: &Path, from: (f64, f64), to: (f64, f64), stops: &[(Rgb, f64)]) {
    let colors: Vec<_> = stops.iter().map(|&(c, a)| color(c, a)).collect();
    let Some(g) = NSGradient::initWithColors(NSGradient::alloc(), &NSArray::from_retained_slice(&colors)) else { return };
    let _s = save();
    p.clip();
    g.drawFromPoint_toPoint_options(pt(from.0, from.1), pt(to.0, to.1), NSGradientDrawingOptions::DrawsBeforeStartingLocation | NSGradientDrawingOptions::DrawsAfterEndingLocation);
}

// and from a circle of radius r0 out to one of r1 round the same middle
pub fn radial(p: &Path, c: (f64, f64), r0: f64, r1: f64, stops: &[(Rgb, f64)]) {
    let colors: Vec<_> = stops.iter().map(|&(c, a)| color(c, a)).collect();
    let Some(g) = NSGradient::initWithColors(NSGradient::alloc(), &NSArray::from_retained_slice(&colors)) else { return };
    let _s = save();
    p.clip();
    g.drawFromCenter_radius_toCenter_radius_options(pt(c.0, c.1), r0, pt(c.0, c.1), r1, NSGradientDrawingOptions::DrawsBeforeStartingLocation);
}

fn attrs(size: f64, weight: f64, c: Rgb, a: f64) -> Retained<NSDictionary<NSString, AnyObject>> {
    let font = NSFont::systemFontOfSize_weight(size, weight);
    let color = color(c, a);
    unsafe { NSDictionary::from_slices(&[NSFontAttributeName, NSForegroundColorAttributeName], &[&*font as &AnyObject, &*color as &AnyObject]) }
}

// `s` with its top left at (x, y)
pub fn text(s: &str, x: f64, y: f64, size: f64, weight: f64, c: Rgb, a: f64) {
    unsafe { NSString::from_str(s).drawAtPoint_withAttributes(pt(x, y), Some(&attrs(size, weight, c, a))) };
}

pub fn width(s: &str, size: f64, weight: f64) -> f64 {
    memo(&WIDTHS, (s.to_string(), size.to_bits(), weight.to_bits()), || unsafe { NSString::from_str(s).sizeWithAttributes(Some(&attrs(size, weight, (0, 0, 0), 1.0))) }.width)
}

// as much of `s` as fits in `max` points, with … for the rest
pub fn fit(s: &str, max: f64, size: f64, weight: f64) -> String {
    memo(&FITS, (s.to_string(), max.to_bits(), size.to_bits(), weight.to_bits()), || {
        if width(s, size, weight) <= max {
            return s.into();
        }
        let chars: Vec<char> = s.chars().collect();
        (0..chars.len()).rev().map(|n| format!("{}…", chars[..n].iter().collect::<String>().trim_end())).find(|t| width(t, size, weight) <= max).unwrap_or_default()
    })
}
