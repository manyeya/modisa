// Agent detection: which panes run an agent (from the foreground process) and what state it's in.
// Each pane has one authority: an integration reporting lifecycle state (hooks or a plugin that see
// every transition), or else the agent's screen rules (./manifest.rs). A dialog visibly waiting on
// you still wins over a reported state.
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;
use serde::{Serialize, Serializer};

use super::manifest::{compile_rules, evaluate, Evidence, Rule, RuleState, Verdict};
use crate::config::agents::{AgentDef, RawRule};
use crate::protocol::types::{AgentInfo, AgentState, PaneInfo};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proc {
    pub pid: i32,
    pub ppid: i32,
    pub tpgid: i32,
    pub args: String,
}

pub type ProcessTable = HashMap<i32, Proc>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Raw {
    Working,
    Blocked,
    Idle,
}

impl From<Raw> for AgentState {
    fn from(r: Raw) -> AgentState {
        match r {
            Raw::Working => AgentState::Working,
            Raw::Blocked => AgentState::Blocked,
            Raw::Idle => AgentState::Idle,
        }
    }
}

// ---------- which agent a process is ----------

fn basename(path: &str) -> &str {
    path.split(['/', '\\']).rfind(|s| !s.is_empty()).unwrap_or(path)
}

static SCRIPT_EXT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\.(exe|cmd|bat|ps1|m?js|cjs|ts|py)$").unwrap());
static RUNTIME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(node|bun|sh|bash|zsh|fish|python([0-9]+(\.[0-9]+)*)?)$").unwrap());

// "codex.js", ".codex-wrapped" (nix) and "Claude.exe" all name the agent inside
fn normalize(token: &str) -> String {
    let t = token.strip_prefix(['"', '\'']).unwrap_or(token);
    let t = t.strip_suffix(['"', '\'']).unwrap_or(t);
    let lower = basename(t).to_lowercase();
    let n = SCRIPT_EXT.replace(&lower, "");
    let n = n.strip_prefix('.').unwrap_or(&n);
    n.strip_suffix("-wrapped").unwrap_or(n).to_string()
}

fn is_runtime(name: &str) -> bool {
    RUNTIME.is_match(name)
}

// Installs whose entry script has a generic name (cli.js, index.js).
fn known_package(path: &str) -> Option<&'static str> {
    let p = path.to_lowercase();
    if p.ends_with("node_modules/@earendil-works/pi-coding-agent/dist/cli.js")
        || p.ends_with("node_modules/@earendil-works/pi-coding-agent/dist/bundle/cli.js")
    {
        return Some("pi");
    }
    if p.contains("node_modules/@qwen-code/qwen-code/dist/index.js") {
        return Some("qwen");
    }
    if p.contains("node_modules/mastracode/dist/cli") {
        return Some("mastracode");
    }
    None
}

// The script an interpreter runs: its first argument that isn't a flag (none for -e / -c / -m).
fn script_of<'s>(argv: &[&'s str], runtime: &str) -> Option<&'s str> {
    let inline: &[&str] = if runtime.starts_with("python") {
        &["-c", "-m"]
    } else if matches!(runtime, "sh" | "bash" | "zsh" | "fish") {
        &["-c"]
    } else {
        &["-e", "--eval", "-p", "--print"]
    };
    const TAKES_VALUE: &[&str] = &[
        "-r",
        "--require",
        "--loader",
        "--import",
        "--experimental-loader",
        "--inspect-port",
        "-W",
        "-X",
        "-S",
        "-L",
        "-o",
    ];
    let mut i = 1;
    while i < argv.len() {
        let a = argv[i];
        if a == "--" {
            return argv.get(i + 1).copied();
        }
        if inline.iter().any(|f| a == *f || a.strip_prefix(f).is_some_and(|r| r.starts_with('='))) {
            return None;
        }
        if a.starts_with('-') {
            if TAKES_VALUE.contains(&a) {
                i += 1;
            }
            i += 1;
            continue;
        }
        return Some(a);
    }
    None
}

