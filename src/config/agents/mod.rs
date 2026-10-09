// Every agent modisa knows: the process names that identify it, how to launch and resume it, and the screen rules that
// read its state. The rules in ./manifests are third-party detection manifests (Apache-2.0, see ./manifests/LICENSE),
// kept as close to upstream as possible so they can be re-synced; `extra` adds rules of ours on top. Embedded so the
// binary carries them.
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

pub mod brands;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RuleState {
    Idle,
    Working,
    Blocked,
    #[default]
    Unknown,
}

// A screen rule (the manifest schema) or a nested gate: every direct matcher must hold, every `all` gate, at least one
// `any` gate (when present), and no `not` gate. The highest priority wins.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RawGate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all: Option<Vec<RawGate>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub any: Option<Vec<RawGate>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not: Option<Vec<RawGate>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contains: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regex: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_regex: Option<Vec<String>>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RawRule {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<RuleState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>, // which part of the screen, e.g. bottom_non_empty_lines(8), after_last_horizontal_rule, osc_title
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_idle: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_blocker: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_working: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_state_update: Option<bool>, // an agent-owned viewer (transcript, model picker): keep the last state
    #[serde(flatten)]
    pub gate: RawGate,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDef {
    pub id: String, // stable: saved sessions, [agents.<id>] config and integrations refer to it
    pub name: String,
    pub process: Vec<String>, // executable names (lowercase, without .exe/.js); the first is what we launch
    pub launch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<String>, // relaunch after a restart when no exact session id was reported
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_session: Option<String>, // relaunch into the exact session an integration reported: {id} is replaced
    #[serde(default)]
    pub activity: bool, // no rules matched: output in the last 2s counts as working
    pub rules: Vec<RawRule>,
}

// A manifest file: its rules are all we read (id, version, aliases are upstream bookkeeping).
#[derive(Deserialize)]
struct Manifest {
    #[serde(default)]
    rules: Vec<RawRule>,
}

fn parse(name: &str, toml_text: &str) -> Vec<RawRule> {
    match toml::from_str::<Manifest>(toml_text) {
        Ok(m) => m.rules,
        Err(e) => panic!("built-in manifest {name}: {e}"), // embedded at build time: a test parses every one
    }
}

// A manifest's rules, then ours on top (`extra` is manifest-format TOML).
fn rules(manifest: (&str, &str), extra: &str) -> Vec<RawRule> {
    let mut r = parse(manifest.0, manifest.1);
    r.extend(parse("extra", extra));
    r
}

macro_rules! manifest {
    ($name:literal) => {
        ($name, include_str!(concat!("manifests/", $name, ".toml")))
    };
}

// Codex's rate-limit "switch model" picker ends in "esc to go back", which the manifest misses.
const CODEX_EXTRA: &str = r#"
[[rules]]
id = "modisa_confirm_go_back"
state = "blocked"
priority = 900
region = "bottom_non_empty_lines(3)"
visible_blocker = true
contains = ["press enter to confirm or esc to go back"]
"#;

const AIDER_RULES: &str = r#"
[[rules]]
id = "confirm"
state = "blocked"
priority = 20
regex = ['(?im)Do you want to|Allow (this|command)|\(y/n\)|\[y/N\]']

[[rules]]
id = "working"
state = "working"
priority = 10
regex = ['(?im)esc to (interrupt|cancel)']
"#;

// builder helpers, so the table below reads like the original's one line per agent
fn def(id: &str, name: &str, process: &[&str], launch: &str) -> AgentDef {
    AgentDef {
        id: id.into(),
        name: name.into(),
        process: process.iter().map(|p| p.to_string()).collect(),
        launch: launch.into(),
        resume: None,
        resume_session: None,
        activity: false,
        rules: vec![],
    }
}

// (a private trait, not inherent methods: an `impl AgentDef` elsewhere can't clash with these names)
trait Build {
    fn session(self, cmd: &str) -> Self;
    fn resume(self, cmd: &str) -> Self;
    fn activity(self) -> Self;
    fn rules(self, rules: Vec<RawRule>) -> Self;
}

impl Build for AgentDef {
    fn session(mut self, cmd: &str) -> Self {
        self.resume_session = Some(cmd.into());
        self
    }
    fn resume(mut self, cmd: &str) -> Self {
        self.resume = Some(cmd.into());
        self
    }
    fn activity(mut self) -> Self {
        self.activity = true;
        self
    }
    fn rules(mut self, rules: Vec<RawRule>) -> Self {
        self.rules = rules;
        self
    }
}

