// Session model: workspaces → tabs → split trees of panes. Owns layout and PTY sizes.
//
// What the TypeScript Session told its hooks synchronously, this one queues as `effects` (and `dirty` for "changed"),
// which the server drains after every operation: the session never reaches back into the server.
pub mod pane;

use std::collections::HashMap;

use indexmap::IndexMap;

use crate::core::layout::{self, display_rects, divider_at, drag_to, neighbor, panes as tree_panes, rects, Axis, Dir, Hit, Node, Rect};
use crate::core::paths;
use crate::protocol::conn::{error, RpcResult};
use crate::protocol::types::{find_pane, GitView, PaneInfo, TabView, View, WorkspaceView};
use pane::{PaneOpts, PtyPane, Sink};

#[derive(Clone, Debug)]
pub struct Tab {
    pub id: String,
    pub name: Option<String>,
    pub tree: Node,
    pub focused: String,
    pub zoomed: bool,
}

#[derive(Clone, Debug)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub cwd: String,
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub git: Option<GitView>, // see ../git.rs
}

#[derive(Clone, Debug, Default)]
pub struct SpawnOpts {
    pub cwd: Option<String>,
    pub command: Option<String>,
    pub harness: Option<String>,
    pub name: Option<String>,
    pub created_by: Option<String>,
    pub ephemeral: bool,
    pub env: Option<IndexMap<String, String>>,
}

// Where a moved pane goes: beside a pane (right of or below it, with `share` of the space), alone in a new tab of a
// space, or alone in a new space.
pub enum MoveTo {
    Beside { beside: String, dir: Axis, share: f64 },
    NewTab { ws: String, name: Option<String> },
    NewSpace { name: Option<String>, cwd: String },
}

pub enum Effect {
    Created(String),
    // a pane is being closed (it's already out of `panes`): whether it had the focus on screen, and how it ended
    Closing { id: String, focused: bool, info: PaneInfo },
    Empty, // the last pane closed
}

pub struct Session {
    pub workspaces: Vec<Workspace>,
    pub active: usize,
    pub panes: IndexMap<String, PtyPane>,
    // panes held at a size of their own, not their box's: one taken over from another terminal (attach.rs)
    pub size_locks: HashMap<String, (u16, u16)>,
    pub area: Rect,
    pub effects: Vec<Effect>,
    pub dirty: bool, // clients need a new view (and the session a save)
    // recently closed panes by instance: their last info, and whether the close is what ended their process. What a
    // wait for their exit, or the exit their close causes, still needs.
    pub retired: IndexMap<String, (PaneInfo, bool)>,
    seq: u32,
    pane_seq: u32,
    drags: HashMap<u64, Hit>,
    sink: Sink,
}

impl Session {
    pub fn new(sink: Sink) -> Session {
        Session {
            workspaces: vec![],
            active: 0,
            panes: IndexMap::new(),
            size_locks: HashMap::new(),
            area: Rect { x: 0, y: 1, w: 120, h: 38 },
            effects: vec![],
            dirty: false,
            retired: IndexMap::new(),
            seq: 0,
            pane_seq: 0,
            drags: HashMap::new(),
            sink,
        }
    }

    pub fn changed(&mut self) {
        self.dirty = true;
    }

    // ---------- lookup ----------

    pub fn ws(&self) -> &Workspace {
        &self.workspaces[self.active]
    }
    fn ws_mut(&mut self) -> &mut Workspace {
        &mut self.workspaces[self.active]
    }
    pub fn tab(&self) -> &Tab {
        let ws = self.ws();
        &ws.tabs[ws.active]
    }
    fn tab_mut(&mut self) -> &mut Tab {
        let ws = self.ws_mut();
        let i = ws.active;
        &mut ws.tabs[i]
    }
    pub fn focused_id(&self) -> Option<String> {
        (!self.workspaces.is_empty()).then(|| self.tab().focused.clone())
    }

