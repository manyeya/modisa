// What a format's {variables} are (CUSTOMIZE.md, Formats), from what the client knows: the session, the space and its
// git, the counts, the time; then, where a format is about one, a tab or a pane (its agent, state, metadata).
use crate::client::format::{clock, sh, Var};
use crate::client::App;
use crate::core::layout::panes as tree_panes;
use crate::protocol::types::{AgentState, PaneInfo, TabView};

fn text(s: impl Into<String>) -> Option<Var> {
    Some(Var::Text(s.into()))
}

fn count(n: usize) -> Option<Var> {
    text(n.to_string())
}

// modisa's own status buttons, as formats so a status format can place them
fn button(name: &str, app: &App) -> Option<String> {
    let side = app.side_width() > 0;
    let (w, b) = app.counts();
    Some(match name {
        "sidebar_button" => format!("#[click=toggle-sidebar]{}#[/click]", if side { " ◧ sidebar " } else { " ◨ sidebar " }),
        "agent_button" => "#[click=new-agent] + agent #[/click]".into(),
        "working_button" => format!("#[click=working-agents] {} {w} working #[/click]", app.icon(AgentState::Working)),
        "needyou_button" => format!("#[click=blocked-agents]{} {} {b} need you #[/click]", if b > 0 { "#[$warn]" } else { "" }, app.icon(AgentState::Blocked)),
        "panes_button" => format!("#[click=pane-picker] {} panes #[/click]", tree_panes(&app.tab().tree).len()),
        _ => return None,
    })
}

// The variables every format has.
pub fn common(app: &App, name: &str, arg: Option<&str>) -> Option<Var> {
    if let Some(f) = button(name, app) {
        return Some(Var::Format(f));
    }
    let ws = app.ws();
    let git = ws.git.as_ref();
    let (working, blocked) = app.counts();
    let state_count = |s: AgentState| app.sorted_agents().iter().filter(|p| p.agent.as_ref().is_some_and(|a| a.state == s)).count();
    match name {
        "session" => text(app.opts.session.clone()),
        "space" => text(ws.name.clone()),
        "spaces" => count(app.view.as_ref().map_or(0, |v| v.workspaces.len())),
        "tabs" => count(ws.tabs.len()),
        "panes" => count(tree_panes(&app.tab().tree).len()),
        "working" => count(working),
        "blocked" => count(blocked),
        "done" => count(state_count(AgentState::Done)),
        "idle" => count(state_count(AgentState::Idle)),
        "mode" => text(app.mode.clone().unwrap_or_default()),
        "prefix" => text(if app.prefix_armed { "on" } else { "" }),
        "git.branch" => text(git.map(|g| g.branch.clone()).unwrap_or_default()),
        "git.repo" => text(git.map(|g| g.repo.clone()).unwrap_or_default()),
        "git.ahead" => count(git.and_then(|g| g.ahead).unwrap_or(0) as usize),
        "git.behind" => count(git.and_then(|g| g.behind).unwrap_or(0) as usize),
        "git.changes" => count(git.map_or(0, |g| g.changes as usize)),
        "git.clean" => text(if git.is_some_and(|g| g.changes == 0 && g.ahead.unwrap_or(0) == 0 && g.behind.unwrap_or(0) == 0) { "yes" } else { "" }),
        "theme" => text(crate::config::themes::active_name(&app.cfg).to_string()),
        "host" => text(hostname()),
        "user" => text(std::env::var("USER").unwrap_or_default()),
        "clock" => text(clock(arg.unwrap_or("%H:%M"))),
        "date" => text(clock(arg.unwrap_or("%Y-%m-%d"))),
        "sh" => text(sh(arg.unwrap_or(""))),
        n if n.starts_with("plugin.") => {
            // plugin.<name>.<id>: a plugin's status piece, as its text
            let (plugin, id) = n["plugin.".len()..].split_once('.')?;
            let p = crate::client::slots::pieces(app, "status.right", |p| p.plugin == plugin && p.id == id).into_iter().next().or_else(|| crate::client::slots::pieces(app, "status.left", |p| p.plugin == plugin && p.id == id).into_iter().next())?;
            text(crate::client::slots::lines(&crate::client::slots::ctx(app), p).into_iter().next().map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>()).unwrap_or_default())
        }
        _ => None,
    }
}

fn hostname() -> String {
    let mut buf = [0u8; 256];
    if unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } != 0 {
        return String::new();
    }
    let s = String::from_utf8_lossy(&buf[..buf.iter().position(|b| *b == 0).unwrap_or(buf.len())]).into_owned();
    s.split('.').next().unwrap_or("").to_string()
}

// A pane's variables, then everyone's.
pub fn pane(app: &App, p: &PaneInfo, name: &str, arg: Option<&str>) -> Option<Var> {
    let agent = p.agent.as_ref();
    match name {
        "pane" => text(p.id.clone()),
        "pane.index" => count(tree_panes(&app.tab().tree).iter().position(|id| *id == p.id).map_or(0, |i| i + 1)),
        "name" => text(p.name.clone().map(|n| format!("@{n}")).unwrap_or_else(|| p.title.clone())),
        "title" => text(p.title.clone()),
        "terminal_title" => text(p.terminal_title.clone().unwrap_or_default()),
        "cwd" => text(match arg {
            Some("short") => p.cwd.replacen(&*crate::core::paths::HOME, "~", 1).rsplit('/').next().unwrap_or("").to_string(),
            _ => p.cwd.replacen(&*crate::core::paths::HOME, "~", 1),
        }),
        "command" => text(p.command.clone().unwrap_or_default()),
        "agent" | "agent.id" => text(agent.map(|a| a.harness.clone()).unwrap_or_default()),
        "state" => text(agent.map(|a| a.state.as_str()).unwrap_or("")),
        "icon" => text(agent.map(|a| app.icon(a.state)).unwrap_or("")),
        "zoomed" => text(if app.tab().zoomed && app.tab().focused == p.id { "yes" } else { "" }),
        "exited" => text(if p.status == "exited" { p.exit_code.map_or("?".into(), |c| c.to_string()) } else { String::new() }),
        n if n.starts_with("meta.") => text(p.meta.get(&n[5..]).cloned().unwrap_or_default()),
        _ => common(app, name, arg),
    }
}

// A tab's variables, then its focused pane's, then everyone's.
pub fn tab(app: &App, t: &TabView, index: usize, name: &str, arg: Option<&str>) -> Option<Var> {
    let blocked = tree_panes(&t.tree).iter().filter(|id| app.info(id).and_then(|p| p.agent.as_ref()).is_some_and(|a| a.state == AgentState::Blocked)).count();
    match name {
        "tab" => text(crate::client::draw::tab_label(app, t)),
        "tab.index" => count(index + 1),
        "panes" => count(tree_panes(&t.tree).len()),
        "blocked" => count(blocked),
        "zoomed" => text(if t.zoomed { "yes" } else { "" }),
        "unread" => text(if t.unread { "yes" } else { "" }),
        _ => match app.info(&t.focused) {
            Some(p) => pane(app, p, name, arg),
            None => common(app, name, arg),
        },
    }
}
