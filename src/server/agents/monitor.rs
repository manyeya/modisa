// Every 500ms: read each agent's state, tell attached clients when one needs attention, and type queued messages into
// agents that are idle.
use std::time::Duration;

use serde_json::json;

use super::detect::{pane_table, DetectPane, Detector};
use super::mailbox::Mailbox;
use crate::protocol::types::{AgentState, PaneInfo};
use crate::server::session::pane::{now_ms, PtyPane};
use crate::server::{Server, Shared};

impl DetectPane for PtyPane {
    fn id(&self) -> &str {
        &self.info.id
    }
    fn pid(&self) -> i32 {
        self.pid
    }
    fn disposed(&self) -> bool {
        self.disposed
    }
    fn info(&self) -> &PaneInfo {
        &self.info
    }
    fn info_mut(&mut self) -> &mut PaneInfo {
        &mut self.info
    }
    fn screen(&mut self) -> String {
        self.screen_text()
    }
    fn osc_title(&self) -> &str {
        &self.osc_title
    }
    fn osc_progress(&self) -> &str {
        &self.osc_progress
    }
    fn last_output(&self) -> u64 {
        self.last_output
    }
    fn generation(&self) -> Option<u64> {
        Some(self.generation)
    }
}

pub fn start(shared: Shared) {
    let me = std::rc::Rc::downgrade(&shared);
    tokio::task::spawn_local(async move {
        let mut every = tokio::time::interval(Duration::from_millis(500));
        loop {
            every.tick().await;
            let Some(s) = me.upgrade() else { return };
            tick(&s);
        }
    });
}

// a report just came in: look again now rather than at the next tick (once the request that brought it is done with
// the server)
pub fn tick_soon(srv: &Server) {
    let shared = srv.shared();
    tokio::task::spawn_local(async move { tick(&shared) });
}

// One look at every pane. A few system calls per pane and the screens that changed: nothing to wait on, so a tick never
// overlaps another.
pub fn tick(shared: &Shared) {
    let mut srv = shared.borrow_mut();
    if !srv.down {
        look(&mut srv);
        srv.settle();
    }
}

fn look(srv: &mut Server) {
    let panes: Vec<(i32, Option<i32>)> = srv.s.panes.values().filter(|p| p.info.running()).map(|p| (p.pid, p.foreground())).collect();
    let extra: Vec<i32> = srv.detector.authority.values().filter_map(|a| a.pid).collect();
    let procs = pane_table(&panes, &extra, &srv.adapters);
    let procs = &procs;
    let attached = !srv.attached().is_empty();
    let focus = srv.s.focused_id().filter(|id| attached && srv.s.is_visible(id));
    let focused = |id: &str| focus.as_deref() == Some(id);
    let adapters = srv.adapters.clone();
    let changes = {
        let Server { s, detector, .. } = srv;
        let detector: &mut Detector = detector;
        let mut panes: Vec<&mut dyn DetectPane> = s.panes.values_mut().map(|p| p as &mut dyn DetectPane).collect();
        detector.tick(procs, &adapters, &mut panes, &focused)
    };
    for c in &changes {
        let Some(pane) = srv.s.panes.get(&c.pane) else { continue };
        let info = pane.info.clone();
        let mut data = json!({ "pane": info.id, "instance": info.instance });
        if let Some(n) = &info.name {
            data["name"] = json!(n);
        }
        if let Some(a) = &info.agent {
            data["harness"] = json!(a.harness);
        }
        if let Some(f) = c.from {
            data["from"] = json!(f);
        }
        data["to"] = json!(c.to);
        srv.emit("agent.state", data);
        if c.to != AgentState::Idle && !focused(&info.id) {
            let what = match c.to {
                AgentState::Blocked => "is blocked — needs you",
                AgentState::Done => "is done",
                _ => "started working",
            };
            let who = info.name.as_ref().map(|n| format!("@{n}")).unwrap_or(info.title.clone());
            srv.broadcast("notify", json!({ "pane": info.id, "state": c.to, "text": format!("{who} {what}") }));
        }
    }
    if !srv.mail.paused {
        let now = now_ms();
        let ready: Vec<String> = srv
            .s
            .panes
            .values()
            .filter(|p| matches!(p.info.agent.as_ref().map(|a| a.state), Some(AgentState::Idle | AgentState::Done)))
            .filter(|p| srv.cooldown.get(p.id()).copied().unwrap_or(0) <= now)
            .map(|p| p.id().to_string())
            .collect();
        for id in ready {
            let Some(m) = srv.mail.pending(&id).into_iter().next() else { continue };
            let pane = &srv.s.panes[&id];
            pane.paste(&Mailbox::frame(&m));
            let (me, instance) = (srv.me.clone(), pane.info.instance.clone());
            let target = id.clone();
            tokio::task::spawn_local(async move {
                tokio::time::sleep(Duration::from_millis(150)).await;
                if let Some(s) = me.upgrade() {
                    if let Some(p) = s.borrow().s.panes.get(&target).filter(|p| p.info.instance == instance) {
                        p.write(b"\r");
                    }
                }
            });
            srv.mail.mark_delivered(m.id);
            srv.cooldown.insert(id.clone(), now + 4000);
            srv.emit("message.delivered", json!({ "id": m.id, "from": m.from_name, "to": m.to_name }));
        }
    }
    if !changes.is_empty() {
        srv.changed();
    }
}
