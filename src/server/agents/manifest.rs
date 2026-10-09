// Screen manifests: per-agent rules that read an agent's live UI (and its terminal title / OSC 9;4 progress) to decide
// idle, working or blocked: rules with priorities, regions and nested all/any/not gates, in the manifest format of
// ../../config/agents/manifests (Apache-2.0).
//
// The manifests' regexes are written in Rust regex syntax (inline flags, \x{…}, \A / \z), so the regex crate takes
// them as they are; the TypeScript original had to translate them to JavaScript.
use regex::Regex;

pub use crate::config::agents::{RawGate, RawRule, RuleState};

#[derive(Debug)]
pub struct Gate {
    pub all: Vec<Gate>,
    pub any: Vec<Gate>,
    pub not: Vec<Gate>,
    pub contains: Vec<String>, // lowercased
    pub regex: Vec<Regex>,
    pub line_regex: Vec<Regex>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Visible {
    pub idle: bool,
    pub blocker: bool,
}

#[derive(Debug)]
pub struct Rule {
    pub id: String,
    pub state: RuleState,
    pub priority: i64,
    pub region: String,
    pub visible: Visible,
    pub skip: bool,
    pub gate: Gate,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Evidence<'a> {
    pub screen: &'a str,
    pub title: &'a str,
    pub progress: &'a str,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Verdict {
    pub state: RuleState,
    pub rule: Option<String>,
    pub visible_idle: bool,
    pub visible_blocker: bool,
    pub skip: bool,
}

fn compile_list(list: &Option<Vec<String>>) -> Result<Vec<Regex>, regex::Error> {
    list.iter().flatten().map(|p| Regex::new(p)).collect()
}

fn compile_gates(list: &Option<Vec<RawGate>>) -> Result<Vec<Gate>, regex::Error> {
    list.iter().flatten().map(compile_gate).collect()
}

fn compile_gate(g: &RawGate) -> Result<Gate, regex::Error> {
    Ok(Gate {
        all: compile_gates(&g.all)?,
        any: compile_gates(&g.any)?,
        not: compile_gates(&g.not)?,
        contains: g.contains.iter().flatten().map(|s| s.to_lowercase()).collect(),
        regex: compile_list(&g.regex)?,
        line_regex: compile_list(&g.line_regex)?,
    })
}

pub fn compile_rules(raw: &[RawRule]) -> Result<Vec<Rule>, String> {
    raw.iter()
        .map(|r| {
            Ok(Rule {
                id: r.id.clone(),
                state: r.state.unwrap_or(RuleState::Unknown),
                priority: r.priority.unwrap_or(0),
                region: r.region.as_deref().unwrap_or("whole_recent").trim().to_string(),
                visible: Visible {
                    idle: r.visible_idle.unwrap_or(false),
                    blocker: r.visible_blocker.unwrap_or(false),
                },
                skip: r.skip_state_update.unwrap_or(false),
                gate: compile_gate(&r.gate).map_err(|e| format!("rule {}: {e}", r.id))?,
            })
        })
        .collect()
}

fn matches(g: &Gate, text: &str, lower: &str, lines: &[&str]) -> bool {
    g.contains.iter().all(|c| lower.contains(c.as_str()))
        && g.regex.iter().all(|r| r.is_match(text))
        && g.line_regex.iter().all(|r| lines.iter().any(|l| r.is_match(l)))
        && g.all.iter().all(|n| matches(n, text, lower, lines))
        && (g.any.is_empty() || g.any.iter().any(|n| matches(n, text, lower, lines)))
        && !g.not.iter().any(|n| matches(n, text, lower, lines))
}

// The highest-priority matching rule wins (the earlier one on a tie); no match means idle.
pub fn evaluate(rules: &[Rule], input: &Evidence) -> Verdict {
    let mut best: Option<&Rule> = None;
    for rule in rules {
        if best.is_some_and(|b| b.priority >= rule.priority) {
            continue;
        }
        let text = region(input, &rule.region);
        let lines = split_lines(text);
        if matches(&rule.gate, text, &text.to_lowercase(), &lines) {
            best = Some(rule);
        }
    }
    match best {
        None => {
            Verdict { state: RuleState::Idle, rule: None, visible_idle: false, visible_blocker: false, skip: false }
        }
        Some(b) => Verdict {
            state: b.state,
            rule: Some(b.id.clone()),
            visible_idle: b.visible.idle && b.state == RuleState::Idle,
            visible_blocker: b.visible.blocker && b.state == RuleState::Blocked,
            skip: b.skip,
        },
    }
}

// ---------- regions: which part of the screen a rule reads ----------

// Like JavaScript's split("\n") with the TS port's tweaks: no trailing empty line, and \r endings trimmed (a lone \r
// at the very end too, unlike str::lines).
fn split_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return vec![];
    }
    let mut lines: Vec<&str> = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect();
    if text.ends_with('\n') {
        lines.pop();
    }
    lines
}

