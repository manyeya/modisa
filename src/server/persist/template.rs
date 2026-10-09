// modisa.toml: a project's starting layout (panes, commands, agents) for a new session.
use serde::Deserialize;

use crate::config::agents::AgentDef;
use crate::core::layout::Axis;
use crate::protocol::conn::{error, RpcResult};
use crate::server::session::SpawnOpts;
use crate::server::Server;

#[derive(Deserialize, Default)]
struct TemplatePane {
    name: Option<String>,
    run: Option<String>,
    agent: Option<String>,
    prompt: Option<String>,
    cwd: Option<String>,
}

#[derive(Deserialize, Default)]
struct Template {
    name: Option<String>,
    pane: Option<Vec<TemplatePane>>,
}

pub fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn template_opts(t: &TemplatePane, adapters: &[AgentDef], cwd: &str) -> RpcResult<SpawnOpts> {
    let a = match &t.agent {
        Some(agent) => Some(adapters.iter().find(|x| &x.id == agent).ok_or_else(|| error(format!("unknown agent \"{agent}\" in modisa.toml")))?),
        None => None,
    };
    let command = match a {
        Some(a) => Some([Some(a.launch.clone()), t.prompt.as_deref().filter(|p| !p.is_empty()).map(quote)].into_iter().flatten().collect::<Vec<_>>().join(" ")),
        None => t.run.clone(),
    };
    let cwd = match &t.cwd {
        Some(c) if c.starts_with('/') => c.clone(),
        Some(c) => format!("{cwd}/{c}"),
        None => cwd.to_string(),
    };
    Ok(SpawnOpts { name: t.name.clone(), command, harness: a.map(|a| a.id.clone()), cwd: Some(cwd), ..Default::default() })
}

// First pane on the left, the rest stacked on the right. Whether the directory had one.
pub fn apply_template(srv: &mut Server, dir: &str) -> RpcResult<bool> {
    let Ok(text) = std::fs::read_to_string(format!("{dir}/modisa.toml")) else { return Ok(false) };
    let t: Template = toml::from_str(&text).map_err(|e| error(format!("modisa.toml: {e}")))?;
    let list = t.pane.unwrap_or_default();
    if list.is_empty() {
        return Ok(false);
    }
    let adapters = srv.adapters.clone();
    let first = srv.s.new_workspace(t.name, Some(dir.into()), template_opts(&list[0], &adapters, dir)?)?;
    let mut prev = first.clone();
    for (i, p) in list[1..].iter().enumerate() {
        let (dir_, at) = if i == 0 { (Axis::Row, first.clone()) } else { (Axis::Col, prev.clone()) };
        if let Some(id) = srv.s.split(dir_, template_opts(p, &adapters, dir)?, Some(&at), false, 0.5)? {
            prev = id;
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    #[test]
    fn quotes_for_a_shell() {
        assert_eq!(super::quote("it's"), "'it'\\''s'");
    }
}
