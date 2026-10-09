// Everything the user can do by name: prefix keys, the command palette, menus and buttons all run these. Each runs as a
// task of its own, so it can wait on a dialog; a failure is a toast.
use serde_json::{json, Map, Value};

use super::draw::tab_label;
use super::plugin_ui::{focused_target, plugin_key, plugin_keys, plugin_ui, run_plugin_action};
use super::slots;
use crate::config::keys::KeyState;
use super::modals::{self, confirm, list, menu, pick, prompt, ListButton, ListItem, ListOptions};
use super::{quit, reload, Shared};
use crate::config::keys::ACTION_IDS;
use crate::core::layout::panes as tree_panes;
use crate::core::paths::self_exe;
use crate::core::version::VERSION;
use crate::protocol::types::AgentState;

// What each action is called in the palette, the keyboard guide and menus.
pub fn label(id: &str) -> String {
    match id {
        "theme-picker" => "Change theme",
        "help" => "Keyboard guide",
        "pane-menu" => "Pane context menu",
        "pane-picker" => "Switch pane",
        "working-agents" => "Agents working",
        "blocked-agents" => "Agents that need you",
        "split-right" => "Split right",
        "split-down" => "Split down",
        "focus-left" => "Focus left",
        "focus-right" => "Focus right",
        "focus-up" => "Focus up",
        "focus-down" => "Focus down",
        "resize-left" => "Resize left",
        "resize-right" => "Resize right",
        "resize-up" => "Resize up",
        "resize-down" => "Resize down",
        "zoom" => "Zoom pane",
        "close-pane" => "Close pane",
        "close-tab" => "Close tab",
        "new-tab" => "New tab",
        "next-tab" => "Next tab",
        "prev-tab" => "Previous tab",
        "workspace-picker" => "Switch space",
        "new-workspace" => "New space",
        "new-agent" => "New agent pane",
        "toggle-sidebar" => "Toggle sidebar",
        "copy-mode" => "Copy mode / scrollback",
        "search" => "Search scrollback",
        "palette" => "Command palette",
        "settings" => "Settings",
        "plugins" => "Plugins…",
        "edit-config" => "Edit config.toml",
        "reload-config" => "Reload config",
        "update-modisa" => "Update modisa",
        "restart-server" => "Restart server (load updated modisa; panes are restored)",
        "toggle-messaging" => "Pause/resume agent messaging",
        "message-log" => "Message log",
        "send-message" => "Send message to agent",
        "rename-tab" => "Rename tab",
        "rename-pane" => "Rename pane (@name)",
        "rename-workspace" => "Rename space",
        "delete-workspace" => "Delete space",
        "detach" => "Detach",
        other => return other.strip_prefix("agent-").map(|n| format!("Jump to agent {n}")).unwrap_or_else(|| other.to_string()),
    }
    .to_string()
}

fn plural(n: usize, what: &str) -> String {
    format!("{n} {what}{}", if n == 1 { "" } else { "s" })
}

// UI events don't wait for actions: each runs as a task, and a failure while still connected is a toast.
pub fn run(shared: &Shared, id: &str) {
    {
        let app = shared.borrow();
        if app.quitting.is_some() || (id != "detach" && app.conn.as_ref().is_none_or(|c| c.closed())) {
            return;
        }
    }
    let (s, id) = (shared.clone(), id.to_string());
    tokio::task::spawn_local(async move {
        if let Err(e) = action(&s, &id).await {
            let mut app = s.borrow_mut();
            if app.quitting.is_none() && app.conn.as_ref().is_some_and(|c| !c.closed()) {
                let b = app.th.blocked;
                app.toast(&e, b);
            }
        }
    });
}

async fn request(shared: &Shared, method: &str, params: Value) -> Result<Value, String> {
    let conn = shared.borrow().conn.clone().ok_or("not connected")?;
    conn.request(method, params, None).await.map_err(|e| e.message)
}

fn call(shared: &Shared, name: &str, args: Value) {
    shared.borrow().call(name, args);
}

