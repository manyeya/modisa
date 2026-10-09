// `modisa plugin search [words]`: plugins published on GitHub with the modisa-tui-plugin topic, most starred first,
// and those your marketplaces list, each with the command that installs it. Nothing is installed or run, and nothing
// in the list is vetted.
// MODISA_PLUGIN_INDEX points the search at another API base (a mirror, a test server), or turns it "off".
use std::process::Stdio;

use serde::Serialize;
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;

use super::commands::{js_str, stringify, truthy};
use crate::config::marketplaces::{js_space, marketplace_plugins};
use crate::core::text::clean_text;

pub const TOPIC: &str = "modisa-tui-plugin";
const DEFAULT_INDEX: &str = "https://api.github.com";

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Found {
    pub name: String,
    pub repo: String,
    pub url: String,
    pub description: String,
    pub stars: Value,
    pub updated: String,
    pub created: String,
    pub archived: bool,
    pub install: String,
}

pub struct Search {
    pub total: Value,
    pub results: Vec<Found>,
}

// Text from the index is a stranger's: cleaned of escape, control and bidi characters, and cut to terminal cells.
fn plain(text: Option<&Value>, cells: usize) -> String {
    let s = match text {
        None | Some(Value::Null) => String::new(),
        Some(v) => js_str(v),
    };
    let mut collapsed = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars() {
        if js_space(c) {
            if !space {
                collapsed.push(' ');
            }
            space = true;
        } else {
            collapsed.push(c);
            space = false;
        }
    }
    clean_text(&collapsed, cells).trim_matches(js_space).to_string()
}

fn failed(message: &str, json: bool) -> i32 {
    if json {
        errln!("{}", stringify(&json!({ "error": { "code": "unreachable", "message": message } }), false));
    } else {
        errln!("modisa: {message}");
    }
    3
}

// encodeURIComponent
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

// An HTTP status and body, or why the index couldn't be reached.
// ponytail: curl fetches it (no HTTP client among the crates); the headers go on its stdin, so a token never shows in ps
async fn get(url: &str, headers: &[String]) -> Result<(u16, Vec<u8>), String> {
    let mut child = tokio::process::Command::new("curl")
        .args(["-sS", "-L", "--max-time", "8", "-H", "@-", "-w", "\n%{http_code}", "--url", url])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all((headers.join("\n") + "\n").as_bytes()).await;
    }
    let out = child.wait_with_output().await.map_err(|e| e.to_string())?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let err = err.strip_prefix("curl: ").map(|e| e.split_once(") ").map_or(e, |(_, m)| m)).unwrap_or(&err).to_string();
        return Err(if err.is_empty() { format!("curl exited with {}", out.status.code().unwrap_or(-1)) } else { err });
    }
    let body = out.stdout;
    let at = body.iter().rposition(|&b| b == b'\n').unwrap_or(0);
    let status = String::from_utf8_lossy(&body[(at + 1).min(body.len())..]).trim().parse().unwrap_or(0);
    Ok((status, body[..at].to_vec()))
}

