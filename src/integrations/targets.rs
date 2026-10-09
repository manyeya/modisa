// Every agent integration: where the agent keeps its config, what modisa adds there, and how to
// take it out again. Two kinds:
//   lifecycle — hooks or a plugin that see every transition report working / blocked / idle
//               (and the session); they are the pane's state authority while they report.
//   session   — hooks that report only the agent's session id, for exact resume; state keeps coming
//               from the agent's screen, because these agents' hooks miss some transitions
//               (interrupts, permission answers).
use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

use serde_json::{json, Map, Value};

use super::edit::{add_flat, add_nested, hooks_of, list, ours_in, read_json, read_text, remove_ours, with_block, with_codex_hooks_feature, with_hermes_plugin, write, write_json, Hooks, Res, MARK};
use super::plugins::{hermes_plugin, opencode_plugin, pi_extension};
use crate::core::paths::stable_self; // hooks outlive upgrades, so never a per-version path

pub const HOOK_VERSION: u32 = 2;
pub const LIFECYCLE: &str = "lifecycle";
pub const SESSION: &str = "session";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Status {
    Current,
    Outdated,
    None,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Current => "current",
            Status::Outdated => "outdated",
            Status::None => "none",
        }
    }
}

// Where agents keep their configs: HOME and the agents' own environment overrides (and PATH, for
// which agents are here). Read from the process environment; tests give their own, so they never
// touch the real home.
#[derive(Clone, Debug, Default)]
pub struct Env {
    pub home: String,
    pub vars: HashMap<String, String>,
}

impl Env {
    pub fn current() -> Env {
        // vars_os: one variable that isn't UTF-8 must not take the rest down
        let vars: HashMap<String, String> = std::env::vars_os().filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?))).collect();
        Env { home: vars.get("HOME").cloned().unwrap_or_else(|| "/tmp".into()), vars }
    }

    // set and not empty
    pub fn var(&self, name: &str) -> Option<&str> {
        self.vars.get(name).map(String::as_str).filter(|v| !v.is_empty())
    }

    fn var_or(&self, name: &str, fallback: impl FnOnce() -> String) -> String {
        self.var(name).map(String::from).unwrap_or_else(fallback)
    }

    // Bun.which: an executable file of that name on PATH
    pub fn which(&self, name: &str) -> bool {
        use std::os::unix::fs::PermissionsExt;
        let Some(path) = self.var("PATH") else { return false };
        path.split(':').filter(|d| !d.is_empty()).any(|d| std::fs::metadata(format!("{d}/{name}")).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0))
    }
}

pub struct Target {
    pub id: &'static str, // the agent's id in modisa
    pub name: &'static str,
    pub binaries: &'static [&'static str], // the agent is "available" when one of these is on PATH
    pub kind: &'static str,                // LIFECYCLE | SESSION
    pub dir: fn(&Env) -> String,           // the agent's config directory; it must exist (the agent is set up) unless `create`
    pub create: bool,
    // Where this agent reads skills from, for the ones that do. Not derived from dir(): several agents
    // keep skills somewhere else entirely (see antigravity, kilo), and writing a skill into the wrong
    // directory is worse than shipping none, so every target opts in by hand.
    pub skills: Option<fn(&Env) -> String>,
    edits: Edits,
}

impl Target {
    pub fn install(&self, e: &Env) -> Res {
        match &self.edits {
            Edits::Json(j) => j.install(e),
            Edits::Files(f) => f.install(e),
            Edits::Block(b) => b.install(e),
        }
    }

    pub fn uninstall(&self, e: &Env) -> Res {
        match &self.edits {
            Edits::Json(j) => j.uninstall(e),
            Edits::Files(f) => f.uninstall(e),
            Edits::Block(b) => b.uninstall(e),
        }
    }

    pub fn status(&self, e: &Env) -> Res<Status> {
        match &self.edits {
            Edits::Json(j) => j.status(e),
            Edits::Files(f) => f.status(e),
            Edits::Block(b) => b.status(e),
        }
    }
}

// What an integration writes: hook entries in a JSON file, files of its own, or a marked block of a
// config file.
enum Edits {
    Json(JsonHooks),
    Files(PluginFiles),
    Block(Block),
}