async fn agent_list(shared: &Shared) -> Result<Vec<(String, String)>, String> {
    let list = request(shared, "adapters", json!({})).await?;
    Ok(list.as_array().into_iter().flatten().map(|a| (a["name"].as_str().unwrap_or("").to_string(), a["id"].as_str().unwrap_or("").to_string())).collect())
}

// Beside the focused pane, to the right or below: asked each time, as the focused pane may be anything
async fn launch_agent(shared: &Shared, harness: &str) -> Result<(), String> {
    let items = vec![ListItem::new("Right", "split the focused pane side by side", "row").key("v"), ListItem::new("Down", "split the focused pane top and bottom", "col").key("-")];
    let Some(dir) = pick(shared, "Open it", items, None).await else { return Ok(()) };
    let name = prompt(shared, "Name the agent", "", "optional: @name lets agents message it").await;
    let name = name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
    call(shared, "spawnAgent", json!({ "harness": harness, "dir": dir, "name": name }));
    Ok(())
}

fn toast(shared: &Shared, text: &str, color: fn(&crate::config::themes::Theme) -> &'static str) {
    let mut app = shared.borrow_mut();
    let c = color(&app.th);
    app.toast(text, c);
}

pub async fn action(shared: &Shared, id: &str) -> Result<(), String> {
    match id {
        "theme-picker" => Ok(modals::settings(shared, "theme").await),
        "help" => {
            // each with its action's id: what [keys] in config.toml rebinds; then plugins' keys as this client's config
            // binds them
            let (items, prefix) = {
                let app = shared.borrow();
                let mut items: Vec<ListItem> = app.bindings.iter().map(|(key, action)| ListItem::new(label(action), *action, *action).key(key.clone())).collect();
                items.extend(plugin_keys(&app).into_iter().map(|k| {
                    let active = k.state == KeyState::Active;
                    let d = &k.declared;
                    let off = if active { String::new() } else { format!("off: {}", k.reason.clone().unwrap_or_default()) };
                    ListItem::new(format!("{}: {}", d.plugin, d.description), off, if active { format!("plugin-key:{}", d.key) } else { String::new() }).key(if d.key.is_empty() { "–".to_string() } else { d.key.clone() })
                }));
                (items, format!("prefix {}", app.cfg.prefix))
            };
            let chosen = pick(shared, "Keyboard", items, Some(prefix)).await;
            if let Some(key) = chosen.as_deref().and_then(|a| a.strip_prefix("plugin-key:")) {
                plugin_key(&mut shared.borrow_mut(), key);
            } else if let Some(a) = chosen.filter(|a| a != "help" && !a.is_empty()) {
                run(shared, &a);
            }
            Ok(())
        }
        "pane-menu" => {
            let (pane, x, y) = {
                let app = shared.borrow();
                (app.tab().focused.clone(), (app.width - 34).min(app.area().x + 3), app.area().y + 1)
            };
            context_menu(shared, &pane, x, y);
            Ok(())
        }
        "pane-picker" => {
            let items: Vec<ListItem> = {
                let app = shared.borrow();
                app.view.as_ref().unwrap().panes.iter().map(|p| ListItem::new(p.name.as_ref().map(|n| format!("@{n}")).unwrap_or(p.title.clone()), format!("{} · {} · {}", p.id, p.agent.as_ref().map(|a| a.state.as_str()).unwrap_or(&p.status), p.cwd), &p.id)).collect()
            };
            if let Some(id) = pick(shared, "Panes", items, None).await {
                call(shared, "focusPane", json!({ "pane": id }));
            }
            Ok(())
        }
        "working-agents" => pick_agents(shared, AgentState::Working, "Working agents").await,
        "blocked-agents" => pick_agents(shared, AgentState::Blocked, "Agents that need you").await,
        "split-right" => Ok(call(shared, "split", json!({ "dir": "row" }))),
        "split-down" => Ok(call(shared, "split", json!({ "dir": "col" }))),
        "focus-left" | "focus-right" | "focus-up" | "focus-down" => Ok(call(shared, "focusDir", json!({ "dir": &id[6..] }))),
        "resize-left" | "resize-right" | "resize-up" | "resize-down" => Ok(call(shared, "resize", json!({ "dir": &id[7..] }))),
        "zoom" => Ok(call(shared, "zoom", json!({}))),
        "close-pane" => Ok(call(shared, "close", json!({}))),
        "close-tab" => {
            let (ids, agents, name) = {
                let app = shared.borrow();
                let ids = tree_panes(&app.tab().tree);
                let agents = ids.iter().filter(|id| app.info(id).is_some_and(|p| p.agent.is_some())).count();
                (ids.len(), agents, tab_label(&app, app.tab()))
            };
            // ponytail: asks only when agents would die; plain shells close straight away
            if agents > 0 && !confirm(shared, "Close tab", &format!("Close \"{name}\"?\nThis closes its {}, {agents} running an agent.", plural(ids, "pane")), "close").await {
                return Ok(());
            }
            Ok(call(shared, "closeTab", json!({})))
        }
        // named, so the sidebar's graph can group its agents under it (Enter keeps the suggestion, Esc cancels)
        "new-tab" => {
            let n = shared.borrow().ws().tabs.len() + 1;
            if let Some(name) = prompt(shared, "New tab", &format!("tab {n}"), "what's in it, e.g. backend or review").await {
                let name = name.trim().to_string();
                call(shared, "newTab", json!({ "name": (!name.is_empty()).then_some(name) }));
            }
            Ok(())
        }
        "next-tab" => Ok(call(shared, "cycleTab", json!({ "step": 1 }))),
        "prev-tab" => Ok(call(shared, "cycleTab", json!({ "step": -1 }))),
        "workspace-picker" => workspace_picker(shared).await,
        "new-workspace" => {
            let (n, cwd) = {
                let app = shared.borrow();
                (app.view.as_ref().unwrap().workspaces.len() + 1, app.ws().cwd.clone())
            };
            if let Some(name) = prompt(shared, "New space", &format!("space {n}"), "").await.filter(|n| !n.trim().is_empty()) {
                call(shared, "newWorkspace", json!({ "cwd": cwd, "name": name.trim() }));
            }
            Ok(())
        }
        "new-agent" => {
            let items = agent_list(shared).await?.into_iter().map(|(name, id)| ListItem::new(name, id.clone(), id)).collect();
            if let Some(h) = pick(shared, "New agent", items, None).await {
                launch_agent(shared, &h).await?;
            }
            Ok(())
        }
        "toggle-sidebar" => {
            let mut app = shared.borrow_mut();
            app.sidebar = !app.sidebar;
            let area = app.area();
            app.notify_server("area", json!({ "area": area }));
            app.dirty();
            Ok(())
        }
        "copy-mode" => {
            let mut app = shared.borrow_mut();
            app.copy_mode = true;
            app.dirty();
            Ok(())
        }
        "search" => {
            let Some(q) = prompt(shared, "Search scrollback", "", "text to find in the focused pane").await.filter(|q| !q.is_empty()) else { return Ok(()) };
            let pane = shared.borrow().tab().focused.clone();
            let res = request(shared, "search", json!({ "pane": pane, "query": q })).await?;
            let found: Vec<usize> = res["matches"].as_array().into_iter().flatten().filter_map(|m| m.as_u64().map(|m| m as usize)).collect();
            if found.is_empty() {
                toast(shared, &format!("no match for \"{q}\""), |th| th.warn);
                return Ok(());
            }
            let mut app = shared.borrow_mut();
            let i = found.len() - 1;
            app.search = Some(super::Search { total: res["total"].as_u64().unwrap_or(0) as usize, matches: found, i });
            app.copy_mode = true;
            super::input::jump(&mut app);
            Ok(())
        }
        "palette" => {
            let agents = agent_list(shared).await?;
            let entries = slots::entries(&shared.borrow(), "palette", |_| true);
            let items: Vec<ListItem> = {
                let app = shared.borrow();
                let mut items: Vec<ListItem> = ACTION_IDS.iter().filter(|k| **k != "palette" && !k.starts_with("agent-")).map(|k| ListItem::new(label(k), "", *k).key(app.key_for(k).unwrap_or_default())).collect();
                items.extend(agents.into_iter().map(|(name, id)| ListItem::new(format!("New agent: {name}"), id.clone(), format!("agent:{id}"))));
                for p in plugin_ui(&app) {
                    let (name, run) = (p["plugin"].as_str().unwrap_or(""), p["run"].as_str().unwrap_or(""));
                    for a in p["actions"].as_array().into_iter().flatten() {
                        let desc = a["description"].as_str().unwrap_or("plugin action");
                        items.push(ListItem::new(format!("{name}: {}", a["title"].as_str().unwrap_or("")), desc, format!("plugin:{name}:{run}:{}", a["id"].as_str().unwrap_or(""))));
                    }
                }
                // plugins' entries (the palette slot), each with its line beside it
                let ctx = slots::ctx(&app);
                for (i, p) in entries.0.iter().enumerate() {
                    let line = slots::lines(&ctx, p).into_iter().next();
                    let text: String = line.iter().flat_map(|l| l.spans.iter().map(|s| s.content.as_ref())).collect();
                    items.push(ListItem { line, ..ListItem::new(format!("{}: {}", p.plugin, p.title.as_deref().unwrap_or(&p.id)), text, format!("slot:{i}")) });
                }
                items.push(ListItem::new("Kill session", "close every pane and stop the server", "kill"));
                items
            };
            let Some(v) = pick(shared, "Commands", items, None).await else { return Ok(()) };
            if let Some(p) = entries.chosen(&v) {
                let app = shared.borrow();
                slots::run(&app, p, Map::new(), focused_target(&app)); // for the pane focused now
                return Ok(());
            }
            if let Some(h) = v.strip_prefix("agent:") {
                return launch_agent(shared, h).await;
            }
            if let Some(rest) = v.strip_prefix("plugin:") {
                let parts: Vec<&str> = rest.splitn(3, ':').collect();
                let app = shared.borrow();
                let target = focused_target(&app); // the target is the pane focused now, not when the action finishes
                run_plugin_action(&app, parts[0], parts.get(1).unwrap_or(&""), parts.get(2).unwrap_or(&""), json!({}), target, None, None);
                return Ok(());
            }
            if v == "kill" {
                request(shared, "kill", json!({})).await?;
                return Ok(());
            }
            run(shared, &v);
            Ok(())
        }
        "settings" => Ok(modals::settings(shared, "theme").await),
        "plugins" => {
            super::plugins::open(shared, "menu").await;
            Ok(())
        }
        "edit-config" => {
            let path = crate::config::ensure_config_file().map_err(|e| e.to_string())?;
            let editor = std::env::var("EDITOR").ok().filter(|e| !e.is_empty()).unwrap_or_else(|| "vi".into());
            Ok(call(shared, "newTab", json!({ "name": "settings", "command": format!("{editor} {path}"), "ephemeral": true })))
        }
        "reload-config" => {
            reload(&mut shared.borrow_mut(), true);
            Ok(())
        }
        "update-modisa" => {
            let found = match shared.borrow().update.clone() {
                Some(m) => Some(m),
                None => None,
            };
            let found = match found {
                Some(m) => Some(m),
                None => crate::cli::update::check_for_update(true).await,
            };
            let Some(m) = found else {
                toast(shared, &format!("modisa {VERSION} is up to date"), |th| th.done);
                return Ok(());
            };
            let managed = crate::cli::update::update_command(); // Homebrew or mise installed it: their command, not ours
            if managed != "modisa update" {
                toast(shared, &format!("modisa {} is out: run {managed}, then modisa restart", m.version), |th| th.warn);
                return Ok(());
            }
            let notes: Vec<String> = m.notes.trim().lines().filter(|l| !l.is_empty()).take(6).map(|l| super::design::fit(l, 60)).collect();
            let mut body = vec![format!("modisa {VERSION} → {}", m.version)];
            if !notes.is_empty() {
                body.push(String::new());
                body.extend(notes);
            }
            body.extend(["".to_string(), "Downloads it, then restarts the server; agents resume.".to_string()]);
            if confirm(shared, &format!("Update to {}", m.version), &body.join("\n"), "update and restart").await {
                let cmd = self_exe();
                call(shared, "newTab", json!({ "name": "update", "command": format!("{cmd} update && {cmd} restart"), "ephemeral": true }));
            }
            Ok(())
        }
        "restart-server" => {
            {
                let mut app = shared.borrow_mut();
                app.restarted_by_us = true;
                app.restarting = true;
            }
            let _ = request(shared, "restart", json!({})).await;
            Ok(())
        }
        "toggle-messaging" => Ok(call(shared, "pause", json!({}))),
        "message-log" => Ok(call(shared, "newTab", json!({ "name": "messages", "command": format!("{} messages --follow", self_exe()), "ephemeral": true }))),
        "send-message" => {
            let items: Vec<ListItem> = {
                let app = shared.borrow();
                app.view.as_ref().unwrap().panes.iter().filter(|p| p.agent.is_some() || p.harness.is_some()).map(|p| ListItem::new(p.name.as_ref().map(|n| format!("@{n}")).unwrap_or(p.title.clone()), p.agent.as_ref().map(|a| a.state.as_str()).unwrap_or(""), &p.id)).collect()
            };
            if items.is_empty() {
                toast(shared, "no agent panes", |th| th.warn);
                return Ok(());
            }
            let Some(to) = pick(shared, "Send to", items, None).await else { return Ok(()) };
            let Some(body) = prompt(shared, "Message", "", "what to tell the agent").await.filter(|b| !b.is_empty()) else { return Ok(()) };
            match request(shared, "send", json!({ "to": to, "body": body })).await {
                Ok(_) => toast(shared, "queued", |th| th.fg),
                Err(e) => toast(shared, &e, |th| th.blocked),
            }
            Ok(())
        }
        "rename-tab" => {
            let name = {
                let app = shared.borrow();
                tab_label(&app, app.tab())
            };
            if let Some(n) = prompt(shared, "Rename tab", &name, "").await {
                call(shared, "renameTab", json!({ "name": n.trim() }));
            }
            Ok(())
        }
        "rename-pane" => {
            let name = {
                let app = shared.borrow();
                app.info(&app.tab().focused).and_then(|p| p.name.clone()).unwrap_or_default()
            };
            if let Some(n) = prompt(shared, "Rename pane", &name, "").await {
                call(shared, "renamePane", json!({ "name": n }));
            }
            Ok(())
        }
        "rename-workspace" => {
            let i = shared.borrow().view.as_ref().unwrap().active;
            rename_space(shared, i).await;
            Ok(())
        }
        "delete-workspace" => {
            let i = shared.borrow().view.as_ref().unwrap().active;
            delete_space(shared, i).await;
            Ok(())
        }
        "detach" => {
            quit(&mut shared.borrow_mut(), "detached");
            Ok(())
        }
        other => {
            if let Some(n) = other.strip_prefix("agent-").and_then(|n| n.parse::<usize>().ok()) {
                let id = shared.borrow().sorted_agents().get(n - 1).map(|p| p.id.clone());
                if let Some(id) = id {
                    call(shared, "focusPane", json!({ "pane": id }));
                }
            }
            Ok(())
        }
    }
}