// Byte offset where line `index` starts (the end of the content past the last line). The lines are slices of the
// content, so this is exact.
// ponytail: the TS summed each line's length plus its \n, which drifts by one per \r\n line; screens never have \r.
fn line_start(content: &str, lines: &[&str], index: usize) -> usize {
    lines.get(index).map_or(content.len(), |l| l.as_ptr() as usize - content.as_ptr() as usize)
}

// "bottom_lines(8)" → 8
fn count(spec: &str, name: &str) -> Option<usize> {
    let n = spec.strip_prefix(name)?.strip_prefix('(')?.strip_suffix(')')?;
    (!n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())).then(|| n.parse().unwrap_or(usize::MAX))
}

fn non_empty(lines: &[&str]) -> Vec<usize> {
    lines.iter().enumerate().filter(|(_, l)| !l.trim().is_empty()).map(|(i, _)| i).collect()
}

pub fn region<'a>(input: &Evidence<'a>, spec: &str) -> &'a str {
    match spec {
        "osc_title" => return input.title,
        "osc_progress" => return input.progress,
        _ => {}
    }
    let content = input.screen;
    let lines = split_lines(content);
    let from = |index: usize| &content[line_start(content, &lines, index)..];
    match spec {
        "whole_recent" => return content,
        "after_last_prompt_marker" => {
            return match lines.iter().rposition(|l| codex_prompt(l)) {
                None => content,
                Some(i) => from(i + 1),
            };
        }
        "before_current_prompt_marker" => {
            return match current_codex_prompt(&lines) {
                None => content,
                Some(i) => &content[..line_start(content, &lines, i)],
            };
        }
        "whole_recent_without_current_prompt_marker" => {
            return if current_codex_prompt(&lines).is_none() { content } else { "" };
        }
        "current_prompt_block_marker" => {
            return current_codex_prompt(&lines)
                .and_then(|p| lines[..p].iter().rev().find(|l| codex_block_marker(l)).copied())
                .unwrap_or("");
        }
        "after_current_prompt_block_marker" => {
            let b = current_codex_prompt(&lines).and_then(|p| lines[..p].iter().rposition(|l| codex_block_marker(l)));
            return b.map_or("", from);
        }
        "prompt_box_body" => {
            let Some(top) = prompt_box_top(&lines) else { return "" };
            let end =
                lines[top + 1..].iter().position(|l| is_horizontal_rule(l)).map_or(lines.len(), |rel| top + 1 + rel);
            return &content[line_start(content, &lines, top + 1)..line_start(content, &lines, end)];
        }
        "above_prompt_box" => return above_box(content, &lines),
        "last_non_empty_above_prompt_box" => {
            return split_lines(above_box(content, &lines))
                .into_iter()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("");
        }
        "after_last_horizontal_rule" => {
            let end =
                lines.iter().rposition(|l| is_horizontal_rule(l)).map_or(0, |i| line_start(content, &lines, i + 1));
            return &content[end..];
        }
        _ => {}
    }
    if let Some(n) = count(spec, "bottom_lines") {
        return from(lines.len().saturating_sub(n));
    }
    if let Some(n) = count(spec, "bottom_non_empty_lines") {
        let ne = non_empty(&lines);
        return if ne.is_empty() || n == 0 { "" } else { from(ne[ne.len().saturating_sub(n)]) };
    }
    if let Some(n) = count(spec, "top_non_empty_lines") {
        let ne = non_empty(&lines);
        return if ne.is_empty() || n == 0 {
            ""
        } else {
            &content[..line_start(content, &lines, ne[n.min(ne.len()) - 1] + 1)]
        };
    }
    ""
}