// The modisa skill lives here once; every agent that reads skills somewhere else gets a symlink to
// it. This is also the directory Kimi and its family read natively, so they need no link.
pub fn skills_home(e: &Env) -> String {
    format!("{}/.agents/skills", e.home)
}
pub fn skill_dir(e: &Env) -> String {
    format!("{}/modisa", skills_home(e))
}

// The common case: the agent reads skills from its own config directory.
fn skills_in(dir: String) -> String {
    format!("{dir}/skills")
}

fn sh(arg: &str) -> String {
    if !arg.is_empty() && arg.chars().all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c)) {
        arg.into()
    } else {
        format!("'{}'", arg.replace('\'', r"'\''"))
    }
}

// The command that runs modisa, as the hooks and plugins call it.
pub fn modisa_cmd() -> Vec<String> {
    vec![stable_self()]
}

// What an agent's hook runs: `modisa hook <agent> <action>`, reading the agent's hook JSON on stdin.
pub fn hook_command(agent: &str, action: &str) -> String {
    let cmd: Vec<String> = modisa_cmd().iter().map(|a| sh(a)).collect();
    format!("{MARK}{HOOK_VERSION} {} hook {agent} {action}", cmd.join(" "))
}

fn exists(path: &str) -> bool {
    Path::new(path).exists()
}

fn same(found: Vec<String>, mut want: Vec<String>) -> Status {
    want.sort();
    if found.is_empty() {
        Status::None
    } else if found == want {
        Status::Current
    } else {
        Status::Outdated
    }
}

type Add = fn(&mut Hooks, &str, &str) -> Res;

// Agents whose hooks live in a JSON file: an events object (under `key`, or the file itself) holding
// modisa's entries next to the user's own.
struct JsonHooks {
    file: fn(&Env) -> String,
    key: Option<&'static str>, // None: events at the top level of the file
    owned: bool,               // the key is ours alone: uninstall deletes it
    entries: fn() -> Vec<(&'static str, String)>, // (event, command)
    add: Add,
    prepare: Option<fn(&mut Map<String, Value>)>,
    after: Option<fn(&Env, bool) -> Res>,
}

fn json_hooks(file: fn(&Env) -> String, entries: fn() -> Vec<(&'static str, String)>, add: Add) -> JsonHooks {
    JsonHooks { file, key: Some("hooks"), owned: false, entries, add, prepare: None, after: None }
}

impl JsonHooks {
    fn events<'a>(&self, root: &'a mut Map<String, Value>) -> Res<&'a mut Hooks> {
        match self.key {
            None => Ok(root),
            Some(key) => hooks_of(root, key),
        }
    }

    fn install(&self, e: &Env) -> Res {
        let file = (self.file)(e);
        let mut root = read_json(&file)?;
        if let Some(prepare) = self.prepare {
            prepare(&mut root);
        }
        let hooks = self.events(&mut root)?;
        remove_ours(hooks);
        for (event, command) in (self.entries)() {
            (self.add)(hooks, event, &command)?;
        }
        write_json(&file, &root)?;
        self.after.map_or(Ok(()), |after| after(e, true))
    }

    fn uninstall(&self, e: &Env) -> Res {
        let file = (self.file)(e);
        if !exists(&file) {
            return Ok(());
        }
        let mut root = read_json(&file)?;
        let hooks = self.events(&mut root)?;
        if remove_ours(hooks) {
            // ours alone, or left empty by taking ours out: no `"hooks": {}` stays behind in their settings
            let empty = hooks.is_empty();
            if let Some(key) = self.key.filter(|_| self.owned || empty) {
                root.shift_remove(key);
            }
            write_json(&file, &root)?;
        }
        self.after.map_or(Ok(()), |after| after(e, false))
    }

    fn status(&self, e: &Env) -> Res<Status> {
        let file = (self.file)(e);
        if !exists(&file) {
            return Ok(Status::None);
        }
        let root = Value::Object(read_json(&file).unwrap_or_default());
        let hooks = match self.key {
            None => Some(&root),
            Some(key) => root.get(key),
        };
        Ok(same(ours_in(hooks), (self.entries)().into_iter().map(|(e, c)| format!("{e}: {c}")).collect()))
    }
}

