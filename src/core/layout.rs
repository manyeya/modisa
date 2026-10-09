// Split tree for one tab. "row" = children side by side, "col" = stacked.
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Axis {
    Row,
    Col,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Node {
    Pane { pane: String },
    Split { dir: Axis, ratio: f64, a: Box<Node>, b: Box<Node> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

pub type Rects = IndexMap<String, Rect>;

const MIN: i32 = 3; // smallest pane edge: two border cells + one content cell
// Below this a pane is too small to use, and the tab shows only its focused pane (see display_rects).
pub const USABLE: (i32, i32) = (24, 6);

impl Node {
    pub fn pane(id: &str) -> Node {
        Node::Pane { pane: id.to_string() }
    }
}

// ratio: the share `id` keeps; the new pane gets the rest, right of or below it.
pub fn split(tree: &Node, id: &str, dir: Axis, new_id: &str, ratio: f64) -> Node {
    match tree {
        Node::Pane { pane } if pane == id => Node::Split { dir, ratio, a: Box::new(tree.clone()), b: Box::new(Node::pane(new_id)) },
        Node::Pane { .. } => tree.clone(),
        Node::Split { dir: d, ratio: r, a, b } => Node::Split { dir: *d, ratio: *r, a: Box::new(split(a, id, dir, new_id, ratio)), b: Box::new(split(b, id, dir, new_id, ratio)) },
    }
}

// The leaf holding `id`, the node itself: changing its pane in place keeps every ratio valid.
pub fn leaf<'a>(tree: &'a mut Node, id: &str) -> Option<&'a mut String> {
    match tree {
        Node::Pane { pane } => (pane == id).then_some(pane),
        Node::Split { a, b, .. } => match leaf(a, id) {
            Some(p) => Some(p),
            None => leaf(b, id),
        },
    }
}

pub fn remove(tree: &Node, id: &str) -> Option<Node> {
    match tree {
        Node::Pane { pane } => (pane != id).then(|| tree.clone()),
        Node::Split { dir, ratio, a, b } => match (remove(a, id), remove(b, id)) {
            (None, b) => b,
            (a, None) => a,
            (Some(a), Some(b)) => Some(Node::Split { dir: *dir, ratio: *ratio, a: Box::new(a), b: Box::new(b) }),
        },
    }
}

pub fn panes(tree: &Node) -> Vec<String> {
    let mut out = vec![];
    fn walk(n: &Node, out: &mut Vec<String>) {
        match n {
            Node::Pane { pane } => out.push(pane.clone()),
            Node::Split { a, b, .. } => {
                walk(a, out);
                walk(b, out);
            }
        }
    }
    walk(tree, &mut out);
    out
}

fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    v.max(lo).min(hi)
}

// JavaScript's Math.round: halves round up, toward +∞
fn round(v: f64) -> i32 {
    (v + 0.5).floor() as i32
}

fn cut(dir: Axis, ratio: f64, r: Rect) -> (Rect, Rect) {
    match dir {
        Axis::Row => {
            let aw = clamp(round(r.w as f64 * ratio) as f64, MIN.min(r.w - 1) as f64, (r.w - MIN).max(1) as f64) as i32;
            (Rect { w: aw, ..r }, Rect { x: r.x + aw, w: r.w - aw, ..r })
        }
        Axis::Col => {
            let ah = clamp(round(r.h as f64 * ratio) as f64, MIN.min(r.h - 1) as f64, (r.h - MIN).max(1) as f64) as i32;
            (Rect { h: ah, ..r }, Rect { y: r.y + ah, h: r.h - ah, ..r })
        }
    }
}

pub fn rects(tree: &Node, r: Rect) -> Rects {
    let mut out = Rects::new();
    fn walk(n: &Node, r: Rect, out: &mut Rects) {
        match n {
            Node::Pane { pane } => {
                out.insert(pane.clone(), r);
            }
            Node::Split { dir, ratio, a, b } => {
                let (ra, rb) = cut(*dir, *ratio, r);
                walk(a, ra, out);
                walk(b, rb, out);
            }
        }
    }
    walk(tree, r, &mut out);
    out
}