// The plugins with the topic (and `words`), most starred first: what `plugin search` prints and the site's plugin
// directory lists. A message for the user when the index can't be read.
pub async fn find(words: &[String], limit: usize) -> Result<Search, String> {
    let base = std::env::var("MODISA_PLUGIN_INDEX").unwrap_or_else(|_| DEFAULT_INDEX.into()).trim_end_matches('/').to_string();
    if base == "off" {
        return Err("plugin search is turned off (MODISA_PLUGIN_INDEX=off)".into());
    }
    let q = std::iter::once(format!("topic:{TOPIC}")).chain(words.iter().cloned()).collect::<Vec<_>>().join(" ");
    let url = format!("{base}/search/repositories?q={}&sort=stars&order=desc&per_page={limit}", encode(&q));
    let mut headers = vec!["accept: application/vnd.github+json".to_string(), "user-agent: modisa".to_string()];
    // a token only ever goes to GitHub itself, never to an overriding index
    if let Some(token) = std::env::var("GITHUB_TOKEN").ok().filter(|t| !t.is_empty() && base == DEFAULT_INDEX) {
        headers.push(format!("authorization: Bearer {token}"));
    }
    let (status, body) = get(&url, &headers).await.map_err(|e| format!("couldn't reach the plugin index at {base}: {e}"))?;
    if status == 403 || status == 429 {
        return Err(format!("the plugin index at {base} is rate-limiting searches: try again in a minute{}", if base == DEFAULT_INDEX { ", or set GITHUB_TOKEN" } else { "" }));
    }
    if !(200..300).contains(&status) {
        return Err(format!("the plugin index at {base} answered {status}"));
    }
    let body: Option<Value> = serde_json::from_slice(&body).ok();
    let Some(items) = body.as_ref().and_then(|b| b.get("items")).and_then(Value::as_array) else {
        return Err(format!("the plugin index at {base} sent something that isn't a search result"));
    };
    let results: Vec<Found> = items
        .iter()
        .filter(|r| r["clone_url"].as_str().is_some_and(|u| u.starts_with("https://"))) // only what install can fetch as-is
        .map(|r| Found {
            name: plain(r.get("name"), 100),
            repo: plain(r.get("full_name"), 200),
            url: plain(r.get("html_url"), 300),
            description: plain(r.get("description"), 200),
            stars: r.get("stargazers_count").filter(|s| s.is_number()).cloned().unwrap_or(json!(0)),
            updated: plain(r.get("pushed_at"), 40),
            created: plain(r.get("created_at"), 40),
            archived: truthy(r.get("archived")),
            install: format!("modisa plugin install {}", plain(r.get("clone_url"), 300)),
        })
        .collect();
    let total = body.as_ref().and_then(|b| b.get("total_count")).filter(|t| t.is_number()).cloned().unwrap_or(json!(results.len()));
    Ok(Search { total, results })
}

// The index's plugins, and the plugins your marketplaces list (`plugin marketplace add`), each labelled with where
// it's from. With a marketplace that matches, an index that can't be read is noted rather than fatal.
pub async fn search(words: &[String], json: bool) -> i32 {
    let query = words.join(" ").trim_matches(js_space).to_string();
    let listed = marketplace_plugins(words);
    let (found, index_error) = match find(words, 30).await {
        Ok(f) => (f, None),
        Err(e) if listed.is_empty() => return failed(&e, json),
        Err(e) => (Search { total: json!(0), results: vec![] }, Some(e)),
    };

    if json {
        let mut out = json!({ "query": query, "total": found.total, "results": found.results });
        if !listed.is_empty() {
            out["marketplacePlugins"] = serde_json::to_value(&listed).unwrap_or_default();
        }
        if let Some(e) = &index_error {
            out["indexError"] = json!(e);
        }
        outln!("{}", stringify(&out, true));
        return 0;
    }
    if let Some(e) = &index_error {
        errln!("modisa: {e}; listing your marketplaces' plugins only");
    }
    if found.results.is_empty() && listed.is_empty() {
        let matching = if query.is_empty() { String::new() } else { format!(" matching \"{}\"", plain(Some(&json!(query)), 200)) };
        outln!("no plugins with the {TOPIC} topic{matching}");
        return 0;
    }
    for p in &listed {
        outln!("{}@{}  marketplace {}{}", p.name, p.marketplace, p.marketplace, if p.installed { "  (installed)" } else { "" });
        if !p.description.is_empty() {
            outln!("  {}", p.description);
        }
        outln!("  {}", p.install);
    }
    for r in &found.results {
        let updated = if r.updated.is_empty() { String::new() } else { format!("  updated {}", r.updated.chars().take(10).collect::<String>()) };
        outln!("{}  ★ {}{}{updated}", r.repo, js_str(&r.stars), if r.archived { "  (archived)" } else { "" });
        if !r.description.is_empty() {
            outln!("  {}", r.description);
        }
        outln!("  {}", r.install);
    }
    outln!("\nNone of these is vetted: a plugin runs as you, with your files and network. Read it before installing it.");
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_text_is_one_clean_line_cut_by_cells() {
        assert_eq!(plain(Some(&json!("safe\x1b]0;owned\x07 text\u{202e}gpj.exe\ron a new line")), 200), "safe textgpj.exe on a new line");
        assert_eq!(plain(Some(&json!("日".repeat(150))), 200), "日".repeat(100));
        assert_eq!(plain(None, 200), "");
        assert_eq!(encode("topic:modisa-tui-plugin github pr"), "topic%3Amodisa-tui-plugin%20github%20pr");
    }
}