// Agents that load a file of ours (a plugin or extension): written whole, removed whole.
// ponytail: contents are built on every call, uninstall's too; the command they embed costs one PATH lookup.
struct PluginFiles {
    files: fn(&Env) -> Vec<(String, String)>, // (path, content)
    after: Option<fn(&Env, bool) -> Res>,
    check: Option<fn(&Env) -> Res>, // refuses an install
}

fn plugin_files(files: fn(&Env) -> Vec<(String, String)>) -> PluginFiles {
    PluginFiles { files, after: None, check: None }
}

impl PluginFiles {
    fn install(&self, e: &Env) -> Res {
        if let Some(check) = self.check {
            check(e)?;
        }
        for (path, content) in (self.files)(e) {
            write(&path, &content)?;
        }
        self.after.map_or(Ok(()), |after| after(e, true))
    }

    fn uninstall(&self, e: &Env) -> Res {
        for (path, _) in (self.files)(e) {
            let _ = std::fs::remove_file(path);
        }
        self.after.map_or(Ok(()), |after| after(e, false))
    }

    fn status(&self, e: &Env) -> Res<Status> {
        let mut found = vec![];
        for (path, content) in (self.files)(e) {
            found.push(if exists(&path) { Some(std::fs::read_to_string(&path).map_err(|err| format!("{path}: {err}"))? == content) } else { None });
        }
        Ok(if found.iter().all(Option::is_none) {
            Status::None
        } else if found.iter().all(|f| *f == Some(true)) {
            Status::Current
        } else {
            Status::Outdated
        })
    }
}

// A block of a config file between markers, ours entirely (Kimi's config.toml).
struct Block {
    file: fn(&Env) -> String,
    begin: &'static str,
    end: &'static str,
    body: fn() -> String,
}

impl Block {
    fn install(&self, e: &Env) -> Res {
        let path = (self.file)(e);
        write(&path, &with_block(&read_text(&path)?, self.begin, self.end, Some(&(self.body)())))
    }

    fn uninstall(&self, e: &Env) -> Res {
        let path = (self.file)(e);
        if exists(&path) {
            write(&path, &with_block(&read_text(&path)?, self.begin, self.end, None))?;
        }
        Ok(())
    }

    fn status(&self, e: &Env) -> Res<Status> {
        let text = read_text(&(self.file)(e))?;
        Ok(if !text.contains(self.begin) {
            Status::None
        } else if text.contains(&format!("{}\n{}{}", self.begin, (self.body)(), self.end)) {
            Status::Current
        } else {
            Status::Outdated
        })
    }
}

fn claude_dir(e: &Env) -> String {
    e.var_or("CLAUDE_CONFIG_DIR", || format!("{}/.claude", e.home))
}
fn codex_dir(e: &Env) -> String {
    e.var_or("CODEX_HOME", || format!("{}/.codex", e.home))
}
fn copilot_dir(e: &Env) -> String {
    e.var_or("COPILOT_HOME", || format!("{}/.copilot", e.home))
}
fn cursor_dir(e: &Env) -> String {
    e.var_or("CURSOR_CONFIG_DIR", || format!("{}/.cursor", e.home))
}
fn devin_dir(e: &Env) -> String {
    format!("{}/devin", e.var_or("XDG_CONFIG_HOME", || format!("{}/.config", e.home)))
}
fn droid_dir(e: &Env) -> String {
    format!("{}/.factory", e.home)
}
fn qoder_dir(e: &Env) -> String {
    e.var_or("QODER_CONFIG_DIR", || format!("{}/.qoder", e.home))
}
fn qwen_dir(e: &Env) -> String {
    e.var_or("QWEN_HOME", || format!("{}/.qwen", e.home))
}
fn grok_dir(e: &Env) -> String {
    e.var_or("GROK_CONFIG_DIR", || e.var_or("GROK_HOME", || format!("{}/.grok", e.home)))
}
fn antigravity_dir(e: &Env) -> String {
    e.var_or("ANTIGRAVITY_CLI_CONFIG_DIR", || format!("{}/.gemini/config", e.home))
}
fn opencode_dir(e: &Env) -> String {
    format!("{}/.config/opencode", e.home)
}
fn kimi_dir(e: &Env) -> String {
    e.var_or("KIMI_CODE_HOME", || format!("{}/.kimi-code", e.home))
}
fn pi_dir(e: &Env) -> String {
    e.var_or("PI_CODING_AGENT_DIR", || format!("{}/.pi/agent", e.home))
}
fn omp_dir(e: &Env) -> String {
    e.var_or("PI_CODING_AGENT_DIR", || format!("{}/{}/agent", e.home, e.var("PI_CONFIG_DIR").unwrap_or(".omp")))
}
fn hermes_dir(e: &Env) -> String {
    e.var_or("HERMES_HOME", || format!("{}/.hermes", e.home))
}
fn mastra_dir(e: &Env) -> String {
    format!("{}/.mastracode", e.home)
}
fn kilo_dir(e: &Env) -> String {
    format!("{}/.config/kilo", e.home)
}