// Only the agents in one state; picking one jumps to it.
async fn pick_agents(shared: &Shared, state: AgentState, title: &str) -> Result<(), String> {
    let items: Vec<ListItem> = {
        let app = shared.borrow();
        let view = app.view.as_ref().unwrap();
        let place = |id: &str| view.workspaces.iter().find(|w| w.tabs.iter().any(|t| tree_panes(&t.tree).iter().any(|p| p == id))).map(|w| w.name.clone()).unwrap_or_default();
        app.sorted_agents().into_iter().filter(|p| p.agent.as_ref().unwrap().state == state).map(|p| ListItem::new(p.name.as_ref().map(|n| format!("@{n}")).unwrap_or(p.title.clone()), format!("{} · {}", p.agent.as_ref().unwrap().harness, place(&p.id)), &p.id)).collect()
    };
    if items.is_empty() {
        toast(shared, if state == AgentState::Working { "no agents are working" } else { "no agents need you" }, |th| th.dim);
        return Ok(());
    }
    if let Some(id) = pick(shared, title, items, None).await {
        call(shared, "focusPane", json!({ "pane": id }));
    }
    Ok(())
}

async fn workspace_picker(shared: &Shared) -> Result<(), String> {
    let items: Vec<ListItem> = {
        let app = shared.borrow();
        let view = app.view.as_ref().unwrap();
        let spaces = &view.workspaces;
        let mut items: Vec<ListItem> = spaces
            .iter()
            .enumerate()
            .map(|(i, w)| {
                let panes: usize = w.tabs.iter().map(|t| tree_panes(&t.tree).len()).sum();
                let git = w.git.as_ref().map(|g| format!("⎇ {} · ", g.branch)).unwrap_or_default();
                let me = app.me.clone();
                let context: std::rc::Rc<dyn Fn(i32, i32)> = std::rc::Rc::new(move |x, y| {
                    if let Some(s) = me.upgrade() {
                        space_menu(&s, i, x, y);
                    }
                });
                // the last space can't be deleted, so it offers no ✕
                let mut buttons = vec![ListButton { icon: "✎".into(), value: format!("rename:{i}"), key: "r".into(), title: "rename".into(), danger: false }];
                if spaces.len() > 1 {
                    buttons.push(ListButton { icon: "✕".into(), value: format!("delete:{i}"), key: "d".into(), title: "delete".into(), danger: true });
                }
                ListItem { buttons, context: Some(context), ..ListItem::new(format!("{}{}", if i == view.active { "● " } else { "  " }, w.name), format!("{git}{} · {}", plural(w.tabs.len(), "tab"), plural(panes, "pane")), i.to_string()) }
            })
            .collect();
        items.push(ListItem::new("+ new space", "A fresh group of tabs and panes", "new"));
        items
    };
    let Some(v) = list(shared, ListOptions { title: "Spaces".into(), items, ..Default::default() }).await else { return Ok(()) };
    match v.split_once(':') {
        Some(("rename", i)) => rename_space(shared, i.parse().unwrap_or(0)).await,
        Some(("delete", i)) => delete_space(shared, i.parse().unwrap_or(0)).await,
        _ if v == "new" => run(shared, "new-workspace"),
        _ => call(shared, "selectWorkspace", json!({ "index": v.parse::<usize>().unwrap_or(0) })),
    }
    Ok(())
}

