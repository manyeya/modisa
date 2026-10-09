// What the user let each plugin do through modisa (examples/plugins/TOOLING.md, Permissions):
// ~/.config/modisa/plugin-grants.json, a plugin's name → the permissions granted to it.
use std::collections::{HashMap, HashSet};

use super::CONFIG_DIR;

pub fn path() -> String {
    format!("{}/plugin-grants.json", *CONFIG_DIR)
}

pub fn read() -> HashMap<String, Vec<String>> {
    std::fs::read_to_string(path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn write(all: &HashMap<String, Vec<String>>) -> std::io::Result<()> {
    std::fs::create_dir_all(&*CONFIG_DIR)?;
    let mut names: Vec<_> = all.iter().collect();
    names.sort();
    let ordered: serde_json::Map<String, serde_json::Value> = names.into_iter().map(|(k, v)| (k.clone(), serde_json::json!(v))).collect();
    std::fs::write(path(), serde_json::to_string_pretty(&ordered)? + "\n")
}

// These permissions granted to `name`, on top of what it has.
pub fn grant(name: &str, perms: &[String]) -> std::io::Result<()> {
    let mut all = read();
    let have = all.entry(name.to_string()).or_default();
    for p in perms {
        if !have.contains(p) {
            have.push(p.clone());
        }
    }
    write(&all)
}

// These taken away from `name` (none named: all of them).
pub fn revoke(name: &str, perms: &[String]) -> std::io::Result<()> {
    let mut all = read();
    match all.get_mut(name) {
        Some(have) if !perms.is_empty() => have.retain(|p| !perms.contains(p)),
        Some(have) => have.clear(),
        None => {
            all.insert(name.to_string(), vec![]);
        }
    }
    write(&all)
}

// What a plugin may do: None for one that declares nothing (undeclared: what any plugin could do before permissions);
// else what it declares and was granted, and what it declares but wasn't. One with no grant on record yet has what it
// declares (`plugin install` and `link` record it).
pub fn effective(name: &str, declared: Option<&[String]>) -> (Option<HashSet<String>>, Vec<String>) {
    let Some(declared) = declared else { return (None, vec![]) };
    match read().get(name) {
        None => (Some(declared.iter().cloned().collect()), vec![]),
        Some(granted) => {
            let (ok, missing): (Vec<String>, Vec<String>) = declared.iter().cloned().partition(|p| granted.contains(p));
            (Some(ok.into_iter().collect()), missing)
        }
    }
}
