// `modisa integration status | install | uninstall <agent|all>`: connect agents to modisa through
// their own hooks or plugins (see ./targets.rs for what each one does).
pub mod edit;
pub mod hook;
pub mod plugins;
pub mod targets;

use serde_json::Value;

use crate::protocol::conn::{error, RpcResult};
use crate::protocol::types::IntegrationStatus;
use crate::skills::SKILL;
use targets::{skill_dir, skills_home, Env, Status, Target, LIFECYCLE};

pub use targets::TARGETS;

// The skill: one copy in the shared skills directory, symlinked into every agent that reads skills
// somewhere else. Agents whose skills directory *is* the shared one just find it there.
// ponytail: symlinks only. An agent that doesn't follow them gets nothing; add a copy fallback if one
// turns up.
fn link_path(t: &Target, e: &Env) -> Option<String> {
    let skills = (t.skills?)(e);
    (skills != skills_home(e)).then(|| format!("{skills}/modisa"))
}
// A directory of their own, as opposed to our symlink (lstat, so a link to a directory is not one).
fn is_dir(path: &str) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
}
fn links_to(link: &str, target: &str) -> bool {
    std::fs::read_link(link).is_ok_and(|p| p.as_os_str() == target)
}

// Status runs for every target on every settings refresh, so this stays free of subprocesses.
fn skill_status(t: &Target, e: &Env) -> Option<Status> {
    t.skills?;
    if std::fs::read_to_string(format!("{}/SKILL.md", skill_dir(e))).unwrap_or_default() != SKILL {
        return Some(Status::None);
    }
    Some(match link_path(t, e) {
        None => Status::Current,
        Some(link) if links_to(&link, &skill_dir(e)) => Status::Current,
        Some(_) => Status::None,
    })
}

fn set_skill(t: &Target, e: &Env, install: bool) -> Result<(), String> {
    let Some(skills) = t.skills else { return Ok(()) };
    let link = link_path(t, e);
    if install {
        edit::write(&format!("{}/SKILL.md", skill_dir(e)), SKILL)?;
        // A real directory there is someone's own copy of the skill, not ours: leave it be rather than
        // link inside it. Status keeps saying "update available" until they take it out.
        if let Some(link) = link.filter(|l| !is_dir(l)) {
            // `mkdir -p && ln -sfn`, failures ignored
            let _ = std::fs::create_dir_all(skills(e));
            let _ = std::fs::remove_file(&link);
            let _ = std::os::unix::fs::symlink(skill_dir(e), &link);
        }
        return Ok(());
    }
    if let Some(link) = link.filter(|l| !is_dir(l)) {
        let _ = std::fs::remove_file(link);
    }
    // The shared copy goes when the last agent that wanted it does.
    let others: Vec<Status> = TARGETS.iter().filter(|o| o.skills.is_some() && o.id != t.id).map(|o| o.status(e).unwrap_or(Status::None)).collect();
    if !others.contains(&Status::Current) && !others.contains(&Status::Outdated) {
        let _ = std::fs::remove_dir_all(skill_dir(e));
    }
    Ok(())
}

// Names people type for an agent, besides its id.
const ALIASES: &[(&str, &str)] = &[("claude", "claude-code"), ("cursor", "cursor-agent"), ("agy", "antigravity"), ("antigravity-cli", "antigravity"), ("kilo-code", "kilo"), ("qoder", "qodercli")];
fn find(name: &str) -> Option<&'static Target> {
    let id = ALIASES.iter().find(|(alias, _)| *alias == name).map_or(name, |(_, id)| id);
    TARGETS.iter().find(|t| t.id == id)
}

fn status_of(t: &Target, e: &Env) -> IntegrationStatus {
    let configured = std::path::Path::new(&(t.dir)(e)).is_dir();
    let hooks = t.status(e).unwrap_or(Status::Outdated);
    // A missing or stale skill makes an otherwise-installed integration an update, not a fresh install.
    let skill = skill_status(t, e);
    IntegrationStatus {
        id: t.id.into(),
        name: t.name.into(),
        kind: t.kind.into(),
        configured,
        available: configured || t.binaries.iter().any(|b| e.which(b)),
        status: if hooks == Status::Current && skill == Some(Status::None) { Status::Outdated } else { hooks }.as_str().into(),
    }
}