fn codex_prompt(l: &str) -> bool {
    l == "›" || l.starts_with("› ")
}

fn codex_block_marker(l: &str) -> bool {
    l.starts_with(['•', '■', '✗', '✓'])
}

// Codex's live prompt: the last › line with no transcript block after it.
fn current_codex_prompt(lines: &[&str]) -> Option<usize> {
    let i = lines.iter().rposition(|l| codex_prompt(l))?;
    (!lines[i + 1..].iter().any(|l| codex_block_marker(l))).then_some(i)
}

// A rule line of ─ (optionally with a short label after at least three of them).
fn is_horizontal_rule(line: &str) -> bool {
    let t = line.trim();
    let rest = t.trim_start_matches('─');
    let rule = (t.len() - rest.len()) / '─'.len_utf8();
    rule > 0 && (rest.trim_start().is_empty() || rule >= 3)
}

// The prompt box is framed by the last two horizontal rules; its top is the second-to-last.
fn prompt_box_top(lines: &[&str]) -> Option<usize> {
    lines.iter().enumerate().rev().filter(|(_, l)| is_horizontal_rule(l)).nth(1).map(|(i, _)| i)
}

fn above_box<'a>(content: &'a str, lines: &[&str]) -> &'a str {
    match prompt_box_top(lines) {
        None => content,
        Some(top) => &content[..line_start(content, lines, top)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::agents::builtin_agents;

    fn ev(screen: &str) -> Evidence<'_> {
        Evidence { screen, ..Default::default() }
    }

    fn raw(id: &str, state: RuleState, priority: i64, gate: RawGate) -> RawRule {
        RawRule { id: id.into(), state: Some(state), priority: Some(priority), gate, ..Default::default() }
    }

    fn contains(s: &[&str]) -> RawGate {
        RawGate { contains: Some(s.iter().map(|s| s.to_string()).collect()), ..Default::default() }
    }

    #[test]
    fn every_bundled_agents_screen_rules_compile() {
        let agents = builtin_agents();
        assert!(agents.len() >= 25);
        for a in &agents {
            let rules = compile_rules(&a.rules).unwrap_or_else(|e| panic!("{}: {e}", a.id));
            assert_eq!(rules.len(), a.rules.len());
        }
    }

    #[test]
    fn the_rule_engine_highest_priority_wins_not_gates_exclude_viewers_keep_the_last_state() {
        let mut high = raw("high", RuleState::Blocked, 5, contains(&["busy"]));
        high.gate.not = Some(vec![contains(&["ignore me"])]);
        let mut viewer = raw(
            "viewer",
            RuleState::Unknown,
            9,
            RawGate { line_regex: Some(vec!["(?i)^transcript$".into()]), ..Default::default() },
        );
        viewer.region = Some("bottom_non_empty_lines(1)".into());
        viewer.skip_state_update = Some(true);
        let rules = compile_rules(&[raw("low", RuleState::Working, 1, contains(&["busy"])), high, viewer]).unwrap();
        assert_eq!(evaluate(&rules, &ev("busy")).rule.as_deref(), Some("high"));
        assert_eq!(evaluate(&rules, &ev("busy, ignore me")).rule.as_deref(), Some("low"));
        let v = evaluate(&rules, &ev("busy\nTRANSCRIPT"));
        assert_eq!((v.rule.as_deref(), v.skip), (Some("viewer"), true));
        let none = evaluate(&rules, &ev("nothing"));
        assert_eq!((none.state, none.rule), (RuleState::Idle, None));
    }

    #[test]
    fn a_tie_keeps_the_earlier_rule_and_any_needs_one() {
        let mut either = raw("either", RuleState::Working, 3, RawGate::default());
        either.gate.any = Some(vec![contains(&["foo"]), contains(&["bar"])]);
        let rules = compile_rules(&[raw("first", RuleState::Blocked, 3, contains(&["bar"])), either]).unwrap();
        assert_eq!(evaluate(&rules, &ev("BAR")).rule.as_deref(), Some("first")); // contains is case-insensitive
        assert_eq!(evaluate(&rules, &ev("foo")).rule.as_deref(), Some("either"));
        assert_eq!(evaluate(&rules, &ev("baz")).rule, None);
        let bad = compile_rules(&[raw(
            "broken",
            RuleState::Idle,
            1,
            RawGate { regex: Some(vec!["(".into()]), ..Default::default() },
        )]);
        assert!(bad.unwrap_err().starts_with("rule broken: "));
    }

    #[test]
    fn regions_and_manifest_regexes_written_in_rust_syntax_work() {
        let screen = "old ─── line\nhistory\n────────\n❯ typed\n────────\n  footer\n\n";
        assert_eq!(region(&ev(screen), "bottom_non_empty_lines(2)"), "────────\n  footer\n\n");
        assert_eq!(region(&ev(screen), "prompt_box_body"), "❯ typed\n");
        assert_eq!(region(&ev(screen), "after_last_horizontal_rule"), "  footer\n\n");
        assert_eq!(region(&ev(screen), "top_non_empty_lines(1)"), "old ─── line\n");
        assert_eq!(region(&ev(screen), "above_prompt_box"), "old ─── line\nhistory\n");
        assert_eq!(region(&ev(screen), "last_non_empty_above_prompt_box"), "history");
        assert_eq!(region(&ev(screen), "bottom_lines(2)"), "  footer\n\n");
        assert_eq!(region(&ev(screen), "whole_recent"), screen);
        assert_eq!(region(&ev(screen), "nonsense"), "");
        assert_eq!(region(&Evidence { screen, title: "⠋ Claude", progress: "" }, "osc_title"), "⠋ Claude");
        assert!(Regex::new(r"^[\x{2800}-\x{28FF}] ").unwrap().is_match("⠙ task"));
        assert!(Regex::new(r"(?i)\Adone\z").unwrap().is_match("DONE"));
        assert!(!Regex::new(r"(?i)\Adone\z").unwrap().is_match("x\ndone"));
    }

    #[test]
    fn codex_prompt_regions() {
        let screen = "• Ran tests\n› old ask\n■ Error\n› \n  gpt · ~/x\n";
        assert_eq!(region(&ev(screen), "after_last_prompt_marker"), "  gpt · ~/x\n");
        assert_eq!(region(&ev(screen), "before_current_prompt_marker"), "• Ran tests\n› old ask\n■ Error\n");
        assert_eq!(region(&ev(screen), "whole_recent_without_current_prompt_marker"), "");
        assert_eq!(region(&ev(screen), "current_prompt_block_marker"), "■ Error");
        assert_eq!(region(&ev(screen), "after_current_prompt_block_marker"), "■ Error\n› \n  gpt · ~/x\n");
        // a block after the last prompt: no live prompt
        let busy = "› ask\n• Working (3s • esc to interrupt)\n";
        assert_eq!(region(&ev(busy), "whole_recent_without_current_prompt_marker"), busy);
        assert_eq!(region(&ev(busy), "current_prompt_block_marker"), "");
        assert_eq!(region(&ev(busy), "before_current_prompt_marker"), busy);
    }

    #[test]
    fn horizontal_rules_take_a_short_label_after_three() {
        assert!(is_horizontal_rule("  ─  "));
        assert!(is_horizontal_rule("─── Plan ───"));
        assert!(!is_horizontal_rule("── x"));
        assert!(!is_horizontal_rule("x ───"));
        assert!(!is_horizontal_rule(""));
    }
}