    // (workspace, tab) holding the pane
    pub fn locate(&self, id: &str) -> Option<(usize, usize)> {
        for (wi, ws) in self.workspaces.iter().enumerate() {
            for (ti, tab) in ws.tabs.iter().enumerate() {
                if tree_panes(&tab.tree).iter().any(|p| p == id) {
                    return Some((wi, ti));
                }
            }
        }
        None
    }

    // A target: see find_pane.
    pub fn resolve(&self, target: Option<&str>, fallback: Option<&str>) -> Option<String> {
        let t = target.or(fallback)?;
        let infos: Vec<&PaneInfo> = self.panes.values().map(|p| &p.info).collect();
        find_pane(infos.iter().copied(), t).map(|i| i.id.clone())
    }

    // Where a pane sits in the layout; a popup has no place.
    pub fn place_of(&self, id: &str) -> RpcResult<(usize, usize)> {
        self.locate(id).ok_or_else(|| error(format!("{id} is a popup: it has no place in a tab")))
    }

    // The pane on that side of `id`, in its tab.
    pub fn neighbor_of(&self, id: &str, dir: Dir) -> Option<String> {
        let (wi, ti) = self.locate(id)?;
        neighbor(&rects(&self.workspaces[wi].tabs[ti].tree, self.area), id, dir)
    }

    pub fn is_visible(&self, id: &str) -> bool {
        let Some((wi, ti)) = self.locate(id) else { return false };
        let tab = &self.workspaces[wi].tabs[ti];
        wi == self.active && ti == self.workspaces[wi].active && display_rects(&tab.tree, self.area, &tab.focused, tab.zoomed).contains_key(id)
    }

    // ---------- panes ----------

    fn spawn(&mut self, o: SpawnOpts, cwd: &str) -> RpcResult<String> {
        self.pane_seq += 1;
        let id = format!("p{}", self.pane_seq);
        let opts = PaneOpts {
            id: id.clone(),
            cwd: o.cwd.clone().unwrap_or_else(|| cwd.to_string()),
            command: o.command,
            harness: o.harness,
            name: o.name,
            created_by: o.created_by.unwrap_or_else(|| "user".into()),
            cols: (self.area.w - 2).max(2) as u16,
            rows: (self.area.h - 2).max(1) as u16,
            env: o.env,
        };
        let mut p = PtyPane::new(opts, self.sink.clone()).map_err(|e| error(format!("could not start a pane: {e}")))?;
        p.ephemeral = o.ephemeral;
        self.panes.insert(id.clone(), p);
        self.effects.push(Effect::Created(id.clone()));
        Ok(id)
    }

    // A pane with no place in any tab (a plugin's popup). Every client gets its output; only the one that opened it
    // shows it. Removed when its process exits, or by drop_hidden.
    pub fn spawn_hidden(&mut self, o: SpawnOpts) -> RpcResult<String> {
        let cwd = o.cwd.clone().unwrap_or_else(paths::cwd);
        let id = self.spawn(SpawnOpts { ephemeral: true, ..o }, &cwd)?;
        self.panes[&id].info.popup = Some(true);
        self.changed();
        Ok(id)
    }

    pub fn drop_hidden(&mut self, id: &str) {
        if self.locate(id).is_some() {
            return;
        }
        let Some(mut p) = self.panes.shift_remove(id) else { return };
        p.dispose();
        self.retire(&p, false); // closing all the same, to whoever watches it
        self.changed();
    }

    // A closed pane's info stays a while: its process reports the exit the close caused after it's gone.
    fn retire(&mut self, p: &PtyPane, focused: bool) {
        self.effects.push(Effect::Closing { id: p.info.id.clone(), focused, info: p.info.clone() });
        self.retired.insert(p.info.instance.clone(), (p.info.clone(), p.closed_while_running));
        if self.retired.len() > 256 {
            self.retired.shift_remove_index(0); // ponytail: the last 256 closes are remembered
        }
    }

    pub fn retired_exit(&self, instance: &str) -> Option<(&PaneInfo, bool)> {
        self.retired.get(instance).map(|(i, c)| (i, *c))
    }