pub fn identify<'a>(args: &str, adapters: &'a [AgentDef]) -> Option<&'a AgentDef> {
    let argv: Vec<&str> = args.split_whitespace().collect();
    let first_arg = *argv.first()?;
    let by_name = |name: &str| {
        adapters.iter().find(|a| a.process.iter().any(|p| p == name)).or_else(|| {
            let muse_bin = name.strip_prefix("muse-bin-").is_some_and(|r| r.starts_with(|c: char| c.is_ascii_digit()));
            if muse_bin {
                adapters.iter().find(|a| a.id == "muse")
            } else {
                None
            }
        })
    };
    let by_path = |path: &str| known_package(path).and_then(|id| adapters.iter().find(|a| a.id == id));
    let first = normalize(first_arg);
    if !is_runtime(&first) {
        return by_name(&first).or_else(|| by_path(first_arg));
    }
    let script = script_of(&argv, &first)?;
    by_name(&normalize(script)).or_else(|| by_path(script))
}

// The shell's foreground job: the process group it handed the terminal to.
pub fn foreground(procs: &ProcessTable, shell_pid: i32) -> Option<&Proc> {
    let shell = procs.get(&shell_pid)?;
    if shell.tpgid > 0 {
        procs.get(&shell.tpgid)
    } else {
        None
    }
}

// How many of a shell's descendants pane_table reads, at most, for the no-job-control fallback: a pane running a big
// parallel build doesn't make every tick read hundreds of processes.
const DESCENDANTS: usize = 64;

// The processes detection looks at for these panes, from the kernel (platform/procs.rs) instead of ps: each pane's
// shell, with the process group its terminal has in the foreground (`fg`, from its pty); the process leading that group;
// and, only where that isn't an agent, the shell's descendants for the no-job-control fallback. Plus any other process
// asked for (`extra`: the agents integrations reported, whose authority lasts while they're alive). What ps -A gave
// tick() for these panes, without spawning ps or reading every process on the machine.
pub fn pane_table(panes: &[(i32, Option<i32>)], extra: &[i32], adapters: &[AgentDef]) -> ProcessTable {
    use crate::platform::procs;
    let mut t = ProcessTable::new();
    let add = |t: &mut ProcessTable, pid: i32, tpgid: i32| match procs::info(pid) {
        Some(i) => {
            t.entry(pid).or_insert(Proc { pid, ppid: i.ppid, tpgid, args: i.args });
            true
        }
        None => false,
    };
    for &(shell, fg) in panes {
        let tpgid = fg.unwrap_or(0);
        if !add(&mut t, shell, tpgid) {
            continue;
        }
        if tpgid > 0 && tpgid != shell {
            add(&mut t, tpgid, 0);
            if t.get(&tpgid).is_some_and(|p| identify(&p.args, adapters).is_some()) {
                continue; // the agent is in the foreground: no need to look further
            }
        }
        let mut queue: VecDeque<i32> = procs::children(shell).into();
        let mut read = 0;
        while let Some(pid) = queue.pop_front() {
            if read == DESCENDANTS {
                break;
            }
            read += 1;
            if add(&mut t, pid, 0) {
                queue.extend(procs::children(pid));
            }
        }
    }
    for &pid in extra {
        add(&mut t, pid, 0);
    }
    t
}

// Fallback when the shell has no foreground group (no job control): any descendant running an agent.
pub fn descendant_agent<'p, 'a>(
    procs: &'p ProcessTable,
    shell_pid: i32,
    adapters: &'a [AgentDef],
) -> Option<(&'p Proc, &'a AgentDef)> {
    let mut kids: HashMap<i32, Vec<&Proc>> = HashMap::new();
    for p in procs.values() {
        if p.pid != p.ppid {
            kids.entry(p.ppid).or_default().push(p); // (pid 0 is its own parent on macOS: no loop)
        }
    }
    // siblings in pid order, as ps lists them (a HashMap has no order of its own)
    for v in kids.values_mut() {
        v.sort_by_key(|p| p.pid);
    }
    let mut queue: VecDeque<&Proc> = kids.get(&shell_pid).into_iter().flatten().copied().collect();
    while let Some(p) = queue.pop_front() {
        if let Some(adapter) = identify(&p.args, adapters) {
            return Some((p, adapter));
        }
        queue.extend(kids.get(&p.pid).into_iter().flatten().copied());
    }
    None
}

// ---------- what state it's in ----------

// Compiled rules, per agent id. An AgentDef is a plain value (the TS kept a WeakMap per object), so an entry stays
// valid while the agent's raw rules are unchanged: a config reload that replaces them recompiles.
type Compiled = HashMap<String, (Vec<RawRule>, Rc<Vec<Rule>>)>;
thread_local! {
    static COMPILED: RefCell<Compiled> = RefCell::new(HashMap::new());
}

