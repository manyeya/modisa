// Agents (adapters): how to recognise an agent, launch it, and read its state from the screen.
// Built-ins live in ./agents; ~/.config/modisa/adapters/<id>.toml adds an agent or overrides one
// (any field; `rules` replaces its screen rules), and [agents.<id>] in config.toml sets launch/resume.
use std::path::Path;

use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;

use super::agents::{builtin_agents, AgentDef, RawGate, RawRule, RuleState};
use super::{overlay, toml_error, Config, CONFIG_DIR};

pub type Adapter = AgentDef;

// An adapter file: any of an adapter's fields, over the built-in of its id if there is one.
// ponytail: a field of the wrong type makes the whole file ignored, with why; the TS took whatever was there.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AdapterFile {
    id: Option<String>,
    name: Option<String>,
    process: Option<Vec<String>>,
    launch: Option<String>,
    resume: Option<String>,
    resume_session: Option<String>,
    activity: Option<bool>,
    rules: Option<Vec<RawRule>>,
    state: Option<Vec<OldState>>,
}

// The older format, still accepted: [[state]] name/match/region (last N non-blank lines), first match wins.
#[derive(Deserialize)]
struct OldState {
    name: Option<RuleState>,
    #[serde(rename = "match")]
    pattern: String,
    region: Option<i64>,
}

fn from_states(states: &[OldState]) -> Vec<RawRule> {
    let n = states.len();
    states
        .iter()
        .enumerate()
        .map(|(i, s)| RawRule {
            id: format!("state-{}", i + 1),
            state: s.name,
            priority: Some((n - i) as i64),
            region: Some(match s.region {
                Some(lines) if lines != 0 => format!("bottom_non_empty_lines({lines})"),
                _ => "whole_recent".into(),
            }),
            gate: RawGate { regex: Some(vec![format!("(?im){}", s.pattern)]), ..RawGate::default() },
            ..RawRule::default()
        })
        .collect()
}

fn from_file(id: String, raw: AdapterFile, base: Option<&Adapter>) -> Adapter {
    let rules = raw.rules.or_else(|| raw.state.map(|s| from_states(&s)));
    Adapter {
        name: raw.name.or_else(|| base.map(|b| b.name.clone())).unwrap_or_else(|| id.clone()),
        process: raw.process.or_else(|| base.map(|b| b.process.clone())).unwrap_or_else(|| vec![id.clone()]),
        launch: raw.launch.or_else(|| base.map(|b| b.launch.clone())).unwrap_or_else(|| id.clone()),
        resume: raw.resume.or_else(|| base.and_then(|b| b.resume.clone())),
        resume_session: raw.resume_session.or_else(|| base.and_then(|b| b.resume_session.clone())),
        activity: raw.activity.or_else(|| base.map(|b| b.activity)).unwrap_or(false),
        rules: rules.or_else(|| base.map(|b| b.rules.clone())).unwrap_or_default(),
        id,
    }
}

pub fn load_adapters(cfg: &Config) -> Vec<Adapter> {
    load_from(builtin_agents(), Path::new(&format!("{}/adapters", *CONFIG_DIR)), cfg)
}