    fn add_workspace(&mut self, name: Option<String>, cwd: &str) -> usize {
        self.seq += 1;
        let name = name.unwrap_or_else(|| cwd.rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or("workspace").to_string());
        self.workspaces.push(Workspace { id: format!("w{}", self.seq), name, cwd: cwd.into(), tabs: vec![], active: 0, git: None });
        self.workspaces.len() - 1
    }

    pub fn new_workspace(&mut self, name: Option<String>, cwd: Option<String>, o: SpawnOpts) -> RpcResult<String> {
        let previous = self.active;
        let cwd = cwd.unwrap_or_else(paths::cwd);
        let wi = self.add_workspace(name, &cwd);
        self.active = wi;
        match self.new_tab_in(None, o, wi) {
            Ok(id) => Ok(id),
            Err(e) => {
                self.workspaces.remove(wi);
                self.active = previous;
                Err(e)
            }
        }
    }

    pub fn new_tab(&mut self, name: Option<String>, o: SpawnOpts) -> RpcResult<String> {
        self.new_tab_in(name, o, self.active)
    }

    pub fn new_tab_in(&mut self, name: Option<String>, o: SpawnOpts, wi: usize) -> RpcResult<String> {
        let cwd = self.workspaces[wi].cwd.clone();
        let id = self.spawn(o, &cwd)?;
        self.add_tab(wi, &id, name, true);
        self.layout();
        Ok(id)
    }

    // A tab holding one pane that already exists; select: show it.
    fn add_tab(&mut self, wi: usize, pane: &str, name: Option<String>, select: bool) -> usize {
        self.seq += 1;
        let tab = Tab { id: format!("t{}", self.seq), name, tree: Node::pane(pane), focused: pane.into(), zoomed: false };
        let ws = &mut self.workspaces[wi];
        ws.tabs.push(tab);
        if select {
            ws.active = ws.tabs.len() - 1;
            self.active = wi;
        }
        self.workspaces[wi].tabs.len() - 1
    }

    // share: the new pane's part of the target's room
    pub fn split(&mut self, dir: Axis, o: SpawnOpts, target: Option<&str>, focus: bool, share: f64) -> RpcResult<Option<String>> {
        let Some(target) = target.map(String::from).or_else(|| self.focused_id()) else { return Ok(None) };
        let Some((wi, ti)) = self.locate(&target) else { return Ok(None) };
        let cwd = o.cwd.clone().or_else(|| self.panes.get(&target).map(|p| p.info.cwd.clone())).unwrap_or_else(|| self.workspaces[wi].cwd.clone());
        let id = self.spawn(SpawnOpts { cwd: Some(cwd.clone()), ..o }, &cwd)?;
        let tab = &mut self.workspaces[wi].tabs[ti];
        tab.tree = layout::split(&tab.tree, &target, dir, &id, 1.0 - share);
        tab.zoomed = false;
        if focus {
            tab.focused = id.clone();
        }
        self.layout();
        Ok(Some(id))
    }

    pub fn close(&mut self, id: Option<&str>) {
        let Some(id) = id.map(String::from).or_else(|| self.focused_id()) else { return };
        let Some((wi, ti)) = self.locate(&id) else { return };
        if !self.panes.contains_key(&id) {
            return;
        }
        let focused = self.workspaces[wi].tabs[ti].focused == id && wi == self.active && ti == self.workspaces[wi].active;
        let mut p = self.panes.shift_remove(&id).unwrap();
        p.dispose();
        self.retire(&p, focused);
        self.unlink(&id);
        if self.workspaces.is_empty() {
            self.effects.push(Effect::Empty);
            return;
        }
        self.layout();
    }

