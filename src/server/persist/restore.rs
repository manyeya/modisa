// Rebuild a saved session on server start.
use std::collections::HashMap;
use std::time::Duration;

use super::store::Saved;
use super::template::quote;
use crate::core::layout::{panes as tree_panes, Node};
use crate::protocol::conn::RpcResult;
use crate::server::session::SpawnOpts;
use crate::server::Server;

// Rebuild the saved layout. Every pane comes back as a shell in its old cwd; agents are resumed in it, plain commands
// are typed but not run (re-running a deploy on reboot would be rude). Whether anything was restored.
pub fn restore(srv: &mut Server, data: &Saved) -> RpcResult<bool> {
    let mut ids: HashMap<String, String> = HashMap::new();
    let mut follow_ups: Vec<(String, String, bool)> = vec![];
    let opts = |old: &str| -> SpawnOpts {
        match data.panes.get(old) {
            Some(sp) => SpawnOpts { cwd: Some(sp.cwd.clone()), name: sp.name.clone(), created_by: Some(sp.created_by.clone()), env: sp.env.clone(), ..Default::default() },
            None => SpawnOpts::default(),
        }
    };
    let mut after = |srv: &Server, old: &str, id: &str, ids: &mut HashMap<String, String>| {
        ids.insert(old.into(), id.into());
        let Some(sp) = data.panes.get(old) else { return };
        let agent = srv.adapters.iter().find(|a| Some(&a.id) == sp.harness.as_ref().or(sp.agent.as_ref()));
        match agent {
            Some(agent) => {
                // the exact session its integration reported, else the agent's "latest session", else a fresh start
                let exact = match (&agent.resume_session, &sp.session) {
                    (Some(rs), Some(ses)) if ses.agent == agent.id => Some(rs.replace("{id}", &quote(&ses.id))),
                    _ => None,
                };
                follow_ups.push((id.into(), exact.or_else(|| agent.resume.clone()).unwrap_or_else(|| agent.launch.clone()), true));
            }
            None => {
                if let Some(c) = &sp.command {
                    follow_ups.push((id.into(), c.clone(), false));
                }
            }
        }
    };
    // Replay a tree: first leaf opens the tab, every split re-creates its right/bottom subtree.
    fn build(srv: &mut Server, node: &Node, at: &str, opts: &dyn Fn(&str) -> SpawnOpts, after: &mut dyn FnMut(&Server, &str, &str, &mut HashMap<String, String>), ids: &mut HashMap<String, String>) -> RpcResult<()> {
        let Node::Split { dir, a, b, .. } = node else { return Ok(()) };
        let first_b = tree_panes(b)[0].clone();
        let Some(p) = srv.s.split(*dir, opts(&first_b), Some(at), false, 0.5)? else { return Ok(()) };
        after(srv, &first_b, &p, ids);
        build(srv, a, at, opts, after, ids)?;
        build(srv, b, &p, opts, after, ids)
    }
    let workspaces: Vec<_> = data.workspaces.iter().filter(|w| !w.tabs.is_empty()).collect();
    let selected = data.workspaces.get(data.active).map(|w| w as *const _);
    for w in &workspaces {
        for (ti, t) in w.tabs.iter().enumerate() {
            let first = tree_panes(&t.tree)[0].clone();
            let p = if ti == 0 { srv.s.new_workspace(Some(w.name.clone()), Some(w.cwd.clone()), opts(&first))? } else { srv.s.new_tab(t.name.clone(), opts(&first))? };
            after(srv, &first, &p, &mut ids);
            build(srv, &t.tree, &p, &opts, &mut after, &mut ids)?;
            let wi = srv.s.active;
            let ti = srv.s.workspaces[wi].active;
            let tab = &mut srv.s.workspaces[wi].tabs[ti];
            tab.name = t.name.clone();
            restore_ratios(&mut tab.tree, &t.tree);
            if let Some(f) = ids.get(&t.focused) {
                tab.focused = f.clone();
            }
            tab.zoomed = t.zoomed;
        }
        let wi = srv.s.active;
        let n = srv.s.workspaces[wi].tabs.len();
        srv.s.workspaces[wi].active = w.active.min(n.saturating_sub(1));
    }
    srv.s.active = workspaces.iter().position(|w| Some(*w as *const _) == selected).unwrap_or(0);
    srv.s.layout();
    let me = srv.me.clone();
    tokio::task::spawn_local(async move {
        tokio::time::sleep(Duration::from_millis(300)).await; // let the shells print their prompts first
        let Some(s) = me.upgrade() else { return };
        let srv = s.borrow();
        for (id, cmd, run) in follow_ups {
            if let Some(p) = srv.s.panes.get(&id) {
                p.write(format!("{cmd}{}", if run { "\r" } else { "" }).as_bytes());
            }
        }
    });
    Ok(!workspaces.is_empty())
}

fn restore_ratios(live: &mut Node, saved: &Node) {
    if let (Node::Split { ratio, a, b, .. }, Node::Split { ratio: r, a: sa, b: sb, .. }) = (live, saved) {
        *ratio = *r;
        restore_ratios(a, sa);
        restore_ratios(b, sb);
    }
}