// Preserve the split tree while temporarily showing the focused pane at small sizes.
// Shared by the client and server so PTY dimensions always match what is displayed.
pub fn display_rects(tree: &Node, area: Rect, focused: &str, zoomed: bool) -> Rects {
    let rs = rects(tree, area);
    if zoomed || rs.values().any(|r| r.w < USABLE.0 || r.h < USABLE.1) {
        return Rects::from([(focused.to_string(), area)]);
    }
    rs
}

// How a message names the side of a pane: "no pane left of p3".
pub fn side(dir: Dir) -> &'static str {
    match dir {
        Dir::Left => "left of",
        Dir::Right => "right of",
        Dir::Up => "above",
        Dir::Down => "below",
    }
}

// Nearest pane in a direction that shares an edge; ties go to the one closest to our centre.
pub fn neighbor(rs: &Rects, id: &str, dir: Dir) -> Option<String> {
    let c = rs.get(id)?;
    let mut best = None;
    let mut best_dist = f64::INFINITY;
    for (other, o) in rs {
        if other == id {
            continue;
        }
        let touches = match dir {
            Dir::Right => o.x == c.x + c.w,
            Dir::Left => o.x + o.w == c.x,
            Dir::Down => o.y == c.y + c.h,
            Dir::Up => o.y + o.h == c.y,
        };
        let horizontal = matches!(dir, Dir::Left | Dir::Right);
        let overlaps = if horizontal { o.y < c.y + c.h && o.y + o.h > c.y } else { o.x < c.x + c.w && o.x + o.w > c.x };
        if !touches || !overlaps {
            continue;
        }
        let dist = if horizontal { ((o.y as f64 + o.h as f64 / 2.0) - (c.y as f64 + c.h as f64 / 2.0)).abs() } else { ((o.x as f64 + o.w as f64 / 2.0) - (c.x as f64 + c.w as f64 / 2.0)).abs() };
        if dist < best_dist {
            best = Some(other.clone());
            best_dist = dist;
        }
    }
    best
}

// Smallest box a subtree needs so every pane in it stays usable.
fn need(n: &Node) -> (i32, i32) {
    match n {
        Node::Pane { .. } => USABLE,
        Node::Split { dir, a, b, .. } => {
            let (a, b) = (need(a), need(b));
            match dir {
                Axis::Row => (a.0 + b.0, a.1.max(b.1)),
                Axis::Col => (a.0.max(b.0), a.1 + b.1),
            }
        }
    }
}

// Keep a divider where both sides stay usable. If the space is too small for that anyway (the terminal shrank), leave
// the ratio alone; display_rects shows one pane until there's room again.
fn settle(node: &mut Node, r: Rect, to: f64) {
    let Node::Split { dir, ratio, a, b } = node else { return };
    let pick = |s: (i32, i32)| if *dir == Axis::Row { s.0 } else { s.1 };
    let size = (if *dir == Axis::Row { r.w } else { r.h }) as f64;
    let (lo, hi) = (pick(need(a)) as f64, size - pick(need(b)) as f64);
    if lo > hi {
        return;
    }
    *ratio = clamp(to, lo / size, hi / size);
}

// Move the nearest divider on the pane's `dir` side by `cells`. Mutates ratios in place.
pub fn resize(tree: &mut Node, r: Rect, id: &str, dir: Dir, cells: i32) -> bool {
    let Node::Split { dir: axis, ratio, a, b } = tree else { return false };
    let (ra, rb) = cut(*axis, *ratio, r);
    let in_a = panes(a).iter().any(|p| p == id);
    if if in_a { resize(a, ra, id, dir, cells) } else { resize(b, rb, id, dir, cells) } {
        return true;
    }
    let want = if matches!(dir, Dir::Left | Dir::Right) { Axis::Row } else { Axis::Col };
    // the divider sits right/below of `a` and left/above of `b`
    let facing = if in_a { matches!(dir, Dir::Right | Dir::Down) } else { matches!(dir, Dir::Left | Dir::Up) };
    if *axis != want || !facing {
        return false;
    }
    let size = if want == Axis::Row { r.w } else { r.h };
    let sign = if matches!(dir, Dir::Right | Dir::Down) { 1.0 } else { -1.0 };
    let to = *ratio + sign * cells as f64 / size as f64;
    settle(tree, r, to);
    true
}

