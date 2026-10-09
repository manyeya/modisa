// Data that crosses the wire: what the server sends clients, and what the API returns.
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::layout::Node;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentState {
    Working,
    Blocked,
    Done,
    Idle,
}

impl AgentState {
    pub fn as_str(self) -> &'static str {
        match self {
            AgentState::Working => "working",
            AgentState::Blocked => "blocked",
            AgentState::Done => "done",
            AgentState::Idle => "idle",
        }
    }
    pub fn parse(s: &str) -> Option<AgentState> {
        serde_json::from_value(Value::String(s.into())).ok()
    }
}

// A failed request's stable code (JSON-RPC error.data.code); the CLI maps some to exit statuses.
pub const ERROR_CODES: &[&str] = &[
    "error", "usage", "unreachable", "timeout", "invalid_params", "unknown_method", "no_such_pane", "pane_gone", "no_such_plugin", "no_such_action", "plugin_unavailable", "plugin_error", "already_running", "rate_limited", "ui_busy",
];

// An agent integration on the server's machine: whether it's installed and current, and whether the agent is there at
// all (on PATH, or its config directory exists).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IntegrationStatus {
    pub id: String,
    pub name: String,
    pub kind: String, // "lifecycle" | "session"
    pub status: String, // "current" | "outdated" | "none"
    pub available: bool,
    pub configured: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentInfo {
    pub harness: String,
    pub state: AgentState,
    pub source: String, // "hook" | "screen"
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionRef {
    pub agent: String,
    pub id: String,
    pub source: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaneInfo {
    pub id: String,
    #[serde(default)]
    pub instance: String, // random per spawned process: tells a pane apart from a later one given the same id or name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal_title: Option<String>, // the title the program last set (OSC 0/2)
    pub cwd: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>, // set for process/agent panes; none = interactive shell
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>, // adapter id when spawned as an agent
    #[serde(default)]
    pub created_by: String, // pane id or "user"
    #[serde(default)]
    pub status: String, // "running" | "exited"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionRef>, // the agent's own session, for exact resume
    pub cols: u16,
    pub rows: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub popup: Option<bool>, // a plugin's popup: no place in the layout, shown only by the client that opened it
    #[serde(skip_serializing_if = "Option::is_none")]
    pub takeover: Option<bool>, // driven from another terminal (pane attach)
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub muted: bool, // its agent makes no sound and no system notification
}

impl PaneInfo {
    pub fn running(&self) -> bool {
        self.status == "running"
    }
}

// What a target is matched against: a pane's id, instance and name.
pub trait PaneRef {
    fn id(&self) -> &str;
    fn instance(&self) -> &str;
    fn name(&self) -> Option<&str>;
}

impl PaneRef for PaneInfo {
    fn id(&self) -> &str {
        &self.id
    }
    fn instance(&self) -> &str {
        &self.instance
    }
    fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

// a pane as JSON (a snapshot the CLI reads)
impl PaneRef for Value {
    fn id(&self) -> &str {
        self["id"].as_str().unwrap_or("")
    }
    fn instance(&self) -> &str {
        self["instance"].as_str().unwrap_or("")
    }
    fn name(&self) -> Option<&str> {
        self["name"].as_str()
    }
}

// "p3:1a2b3c4d": one instance of a pane
pub fn instance_target(target: &str) -> Option<(&str, &str)> {
    let (id, inst) = target.split_once(':')?;
    let is_id = id.len() > 1 && id.starts_with('p') && id[1..].bytes().all(|b| b.is_ascii_digit());
    (is_id && !inst.is_empty() && inst.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')).then_some((id, inst))
}

// The pane a target names: "p3", "@coder", "coder", "@p3" (ids still work once a pane is named), "p3:1a2b3c4d" (only that
// instance of p3). The server resolves targets with it, and so does the CLI where it works from a snapshot.
pub fn find_pane<'a, T: PaneRef>(panes: impl IntoIterator<Item = &'a T> + Clone, target: &str) -> Option<&'a T> {
    if let Some((id, inst)) = instance_target(target) {
        return panes.into_iter().find(|p| p.id() == id && p.instance() == inst);
    }
    let name = target.strip_prefix('@').unwrap_or(target);
    let by = |f: &dyn Fn(&T) -> bool| panes.clone().into_iter().find(|p| f(p));
    by(&|p| p.id() == target).or_else(|| by(&|p| p.name() == Some(name))).or_else(|| by(&|p| p.id() == name))
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TabView {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub tree: Node,
    pub focused: String,
    pub zoomed: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unread: bool, // an agent in it needed you or finished, and the tab hasn't been looked at since
}

// A space's repository, where its focused pane is: ahead/behind are only there when the branch has an upstream.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GitView {
    pub repo: String,
    pub branch: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ahead: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub behind: Option<u32>,
    pub changes: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceView {
    pub id: String,
    pub name: String,
    pub cwd: String,
    pub active: usize,
    pub tabs: Vec<TabView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git: Option<GitView>,
}

// What a plugin's current run shows in the TUI, from its ui.* calls: drawn by modisa, in the user's theme. Kept as the
// JSON the server checked; the client reads what it draws from it.
pub type PluginUiView = Value;

// The plugin UI a client understands, sent with attach: the server sends plugins' UI only to clients at this version or
// later, so an older client never gets what it can't draw. 3: views as examples/plugins/VIEWS.md says them.
pub const PLUGIN_UI: u32 = 3;

// Everything a client needs to draw the session.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct View {
    pub active: usize,
    pub workspaces: Vec<WorkspaceView>,
    pub panes: Vec<PaneInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugins: Option<Vec<PluginUiView>>,
    #[serde(default)]
    pub paused: bool,
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_panes() {
        let panes: Vec<Value> = serde_json::from_str(r#"[{"id":"p1","instance":"aaaa"},{"id":"p2","instance":"bbbb","name":"coder"},{"id":"p3","instance":"cccc","name":"p1"}]"#).unwrap();
        let find = |t: &str| find_pane(&panes, t).map(|p| p.id().to_string());
        assert_eq!(find("p1").as_deref(), Some("p1"));
        assert_eq!(find("@coder").as_deref(), Some("p2"));
        assert_eq!(find("coder").as_deref(), Some("p2"));
        assert_eq!(find("@p2").as_deref(), Some("p2"));
        assert_eq!(find("@p1").as_deref(), Some("p3"));
        assert_eq!(find("p2:bbbb").as_deref(), Some("p2"));
        assert_eq!(find("p2:zzzz"), None);
    }
}