// Renaming and deleting spaces, from the space picker's right-click menu, keys or the palette.
pub async fn rename_space(shared: &Shared, index: usize) {
    let Some(current) = shared.borrow().view.as_ref().and_then(|v| v.workspaces.get(index).map(|w| w.name.clone())) else { return };
    if let Some(name) = prompt(shared, "Rename space", &current, "").await.filter(|n| !n.trim().is_empty()) {
        call(shared, "renameWorkspace", json!({ "index": index, "name": name }));
    }
}

pub async fn delete_space(shared: &Shared, index: usize) {
    let (name, count, agents, only) = {
        let app = shared.borrow();
        let view = app.view.as_ref().unwrap();
        let Some(target) = view.workspaces.get(index) else { return };
        let ids: Vec<String> = target.tabs.iter().flat_map(|t| tree_panes(&t.tree)).collect();
        let agents = ids.iter().filter(|id| app.info(id).is_some_and(|p| p.agent.is_some())).count();
        (target.name.clone(), ids.len(), agents, view.workspaces.len() < 2)
    };
    if only {
        return toast(shared, "can't delete the only space", |th| th.warn);
    }
    let what = format!("{count} pane{}{}", if count == 1 { "" } else { "s" }, if agents > 0 { format!(", {agents} running an agent") } else { String::new() });
    if confirm(shared, "Delete space", &format!("Delete space \"{name}\"?\nThis closes its {what}."), "delete").await {
        call(shared, "closeWorkspace", json!({ "index": index }));
    }
}

