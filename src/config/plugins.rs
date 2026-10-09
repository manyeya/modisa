// Linked plugins: ~/.config/modisa/plugins/<name> links to a directory holding a plugin.json. Plugins fetched with
// `modisa plugin install` live under <state>/plugins-src/<name>: the checkout, and modisa's own install record.
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::CONFIG_DIR;
use crate::core::paths::DIR;
use crate::protocol::plugin::{check_manifest, describe, PluginManifest};

pub static PLUGINS_DIR: LazyLock<String> = LazyLock::new(|| format!("{}/plugins", *CONFIG_DIR));
pub static MANAGED_DIR: LazyLock<String> = LazyLock::new(|| format!("{}/plugins-src", *DIR));

// Written by `plugin install` next to (not inside) the checkout. It's the only thing that makes a checkout modisa's to
// delete: a manifest or a path can't claim that. marketplace: installed as <name>@<marketplace>.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallRecord {
    pub name: String,
    pub source: String,
    pub r#ref: Option<String>,
    pub commit: String,
    pub checkout: String,
    pub dir: String,
    pub subdir: Option<String>,
    pub installed_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marketplace: Option<String>,
}

pub fn read_install(name: &str) -> Option<InstallRecord> {
    serde_json::from_str(&std::fs::read_to_string(format!("{}/{name}/install.json", *MANAGED_DIR)).ok()?).ok()
}

pub fn write_install(record: &InstallRecord) -> std::io::Result<()> {
    let dir = format!("{}/{}", *MANAGED_DIR, record.name);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(format!("{dir}/install.json"), serde_json::to_string_pretty(record).unwrap() + "\n")
}

// user:password@ or token@ in a URL is never recorded or shown
pub fn without_credentials(url: &str) -> String {
    static CREDENTIALS: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"(?i)^([a-z][a-z0-9+.-]*://)[^/@]*@").unwrap());
    CREDENTIALS.replace(url, "$1").into_owned()
}

// Paths checked by where they really are: a checkout's files (plugin.json, a subdir, a marketplace's source) must be
// inside it, not reached through .. or a symlink out. real() is "" for a path that doesn't exist.
pub fn real(path: &str) -> String {
    std::fs::canonicalize(path).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
}

pub fn inside(path: &str, root: &str) -> bool {
    path == root || path.starts_with(&format!("{root}/"))
}

pub fn is_dir(path: &str) -> bool {
    std::path::Path::new(path).is_dir()
}

#[derive(Clone, Debug)]
pub struct Linked {
    pub name: String,
    pub dir: String,
    pub error: Option<String>,
}

// plugin.json, validated; an error that says what to fix otherwise
pub fn read_manifest(dir: &str) -> Result<PluginManifest, String> {
    let file = format!("{dir}/plugin.json");
    let Ok(text) = std::fs::read_to_string(&file) else { return Err(format!("no plugin.json in {dir}")) };
    let raw: Value = serde_json::from_str(&text).map_err(|e| format!("{file} isn't valid JSON: {e}"))?;
    check_manifest(&raw).map_err(|issues| format!("{file}: {}", describe(&issues)))
}

pub fn linked_plugins() -> Vec<Linked> {
    let mut names: Vec<String> = std::fs::read_dir(&*PLUGINS_DIR).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| !n.starts_with('.')).collect();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let dir = real(&format!("{}/{name}", *PLUGINS_DIR));
            if dir.is_empty() {
                return Linked { dir: format!("{}/{name}", *PLUGINS_DIR), name, error: Some("the link points nowhere (was the directory moved?)".into()) };
            }
            match read_manifest(&dir) {
                Ok(m) if m.name != name => Linked { error: Some(format!("linked as {name}, but plugin.json names it {}", m.name)), name, dir },
                Ok(_) => Linked { name, dir, error: None },
                Err(e) => Linked { name, dir, error: Some(e) },
            }
        })
        .collect()
}

// The install record of each linked plugin `plugin install` fetched (the link is the one it made)
pub fn installs() -> Vec<InstallRecord> {
    linked_plugins().into_iter().filter_map(|l| read_install(&l.name).filter(|r| r.dir == l.dir)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_credentials() {
        assert_eq!(without_credentials("https://user:pw@github.com/a/b"), "https://github.com/a/b");
        assert_eq!(without_credentials("https://github.com/a/b"), "https://github.com/a/b");
        assert!(inside("/a/b", "/a") && inside("/a", "/a") && !inside("/ab", "/a"));
    }
}