    // Take a pane out of its tab, which focuses its nearest neighbour; a tab left empty goes, then a space left empty.
    // The pane itself is untouched: close disposes of it, move puts it somewhere else.
    fn unlink(&mut self, id: &str) {
        let (wi, ti) = self.locate(id).unwrap();
        let area = self.area;
        let tab = &mut self.workspaces[wi].tabs[ti];
        let rs = rects(&tab.tree, area);
        let next = [Dir::Left, Dir::Up, Dir::Right, Dir::Down].into_iter().find_map(|d| neighbor(&rs, id, d));
        if let Some(tree) = layout::remove(&tab.tree, id) {
            if tab.focused == id {
                tab.focused = next.unwrap_or_else(|| tree_panes(&tree)[0].clone());
            }
            tab.tree = tree;
            tab.zoomed = false;
            return;
        }
        let ws = &mut self.workspaces[wi];
        ws.tabs.remove(ti);
        if ws.active >= ti {
            ws.active = ws.active.saturating_sub(1);
        }
        if !ws.tabs.is_empty() {
            return;
        }
        self.workspaces.remove(wi);
        if self.active >= wi {
            self.active = self.active.saturating_sub(1);
        }
    }

    // Move a pane, process and all. Refused, before anything changes, where it would go nowhere. What it leaves empty
    // closes, the tab it lands in is unzoomed, and the view stays where it is unless `focus`. Returns where it is now.
    pub fn move_pane(&mut self, id: &str, to: MoveTo, focus: bool) -> RpcResult<(String, String)> {
        let (fw, ft) = self.place_of(id)?;
        let alone = tree_panes(&self.workspaces[fw].tabs[ft].tree).len() == 1;
        match &to {
            MoveTo::Beside { beside, .. } if beside == id => return Err(error(format!("can't move {id} beside itself"))),
            MoveTo::Beside { beside, .. } => {
                self.place_of(beside)?;
            }
            MoveTo::NewTab { ws, .. } if alone && *ws == self.workspaces[fw].id => return Err(error(format!("{id} is already alone in its tab"))),
            MoveTo::NewSpace { .. } if alone && self.workspaces[fw].tabs.len() == 1 => return Err(error(format!("{id} is already alone in its space"))),
            _ => {}
        }
        self.unlink(id);
        let (wi, ti) = match to {
            MoveTo::Beside { beside, dir, share } => {
                let (wi, ti) = self.locate(&beside).unwrap();
                let tab = &mut self.workspaces[wi].tabs[ti];
                tab.tree = layout::split(&tab.tree, &beside, dir, id, 1.0 - share);
                tab.zoomed = false;
                (wi, ti)
            }
            MoveTo::NewTab { ws, name } => {
                let wi = self.workspaces.iter().position(|w| w.id == ws).unwrap();
                (wi, self.add_tab(wi, id, name, false))
            }
            MoveTo::NewSpace { name, cwd } => {
                let wi = self.add_workspace(name, &cwd);
                (wi, self.add_tab(wi, id, None, false))
            }
        };
        let place = (self.workspaces[wi].id.clone(), self.workspaces[wi].tabs[ti].id.clone());
        if focus {
            self.focus_pane(id);
        } else {
            self.layout();
        }
        Ok(place)
    }

    // Two panes trade places, in one tab or across tabs; the tree keeps its shape and ratios. Each tab keeps focus on its
    // pane if it's still there, and otherwise gives it to the pane that took its place.
    pub fn swap(&mut self, a: &str, b: &str) -> RpcResult<()> {
        let la = self.place_of(a)?;
        let lb = self.place_of(b)?;
        if a == b {
            return Err(error(format!("can't swap {a} with itself")));
        }
        *layout::leaf(&mut self.workspaces[la.0].tabs[la.1].tree, a).unwrap() = "\0".into();
        *layout::leaf(&mut self.workspaces[lb.0].tabs[lb.1].tree, b).unwrap() = a.into();
        *layout::leaf(&mut self.workspaces[la.0].tabs[la.1].tree, "\0").unwrap() = b.into();
        if la != lb {
            let ta = &mut self.workspaces[la.0].tabs[la.1];
            if ta.focused == a {
                ta.focused = b.into();
            }
            let tb = &mut self.workspaces[lb.0].tabs[lb.1];
            if tb.focused == b {
                tb.focused = a.into();
            }
        }
        self.layout();
        Ok(())
    }