// A divider found under the pointer: the path to its split node (false = a, true = b) and that node's box.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub path: Vec<bool>,
    pub dir: Axis,
    pub rect: Rect,
}

// Split node whose divider is under (x, y): the two border cells either side of the cut.
pub fn divider_at(tree: &Node, r: Rect, x: i32, y: i32) -> Option<Hit> {
    let mut path = vec![];
    let (mut node, mut r) = (tree, r);
    loop {
        let Node::Split { dir, ratio, a, b } = node else { return None };
        let (ra, rb) = cut(*dir, *ratio, r);
        let on = match dir {
            Axis::Row => (x == rb.x - 1 || x == rb.x) && y >= r.y && y < r.y + r.h,
            Axis::Col => (y == rb.y - 1 || y == rb.y) && x >= r.x && x < r.x + r.w,
        };
        if on {
            return Some(Hit { path, dir: *dir, rect: r });
        }
        let in_a = x < ra.x + ra.w && y < ra.y + ra.h;
        path.push(!in_a);
        (node, r) = if in_a { (a.as_ref(), ra) } else { (b.as_ref(), rb) };
    }
}

fn at_path<'a>(tree: &'a mut Node, path: &[bool]) -> Option<&'a mut Node> {
    let mut node = tree;
    for &right in path {
        let Node::Split { a, b, .. } = node else { return None };
        node = if right { b } else { a };
    }
    Some(node)
}