// Opened from a click or a key, while the App may still be borrowed: it runs once that's over.
pub fn space_menu(shared: &Shared, index: usize, x: i32, y: i32) {
    let s = shared.clone();
    tokio::task::spawn_local(async move { space_menu_now(&s, index, x, y) });
}

fn space_menu_now(shared: &Shared, index: usize, x: i32, y: i32) {
    let Some(name) = shared.borrow().view.as_ref().and_then(|v| v.workspaces.get(index).map(|w| w.name.clone())) else { return };
    if shared.borrow().modal.is_some() {
        return;
    }
    let entries = slots::entries(&shared.borrow(), "menu.space", |p| p.space.as_deref() == Some(name.as_str()));
    let s = shared.clone();
    tokio::task::spawn_local(async move {
        // plugins' entries: theirs before go first, the rest before Delete
        let (before, after) = (entries.named(true), entries.named(false));
        let mut options: Vec<(&str, &str, &str, bool)> = before.iter().map(|(n, v)| (n.as_str(), "", v.as_str(), false)).collect();
        options.extend([("Switch to space", "Enter", "switch", false), ("Rename space", "r", "rename", false)]);
        options.extend(after.iter().map(|(n, v)| (n.as_str(), "", v.as_str(), false)));
        options.push(("Delete space", "d", "delete", true));
        let chosen = menu(&s, &format!("Space · {name}"), options, x, y).await;
        if let Some(p) = chosen.as_deref().and_then(|v| entries.chosen(v)) {
            return slots::run(&s.borrow(), p, Map::from_iter([("space".to_string(), json!(name))]), None);
        }
        match chosen.as_deref() {
            Some("switch") => call(&s, "selectWorkspace", json!({ "index": index })),
            Some("rename") => rename_space(&s, index).await,
            Some("delete") => delete_space(&s, index).await,
            _ => {}
        }
    });
}