    pub fn close_tab(&mut self) {
        for id in tree_panes(&self.tab().tree) {
            self.close(Some(&id));
        }
    }

    // ---------- focus & navigation ----------

    pub fn focus_pane(&mut self, id: &str) {
        let Some((wi, ti)) = self.locate(id) else { return };
        self.active = wi;
        self.workspaces[wi].active = ti;
        let tab = &mut self.workspaces[wi].tabs[ti];
        if tab.zoomed && tab.focused != id {
            tab.zoomed = false;
        }
        tab.focused = id.into();
        self.layout();
    }

    // Focus the pane on that side of `from`; returns it, or None when there's none.
    pub fn focus_dir(&mut self, dir: Dir, from: Option<&str>) -> Option<String> {
        let from = from.map(String::from).or_else(|| self.focused_id())?;
        let next = self.neighbor_of(&from, dir)?;
        self.focus_pane(&next);
        Some(next)
    }

    // A zoomed tab shows only its focused pane, so zooming a pane focuses it in its tab (the view stays where it is).
    // Returns whether its tab is zoomed now.
    pub fn zoom(&mut self, id: Option<&str>, mode: &str) -> bool {
        let Some(id) = id.map(String::from).or_else(|| self.focused_id()) else { return false };
        let Some((wi, ti)) = self.locate(&id) else { return false };
        let tab = &mut self.workspaces[wi].tabs[ti];
        let on = if mode == "toggle" { !(tab.zoomed && tab.focused == id) } else { mode == "on" };
        if on {
            tab.focused = id;
        }
        tab.zoomed = on;
        self.layout();
        on
    }

    // Move the divider on the pane's `dir` side; returns whether the pane's size changed (not when there's no divider
    // there, or it's as far as it goes).
    pub fn resize_pane(&mut self, dir: Dir, cells: i32, id: Option<&str>) -> bool {
        let Some(id) = id.map(String::from).or_else(|| self.focused_id()) else { return false };
        let Some((wi, ti)) = self.locate(&id) else { return false };
        let area = self.area;
        let tab = &mut self.workspaces[wi].tabs[ti];
        let was = rects(&tab.tree, area)[&id];
        if !layout::resize(&mut tab.tree, area, &id, dir, cells) {
            return false;
        }
        let now = rects(&tab.tree, area)[&id];
        self.layout();
        now.w != was.w || now.h != was.h
    }

    pub fn drag_start(&mut self, key: u64, x: i32, y: i32) -> bool {
        let area = self.area;
        let tab = self.tab();
        if display_rects(&tab.tree, area, &tab.focused, tab.zoomed).len() < tree_panes(&tab.tree).len() {
            return false;
        }
        let Some(hit) = divider_at(&tab.tree, area, x, y) else { return false };
        self.drags.insert(key, hit);
        true
    }
    pub fn drag_move(&mut self, key: u64, x: i32, y: i32) {
        let Some(hit) = self.drags.get(&key).cloned() else { return };
        drag_to(&mut self.tab_mut().tree, &hit, x, y);
        self.layout();
    }
    pub fn drag_end(&mut self, key: u64) {
        self.drags.remove(&key);
    }