static BUILTIN_AGENTS: LazyLock<Vec<AgentDef>> = LazyLock::new(|| {
    let m = |manifest| rules(manifest, "");
    vec![
        def("claude-code", "Claude Code", &["claude", "claude-code"], "claude")
            .session("claude --resume {id}")
            .resume("claude --continue")
            .rules(m(manifest!("claude"))),
        def("codex", "Codex", &["codex"], "codex")
            .session("codex resume {id}")
            .resume("codex resume --last")
            .rules(rules(manifest!("codex"), CODEX_EXTRA)),
        def("gemini", "Gemini CLI", &["gemini"], "gemini").resume("gemini --resume").rules(m(manifest!("gemini"))),
        def("cursor-agent", "Cursor Agent", &["cursor-agent", "cursor"], "cursor-agent")
            .session("cursor-agent --resume {id}")
            .rules(m(manifest!("cursor"))),
        def("copilot", "Copilot CLI", &["copilot", "github-copilot", "ghcs"], "copilot")
            .session("copilot --resume={id}")
            .rules(m(manifest!("github-copilot"))),
        def("opencode", "OpenCode", &["opencode", "opencode2", "open-code"], "opencode")
            .session("opencode --session {id}")
            .resume("opencode --continue")
            .rules(m(manifest!("opencode"))),
        def("pi", "Pi", &["pi"], "pi").session("pi --session {id}").resume("pi --continue").rules(m(manifest!("pi"))),
        def("omp", "OMP", &["omp"], "omp").session("omp --resume={id}").activity(), // state comes from its extension
        def("droid", "Droid", &["droid"], "droid").session("droid --resume {id}").rules(m(manifest!("droid"))),
        def("amp", "Amp", &["amp", "amp-local"], "amp").rules(m(manifest!("amp"))),
        def("kiro", "Kiro CLI", &["kiro-cli", "kiro"], "kiro-cli").rules(m(manifest!("kiro"))),
        def("kimi", "Kimi Code", &["kimi", "kimi-code"], "kimi")
            .session("kimi --session {id}")
            .rules(m(manifest!("kimi"))),
        def("kilo", "Kilo Code", &["kilo", "kilo-code"], "kilo")
            .session("kilo --session {id}")
            .rules(m(manifest!("kilo"))),
        def("devin", "Devin CLI", &["devin", "devin-cli"], "devin")
            .session("devin --resume {id}")
            .rules(m(manifest!("devin"))),
        def("grok", "Grok CLI", &["grok", "grok-build"], "grok")
            .session("grok --resume {id}")
            .rules(m(manifest!("grok"))),
        def("hermes", "Hermes Agent", &["hermes", "hermes-agent"], "hermes")
            .session("hermes --resume {id}")
            .rules(m(manifest!("hermes"))),
        def("qodercli", "Qoder CLI", &["qodercli", "qoderclicn", "qoder", "qodercn"], "qodercli")
            .session("qodercli --resume {id}")
            .rules(m(manifest!("qodercli"))),
        def("qwen", "Qwen Code", &["qwen", "qwen-code"], "qwen")
            .session("qwen --resume {id}")
            .rules(m(manifest!("qwen"))),
        def("antigravity", "Antigravity CLI", &["agy", "antigravity", "antigravity-cli"], "agy")
            .session("agy --conversation {id}")
            .rules(m(manifest!("antigravity"))),
        def("cline", "Cline", &["cline"], "cline").rules(m(manifest!("cline"))),
        def("mastracode", "MastraCode", &["mastracode", "mastra-code"], "mastracode")
            .session("mastracode --thread {id}")
            .activity(), // state comes from its hooks
        def("maki", "Maki", &["maki"], "maki").rules(m(manifest!("maki"))),
        def("muse", "Muse", &["muse", "muse-code", "muse-cli"], "muse").rules(m(manifest!("muse"))),
        def("aider", "Aider", &["aider"], "aider").activity().rules(parse("aider", AIDER_RULES)),
        // any agent spawned by name that modisa doesn't know: output in the last 2s = working
        def("generic", "Agent", &[], "").activity(),
    ]
});

// Every built-in agent (parsed once; each call hands out a copy the caller may override).
pub fn builtin_agents() -> Vec<AgentDef> {
    BUILTIN_AGENTS.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_carry_their_manifests() {
        let all = builtin_agents();
        assert_eq!(all.len(), 25);
        let ids: std::collections::HashSet<_> = all.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids.len(), all.len()); // ids are unique
        let codex = all.iter().find(|a| a.id == "codex").unwrap();
        let extra = codex.rules.last().unwrap();
        assert_eq!(extra.id, "modisa_confirm_go_back");
        assert_eq!(
            (extra.state, extra.priority, extra.visible_blocker),
            (Some(RuleState::Blocked), Some(900), Some(true))
        );
        assert_eq!(extra.gate.contains.as_deref(), Some(&["press enter to confirm or esc to go back".to_string()][..]));
        // every agent with a manifest got its rules; nested gates survive the flattening
        for a in &all {
            assert_eq!(a.rules.is_empty(), ["omp", "mastracode", "generic"].contains(&a.id.as_str()), "{}", a.id);
        }
        let claude = all.iter().find(|a| a.id == "claude-code").unwrap();
        assert!(claude.rules.iter().any(|r| r.gate.any.as_ref().is_some_and(|g| !g.is_empty())));
        assert!(claude.rules.iter().any(|r| r.gate.not.as_ref().is_some_and(|g| !g.is_empty())));
        assert_eq!(
            all.iter().find(|a| a.id == "aider").unwrap().rules[0].gate.regex.as_deref().unwrap()[0],
            r"(?im)Do you want to|Allow (this|command)|\(y/n\)|\[y/N\]"
        );
    }
}