pub fn integration_status() -> Vec<IntegrationStatus> {
    status_in(&Env::current())
}
fn status_in(e: &Env) -> Vec<IntegrationStatus> {
    TARGETS.iter().map(|t| status_of(t, e)).collect()
}

// Recommended: agents you have (on PATH or set up) whose integration is missing or out of date.
pub fn recommended(list: &[IntegrationStatus]) -> Vec<&IntegrationStatus> {
    list.iter().filter(|s| s.status == "outdated" || (s.available && s.status == "none")).collect()
}

// The line to show the user, as the RPC result.
pub fn set_integration(id: &str, install: bool) -> RpcResult<Value> {
    set_in(&Env::current(), id, install).map(Value::String).map_err(error)
}
fn set_in(e: &Env, id: &str, install: bool) -> Result<String, String> {
    let t = find(id).ok_or_else(|| format!("no integration for {id}"))?;
    if install && !t.create && !status_of(t, e).configured {
        return Err(format!("{} isn't set up here (no {}); run it once, then install", t.name, (t.dir)(e)));
    }
    if install { t.install(e) } else { t.uninstall(e) }?;
    set_skill(t, e, install)?;
    let what = format!("{}{}", if t.kind == LIFECYCLE { "state and session reports" } else { "session reports" }, if t.skills.is_some() { " and the modisa skill" } else { "" });
    Ok(if install { format!("{}: installed {what} (restart running {} sessions to load it)", t.name, t.name) } else { format!("{}: removed", t.name) })
}

// Everything modisa put into agents' configs, for `modisa uninstall`: each installed integration,
// then any skill link or shared skill copy an earlier install left without its hooks.
// Returns (removed, failed): a line for each.
pub fn uninstall_all() -> (Vec<String>, Vec<String>) {
    uninstall_all_in(&Env::current())
}
fn uninstall_all_in(e: &Env) -> (Vec<String>, Vec<String>) {
    let (mut removed, mut failed) = (vec![], vec![]);
    for s in status_in(e) {
        if s.status == "none" {
            continue;
        }
        match set_in(e, &s.id, false) {
            Ok(line) => removed.push(line),
            Err(m) => failed.push(format!("{}: {m}", s.name)),
        }
    }
    for t in TARGETS.iter() {
        if let Some(link) = link_path(t, e).filter(|l| links_to(l, &skill_dir(e))) {
            let _ = std::fs::remove_file(link);
        }
    }
    let _ = std::fs::remove_dir_all(skill_dir(e));
    (removed, failed)
}

fn label(status: &str) -> &'static str {
    match status {
        "current" => "✓ installed",
        "outdated" => "↻ update available",
        _ => "not installed",
    }
}

// `modisa integration <verb> <agent>`: the CLI passes its first two words, as the original's main.ts
// does (`runIntegration(rest[0], rest[1])`). There the result was dropped, so the command always
// exited 0; this returns 0, 1 (an install failed) or 2 (usage), for the CLI to use or not.
pub async fn run_integration(verb: Option<&str>, agent: Option<&str>) -> i32 {
    run_in(&Env::current(), verb, agent)
}
fn run_in(e: &Env, verb: Option<&str>, agent: Option<&str>) -> i32 {
    if verb == Some("status") {
        for s in status_in(e) {
            let note = if s.status == "none" && !s.available { "  (not found)" } else { "" };
            println!("{:<16} {:<20} {}{note}", s.name, label(&s.status), if s.kind == LIFECYCLE { "state + session" } else { "session" });
        }
        return 0;
    }
    let install = verb == Some("install");
    let agent = match agent.filter(|a| !a.is_empty()) {
        Some(a) if (install || verb == Some("uninstall")) && (a == "all" || find(a).is_some()) => a,
        _ => {
            let ids: Vec<&str> = TARGETS.iter().map(|t| t.id).collect();
            let lifecycle: Vec<&str> = TARGETS.iter().filter(|t| t.kind == LIFECYCLE).map(|t| t.name).collect();
            eprintln!("usage: modisa integration status | install|uninstall <agent|all>\nagents: {}\nevery agent's state is read from its screen with no setup; integrations add session resume, and exact state for {}.", ids.join(", "), lifecycle.join(", "));
            return 2;
        }
    };
    let all = if agent == "all" { status_in(e) } else { vec![] };
    let ids: Vec<String> = if agent != "all" {
        vec![agent.into()]
    } else if install {
        recommended(&all).iter().map(|s| s.id.clone()).collect()
    } else {
        all.iter().filter(|s| s.status != "none").map(|s| s.id.clone()).collect()
    };
    if ids.is_empty() {
        println!("{}", if install { "nothing to install: every agent found here is up to date" } else { "no integrations installed" });
    }
    let mut failed = 0;
    for id in ids {
        match set_in(e, &id, install) {
            Ok(line) => println!("{line}"),
            Err(m) => {
                failed += 1;
                eprintln!("{m}");
            }
        }
    }
    if failed > 0 { 1 } else { 0 }
}