fn rules_of(a: &AgentDef) -> Rc<Vec<Rule>> {
    COMPILED.with_borrow_mut(|cache| {
        if let Some((raw, rules)) = cache.get(&a.id) {
            if *raw == a.rules {
                return rules.clone();
            }
        }
        let rules = Rc::new(compile_rules(&a.rules).unwrap_or_else(|e| {
            eprintln!("modisa: {} screen rules: {e}", a.id);
            vec![]
        }));
        cache.insert(a.id.clone(), (a.rules.clone(), rules.clone()));
        rules
    })
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

// The screen rules' verdict; agents that opt in count recent output as working when nothing matched.
#[cfg(test)]
pub fn screen_verdict(
    a: &AgentDef,
    screen: &str,
    title: &str,
    progress: &str,
    last_output_ms: u64,
    now_ms: u64,
) -> Verdict {
    with_activity(a, evaluate(&rules_of(a), &Evidence { screen, title, progress }), last_output_ms, now_ms)
}

// what the time adds: recent output, for an agent that counts it, when no rule matched
fn with_activity(a: &AgentDef, v: Verdict, last_output_ms: u64, now_ms: u64) -> Verdict {
    if v.rule.is_none() && a.activity && now_ms.saturating_sub(last_output_ms) < 2000 {
        return Verdict { state: RuleState::Working, rule: Some("recent output".into()), ..v };
    }
    v
}

// working → idle while you weren't looking = done; done → idle once you focus it.
pub fn next_state(prev: Option<AgentState>, raw: Raw, focused: bool) -> AgentState {
    if raw != Raw::Idle {
        return raw.into();
    }
    if focused {
        return AgentState::Idle;
    }
    match prev {
        Some(AgentState::Working | AgentState::Blocked | AgentState::Done) => AgentState::Done,
        _ => AgentState::Idle,
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Detection {
    pub harness: String,
    pub raw: Raw,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
    pub source: String, // "hook" | "screen"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fg: Option<String>,
}

// An integration that reports lifecycle state for a pane, until it releases or its agent exits.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Authority {
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    pub state: Raw,
    #[serde(skip_serializing_if = "Option::is_none", serialize_with = "js_number")]
    pub seq: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<i32>,
}

// A JS number in JSON: 3, not 3.0.
fn js_number<S: Serializer>(n: &Option<f64>, s: S) -> Result<S::Ok, S::Error> {
    match *n {
        Some(n) if n.fract() == 0.0 && n.abs() < 9.007_199_254_740_992e15 => s.serialize_i64(n as i64),
        Some(n) => s.serialize_f64(n),
        None => s.serialize_none(),
    }
}

// What the detector needs from a pane.
pub trait DetectPane {
    fn id(&self) -> &str;
    fn pid(&self) -> i32; // the pane's shell/command process
    fn disposed(&self) -> bool;
    fn info(&self) -> &PaneInfo;
    fn info_mut(&mut self) -> &mut PaneInfo;
    fn screen(&mut self) -> String; // visible screen as text (only called when an adapter matched: it's not free)
    fn osc_title(&self) -> &str;
    fn osc_progress(&self) -> &str;
    fn last_output(&self) -> u64; // epoch ms of its last output
    // rises with every change to what's on its screen; None: unknown, so it's read every time
    fn generation(&self) -> Option<u64> {
        None
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub pane: String,
    pub from: Option<AgentState>,
    pub to: AgentState,
}

#[derive(Default)]
pub struct Detector {
    pub authority: HashMap<String, Authority>,
    pub last: HashMap<String, Detection>,
    agent_pid: HashMap<String, i32>,
    pending_idle: HashSet<String>,
    // each pane's last screen-rule verdict, and what it was read from (the rules, by address, and the pane's
    // generation): a screen that hasn't changed under the same rules gets the same verdict without being read again
    seen: HashMap<String, (usize, u64, Verdict)>,
}

impl Detector {
    pub fn new() -> Detector {
        Detector::default()
    }

    // The processes it looks at come from pane_table (or, in tests, a table of their own).
    pub fn tick(
        &mut self,
        procs: &ProcessTable,
        adapters: &[AgentDef],
        panes: &mut [&mut dyn DetectPane],
        focused: &dyn Fn(&str) -> bool,
    ) -> Vec<Change> {
        let mut changes = vec![];
        for p in panes.iter_mut() {
            if p.disposed() {
                continue; // closed while the caller waited on the process table
            }
            let id = p.id().to_string();
            if !p.info().running() {
                self.forget(&id);
                if let Some(agent) = p.info_mut().agent.as_mut().filter(|a| a.state != AgentState::Idle) {
                    let from = agent.state;
                    agent.state = if focused(&id) { AgentState::Idle } else { AgentState::Done };
                    if from != agent.state {
                        changes.push(Change { pane: id, from: Some(from), to: agent.state });
                    }
                }
                continue;
            }
            let shell_pid = p.pid();
            let mut fg = foreground(procs, shell_pid);
            let mut adapter = fg.and_then(|f| identify(&f.args, adapters));
            if adapter.is_none() {
                if let Some((proc, a)) = descendant_agent(procs, shell_pid, adapters) {
                    (adapter, fg) = (Some(a), Some(proc));
                }
            }
            // an integration's authority ends when the agent it spoke for is gone
            if let Some(auth) = self.authority.get_mut(&id) {
                let shell_owns_terminal = fg.is_none_or(|f| f.pid == shell_pid);
                let gone = match auth.pid {
                    Some(pid) => !procs.contains_key(&pid),
                    None => shell_owns_terminal,
                };
                if gone {
                    self.authority.remove(&id);
                } else if auth.pid.is_none() {
                    auth.pid = fg.map(|f| f.pid); // reported before we'd seen its process
                }
            }
            let live = self.authority.get(&id).cloned();
            if adapter.is_none() {
                if let Some(agent) = live.as_ref().and_then(|l| l.agent.as_deref()) {
                    // agents only their plugin identifies
                    adapter = adapters.iter().find(|a| a.id == agent || a.process.iter().any(|p| p == agent));
                }
            }
            if adapter.is_none() {
                if let Some(harness) = p.info().harness.as_deref() {
                    adapter = adapters
                        .iter()
                        .find(|a| a.id == harness)
                        .or_else(|| adapters.iter().find(|a| a.id == "generic"));
                }
            }
            let Some(adapter) = adapter else {
                if p.info().agent.is_some() {
                    p.info_mut().agent = None;
                    changes.push(Change { pane: id.clone(), from: None, to: AgentState::Idle });
                }
                self.forget(&id);
                continue;
            };
            if let Some(f) = fg {
                self.agent_pid.insert(id.clone(), f.pid);
            }
            let rules = rules_of(adapter);
            let key = Rc::as_ptr(&rules) as usize;
            let cached = match (self.seen.get(&id), p.generation()) {
                (Some((k, g, v)), Some(gen)) if *k == key && *g == gen => Some(v.clone()),
                _ => None,
            };
            let base = match cached {
                Some(v) => v,
                None => {
                    let screen = p.screen();
                    let v = evaluate(&rules, &Evidence { screen: &screen, title: p.osc_title(), progress: p.osc_progress() });
                    if let Some(gen) = p.generation() {
                        self.seen.insert(id.clone(), (key, gen, v.clone()));
                    }
                    v
                }
            };
            let seen = with_activity(adapter, base, p.last_output(), now_ms());
            let prev = self.last.get(&id).map(|d| d.raw);
            let (raw, rule, source) = match &live {
                Some(live) if !seen.visible_blocker => (live.state, Some(format!("{} report", live.source)), "hook"),
                // an agent-owned viewer (transcript, picker): keep the last state
                _ if seen.skip || seen.state == RuleState::Unknown => (prev.unwrap_or(Raw::Idle), seen.rule, "screen"),
                _ => {
                    let mut raw = match seen.state {
                        RuleState::Working => Raw::Working,
                        RuleState::Blocked => Raw::Blocked,
                        _ => Raw::Idle,
                    };
                    // a spinner frame without its marker reads as idle: wait one more look unless idle is visible
                    if prev == Some(Raw::Working)
                        && raw == Raw::Idle
                        && !seen.visible_idle
                        && !self.pending_idle.contains(&id)
                    {
                        self.pending_idle.insert(id.clone());
                        raw = Raw::Working;
                    } else {
                        self.pending_idle.remove(&id);
                    }
                    (raw, seen.rule, "screen")
                }
            };
            self.last.insert(
                id.clone(),
                Detection {
                    harness: adapter.id.clone(),
                    raw,
                    rule,
                    source: source.into(),
                    fg: fg.map(|f| f.args.clone()),
                },
            );
            let from = p.info().agent.as_ref().map(|a| a.state);
            let same_agent = p.info().agent.as_ref().is_some_and(|a| a.harness == adapter.id);
            let to = next_state(if same_agent { from } else { None }, raw, focused(&id));
            p.info_mut().agent = Some(AgentInfo { harness: adapter.id.clone(), state: to, source: source.into() });
            if from != Some(to) {
                changes.push(Change { pane: id, from, to });
            }
        }
        changes
    }

    // A lifecycle report. Reports older than the last one from the same source (by seq) are ignored.
    pub fn report(
        &mut self,
        pane_id: &str,
        source: &str,
        agent: Option<&str>,
        state: AgentState,
        seq: Option<f64>,
    ) -> bool {
        if let Some(cur) = self.authority.get(pane_id) {
            if let (true, Some(new), Some(old)) = (cur.source == source, seq, cur.seq) {
                if new <= old {
                    return false;
                }
            }
        }
        let state = match state {
            AgentState::Working => Raw::Working,
            AgentState::Blocked => Raw::Blocked,
            AgentState::Done | AgentState::Idle => Raw::Idle,
        };
        let pid = self.agent_pid.get(pane_id).copied();
        self.authority
            .insert(pane_id.into(), Authority { source: source.into(), agent: agent.map(Into::into), state, seq, pid });
        true
    }

    pub fn release(&mut self, pane_id: &str, source: &str) {
        if self.authority.get(pane_id).is_some_and(|a| a.source == source) {
            self.authority.remove(pane_id);
        }
    }

    fn forget(&mut self, id: &str) {
        self.seen.remove(id);
        self.authority.remove(id);
        self.agent_pid.remove(id);
        self.pending_idle.remove(id);
        self.last.remove(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::agents::builtin_agents;

    fn agents() -> Vec<AgentDef> {
        builtin_agents()
    }

    fn agent<'a>(all: &'a [AgentDef], id: &str) -> &'a AgentDef {
        all.iter().find(|a| a.id == id).unwrap()
    }

    fn state(all: &[AgentDef], id: &str, screen: &str, title: &str) -> RuleState {
        screen_verdict(agent(all, id), screen, title, "", 0, now_ms()).state
    }

    fn table(rows: &[(i32, i32, i32, &str)]) -> ProcessTable {
        rows.iter().map(|&(pid, ppid, tpgid, args)| (pid, Proc { pid, ppid, tpgid, args: args.into() })).collect()
    }

    #[test]
    fn agents_are_recognised_by_name_alias_and_through_interpreters_and_wrappers() {
        let all = agents();
        let cases: &[(&str, Option<&str>)] = &[
            ("claude", Some("claude-code")),
            ("/opt/homebrew/bin/codex", Some("codex")),
            ("node /usr/local/lib/node_modules/@openai/codex/bin/codex.js", Some("codex")),
            ("/nix/store/abc-codex/bin/.codex-wrapped --yolo", Some("codex")),
            ("bun /home/me/.bun/bin/omp", Some("omp")),
            ("python3.12 /home/me/.local/bin/hermes chat", Some("hermes")),
            ("node /usr/lib/node_modules/@earendil-works/pi-coding-agent/dist/cli.js", Some("pi")),
            ("node /usr/lib/node_modules/@qwen-code/qwen-code/dist/index.js", Some("qwen")),
            ("/opt/muse/muse-bin-0.1.0-R708.1", Some("muse")),
            ("agy", Some("antigravity")),
            ("kiro-cli chat", Some("kiro")),
            ("ghcs", Some("copilot")),
            ("node --require ./x.js /usr/lib/node_modules/@qwen-code/qwen-code/dist/index.js", Some("qwen")),
            ("\"C:\\Tools\\Claude.exe\" --resume", Some("claude-code")),
            ("node -- /x/codex.mjs", Some("codex")),
            ("node -e console.log(1)", None),
            ("python3 -m http.server", None),
            ("bash -c claude", None),
            ("-zsh", None),
            ("muse-binary", None),
            ("", None),
        ];
        for (args, id) in cases {
            assert_eq!((args, identify(args, &all).map(|a| a.id.as_str())), (args, *id));
        }
    }

    #[test]
    fn foreground_job_resolves_through_the_shells_tpgid_without_one_any_descendant_agent_counts() {
        let all = agents();
        let procs =
            table(&[(10, 1, 20, "-zsh"), (20, 10, 20, "node /usr/local/lib/node_modules/@openai/codex/bin/codex.js")]);
        assert_eq!(foreground(&procs, 10).unwrap().pid, 20);
        let no_job_control = table(&[(10, 1, 0, "/bin/zsh -l"), (11, 10, 0, "sleep 5"), (12, 10, 0, "claude")]);
        assert!(foreground(&no_job_control, 10).is_none());
        assert_eq!(descendant_agent(&no_job_control, 10, &all).map(|(_, a)| a.id.as_str()), Some("claude-code"));
        assert!(descendant_agent(&no_job_control, 11, &all).is_none());
        // a grandchild counts too, and a self-parented process doesn't loop
        let deep = table(&[(0, 0, 0, "kernel_task"), (10, 0, 0, "zsh"), (11, 10, 0, "sh -c x"), (12, 11, 0, "codex")]);
        assert_eq!(descendant_agent(&deep, 10, &all).map(|(p, _)| p.pid), Some(12));
        assert!(descendant_agent(&deep, 0, &all).is_some());
    }

    #[test]
    fn pane_table_reads_the_kernel_not_ps() {
        let all = agents();
        let me = std::process::id() as i32;
        // a real child named like an agent, as a shell with no job control would have it
        let dir = std::env::temp_dir().join(format!("modisa-pane-table-{me}"));
        std::fs::create_dir_all(&dir).unwrap();
        let codex = dir.join("codex");
        std::fs::copy("/bin/sleep", &codex).unwrap();
        let mut child = std::process::Command::new(&codex).arg("5").spawn().unwrap();
        let pid = child.id() as i32;
        // Linux: a moment after a spawn, its command line can still be empty
        for _ in 0..100 {
            if crate::platform::procs::info(pid).is_some_and(|i| !i.args.is_empty()) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let t = pane_table(&[(me, Some(pid))], &[], &all);
        assert_eq!(t[&me].tpgid, pid);
        assert_eq!(t[&pid].ppid, me);
        assert!(t[&pid].args.ends_with("codex 5"), "{}", t[&pid].args);
        assert_eq!(foreground(&t, me).map(|p| p.pid), Some(pid));
        // the shell in the foreground itself: the agent is found among its descendants
        let t = pane_table(&[(me, Some(me))], &[], &all);
        assert_eq!(descendant_agent(&t, me, &all).map(|(p, a)| (p.pid, a.id.as_str())), Some((pid, "codex")));
        child.kill().unwrap();
        child.wait().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn claudes_terminal_title_spinner_means_working() {
        let all = agents();
        assert_eq!(state(&all, "claude-code", "❯ ", "⠙ Refactoring auth"), RuleState::Working);
        assert_eq!(state(&all, "claude-code", "❯ ", "Claude Code"), RuleState::Idle);
    }

    #[test]
    fn agents_that_opt_in_count_recent_output_as_working_when_no_rule_matches() {
        let all = agents();
        let now = now_ms();
        let v = screen_verdict(agent(&all, "generic"), "", "", "", now, now);
        assert_eq!((v.state, v.rule.as_deref()), (RuleState::Working, Some("recent output")));
        assert_eq!(screen_verdict(agent(&all, "generic"), "", "", "", 0, now).state, RuleState::Idle);
    }

    #[test]
    fn done_means_finished_while_unfocused_and_clears_on_focus() {
        use AgentState::*;
        assert_eq!(next_state(Some(Working), Raw::Idle, false), Done);
        assert_eq!(next_state(Some(Done), Raw::Idle, false), Done);
        assert_eq!(next_state(Some(Done), Raw::Idle, true), Idle);
        assert_eq!(next_state(None, Raw::Idle, false), Idle);
        assert_eq!(next_state(Some(Idle), Raw::Blocked, true), Blocked);
    }

    fn fixture(dir: &str, name: &str) -> String {
        let path = format!("{}/tests/fixtures/{dir}/{name}.txt", env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    // Screens captured from the real Claude Code v2.1.268 running in a modisa pane.
    #[test]
    fn claude_code_rules_on_real_screens() {
        let all = agents();
        let expected = [
            ("idle", RuleState::Idle),
            ("done", RuleState::Idle), // finished turn: the input box is back
            ("working", RuleState::Working),
            ("question", RuleState::Blocked),        // AskUserQuestion picker
            ("permission", RuleState::Blocked),      // Bash permission prompt
            ("trust", RuleState::Blocked),           // folder trust dialog
            ("idle-chat-question", RuleState::Idle), // chat text asking "Do you want to…?" with the input box still there
        ];
        for (name, want) in expected {
            assert_eq!((name, state(&all, "claude-code", &fixture("claude-code", name), "")), (name, want));
        }
    }

    // Codex v0.154.0 screens (working is reconstructed from its status line format).
    #[test]
    fn codex_rules_on_real_screens() {
        let all = agents();
        let expected = [
            ("idle", RuleState::Idle),
            ("confirm-dialog", RuleState::Blocked),
            ("idle-after-dialog", RuleState::Idle),
            ("working", RuleState::Working),
        ];
        for (name, want) in expected {
            assert_eq!((name, state(&all, "codex", &fixture("codex", name), "")), (name, want));
        }
    }

    #[test]
    fn rules_recompile_when_an_agents_rules_change() {
        let mut a = agents().into_iter().find(|a| a.id == "aider").unwrap();
        let quiet = 10_000; // no output for 10s: aider's activity fallback stays out of it
        assert_eq!(screen_verdict(&a, "esc to interrupt", "", "", 0, quiet).rule.as_deref(), Some("working"));
        a.rules.truncate(1);
        assert_eq!(screen_verdict(&a, "esc to interrupt", "", "", 0, quiet).rule, None);
        assert_eq!(
            screen_verdict(&a, "Do you want to run it? (y/n)", "", "", 0, quiet).rule.as_deref(),
            Some("confirm")
        );
    }

    // ---------- the detector ----------

    struct FakePane {
        id: String,
        pid: i32,
        disposed: bool,
        info: PaneInfo,
        screen: String,
        title: String,
        screens_read: usize,
    }

    impl FakePane {
        fn new(id: &str, pid: i32, harness: Option<&str>) -> FakePane {
            let info = PaneInfo {
                id: id.into(),
                instance: "i".into(),
                name: None,
                title: String::new(),
                terminal_title: None,
                cwd: "/".into(),
                command: None,
                harness: harness.map(Into::into),
                created_by: "user".into(),
                status: "running".into(),
                exit_code: None,
                agent: None,
                session: None,
                cols: 80,
                rows: 24,
                popup: None,
                takeover: None,
                muted: false,
            };
            FakePane {
                id: id.into(),
                pid,
                disposed: false,
                info,
                screen: String::new(),
                title: String::new(),
                screens_read: 0,
            }
        }
    }

    impl DetectPane for FakePane {
        fn id(&self) -> &str {
            &self.id
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
            self.screens_read += 1;
            self.screen.clone()
        }
        fn osc_title(&self) -> &str {
            &self.title
        }
        fn osc_progress(&self) -> &str {
            ""
        }
        fn last_output(&self) -> u64 {
            0
        }
    }

    fn tick(d: &mut Detector, procs: &ProcessTable, all: &[AgentDef], p: &mut FakePane, focused: bool) -> Vec<Change> {
        d.tick(procs, all, &mut [p as &mut dyn DetectPane], &|_| focused)
    }

    #[test]
    fn a_disposed_pane_is_skipped() {
        let all = agents();
        let mut p = FakePane::new("p1", 10, Some("claude-code"));
        p.disposed = true;
        assert_eq!(tick(&mut Detector::new(), &ProcessTable::new(), &all, &mut p, false), vec![]);
        assert!(p.info.agent.is_none());
    }

    #[test]
    fn working_then_done_while_unfocused_then_idle_on_focus() {
        let all = agents();
        let procs = table(&[(10, 1, 20, "-zsh"), (20, 10, 20, "claude")]);
        let mut d = Detector::new();
        let mut p = FakePane::new("p1", 10, None);
        p.title = "⠙ Refactoring auth".into();
        let c = tick(&mut d, &procs, &all, &mut p, false);
        assert_eq!(c, vec![Change { pane: "p1".into(), from: None, to: AgentState::Working }]);
        let det = &d.last["p1"];
        assert_eq!(
            (det.harness.as_str(), det.raw, det.rule.as_deref(), det.fg.as_deref()),
            ("claude-code", Raw::Working, Some("osc_title_working"), Some("claude"))
        );
        // the spinner's gone but idle isn't visible: one more look first
        p.title = "Claude Code".into();
        p.screen = "nothing to see".into();
        assert_eq!(tick(&mut d, &procs, &all, &mut p, false), vec![]);
        assert_eq!(
            tick(&mut d, &procs, &all, &mut p, false),
            vec![Change { pane: "p1".into(), from: Some(AgentState::Working), to: AgentState::Done }]
        );
        assert_eq!(
            tick(&mut d, &procs, &all, &mut p, true),
            vec![Change { pane: "p1".into(), from: Some(AgentState::Done), to: AgentState::Idle }]
        );
        // the agent exits: the shell owns the terminal again, and nothing reads its screen
        let shell_only = table(&[(10, 1, 10, "-zsh")]);
        let reads = p.screens_read;
        assert_eq!(
            tick(&mut d, &shell_only, &all, &mut p, true),
            vec![Change { pane: "p1".into(), from: None, to: AgentState::Idle }]
        );
        assert!(p.info.agent.is_none() && !d.last.contains_key("p1"));
        assert_eq!(p.screens_read, reads);
    }

    #[test]
    fn an_exited_pane_finishes_its_agent() {
        let all = agents();
        let mut d = Detector::new();
        let mut p = FakePane::new("p1", 10, Some("claude-code"));
        p.info.agent =
            Some(AgentInfo { harness: "claude-code".into(), state: AgentState::Working, source: "screen".into() });
        p.info.status = "exited".into();
        assert_eq!(
            tick(&mut d, &ProcessTable::new(), &all, &mut p, false),
            vec![Change { pane: "p1".into(), from: Some(AgentState::Working), to: AgentState::Done }]
        );
        assert_eq!(tick(&mut d, &ProcessTable::new(), &all, &mut p, false), vec![]);
    }

    #[test]
    fn a_reported_state_wins_until_its_agent_exits_but_a_visible_dialog_wins_over_it() {
        let all = agents();
        let procs = table(&[(10, 1, 20, "-zsh"), (20, 10, 20, "claude")]);
        let mut d = Detector::new();
        let mut p = FakePane::new("p1", 10, None);
        tick(&mut d, &procs, &all, &mut p, true); // sees the agent's pid
        assert!(d.report("p1", "claude-hooks", Some("claude-code"), AgentState::Working, Some(2.0)));
        assert!(!d.report("p1", "claude-hooks", None, AgentState::Idle, Some(1.0))); // stale
        assert_eq!(d.authority["p1"].pid, Some(20));
        tick(&mut d, &procs, &all, &mut p, true);
        assert_eq!(
            (d.last["p1"].raw, d.last["p1"].source.as_str(), d.last["p1"].rule.as_deref()),
            (Raw::Working, "hook", Some("claude-hooks report"))
        );
        assert_eq!(p.info.agent.as_ref().unwrap().source, "hook");
        // a permission prompt on screen beats the report
        p.screen = fixture("claude-code", "permission");
        tick(&mut d, &procs, &all, &mut p, true);
        assert_eq!((d.last["p1"].raw, d.last["p1"].source.as_str()), (Raw::Blocked, "screen"));
        // the agent exits: its authority goes with it
        let shell_only = table(&[(10, 1, 10, "-zsh")]);
        tick(&mut d, &shell_only, &all, &mut p, true);
        assert!(d.authority.is_empty());
        // JSON with the TS field names, a whole seq as an integer
        assert!(d.report("p1", "plugin", Some("omp"), AgentState::Done, Some(3.0)));
        assert_eq!(
            serde_json::to_string(&d.authority["p1"]).unwrap(),
            r#"{"source":"plugin","agent":"omp","state":"idle","seq":3}"#
        );
        d.release("p1", "other");
        assert!(d.authority.contains_key("p1"));
        d.release("p1", "plugin");
        assert!(d.authority.is_empty());
    }

    #[test]
    fn a_plugin_names_an_agent_no_process_identifies() {
        let all = agents();
        let procs = table(&[(10, 1, 30, "-zsh"), (30, 10, 30, "node /opt/thing/main.js")]);
        let mut d = Detector::new();
        let mut p = FakePane::new("p1", 10, None);
        assert_eq!(tick(&mut d, &procs, &all, &mut p, false), vec![]);
        d.report("p1", "plugin", Some("omp"), AgentState::Blocked, None);
        let c = tick(&mut d, &procs, &all, &mut p, false);
        assert_eq!(c, vec![Change { pane: "p1".into(), from: None, to: AgentState::Blocked }]);
        assert_eq!(d.authority["p1"].pid, Some(30)); // learnt on the next look
        let det = serde_json::to_value(&d.last["p1"]).unwrap();
        assert_eq!(
            det,
            serde_json::json!({ "harness": "omp", "raw": "blocked", "rule": "plugin report", "source": "hook", "fg": "node /opt/thing/main.js" })
        );
    }
}
