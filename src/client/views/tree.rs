// A tree (tui-tree-widget): nodes the user opens, closes and chooses in, each named by its path, the ids from the root
// down. Which are open and which is chosen is kept until the plugin changes `open` or `selected`.
use std::collections::HashSet;

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Scrollbar, ScrollbarOrientation, StatefulWidget};
use serde_json::{json, Value};
use tui_tree_widget::{Tree, TreeItem, TreeState};

use super::build::{fired, hit, Ctx, Draw, ElState, Elems, Fired, Took};
use crate::client::draw::Canvas;

// `[{ "id": "…", "text": Line, "children": [ … ] }]`; children whose ids repeat are left out.
fn items(ctx: &Ctx, v: &Value) -> Vec<TreeItem<'static, String>> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|it| {
            let id = it["id"].as_str()?.to_string();
            let text = ctx.line(&it["text"]);
            Some(TreeItem::new(id.clone(), text.clone(), items(ctx, &it["children"])).unwrap_or_else(|_| TreeItem::new_leaf(id, text)))
        })
        .collect()
}

fn path(v: &Value) -> Option<Vec<String>> {
    v.as_array()?.iter().map(|x| x.as_str().map(String::from)).collect::<Option<Vec<_>>>().filter(|p| !p.is_empty())
}

pub fn seed(n: &Value, st: &mut ElState) {
    st.tree = TreeState::default();
    for p in n["open"].as_array().into_iter().flatten().filter_map(path) {
        st.tree.open(p);
    }
    if let Some(p) = path(&n["selected"]) {
        st.tree.select(p);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn draw(ctx: &Ctx, c: &mut Canvas, n: &Value, key: &str, area: Rect, elems: &mut Elems, d: &Draw, on: bool) {
    let items = items(ctx, &n["items"]);
    let Ok(mut t) = Tree::new(&items) else { return }; // its top level's ids repeat
    t = t.highlight_style(ctx.style_or(n, "highlight_style", ctx.selection(on))).node_closed_symbol("▸ ").node_open_symbol("▾ ").node_no_children_symbol("  ");
    if let Some(s) = n["highlight_symbol"].as_str() {
        t = t.highlight_symbol(s);
    }
    if n["scrollbar"] != false {
        t = t.experimental_scrollbar(Some(Scrollbar::new(ScrollbarOrientation::VerticalRight).begin_symbol(None).end_symbol(None).track_symbol(Some(" ")).thumb_symbol("▐").thumb_style(Style::new().fg(ctx.tone("border")))));
    }
    let st = elems.entry(key.to_string()).or_default();
    StatefulWidget::render(t, area, c.buf, &mut st.tree);
    st.shown = area.height as usize;
    // each row's node, for clicks
    st.drawn = st.tree.flatten(&items).into_iter().skip(st.tree.get_offset()).flat_map(|f| vec![f.identifier; f.item.height()]).take(area.height as usize).collect();
    for (i, _) in st.drawn.iter().enumerate() {
        hit(c, d, key, Rect { y: area.y + i as u16, height: 1, ..area }, i as i32);
    }
}

// What changed in a tree since `before` (its selection, and which nodes were open): its change and toggles.
fn events(n: &Value, st: &ElState, before: (Vec<String>, HashSet<Vec<String>>)) -> Vec<Fired> {
    let mut out = vec![];
    let open = st.tree.opened();
    for p in open.difference(&before.1) {
        out.extend(fired(n, "toggle", json!({ "path": p, "open": true })));
    }
    for p in before.1.difference(open) {
        out.extend(fired(n, "toggle", json!({ "path": p, "open": false })));
    }
    if st.tree.selected() != before.0.as_slice() {
        out.extend(fired(n, "change", json!({ "path": st.tree.selected() })));
    }
    out
}

fn now(st: &ElState) -> (Vec<String>, HashSet<Vec<String>>) {
    (st.tree.selected().to_vec(), st.tree.opened().clone())
}

// ↑ ↓ (j k), pages, home and end choose; ← closes (or goes up to the parent), → opens; Enter runs the action (or, with
// none, opens and closes), Space opens and closes.
pub fn key(n: &Value, st: &mut ElState, name: &str) -> Took {
    let before = now(st);
    let page = st.shown.max(1);
    let t = &mut st.tree;
    match name {
        "up" | "k" => drop(t.key_up()),
        "down" | "j" => drop(t.key_down()),
        "left" => drop(t.key_left()),
        "right" => drop(t.key_right()),
        "home" => drop(t.select_first()),
        "end" => drop(t.select_last()),
        "pageup" => drop(t.select_relative(|c| c.map_or(0, |c| c.saturating_sub(page)))),
        "pagedown" => drop(t.select_relative(|c| c.map_or(0, |c| c.saturating_add(page)))),
        "enter" if !n["action"].is_null() && !t.selected().is_empty() => return Some(fired(n, "action", json!({ "path": t.selected() })).into_iter().collect()),
        "enter" | "space" => drop(t.toggle_selected()),
        _ => return None,
    }
    Some(events(n, st, before))
}

// A click on a row chooses its node, or opens or closes it when it's chosen already; a double click runs the action.
pub fn click(n: &Value, st: &mut ElState, row: usize, double: bool) -> Vec<Fired> {
    let Some(p) = st.drawn.get(row).cloned() else { return vec![] };
    let before = now(st);
    if st.tree.selected() == p.as_slice() {
        st.tree.toggle_selected();
    } else {
        st.tree.select(p.clone());
    }
    let mut out = events(n, st, before);
    if double {
        out.extend(fired(n, "action", json!({ "path": p })));
    }
    out
}