    pub fn select_tab(&mut self, i: i64) {
        if i < 0 || i as usize >= self.ws().tabs.len() {
            return;
        }
        self.ws_mut().active = i as usize;
        self.changed();
    }
    pub fn cycle_tab(&mut self, step: i64) {
        let n = self.ws().tabs.len() as i64;
        self.select_tab((self.ws().active as i64 + step + n).rem_euclid(n));
    }
    pub fn select_workspace(&mut self, i: i64) {
        if i < 0 || i as usize >= self.workspaces.len() {
            return;
        }
        self.active = i as usize;
        self.changed();
    }
    pub fn rename_tab(&mut self, name: &str) {
        self.tab_mut().name = (!name.is_empty()).then(|| name.to_string());
        self.changed();
    }
    pub fn rename_workspace(&mut self, name: &str, i: Option<usize>) {
        let i = i.unwrap_or(self.active);
        if let Some(ws) = self.workspaces.get_mut(i) {
            if !name.trim().is_empty() {
                ws.name = name.trim().into();
            }
        }
        self.changed();
    }
    // Close every pane in a space. The last space can't go: that would end the session.
    pub fn close_workspace(&mut self, i: Option<usize>) -> RpcResult<()> {
        let i = i.unwrap_or(self.active);
        let Some(ws) = self.workspaces.get(i) else { return Err(error(format!("no such space: {i}"))) };
        if self.workspaces.len() < 2 {
            return Err(error("can't delete the only space"));
        }
        let ids: Vec<String> = ws.tabs.iter().flat_map(|t| tree_panes(&t.tree)).collect();
        for id in ids {
            self.close(Some(&id));
        }
        Ok(())
    }
    // A space by index, id or name.
    pub fn find_workspace(&self, r: &str) -> RpcResult<usize> {
        self.workspaces.iter().position(|w| w.id == r || w.name == r).ok_or_else(|| error(format!("no such space: {r}")))
    }
    // A tab by id, or by name (in the active space first).
    pub fn find_tab(&self, r: &str) -> RpcResult<(usize, usize)> {
        let order = std::iter::once(self.active).chain((0..self.workspaces.len()).filter(|&i| i != self.active));
        let all: Vec<(usize, usize)> = order.flat_map(|wi| (0..self.workspaces[wi].tabs.len()).map(move |ti| (wi, ti))).collect();
        let tab = |&(wi, ti): &(usize, usize)| &self.workspaces[wi].tabs[ti];
        all.iter().find(|x| tab(x).id == r).or_else(|| all.iter().find(|x| tab(x).name.as_deref() == Some(r))).copied().ok_or_else(|| error(format!("no such tab: {r}")))
    }
    pub fn rename_pane(&mut self, id: &str, name: &str) {
        let Some(p) = self.panes.get_mut(id) else { return };
        let name = name.strip_prefix('@').unwrap_or(name);
        p.info.name = (!name.is_empty()).then(|| name.to_string());
        p.refresh_title(); // a name cleared gives the title back to what else names it
        self.changed();
    }

    // ---------- layout ----------

    pub fn set_area(&mut self, a: Rect) {
        self.area = a;
        self.layout();
    }

    // Size every PTY to its box (hidden tabs too, so they're right when shown), or to its lock, then notify.
    pub fn layout(&mut self) {
        for ws in &self.workspaces {
            for tab in &ws.tabs {
                let rs = rects(&tab.tree, self.area);
                let displayed = display_rects(&tab.tree, self.area, &tab.focused, tab.zoomed);
                for (id, r) in &rs {
                    let full = displayed.get(id).unwrap_or(r);
                    let lock = self.size_locks.get(id);
                    if let Some(p) = self.panes.get_mut(id) {
                        let (cols, rows) = lock.copied().unwrap_or(((full.w - 2).max(0) as u16, (full.h - 2).max(0) as u16));
                        p.resize(cols, rows);
                    }
                }
            }
        }
        self.changed();
    }

    // What clients render.
    pub fn view(&self) -> View {
        View {
            active: self.active,
            workspaces: self
                .workspaces
                .iter()
                .map(|ws| WorkspaceView {
                    id: ws.id.clone(),
                    name: ws.name.clone(),
                    cwd: ws.cwd.clone(),
                    active: ws.active,
                    tabs: ws.tabs.iter().map(|t| TabView { id: t.id.clone(), name: t.name.clone(), tree: t.tree.clone(), focused: t.focused.clone(), zoomed: t.zoomed }).collect(),
                    git: ws.git.clone(),
                })
                .collect(),
            panes: self.panes.values().map(|p| p.info.clone()).collect(),
            plugins: None,
            paused: false,
        }
    }

    pub fn destroy(&mut self) {
        for p in self.panes.values_mut() {
            p.dispose();
        }
        self.panes.clear();
    }
}
