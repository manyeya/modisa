// `modisa profile export` and `modisa profile import <file or URL> [--yes]`: a whole setup in one file — config.toml as
// it's written (comments and all), the themes you made, and where each plugin you installed came from — to share as a
// gist or carry to another machine. Importing shows what it will change and asks first; your config.toml is kept as
// config.toml.bak.
use std::io::IsTerminal;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::config::plugins::{installs, linked_plugins};
use crate::config::{CONFIG_DIR, CONFIG_PATH};

#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
struct Profile {
    modisa: String, // the version that wrote it
    config: String, // config.toml
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    themes: IndexMap<String, String>, // a theme's name → its file
    #[serde(default, rename = "plugin", skip_serializing_if = "Vec::is_empty")]
    plugins: Vec<PluginSource>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    local: Vec<String>, // plugins linked from a directory here: named, since they can't come along
}

#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
struct PluginSource {
    name: String,
    source: String,
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    r#ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    subdir: Option<String>,
}

fn themes_dir() -> String {
    format!("{}/themes", *CONFIG_DIR)
}

fn gather() -> Profile {
    let mut themes = IndexMap::new();
    let mut files: Vec<_> = std::fs::read_dir(themes_dir()).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "toml")).collect();
    files.sort();
    for f in files {
        if let (Some(name), Ok(text)) = (f.file_stem().map(|s| s.to_string_lossy().into_owned()), std::fs::read_to_string(&f)) {
            themes.insert(name, text);
        }
    }
    let installed = installs();
    let plugins = installed.iter().filter(|r| r.marketplace.is_none()).map(|r| PluginSource { name: r.name.clone(), source: r.source.clone(), r#ref: r.r#ref.clone(), subdir: r.subdir.clone() }).collect();
    let local = linked_plugins().into_iter().filter(|l| !installed.iter().any(|r| r.name == l.name)).map(|l| l.name).collect();
    Profile { modisa: crate::core::version::VERSION.into(), config: std::fs::read_to_string(&*CONFIG_PATH).unwrap_or_default(), themes, plugins, local }
}

pub fn export() -> i32 {
    match toml::to_string_pretty(&gather()) {
        Ok(text) => {
            println!("# a modisa setup: modisa profile import <this file, or its URL>\n{text}");
            0
        }
        Err(e) => {
            eprintln!("modisa: {e}");
            1
        }
    }
}

async fn read(from: &str) -> Result<String, String> {
    if from.starts_with("https://") || from.starts_with("http://") {
        // ponytail: curl fetches it (no HTTP client among the crates)
        let out = tokio::process::Command::new("curl").args(["-fsSL", "--max-time", "20", from]).output().await.map_err(|e| format!("couldn't run curl: {e}"))?;
        return if out.status.success() { Ok(String::from_utf8_lossy(&out.stdout).into_owned()) } else { Err(format!("couldn't fetch {from}: {}", String::from_utf8_lossy(&out.stderr).trim())) };
    }
    std::fs::read_to_string(crate::core::paths::abs_path(from)).map_err(|e| format!("{from}: {e}"))
}

pub async fn import(from: Option<&str>, yes: bool) -> i32 {
    let Some(from) = from else {
        eprintln!("usage: modisa profile import <file or URL> [--yes]");
        return 2;
    };
    let p: Profile = match read(from).await.and_then(|t| toml::from_str(&t).map_err(|e| format!("{from} isn't a modisa profile: {}", e.message()))) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("modisa: {e}");
            return 1;
        }
    };
    let errors: Vec<_> = crate::config::check::check_config(&p.config).into_iter().filter(|x| x.level == crate::config::check::Level::Error).collect();
    if let Some(e) = errors.first() {
        eprintln!("modisa: its config has a problem: {} (line {})", e.message, e.line.map_or("?".into(), |l| l.to_string()));
        return 1;
    }
    let have: Vec<String> = linked_plugins().into_iter().map(|l| l.name).collect();
    let new_plugins: Vec<&PluginSource> = p.plugins.iter().filter(|x| !have.contains(&x.name)).collect();
    println!("This profile (from modisa {}) will:", p.modisa);
    println!("  · replace your config.toml (yours is kept as config.toml.bak)");
    if !p.themes.is_empty() {
        println!("  · add themes: {} (one you have by that name is kept as <name>.toml.bak)", p.themes.keys().cloned().collect::<Vec<_>>().join(", "));
    }
    for x in &new_plugins {
        println!("  · install the plugin {} from {}{} — a program that runs as you, so only if you trust it", x.name, x.source, x.r#ref.as_ref().map(|r| format!(" at {r}")).unwrap_or_default());
    }
    if !p.local.is_empty() {
        println!("  (it also used {}, linked from directories on its machine: not brought along)", p.local.join(", "));
    }
    if !yes {
        if !std::io::stdin().is_terminal() {
            eprintln!("modisa: nothing changed: pass --yes to import without asking");
            return 1;
        }
        eprint!("Go ahead? [y/N] ");
        let mut answer = String::new();
        if std::io::stdin().read_line(&mut answer).is_err() || !answer.trim().eq_ignore_ascii_case("y") {
            println!("nothing changed");
            return 1;
        }
    }
    let written = (|| -> std::io::Result<()> {
        std::fs::create_dir_all(&*CONFIG_DIR)?;
        if std::path::Path::new(&*CONFIG_PATH).exists() {
            std::fs::copy(&*CONFIG_PATH, format!("{}.bak", *CONFIG_PATH))?;
        }
        std::fs::write(&*CONFIG_PATH, &p.config)?;
        std::fs::create_dir_all(themes_dir())?;
        for (name, text) in &p.themes {
            let file = format!("{}/{name}.toml", themes_dir());
            if std::fs::read_to_string(&file).is_ok_and(|old| old != *text) {
                std::fs::copy(&file, format!("{file}.bak"))?;
            }
            std::fs::write(&file, text)?;
        }
        Ok(())
    })();
    if let Err(e) = written {
        eprintln!("modisa: {e}");
        return 1;
    }
    println!("config.toml and {} theme(s) written", p.themes.len());
    let mut code = 0;
    for x in new_plugins {
        println!("installing {}…", x.name);
        if super::plugin_install::install(Some(&x.source), x.r#ref.as_deref(), x.subdir.as_deref(), None, false).await != 0 {
            code = 1;
        }
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_profile_round_trips_through_toml() {
        let p = Profile {
            modisa: "0.3.0".into(),
            config: "# mine\ntheme = \"nord\"\n\n[keys]\nzoom = \"f\"\n".into(),
            themes: [("night".to_string(), "inherits = \"nord\"\naccent = \"#ff00ff\"\n".to_string())].into_iter().collect(),
            plugins: vec![PluginSource { name: "radar".into(), source: "https://github.com/x/radar".into(), r#ref: Some("v1".into()), subdir: None }],
            local: vec!["review".into()],
        };
        let text = toml::to_string_pretty(&p).unwrap();
        assert!(text.contains("[[plugin]]") && text.contains("# mine"), "{text}");
        assert_eq!(toml::from_str::<Profile>(&text).unwrap(), p);
    }
}