const KIMI_BEGIN: &str = "# >>> modisa kimi integration (managed; reinstalling replaces this block)";
const KIMI_END: &str = "# <<< modisa kimi integration";
const KIMI_EVENTS: &[(&str, &str, Option<&str>)] = &[
    ("SessionStart", "session", None), ("UserPromptSubmit", "working", None),
    ("PreToolUse", "working", Some("^(?!AskUserQuestion$).*$")), ("PreToolUse", "blocked", Some("^AskUserQuestion$")),
    ("PostToolUse", "working", Some("^AskUserQuestion$")), ("PostToolUseFailure", "working", Some("^AskUserQuestion$")),
    ("SubagentStart", "working", None), ("PreCompact", "working", None), ("PermissionRequest", "blocked", None), ("PermissionResult", "working", None),
    ("Stop", "idle", None), ("Interrupt", "idle", None),
];
const MASTRA_EVENTS: &[(&str, &str)] = &[
    ("SessionStart", "session"), ("UserPromptSubmit", "working"), ("AgentStart", "working"), ("PreToolUse", "working"),
    ("PermissionRequest", "blocked"), ("PermissionResult", "working"), ("SubagentStart", "working"), ("SubagentEnd", "working"),
    ("Interrupt", "idle"), ("AgentEnd", "idle"), ("Stop", "idle"),
];