pub fn drag_to(tree: &mut Node, hit: &Hit, x: i32, y: i32) {
    let Some(node) = at_path(tree, &hit.path) else { return };
    let r = hit.rect;
    let (at, size) = if hit.dir == Axis::Row { (x - r.x, r.w) } else { (y - r.y, r.h) };
    settle(node, r, (at as f64 + 0.5) / size as f64);
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect { x: 0, y: 1, w: 100, h: 40 };

    // a | b
    //   | c
    fn tree() -> Node {
        split(&split(&Node::pane("a"), "a", Axis::Row, "b", 0.5), "b", Axis::Col, "c", 0.5)
    }

    fn ratio(n: &Node) -> f64 {
        match n {
            Node::Split { ratio, .. } => *ratio,
            _ => panic!("a leaf"),
        }
    }

    #[test]
    fn rects_tile_the_area_exactly() {
        let rs = rects(&tree(), AREA);
        assert_eq!(rs.keys().collect::<Vec<_>>(), ["a", "b", "c"]);
        let mut seen = std::collections::HashSet::new();
        for r in rs.values() {
            for x in r.x..r.x + r.w {
                for y in r.y..r.y + r.h {
                    assert!(seen.insert((x, y)));
                }
            }
        }
        assert_eq!(seen.len() as i32, AREA.w * AREA.h);
    }

    #[test]
    fn split_gives_ratio_and_rest() {
        assert_eq!(split(&Node::pane("a"), "a", Axis::Row, "b", 0.5), Node::Split { dir: Axis::Row, ratio: 0.5, a: Box::new(Node::pane("a")), b: Box::new(Node::pane("b")) });
        let t = split(&tree(), "c", Axis::Row, "d", 0.75);
        let rs = rects(&t, AREA);
        assert_eq!((rs["c"].w, rs["d"].w), (38, 12));
        assert_eq!(split(&tree(), "nope", Axis::Col, "d", 0.2), tree());
        assert_eq!(serde_json::to_string(&Node::pane("a")).unwrap(), r#"{"pane":"a"}"#);
        assert_eq!(serde_json::from_str::<Node>(&serde_json::to_string(&t).unwrap()).unwrap(), t);
    }

    #[test]
    fn leaf_swaps_in_place() {
        let mut t = tree();
        resize(&mut t, AREA, "a", Dir::Right, 10);
        let before = rects(&t, AREA);
        *leaf(&mut t, "a").unwrap() = "x".into();
        *leaf(&mut t, "c").unwrap() = "a".into();
        *leaf(&mut t, "x").unwrap() = "c".into();
        assert!(leaf(&mut t, "nope").is_none());
        assert_eq!(panes(&t), ["c", "b", "a"]);
        let after = rects(&t, AREA);
        assert_eq!(after["c"], before["a"]);
        assert_eq!(after["a"], before["c"]);
    }

    #[test]
    fn remove_collapses() {
        assert_eq!(remove(&tree(), "b"), Some(Node::Split { dir: Axis::Row, ratio: 0.5, a: Box::new(Node::pane("a")), b: Box::new(Node::pane("c")) }));
        assert_eq!(remove(&Node::pane("a"), "a"), None);
        assert_eq!(panes(&remove(&tree(), "a").unwrap()), ["b", "c"]);
    }

    #[test]
    fn neighbors() {
        let rs = rects(&tree(), AREA);
        assert!(neighbor(&rs, "a", Dir::Right).is_some());
        assert_eq!(neighbor(&rs, "b", Dir::Down).as_deref(), Some("c"));
        assert_eq!(neighbor(&rs, "c", Dir::Up).as_deref(), Some("b"));
        assert_eq!(neighbor(&rs, "c", Dir::Left).as_deref(), Some("a"));
        assert_eq!(neighbor(&rs, "a", Dir::Left), None);
    }

    #[test]
    fn resize_moves_and_clamps() {
        let mut t = tree();
        assert!(resize(&mut t, AREA, "a", Dir::Right, 10));
        assert_eq!(rects(&t, AREA)["a"].w, 60);
        assert!(resize(&mut t, AREA, "c", Dir::Up, 4));
        assert_eq!(rects(&t, AREA)["b"].h, 16);
        assert!(!resize(&mut t, AREA, "a", Dir::Left, 10));
        resize(&mut t, AREA, "a", Dir::Right, 1000);
        assert_eq!(rects(&t, AREA)["a"].w, 100 - USABLE.0);
    }

    #[test]
    fn divider_drag() {
        let mut t = tree();
        let hit = divider_at(&t, AREA, 50, 10).unwrap();
        assert_eq!(hit.dir, Axis::Row);
        drag_to(&mut t, &hit, 30, 10);
        assert_eq!(rects(&t, AREA)["a"].w, 31);
        assert!(divider_at(&t, AREA, 10, 10).is_none());
    }

    #[test]
    fn dragging_keeps_panes_usable() {
        let short = Rect { x: 0, y: 1, w: 140, h: 18 };
        let mut t = split(&Node::pane("top"), "top", Axis::Col, "bottom", 0.5);
        let hit = divider_at(&t, short, 10, rects(&t, short)["bottom"].y).unwrap();
        for y in [short.y + short.h - 1, short.y, 100, -100] {
            drag_to(&mut t, &hit, 10, y);
            let rs = rects(&t, short);
            assert!(rs["top"].h >= USABLE.1 && rs["bottom"].h >= USABLE.1);
            assert_eq!(display_rects(&t, short, "top", false).len(), 2);
        }
        for _ in 0..20 {
            resize(&mut t, short, "top", Dir::Down, 3);
        }
        assert_eq!(rects(&t, short)["bottom"].h, USABLE.1);
        let mut n = tree();
        let wide = Rect { x: 0, y: 0, w: 100, h: 20 };
        let hit = divider_at(&n, wide, rects(&n, wide)["b"].x, 5).unwrap();
        drag_to(&mut n, &hit, 99, 5);
        assert_eq!(rects(&n, wide)["b"].w, USABLE.0);
        let tiny = Rect { x: 0, y: 0, w: 140, h: 10 };
        let mut m = split(&Node::pane("p"), "p", Axis::Col, "q", 0.5);
        let hit = divider_at(&m, tiny, 5, rects(&m, tiny)["q"].y).unwrap();
        drag_to(&mut m, &hit, 5, 9);
        assert_eq!(ratio(&m), 0.5);
    }
}