// Right-click on a tab: it becomes the active one, then rename, close, or its focused pane's menu.
// Opened from a click or a key, while the App may still be borrowed: it runs once that's over.
pub fn tab_menu(shared: &Shared, index: usize, x: i32, y: i32) {
    let s = shared.clone();
    tokio::task::spawn_local(async move { tab_menu_now(&s, index, x, y) });
}

fn tab_menu_now(shared: &Shared, index: usize, x: i32, y: i32) {
    let Some(t) = ({
        let app = shared.borrow();
        if app.modal.is_some() {
            return;
        }
        app.ws().tabs.get(index).map(|t| (tab_label(&app, t), t.focused.clone(), t.id.clone()))
    }) else {
        return;
    };
    let entries = slots::entries(&shared.borrow(), "menu.tab", |p| p.tab.as_deref() == Some(t.2.as_str()));
    let s = shared.clone();
    tokio::task::spawn_local(async move {
        if request(&s, "cmd", json!({ "name": "selectTab", "args": { "index": index } })).await.is_err() {
            return;
        }
        // plugins' entries: theirs before go first, the rest before Close tab
        let (before, after) = (entries.named(true), entries.named(false));
        let mut options: Vec<(&str, &str, &str, bool)> = before.iter().map(|(n, v)| (n.as_str(), "", v.as_str(), false)).collect();
        options.extend([("Rename tab", "r", "rename-tab", false), ("Pane menu", "p", "pane", false)]);
        options.extend(after.iter().map(|(n, v)| (n.as_str(), "", v.as_str(), false)));
        options.push(("Close tab", "x", "close-tab", true));
        let chosen = menu(&s, &format!("Tab · {}", t.0), options, x, y).await;
        if let Some(p) = chosen.as_deref().and_then(|v| entries.chosen(v)) {
            return slots::run(&s.borrow(), p, Map::from_iter([("tab".to_string(), json!(t.2))]), None);
        }
        match chosen.as_deref() {
            Some("pane") => context_menu(&s, &t.1, x, y),
            Some(a) => run(&s, a),
            None => {}
        }
    });
}