fn session(agent: &str, events: &[&'static str]) -> Vec<(&'static str, String)> {
    events.iter().map(|&e| (e, hook_command(agent, "session"))).collect()
}

fn kimi_body() -> String {
    let q = |s: &str| serde_json::to_string(s).unwrap_or_default();
    KIMI_EVENTS
        .iter()
        .map(|&(event, action, matcher)| {
            let matcher = matcher.map(|m| format!("matcher = {}\n", q(m))).unwrap_or_default();
            format!("[[hooks]]\nevent = {}\n{matcher}command = {}\ntimeout = 10\n\n", q(event), q(&hook_command("kimi", action)))
        })
        .collect()
}

// Codex: hooks.json, plus the hooks feature switched on in config.toml (and our old notify line gone).
fn codex_config(e: &Env, install: bool) -> Res {
    let path = format!("{}/config.toml", codex_dir(e));
    let text = read_text(&path)?;
    let kept = text.split('\n').filter(|l| !l.contains(MARK)).collect::<Vec<_>>().join("\n"); // our old notify line
    let next = if install { with_codex_hooks_feature(&kept) } else { kept };
    if next != text {
        write(&path, &next)?;
    }
    Ok(())
}

// Hermes: the plugin enabled in config.yaml, and its directory gone with it.
fn hermes_config(e: &Env, install: bool) -> Res {
    let path = format!("{}/config.yaml", hermes_dir(e));
    let text = read_text(&path)?;
    let next = with_hermes_plugin(&text, "modisa-agent-state", install);
    if next != text {
        write(&path, &next)?;
    }
    if !install {
        let _ = std::fs::remove_dir_all(format!("{}/plugins/modisa-agent-state", hermes_dir(e)));
    }
    Ok(())
}

pub static TARGETS: LazyLock<Vec<Target>> = LazyLock::new(|| {
    vec![
        Target {
            id: "claude-code", name: "Claude Code", binaries: &["claude"], kind: SESSION, dir: claude_dir, create: false, skills: Some(|e| skills_in(claude_dir(e))),
            edits: Edits::Json(json_hooks(|e| format!("{}/settings.json", claude_dir(e)), || session("claude-code", &["SessionStart"]), |h, e, c| add_nested(h, e, c, Some("*"), None))),
        },
        Target {
            id: "codex", name: "Codex", binaries: &["codex"], kind: SESSION, dir: codex_dir, create: false, skills: Some(|e| skills_in(codex_dir(e))),
            edits: Edits::Json(JsonHooks { after: Some(codex_config), ..json_hooks(|e| format!("{}/hooks.json", codex_dir(e)), || session("codex", &["SessionStart"]), |h, e, c| add_nested(h, e, c, None, None)) }),
        },
        Target {
            id: "copilot", name: "Copilot CLI", binaries: &["copilot"], kind: SESSION, dir: copilot_dir, create: false, skills: Some(|e| skills_in(copilot_dir(e))),
            edits: Edits::Json(json_hooks(|e| format!("{}/settings.json", copilot_dir(e)), || session("copilot", &["SessionStart"]), |h, e, c| add_flat(h, e, c, json!({ "timeoutSec": 10 }), "bash"))),
        },
        Target {
            id: "cursor-agent", name: "Cursor Agent", binaries: &["cursor-agent"], kind: SESSION, dir: cursor_dir, create: false, skills: Some(|e| skills_in(cursor_dir(e))),
            edits: Edits::Json(JsonHooks {
                prepare: Some(|root| {
                    if root.get("version").is_none_or(Value::is_null) {
                        root.insert("version".into(), 1.into());
                    }
                }),
                ..json_hooks(|e| format!("{}/hooks.json", cursor_dir(e)), || session("cursor-agent", &["sessionStart"]), |h, e, c| {
                    list(h, e)?.push(json!({ "command": c }));
                    Ok(())
                })
            }),
        },
        Target {
            id: "devin", name: "Devin CLI", binaries: &["devin"], kind: SESSION, dir: devin_dir, create: false, skills: Some(|e| skills_in(devin_dir(e))),
            edits: Edits::Json(json_hooks(
                |e| format!("{}/config.json", devin_dir(e)),
                || session("devin", &["SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse", "PermissionRequest", "Stop"]),
                |h, e, c| add_nested(h, e, c, None, None),
            )),
        },
        Target {
            id: "droid", name: "Droid", binaries: &["droid"], kind: SESSION, dir: droid_dir, create: false, skills: Some(|e| skills_in(droid_dir(e))),
            edits: Edits::Json(json_hooks(|e| format!("{}/settings.json", droid_dir(e)), || session("droid", &["SessionStart"]), |h, e, c| add_nested(h, e, c, None, None))),
        },
        Target {
            id: "qodercli", name: "Qoder CLI", binaries: &["qodercli"], kind: SESSION, dir: qoder_dir, create: false, skills: Some(|e| skills_in(qoder_dir(e))),
            edits: Edits::Json(json_hooks(|e| format!("{}/settings.json", qoder_dir(e)), || session("qodercli", &["SessionStart"]), |h, e, c| add_nested(h, e, c, Some("*"), None))),
        },
        Target {
            id: "qwen", name: "Qwen Code", binaries: &["qwen"], kind: SESSION, dir: qwen_dir, create: false, skills: Some(|e| skills_in(qwen_dir(e))),
            edits: Edits::Json(json_hooks(|e| format!("{}/settings.json", qwen_dir(e)), || session("qwen", &["SessionStart"]), |h, e, c| add_nested(h, e, c, Some("*"), Some(10_000)))),
        },
        Target {
            // Grok merges every hooks/*.json, so ours is a file of its own
            id: "grok", name: "Grok CLI", binaries: &["grok"], kind: SESSION, dir: grok_dir, create: false, skills: Some(|e| skills_in(grok_dir(e))),
            edits: Edits::Files(plugin_files(|e| {
                let hooks = json!({ "hooks": { "SessionStart": [{ "hooks": [{ "type": "command", "command": hook_command("grok", "session"), "timeout": 10 }] }] } });
                vec![(format!("{}/hooks/modisa.json", grok_dir(e)), serde_json::to_string_pretty(&hooks).unwrap_or_default() + "\n")]
            })),
        },
        Target {
            // Antigravity keys hooks.json by hook name: the "modisa" block is ours. Its skills live beside
            // its config directory, not inside it.
            id: "antigravity", name: "Antigravity CLI", binaries: &["agy"], kind: SESSION, dir: antigravity_dir, create: false, skills: Some(|e| format!("{}/.gemini/antigravity-cli/skills", e.home)),
            edits: Edits::Json(JsonHooks {
                key: Some("modisa"),
                owned: true,
                ..json_hooks(|e| format!("{}/hooks.json", antigravity_dir(e)), || session("antigravity", &["PreInvocation"]), |h, e, c| add_flat(h, e, c, json!({ "timeout": 10 }), "command"))
            }),
        },
        Target {
            id: "hermes", name: "Hermes Agent", binaries: &["hermes"], kind: SESSION, dir: hermes_dir, create: false, skills: Some(|e| skills_in(hermes_dir(e))),
            edits: Edits::Files(PluginFiles {
                after: Some(hermes_config),
                ..plugin_files(|e| {
                    let plugin = hermes_plugin(&modisa_cmd());
                    let dir = format!("{}/plugins/modisa-agent-state", hermes_dir(e));
                    vec![(format!("{dir}/plugin.yaml"), plugin.yaml), (format!("{dir}/__init__.py"), plugin.py)]
                })
            }),
        },
        Target {
            // Kimi reads the shared skills directory, which is where modisa keeps the skill anyway: nothing
            // to link, the skill is simply there.
            id: "kimi", name: "Kimi Code", binaries: &["kimi"], kind: LIFECYCLE, dir: kimi_dir, create: false, skills: Some(skills_home),
            edits: Edits::Block(Block { file: |e| format!("{}/config.toml", kimi_dir(e)), begin: KIMI_BEGIN, end: KIMI_END, body: kimi_body }),
        },
        Target {
            // MastraCode's hooks.json maps events straight to handlers
            id: "mastracode", name: "MastraCode", binaries: &["mastracode"], kind: LIFECYCLE, dir: mastra_dir, create: true, skills: None,
            edits: Edits::Json(JsonHooks {
                key: None,
                ..json_hooks(
                    |e| format!("{}/hooks.json", mastra_dir(e)),
                    || MASTRA_EVENTS.iter().map(|&(e, a)| (e, hook_command("mastracode", a))).collect(),
                    |h, e, c| add_flat(h, e, c, json!({ "timeout": 10_000, "description": "Report MastraCode state to modisa" }), "command"),
                )
            }),
        },
        Target {
            id: "opencode", name: "OpenCode", binaries: &["opencode"], kind: LIFECYCLE, dir: opencode_dir, create: false, skills: Some(|e| skills_in(opencode_dir(e))),
            edits: Edits::Files(plugin_files(|e| vec![(format!("{}/plugins/modisa-agent-state.js", opencode_dir(e)), opencode_plugin("opencode", &modisa_cmd()))])),
        },
        // Kilo's plugin lives under ~/.config/kilo but it reads skills from ~/.kilo
        Target {
            id: "kilo", name: "Kilo Code", binaries: &["kilo", "kilo-code"], kind: LIFECYCLE, dir: kilo_dir, create: false, skills: Some(|e| format!("{}/.kilo/skills", e.home)),
            edits: Edits::Files(plugin_files(|e| vec![(format!("{}/plugin/modisa-agent-state.js", kilo_dir(e)), opencode_plugin("kilo", &modisa_cmd()))])),
        },
        Target {
            id: "pi", name: "Pi", binaries: &["pi"], kind: LIFECYCLE, dir: pi_dir, create: false, skills: Some(|e| skills_in(pi_dir(e))),
            edits: Edits::Files(plugin_files(|e| vec![(format!("{}/extensions/modisa-agent-state.ts", pi_dir(e)), pi_extension("pi", &modisa_cmd()))])),
        },
        Target {
            id: "omp", name: "OMP", binaries: &["omp"], kind: LIFECYCLE, dir: omp_dir, create: false, skills: None,
            edits: Edits::Files(PluginFiles {
                check: Some(|e| {
                    if omp_dir(e) == pi_dir(e) {
                        return Err("OMP and Pi share an agent directory, so Pi would load OMP's extension; give them separate directories first".into());
                    }
                    Ok(())
                }),
                ..plugin_files(|e| vec![(format!("{}/extensions/modisa-omp-agent-state.ts", omp_dir(e)), pi_extension("omp", &modisa_cmd()))])
            }),
        },
    ]
});