fn load_from(builtin: Vec<Adapter>, dir: &Path, cfg: &Config) -> Vec<Adapter> {
    let mut by_id: IndexMap<String, Adapter> = builtin.into_iter().map(|a| (a.id.clone(), a)).collect();
    // *.toml as Bun's glob finds them: files, not hidden ones
    let mut files: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| {
            let names = entries.filter_map(|e| e.ok()?.file_name().into_string().ok());
            names.filter(|f| f.ends_with(".toml") && !f.starts_with('.') && dir.join(f).is_file()).collect()
        })
        .unwrap_or_default();
    files.sort();
    for f in files {
        let path = dir.join(&f);
        let parsed = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|text| toml::from_str::<AdapterFile>(&text).map_err(|e| toml_error(&e, &text).describe()));
        match parsed {
            Ok(raw) => {
                let id = raw.id.clone().unwrap_or_else(|| f.strip_suffix(".toml").unwrap_or(&f).to_string());
                let adapter = from_file(id.clone(), raw, by_id.get(&id));
                by_id.insert(id, adapter);
            }
            Err(e) => eprintln!("modisa: ignoring {}: {e}", path.display()),
        }
    }
    for (id, o) in &cfg.agents {
        if let Some(a) = by_id.get_mut(id) {
            *a = overlay(a.clone(), Some(&Value::Object(o.clone())));
        }
    }
    by_id.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{defaults, merge, parse_toml};

    fn builtin() -> Vec<Adapter> {
        let rule = RawRule { id: "prompt".into(), state: Some(RuleState::Idle), gate: RawGate { contains: Some(vec!["> ".into()]), ..RawGate::default() }, ..RawRule::default() };
        vec![Adapter { id: "alpha".into(), name: "Alpha".into(), process: vec!["alpha".into(), "alpha-cli".into()], launch: "alpha".into(), resume: None, resume_session: Some("alpha --session {id}".into()), activity: true, rules: vec![rule] }]
    }

    #[test]
    fn adapter_files_add_and_override_agents_and_config_sets_their_fields() {
        let dir = crate::config::tests::scratch("adapters");
        let write = |f: &str, s: &str| std::fs::write(dir.join(f), s).unwrap();
        write("alpha.toml", "launch = \"alpha --fast\"\n"); // over the built-in: only what it says changes
        write("beta.toml", "[[state]]\nname = \"working\"\nmatch = \"esc to interrupt\"\nregion = 8\n\n[[state]]\nname = \"blocked\"\nmatch = 'allow\\?'\n");
        write("gamma.toml", "id = \"delta\"\nname = \"Delta\"\n[[rules]]\nid = \"ask\"\nstate = \"blocked\"\npriority = 5\nregion = \"osc_title\"\ncontains = [\"Allow?\"]\n");
        write("bad.toml", "name = \n"); // doesn't parse: ignored
        write("worse.toml", "activity = \"yes\"\n"); // the wrong type: ignored
        write(".hidden.toml", "name = \"Hidden\"\n");
        write("notes.txt", "name = \"Notes\"\n");
        let cfg = merge(&parse_toml("[agents.alpha]\nresume = \"alpha --continue\"\n[agents.beta]\nlaunch = \"beta --x\"\nname = 3\n[agents.nobody]\nlaunch = \"x\"\n").unwrap());
        let got = load_from(builtin(), &dir, &cfg);
        assert_eq!(got.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["alpha", "beta", "delta"]);

        let alpha = &got[0];
        assert_eq!(alpha, &Adapter { launch: "alpha --fast".into(), resume: Some("alpha --continue".into()), ..builtin().remove(0) });

        let beta = &got[1];
        assert_eq!((beta.name.as_str(), beta.process.clone(), beta.launch.as_str(), beta.activity), ("beta", vec!["beta".to_string()], "beta --x", false));
        assert_eq!(
            serde_json::to_value(&beta.rules).unwrap(),
            serde_json::json!([
                { "id": "state-1", "state": "working", "priority": 2, "region": "bottom_non_empty_lines(8)", "regex": ["(?im)esc to interrupt"] },
                { "id": "state-2", "state": "blocked", "priority": 1, "region": "whole_recent", "regex": ["(?im)allow\\?"] },
            ])
        );

        let delta = &got[2];
        assert_eq!((delta.name.as_str(), delta.launch.as_str(), delta.rules.len()), ("Delta", "delta", 1));
        assert_eq!((delta.rules[0].state, delta.rules[0].priority, delta.rules[0].gate.contains.clone()), (Some(RuleState::Blocked), Some(5), Some(vec!["Allow?".to_string()])));

        // no adapters directory: the built-ins as they are
        assert_eq!(load_from(builtin(), &dir.join("absent"), &defaults()), builtin());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