// Right-click on a pane or tab: the pane's actions.
// Opened from a click or a key, while the App may still be borrowed: it runs once that's over.
pub fn context_menu(shared: &Shared, pane: &str, x: i32, y: i32) {
    let s = shared.clone();
    let pane = pane.to_string();
    tokio::task::spawn_local(async move { context_menu_now(&s, &pane, x, y) });
}

fn context_menu_now(shared: &Shared, pane: &str, x: i32, y: i32) {
    let Some(title) = ({
        let app = shared.borrow();
        if app.modal.is_some() || !app.views.is_empty() {
            return;
        }
        app.info(pane).map(|p| (p.name.as_ref().map(|n| format!("@{n}")).unwrap_or(p.title.clone()), app.sidebar, p.instance.clone()))
    }) else {
        return;
    };
    // plugins' entries, before Close pane: each runs its plugin's action for this pane's process
    let extra: Vec<(String, String)> = plugin_ui(&shared.borrow())
        .iter()
        .flat_map(|p| {
            let (name, run) = (p["plugin"].as_str().unwrap_or("").to_string(), p["run"].as_str().unwrap_or("").to_string());
            p["menu"].as_array().into_iter().flatten().map(move |m| (format!("{name}: {}", m["title"].as_str().unwrap_or("")), format!("plugin:{name}:{run}:{}", m["action"].as_str().unwrap_or(""))))
        })
        .collect();
    let entries = {
        let app = shared.borrow();
        let info = app.info(pane);
        slots::entries(&app, "menu.pane", |p| info.is_some_and(|i| slots::of_pane(p, i)))
    };
    let (s, pane) = (shared.clone(), pane.to_string());
    tokio::task::spawn_local(async move {
        // plugins' entries: theirs before go first, the rest before Close pane
        let (before, after) = (entries.named(true), entries.named(false));
        let mut options: Vec<(&str, &str, &str, bool)> = before.iter().map(|(n, v)| (n.as_str(), "", v.as_str(), false)).collect();
        options.extend([
            ("Focus pane", "Enter", "focus", false),
            ("Split right", "v", "split-right", false),
            ("Split down", "-", "split-down", false),
            ("Zoom / restore", "z", "zoom", false),
            ("Rename pane", ".", "rename-pane", false),
            ("Copy visible output", "y", "copy-output", false),
            ("Search scrollback", "/", "search", false),
            ("New tab", "c", "new-tab", false),
            ("Launch agent", "a", "new-agent", false),
            ("All panes", "o", "pane-picker", false),
            (if title.1 { "Hide sidebar" } else { "Show sidebar" }, "b", "toggle-sidebar", false),
            ("Change theme", "t", "theme-picker", false),
        ]);
        options.extend(extra.iter().map(|(n, v)| (n.as_str(), "", v.as_str(), false)));
        options.extend(after.iter().map(|(n, v)| (n.as_str(), "", v.as_str(), false)));
        options.push(("Close pane", "x", "close-pane", true));
        let Some(action) = menu(&s, &title.0, options, x, y).await else { return };
        if s.borrow().info(&pane).is_none() {
            return;
        }
        if let Some(p) = entries.chosen(&action) {
            let target = Some((pane.clone(), title.2.clone())).filter(|(_, i)| !i.is_empty());
            let on = Map::from_iter([("pane".to_string(), json!(pane)), ("instance".to_string(), json!(title.2))]);
            return slots::run(&s.borrow(), p, on, target);
        }
        // a plugin's entry runs its action for this pane
        if let Some(rest) = action.strip_prefix("plugin:") {
            let parts: Vec<&str> = rest.splitn(3, ':').collect();
            let target = Some((pane.clone(), title.2.clone())).filter(|(_, i)| !i.is_empty());
            run_plugin_action(&s.borrow(), parts[0], parts.get(1).unwrap_or(&""), parts.get(2).unwrap_or(&""), json!({}), target, None, None);
            return;
        }
        // Complete target selection before running actions that use the active pane.
        if let Err(e) = request(&s, "cmd", json!({ "name": "focusPane", "args": { "pane": pane } })).await {
            return toast(&s, &e, |th| th.blocked);
        }
        match action.as_str() {
            "focus" => {}
            "copy-output" => {
                let text = s.borrow().panes.get(&pane).map(|p| p.screen.screen_text());
                if let Some(t) = text.filter(|t| !t.is_empty()) {
                    s.borrow().copy(&t);
                    toast(&s, "Visible output copied", |th| th.focus);
                }
            }
            a => run(&s, a),
        }
    });
}

pub fn _close(shared: &Shared) {
    modals::close(&mut shared.borrow_mut(), None);
}