// The e2e suite's `modisa integration` checks, against a home of its own: HOME is a temp directory,
// no agent overrides and no PATH, so nothing here can reach the real agents' configs.
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    struct Sandbox {
        root: String,
        env: Env,
    }

    impl Sandbox {
        fn new(name: &str) -> Sandbox {
            let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
            let root = std::env::temp_dir().join(format!("modisa-integrations-{name}-{}-{nanos}", std::process::id())).to_string_lossy().into_owned();
            let home = format!("{root}/home");
            std::fs::create_dir_all(&home).unwrap();
            Sandbox { root, env: Env { home: home.clone(), vars: HashMap::from([("HOME".to_string(), home)]) } }
        }
        fn path(&self, p: &str) -> String {
            format!("{}/{p}", self.env.home)
        }
        fn write(&self, p: &str, text: &str) {
            edit::write(&self.path(p), text).unwrap();
        }
        fn text(&self, p: &str) -> String {
            std::fs::read_to_string(self.path(p)).unwrap()
        }
        fn json(&self, p: &str) -> serde_json::Value {
            serde_json::from_str(&self.text(p)).unwrap()
        }
        fn link_of(&self, p: &str) -> String {
            std::fs::read_link(self.path(p)).map(|l| l.to_string_lossy().into_owned()).unwrap_or_default()
        }
        fn exists(&self, p: &str) -> bool {
            std::path::Path::new(&self.path(p)).exists()
        }
        fn status(&self) -> HashMap<String, &'static str> {
            status_in(&self.env).into_iter().map(|s| (s.name, label(&s.status))).collect()
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn command_of(v: &serde_json::Value) -> &str {
        v.as_str().unwrap_or_default()
    }

    #[test]
    fn install_all_status_and_uninstall_across_every_agents_config_format() {
        let sb = Sandbox::new("all");
        let e = &sb.env;
        // every agent set up, with settings of the user's own (and an older modisa's hooks for Claude and Codex)
        let mine = json!({ "hooks": [{ "type": "command", "command": "echo mine" }] });
        sb.write(".claude/settings.json", &json!({ "model": "opus", "hooks": { "Stop": [mine, { "hooks": [{ "type": "command", "command": "MODISA_HOOK=1 modisa report --state done" }] }] } }).to_string());
        sb.write(".codex/config.toml", "model = \"o3\"\nnotify = [\"modisa\",\"report\",\"--state\",\"done\"] # MODISA_HOOK=1\n");
        sb.write(".copilot/settings.json", "{}");
        sb.write(".cursor/hooks.json", &json!({ "version": 1, "hooks": { "stop": [{ "command": "echo mine" }] } }).to_string());
        sb.write(".config/devin/config.json", "{}");
        sb.write(".factory/settings.json", "{}");
        sb.write(".qoder/settings.json", "{}");
        sb.write(".qwen/settings.json", "{}");
        sb.write(".grok/config.toml", "");
        sb.write(".gemini/config/hooks.json", &json!({ "mine": { "Stop": [] } }).to_string());
        sb.write(".hermes/config.yaml", "model: x\n");
        sb.write(".kimi-code/config.toml", "theme = \"dark\"\n");
        for dir in [".config/opencode", ".config/kilo", ".pi/agent", ".omp/agent"] {
            std::fs::create_dir_all(sb.path(dir)).unwrap();
        }

        let s = sb.status();
        assert_eq!(s["Claude Code"], "↻ update available"); // the older modisa's hooks
        assert_eq!(s["Pi"], "not installed");

        // "all" installs what's here: every configured agent (MastraCode isn't, so it's left out)
        assert_eq!(run_in(e, Some("install"), Some("all")), 0);
        assert_eq!(sb.status()["MastraCode"], "not installed");
        assert_eq!(run_in(e, Some("install"), Some("mastracode")), 0);
        let s = sb.status();
        assert!(s.values().all(|v| *v == "✓ installed"), "{s:?}");
        assert_eq!(s.len(), 17);

        // each agent's own format, the user's settings kept
        let claude = sb.json(".claude/settings.json");
        assert_eq!(claude["model"], "opus");
        assert_eq!(claude["hooks"]["Stop"], json!([mine])); // the old state hook is gone
        let start = &claude["hooks"]["SessionStart"][0];
        assert_eq!((&start["matcher"], &start["hooks"][0]["type"], &start["hooks"][0]["timeout"]), (&json!("*"), &json!("command"), &json!(10)));
        let command = command_of(&start["hooks"][0]["command"]);
        assert!(command.starts_with("MODISA_HOOK=2 ") && command.ends_with(" hook claude-code session"), "{command}");
        assert!(command_of(&sb.json(".codex/hooks.json")["hooks"]["SessionStart"][0]["hooks"][0]["command"]).contains("hook codex session"));
        assert_eq!(sb.text(".codex/config.toml"), "model = \"o3\"\n\n[features]\nhooks = true\n");
        let copilot = &sb.json(".copilot/settings.json")["hooks"]["SessionStart"][0];
        assert_eq!((&copilot["type"], &copilot["timeoutSec"]), (&json!("command"), &json!(10)));
        assert!(command_of(&copilot["bash"]).contains("hook copilot session"));
        let cursor = sb.json(".cursor/hooks.json");
        assert_eq!((&cursor["version"], &cursor["hooks"]["stop"]), (&json!(1), &json!([{ "command": "echo mine" }])));
        assert!(command_of(&cursor["hooks"]["sessionStart"][0]["command"]).contains("hook cursor-agent session"));
        let devin: Vec<String> = sb.json(".config/devin/config.json")["hooks"].as_object().unwrap().keys().cloned().collect();
        assert_eq!(devin, ["SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse", "PermissionRequest", "Stop"]);
        let qwen = &sb.json(".qwen/settings.json")["hooks"]["SessionStart"][0];
        assert_eq!((&qwen["matcher"], &qwen["hooks"][0]["timeout"]), (&json!("*"), &json!(10000)));
        assert!(command_of(&sb.json(".grok/hooks/modisa.json")["hooks"]["SessionStart"][0]["hooks"][0]["command"]).contains("hook grok session"));
        let agy = sb.json(".gemini/config/hooks.json");
        assert_eq!(agy["mine"], json!({ "Stop": [] }));
        assert!(command_of(&agy["modisa"]["PreInvocation"][0]["command"]).contains("hook antigravity session"));
        let kimi = sb.text(".kimi-code/config.toml");
        assert!(kimi.starts_with("theme = \"dark\"\n\n# >>> modisa kimi integration"));
        assert_eq!(kimi.matches("[[hooks]]").count(), 12);
        let kimi_toml: toml::Table = toml::from_str(&kimi).unwrap();
        assert_eq!(kimi_toml["theme"].as_str(), Some("dark"));
        assert!(sb.json(".mastracode/hooks.json").as_object().unwrap().contains_key("PermissionRequest"));
        assert_eq!(sb.text(".hermes/config.yaml"), "model: x\nplugins:\n  enabled:\n    - modisa-agent-state\n");
        assert!(sb.text(".hermes/plugins/modisa-agent-state/__init__.py").contains("ctx.register_hook(\"on_session_start\""));
        assert!(sb.text(".config/opencode/plugins/modisa-agent-state.js").contains("export const ModisaAgentState"));
        assert!(sb.text(".config/kilo/plugin/modisa-agent-state.js").contains("const AGENT = \"kilo\""));
        assert!(sb.text(".pi/agent/extensions/modisa-agent-state.ts").contains("pi.on(\"agent_settled\""));
        assert!(sb.text(".omp/agent/extensions/modisa-omp-agent-state.ts").contains("pi.on(\"tool_approval_requested\""));

        // one skill in the shared directory, symlinked into each agent that reads skills of its own
        let shared = sb.path(".agents/skills/modisa");
        assert!(sb.text(".agents/skills/modisa/SKILL.md").contains("modisa pane split"));
        for link in [".claude/skills/modisa", ".codex/skills/modisa", ".config/opencode/skills/modisa", ".pi/agent/skills/modisa"] {
            assert_eq!(sb.link_of(link), shared, "{link}");
        }
        // these two don't keep skills in their config directory, and Kimi reads the shared one directly
        assert_eq!(sb.link_of(".kilo/skills/modisa"), shared);
        assert_eq!(sb.link_of(".gemini/antigravity-cli/skills/modisa"), shared);
        assert_eq!(sb.link_of(".kimi-code/skills/modisa"), "");
        // and the two agents whose skills directory we can't confirm get none
        assert_eq!(sb.link_of(".mastracode/skills/modisa"), "");
        assert_eq!(sb.link_of(".omp/agent/skills/modisa"), "");

        // reinstalling changes nothing; an edited file is outdated
        set_in(e, "claude", true).unwrap();
        assert_eq!(sb.json(".claude/settings.json")["hooks"]["SessionStart"].as_array().unwrap().len(), 1);
        sb.write(".pi/agent/extensions/modisa-agent-state.ts", "// edited");
        assert_eq!(sb.status()["Pi"], "↻ update available");

        // a stale skill is an update too, even when the hooks are current
        sb.write(".agents/skills/modisa/SKILL.md", "# old");
        assert_eq!(sb.status()["Claude Code"], "↻ update available");
        set_in(e, "claude", true).unwrap();
        assert_eq!(sb.status()["Claude Code"], "✓ installed");

        // uninstall takes out only modisa's parts
        assert_eq!(run_in(e, Some("uninstall"), Some("all")), 0);
        let s = sb.status();
        assert!(s.values().all(|v| *v == "not installed"), "{s:?}");
        assert_eq!(sb.json(".claude/settings.json"), json!({ "model": "opus", "hooks": { "Stop": [mine] } }));
        assert_eq!(sb.json(".cursor/hooks.json"), json!({ "version": 1, "hooks": { "stop": [{ "command": "echo mine" }] } }));
        assert_eq!(sb.json(".gemini/config/hooks.json"), json!({ "mine": { "Stop": [] } }));
        assert_eq!(sb.text(".kimi-code/config.toml"), "theme = \"dark\"\n\n");
        assert_eq!(sb.text(".hermes/config.yaml"), "model: x\nplugins:\n  enabled:\n");
        assert!(!sb.exists(".config/opencode/plugins/modisa-agent-state.js"));
        assert!(!sb.exists(".grok/hooks/modisa.json"));
        // the skill goes with the last agent that wanted it
        assert_eq!(sb.link_of(".claude/skills/modisa"), "");
        assert!(!sb.exists(".agents/skills/modisa/SKILL.md"));
    }

    #[test]
    fn installing_for_an_agent_that_isnt_set_up_says_so() {
        let mut sb = Sandbox::new("unset");
        sb.write(".factory/settings.json", "{}");
        assert!(set_in(&sb.env, "droid", true).is_ok());
        let err = set_in(&sb.env, "qodercli", true).unwrap_err();
        assert!(err.contains("isn't set up here"), "{err}");
        assert_eq!(run_in(&sb.env, Some("install"), Some("qoder")), 1);
        assert_eq!(run_in(&sb.env, Some("install"), Some("nobody")), 2);
        // OMP would write into Pi's directory
        let pi = sb.path(".pi/agent");
        std::fs::create_dir_all(&pi).unwrap();
        sb.env.vars.insert("PI_CODING_AGENT_DIR".into(), pi);
        assert!(set_in(&sb.env, "omp", true).unwrap_err().contains("share an agent directory"));
    }

    #[test]
    fn uninstall_all_takes_skill_links_left_without_their_hooks() {
        let sb = Sandbox::new("leftover");
        sb.write(".claude/settings.json", "{}");
        set_in(&sb.env, "claude-code", true).unwrap();
        sb.write(".claude/settings.json", "{}"); // the hooks went, the skill stayed
        assert_eq!(sb.link_of(".claude/skills/modisa"), sb.path(".agents/skills/modisa"));
        let (removed, failed) = uninstall_all_in(&sb.env);
        assert!(removed.is_empty() && failed.is_empty(), "{removed:?} {failed:?}");
        assert_eq!(sb.link_of(".claude/skills/modisa"), "");
        assert!(!sb.exists(".agents/skills/modisa"));
    }
}
