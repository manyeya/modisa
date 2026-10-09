// The plugin manager (prefix P, "Plugins…" in the palette, or the settings page): discover plugins in the index and
// your marketplaces, install one once you've seen exactly where it comes from, and start, stop, restart, read the log
// of, update and remove what's installed. It asks the server for all of it (plugin.*, marketplace.*), so with --remote
// it acts on the server's machine, where plugins run. Each view is a list; an action runs behind a spinner (esc hides
// it, and the result is still toasted), then its view opens again with what changed.
use std::future::Future;

use serde_json::{json, Value};

use super::design::fit;
use super::modals::{self, ask_width, confirm, list, prompt, Choice, ListButton, ListItem, ListOptions};
use super::Shared;

// The view to show next.
enum Step {
    Menu,
    Discover(Option<String>), // the index and every marketplace, or one marketplace's plugins
    Installed,
    Marketplaces,
    Url,
    Act(String, Value), // a verb on an installed plugin: "<verb>:<name>", and its status
}

pub async fn open(shared: &Shared, start: &str) {
    let mut step = Some(match start {
        "discover" => Step::Discover(None),
        "installed" => Step::Installed,
        "marketplaces" => Step::Marketplaces,
        "url" => Step::Url,
        _ => Step::Menu,
    });
    while let Some(s) = step {
        step = match s {
            Step::Menu => menu(shared).await,
            Step::Discover(only) => discover(shared, only).await,
            Step::Installed => installed(shared).await,
            Step::Marketplaces => marketplaces(shared).await,
            Step::Url => from_url(shared).await,
            Step::Act(v, p) => act(shared, &v, &p).await,
        };
    }
}

// ---------- waiting, and saying what happened ----------

fn request(shared: &Shared, method: &str, params: Value) -> impl Future<Output = Result<Value, String>> + 'static {
    let conn = shared.borrow().conn.clone();
    let method = method.to_string();
    async move {
        let conn = conn.ok_or("not connected")?;
        conn.request(&method, params, None).await.map_err(|e| e.message)
    }
}

// `work` behind a spinner, until it settles: its outcome, or None if the user hid it (esc). Hidden work carries on,
// and `report` still toasts how it went.
async fn busy(shared: &Shared, title: &str, what: &str, work: impl Future<Output = Result<Value, String>> + 'static, report: impl FnOnce(&Shared, Value) + 'static) -> Option<Result<Value, String>> {
    let (id, hidden) = modals::open_busy(shared, title, what);
    let mut work = Box::pin(work);
    tokio::select! {
        r = &mut work => {
            modals::close_busy(&mut shared.borrow_mut(), id);
            Some(r)
        }
        _ = hidden => {
            let s = shared.clone();
            tokio::task::spawn_local(async move {
                match work.await {
                    Ok(v) => report(&s, v),
                    Err(e) => failed(&s, &e),
                }
            });
            None
        }
    }
}

fn toast(shared: &Shared, text: &str, color: fn(&crate::config::themes::Theme) -> &'static str) {
    let mut app = shared.borrow_mut();
    let c = color(&app.th);
    app.toast(text, c);
}

fn failed(shared: &Shared, e: &str) {
    toast(shared, e, |th| th.blocked);
}

fn short(commit: &Value) -> String {
    commit.as_str().map(|c| c.chars().take(7).collect()).unwrap_or_default()
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}

fn place(shared: &Shared) -> &'static str {
    if shared.borrow().opts.remote { "on the server's machine" } else { "on this machine" }
}

fn item(name: impl Into<String>, description: impl Into<String>, value: impl Into<String>) -> ListItem {
    ListItem::new(name, description, value)
}

// ---------- the views ----------

async fn menu(shared: &Shared) -> Option<Step> {
    let meta = if shared.borrow().opts.remote { "on the server's machine" } else { "" };
    let items = vec![
        item("Discover", "plugins in the index and in your marketplaces", "discover"),
        item("Installed", "start, stop, restart, logs, update, remove", "installed"),
        item("Marketplaces", "repositories that list plugins: add, update, remove", "marketplaces"),
        item("Add from URL…", "install a plugin from a git URL", "url"),
    ];
    let v = list(shared, ListOptions { title: "Plugins".into(), meta: Some(meta.into()), width: Some(80), items, ..Default::default() }).await;
    match v.as_deref() {
        Some("discover") => Some(Step::Discover(None)),
        Some("installed") => Some(Step::Installed),
        Some("marketplaces") => Some(Step::Marketplaces),
        Some("url") => Some(Step::Url),
        _ => None,
    }
}

// The index's plugins and the marketplaces' (or one marketplace's), searchable; choosing one installs it.
async fn discover(shared: &Shared, only: Option<String>) -> Option<Step> {
    let back = || if only.is_some() { Step::Marketplaces } else { Step::Menu };
    let what = match &only {
        Some(m) => format!("Reading marketplace {m}…"),
        None => "Reading the plugin index and your marketplaces…".into(),
    };
    let cat = match busy(shared, "Plugins", &what, request(shared, "plugin.catalog", json!({})), |_, _| {}).await? {
        Ok(c) => c,
        Err(e) => {
            failed(shared, &e);
            return Some(back());
        }
    };
    let listed: Vec<&Value> = cat["marketplaces"].as_array().into_iter().flatten().filter(|p| only.as_deref().is_none_or(|m| p["marketplace"] == m)).collect();
    let index: Vec<&Value> = if only.is_some() { vec![] } else { cat["index"]["results"].as_array().into_iter().flatten().collect() };
    let mut items: Vec<ListItem> = listed
        .iter()
        .map(|p| {
            let desc = p["description"].as_str().filter(|d| !d.is_empty()).map(|d| format!(" · {d}")).unwrap_or_default();
            item(s(p, "name"), format!("@{}{desc}", s(p, "marketplace")), format!("m:{}@{}", s(p, "name"), s(p, "marketplace"))).key(if p["installed"] == true { "✓ installed" } else { "" })
        })
        .collect();
    items.extend(index.iter().enumerate().map(|(i, r)| {
        let desc = r["description"].as_str().filter(|d| !d.is_empty()).map(|d| format!(" · {d}")).unwrap_or_default();
        let key = if r["archived"] == true { "archived" } else if r["installed"] == true { "✓ installed" } else { "" };
        item(s(r, "name"), format!("★{} {}{desc}", r["stars"].as_u64().unwrap_or(0), s(r, "repo")), format!("i:{i}")).key(key)
    }));
    if let Some(e) = cat["index"]["error"].as_str().filter(|_| only.is_none()) {
        items.push(item("(the plugin index)", e, ""));
    }
    if items.is_empty() {
        items.push(match &only {
            Some(m) => item(format!("(marketplace {m} lists no plugins)"), "", ""),
            None => item("(nothing found)", "add a marketplace, or search the index again later", ""),
        });
    }
    let title = only.as_ref().map(|m| format!("Marketplace {m}")).unwrap_or("Discover plugins".into());
    let meta = format!("{} · none of them vetted", listed.len() + index.len());
    let Some(v) = list(shared, ListOptions { title, meta: Some(meta), items, width: Some(100), rows: Some(16), ..Default::default() }).await else { return Some(back()) };
    if v.is_empty() {
        return Some(Step::Discover(only));
    }
    let (from, name) = match v.strip_prefix("m:") {
        Some(m) => (json!({ "marketplacePlugin": m }), m.to_string()),
        None => {
            let r = index.get(v[2..].parse::<usize>().unwrap_or(usize::MAX))?;
            (json!({ "source": r["source"] }), s(r, "repo").to_string())
        }
    };
    Some(if install(shared, from, &name).await { Step::Installed } else { Step::Discover(only) })
}

// Looked up first (the exact source and the commit its ref is at), shown with a warning, and installed only if the user
// says so, at that commit: if the ref has moved meanwhile, the install refuses.
async fn install(shared: &Shared, from: Value, name: &str) -> bool {
    let what = from["marketplacePlugin"].as_str().or(from["source"].as_str()).unwrap_or("").to_string();
    let r = match busy(shared, &format!("Install {name}"), &format!("Looking up {what}…"), request(shared, "plugin.resolve", from.clone()), |_, _| {}).await {
        None => return false,
        Some(Err(e)) => {
            failed(shared, &e);
            return false;
        }
        Some(Ok(r)) => r,
    };
    let width = (shared.borrow().width - 4).clamp(60, 100);
    let room = (width - 4 - 9).max(1) as usize; // the border and padding, and the labels
    let wrap = |label: &str, value: &str| -> Vec<String> {
        let chars: Vec<char> = value.chars().collect();
        let parts: Vec<String> = if chars.is_empty() { vec![String::new()] } else { chars.chunks(room).map(|c| c.iter().collect()).collect() };
        parts.into_iter().enumerate().map(|(i, p)| format!("{:<9}{p}", if i == 0 { label } else { "" })).collect()
    };
    let rname = r["name"].as_str().unwrap_or(name).to_string();
    let mut body = vec![format!("{rname} will run unsandboxed, with your permissions, {}:", place(shared)), "your files, your network, your credentials. Nothing has vetted it.".into(), String::new()];
    body.extend(wrap("from", s(&r, "from")));
    if let Some(sub) = r["subdir"].as_str().filter(|x| !x.is_empty()) {
        body.extend(wrap("folder", sub));
    }
    body.extend(wrap("ref", r["ref"].as_str().unwrap_or("the default branch")));
    body.extend(wrap("commit", r["commit"].as_str().unwrap_or("(known once fetched: an abbreviated commit)")));
    if let Some(m) = r["marketplace"].as_str() {
        body.extend(wrap("listed", &format!("by marketplace {m}")));
    }
    let choices = vec![Choice { label: "Cancel".into(), key: "n".into(), value: "no".into(), tone: None }, Choice { label: "Install".into(), key: "y".into(), value: "yes".into(), tone: Some("primary") }];
    if ask_width(shared, &format!("Install {rname}?"), &body.join("\n"), choices, 0, width).await.as_deref() != Some("yes") {
        return false;
    }
    let mut params = from;
    if let Some(c) = r["commit"].as_str() {
        params["commit"] = json!(c);
    }
    let at = r["commit"].as_str().map(|_| format!(" at {}", short(&r["commit"]))).unwrap_or_default();
    match busy(shared, &format!("Installing {rname}"), &format!("Fetching {}{at}…", s(&r, "from")), request(shared, "plugin.install", params), installed_toast).await {
        None => false,
        Some(Err(e)) => {
            failed(shared, &e);
            false
        }
        Some(Ok(v)) => {
            installed_toast(shared, v.clone());
            v["installed"] == true || v["alreadyInstalled"] == true
        }
    }
}

fn installed_toast(shared: &Shared, r: Value) {
    if r["installed"] != true && r["alreadyInstalled"] != true {
        return toast(shared, &format!("not installed ({}): {}", s(&r, "stage"), s(&r, "reason")), |th| th.blocked);
    }
    let st = &r["start"];
    let state = match st["state"].as_str() {
        None => String::new(),
        Some("started") => format!(", running (pid {})", st["pid"]),
        Some("already-running") => ", already running".into(),
        Some("no-hello") => ", but it never connected".into(),
        Some(_) => format!(", but it didn't start: {}", s(st, "reason")),
    };
    let fine = st.is_null() || matches!(st["state"].as_str(), Some("started" | "already-running"));
    let text = format!("{} {} @{}{state}", if r["alreadyInstalled"] == true { "already installed" } else { "installed" }, s(&r, "name"), short(&r["commit"]));
    if fine {
        toast(shared, &text, |th| th.done);
    } else {
        toast(shared, &text, |th| th.warn);
    }
    for hint in r["hints"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        toast(shared, hint, |th| th.warn);
    }
}

fn status(p: &Value) -> String {
    match s(p, "status") {
        "running" => format!("running · pid {}", p["pid"]),
        "failed" => format!("failed{}", p["error"].as_str().map(|e| format!(": {e}")).unwrap_or_default()),
        "exited" => format!("exited {}", p["exitCode"].as_i64().map(|c| c.to_string()).unwrap_or_default()).trim().to_string(),
        other => other.to_string(),
    }
}

fn origin(p: &Value) -> String {
    let i = &p["install"];
    if i.is_object() {
        let src = s(i, "source");
        let src = src.split_once("://").filter(|(scheme, _)| !scheme.is_empty() && scheme.chars().all(|c| c.is_ascii_lowercase())).map(|(_, rest)| rest).unwrap_or(src);
        let src = src.strip_suffix(".git").unwrap_or(src);
        let market = i["marketplace"].as_str().map(|m| format!("@{m} · ")).unwrap_or_default();
        let r = i["ref"].as_str().map(|r| format!(" {r}")).unwrap_or_default();
        return format!("{market}{src}{r} @{}", short(&i["commit"]));
    }
    if p["source"] == "config" { "from config.toml".into() } else { format!("linked {}", s(p, "dir")) }
}

fn live(p: &Value) -> bool {
    matches!(s(p, "status"), "running" | "starting")
}

fn button(icon: &str, value: String, key: &str, title: &str, danger: bool) -> ListButton {
    ListButton { icon: icon.into(), value, key: key.into(), title: title.into(), danger }
}

// What can be done to one: a [[plugin]] line from config.toml only starts and stops
fn actions_of(p: &Value) -> Vec<ListButton> {
    let name = s(p, "name");
    let toggle = if live(p) { button("■", format!("stop:{name}"), "t", "stop", false) } else { button("▶", format!("start:{name}"), "t", "start", false) };
    if p["source"] == "config" {
        return vec![toggle];
    }
    let mut out = vec![toggle, button("↻", format!("restart:{name}"), "r", "restart", false), button("≡", format!("logs:{name}"), "l", "logs", false)];
    if p["install"].is_object() {
        out.push(button("⇣", format!("update:{name}"), "g", "update", false));
    }
    out.push(button("✕", format!("remove:{name}"), "d", "remove", true));
    out
}

async fn installed(shared: &Shared) -> Option<Step> {
    let ps = match request(shared, "plugin.list", json!({})).await {
        Ok(v) => v.as_array().cloned().unwrap_or_default(),
        Err(e) => {
            failed(shared, &e);
            return Some(Step::Menu);
        }
    };
    let mut items: Vec<ListItem> = ps
        .iter()
        .map(|p| {
            let key = if live(p) { "● on" } else if p["status"] == "failed" { "✕ failed" } else { "○ off" };
            ListItem { buttons: actions_of(p), ..item(s(p, "name"), format!("{} · {}", status(p), origin(p)), format!("open:{}", s(p, "name"))).key(key) }
        })
        .collect();
    items.push(item("+ Discover plugins", "in the index and your marketplaces", "discover"));
    items.push(item("+ Add from URL…", "install a plugin from a git URL", "url"));
    let meta = format!("{} {}", ps.len(), place(shared));
    let v = list(shared, ListOptions { title: "Installed plugins".into(), meta: Some(meta), items, width: Some(100), rows: Some(14), ..Default::default() }).await;
    let Some(v) = v else { return Some(Step::Menu) };
    match v.as_str() {
        "discover" => return Some(Step::Discover(None)),
        "url" => return Some(Step::Url),
        _ => {}
    }
    let (verb, name) = v.split_once(':').unwrap_or((&v, ""));
    let p = ps.iter().find(|x| x["name"] == name)?.clone();
    if verb == "open" {
        // every action, by name: what the row's buttons do
        let items = actions_of(&p).into_iter().map(|b| ListItem { danger: b.danger, ..item(format!("{}{}", b.title[..1].to_uppercase(), &b.title[1..]), format!("^{} in the list", b.key), b.value) }).collect();
        let chosen = list(shared, ListOptions { title: format!("{name}: {}", status(&p)), items, ..Default::default() }).await;
        return Some(match chosen {
            Some(c) => Step::Act(c, p),
            None => Step::Installed,
        });
    }
    Some(Step::Act(v, p))
}

async fn act(shared: &Shared, value: &str, p: &Value) -> Option<Step> {
    let verb = value.split(':').next().unwrap_or("");
    let name = s(p, "name").to_string();
    let said = {
        let name = name.clone();
        move |sh: &Shared, st: Value| {
            let text = format!("{name}: {}", status(&st));
            match s(&st, "status") {
                "running" => toast(sh, &text, |th| th.done),
                "failed" => toast(sh, &text, |th| th.blocked),
                _ => toast(sh, &text, |th| th.fg),
            }
        }
    };
    let settle = |out: Option<Result<Value, String>>, report: &dyn Fn(Value)| match out {
        Some(Ok(v)) => report(v),
        Some(Err(e)) => failed(shared, &e),
        None => {}
    };
    match verb {
        "start" | "stop" => {
            let what = if verb == "start" { format!("Starting {name}…") } else { format!("Stopping {name}…") };
            let out = busy(shared, &name, &what, request(shared, &format!("plugin.{verb}"), json!({ "name": name })), said.clone()).await;
            settle(out, &|v| said(shared, v));
        }
        "restart" => {
            let (stop, start) = (request(shared, "plugin.stop", json!({ "name": name })), request(shared, "plugin.start", json!({ "name": name })));
            let work = async move {
                stop.await?;
                start.await
            };
            let out = busy(shared, &name, &format!("Restarting {name}…"), work, said.clone()).await;
            settle(out, &|v| said(shared, v));
        }
        "logs" => {
            let log = match request(shared, "plugin.logs", json!({ "name": name, "lines": 500 })).await {
                Ok(l) => l,
                Err(e) => {
                    failed(shared, &e);
                    return Some(Step::Installed);
                }
            };
            let text = s(&log, "text");
            let lines: Vec<&str> = if text.is_empty() { vec!["(empty)"] } else { text.split('\n').collect() };
            let path: Vec<&str> = s(&log, "log").split('/').collect();
            let meta = path[path.len().saturating_sub(2)..].join("/");
            let n = lines.len();
            let items = lines.into_iter().map(|l| item(if l.is_empty() { " " } else { l }, "", "")).collect();
            list(shared, ListOptions { title: format!("{name}: its log"), meta: Some(meta), items, width: Some(120), rows: Some(24), selected: Some(n - 1), placeholder: Some("Type to search the log".into()), ..Default::default() }).await;
        }
        "update" => {
            let report = {
                let name = name.clone();
                move |sh: &Shared, r: Value| {
                    if let Some(stage) = r["stage"].as_str() {
                        return toast(sh, &format!("{name} not updated ({stage}): {}", s(&r, "reason")), |th| th.blocked);
                    }
                    if r["upToDate"] == true {
                        return toast(sh, &format!("{name} is up to date (@{})", short(&r["commit"])), |th| th.done);
                    }
                    let restarted: Vec<&Value> = r["restarted"].as_array().into_iter().flatten().collect();
                    let how = if restarted.is_empty() { String::new() } else { format!("; restarted ({})", restarted.iter().map(|x| s(x, "state")).collect::<Vec<_>>().join(", ")) };
                    let text = format!("updated {name} {} → {}{how}", short(&r["from"]), short(&r["commit"]));
                    if restarted.iter().all(|x| x["state"] == "started") {
                        toast(sh, &text, |th| th.done);
                    } else {
                        toast(sh, &text, |th| th.warn);
                    }
                }
            };
            let out = busy(shared, &name, &format!("Fetching the latest of {name}…"), request(shared, "plugin.update", json!({ "name": name })), report.clone()).await;
            settle(out, &|v| report(shared, v));
        }
        "remove" => {
            let text = if p["install"].is_object() {
                format!("Stop {name} in every session and delete its checkout?\nIts data and logs are kept.")
            } else {
                format!("Unlink {name} and stop it here?\nIts directory ({}) is untouched.", fit(s(p, "dir"), 40))
            };
            if !confirm(shared, &format!("Remove {name}"), &text, "remove").await {
                return Some(Step::Installed);
            }
            let report = {
                let name = name.clone();
                move |sh: &Shared, r: Value| {
                    let kept = r["checkout"].is_object() && r["checkout"]["deleted"] != true;
                    let users: Vec<&str> = r["stillUsing"].as_array().into_iter().flatten().chain(r["unreachable"].as_array().into_iter().flatten()).filter_map(Value::as_str).collect();
                    let note = if kept { format!("; its checkout is kept: {} still use it", users.join(", ")) } else { String::new() };
                    toast(sh, &format!("removed {name}{note}"), |th| th.done);
                }
            };
            let out = busy(shared, &name, &format!("Removing {name}…"), request(shared, "plugin.unlink", json!({ "name": name })), report.clone()).await;
            settle(out, &|v| report(shared, v));
        }
        _ => {}
    }
    Some(Step::Installed)
}

async fn marketplaces(shared: &Shared) -> Option<Step> {
    let ms = match request(shared, "marketplace.list", json!({})).await {
        Ok(v) => v.as_array().cloned().unwrap_or_default(),
        Err(e) => {
            failed(shared, &e);
            return Some(Step::Menu);
        }
    };
    let mut items: Vec<ListItem> = ms
        .iter()
        .map(|m| {
            let n = m["plugins"].as_u64().unwrap_or(0);
            let state = m["error"].as_str().map(String::from).unwrap_or_else(|| format!("{n} plugin{}", if n == 1 { "" } else { "s" }));
            let src = s(m, "source");
            let src = src.split_once("://").map(|(_, r)| r).unwrap_or(src);
            let desc = m["description"].as_str().filter(|d| !d.is_empty()).map(|d| format!(" · {d}")).unwrap_or_default();
            let name = s(m, "name");
            ListItem {
                buttons: vec![button("↻", format!("update:{name}"), "r", "update", false), button("✕", format!("remove:{name}"), "d", "remove", true)],
                ..item(name, format!("{state} · {src} @{}{desc}", short(&m["commit"])), format!("open:{name}"))
            }
        })
        .collect();
    items.push(item("+ Add marketplace…", "owner/repo on GitHub, or a git URL", "add"));
    let meta = format!("{} {}", ms.len(), place(shared));
    let Some(v) = list(shared, ListOptions { title: "Marketplaces".into(), meta: Some(meta), items, width: Some(100), ..Default::default() }).await else { return Some(Step::Menu) };
    if v == "add" {
        let source = prompt(shared, "Add a marketplace", "", "owner/repo, or a git URL").await.map(|x| x.trim().to_string()).unwrap_or_default();
        if source.is_empty() {
            return Some(Step::Marketplaces);
        }
        let report = |sh: &Shared, r: Value| {
            if let Some(stage) = r["stage"].as_str() {
                return toast(sh, &format!("marketplace not added ({stage}): {}", s(&r, "reason")), |th| th.blocked);
            }
            let n = r["plugins"].as_array().map(Vec::len).unwrap_or(0);
            toast(sh, &format!("{} marketplace {}: {n} plugins", if r["alreadyAdded"] == true { "already added" } else { "added" }, s(&r, "name")), |th| th.done);
        };
        match busy(shared, "Marketplaces", &format!("Fetching {source}…"), request(shared, "marketplace.add", json!({ "source": source })), report).await {
            Some(Ok(r)) => report(shared, r),
            Some(Err(e)) => failed(shared, &e),
            None => {}
        }
        return Some(Step::Marketplaces);
    }
    let (verb, name) = v.split_once(':').unwrap_or((&v, ""));
    match verb {
        "open" => return Some(Step::Discover(Some(name.to_string()))),
        "update" => {
            let report = |sh: &Shared, rs: Value| {
                for r in rs.as_array().into_iter().flatten() {
                    let n = s(r, "name");
                    match r["reason"].as_str() {
                        Some(why) => toast(sh, &format!("marketplace {n} not updated: {why}"), |th| th.blocked),
                        None if r["updated"] == true => toast(sh, &format!("updated marketplace {n}: {} → {}", short(&r["from"]), short(&r["commit"])), |th| th.done),
                        None => toast(sh, &format!("marketplace {n} is up to date"), |th| th.done),
                    }
                }
            };
            match busy(shared, "Marketplaces", &format!("Fetching the latest of {name}…"), request(shared, "marketplace.update", json!({ "name": name })), report).await {
                Some(Ok(r)) => report(shared, r),
                Some(Err(e)) => failed(shared, &e),
                None => {}
            }
        }
        "remove" if confirm(shared, &format!("Remove {name}"), &format!("Remove marketplace {name}?\nPlugins installed from it stay installed."), "remove").await => match request(shared, "marketplace.remove", json!({ "name": name })).await {
            Ok(r) => {
                let still: Vec<&str> = r["installed"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                let note = if still.is_empty() { String::new() } else { format!("; still installed from it: {}", still.join(", ")) };
                toast(shared, &format!("removed marketplace {name}{note}"), |th| th.done);
            }
            Err(e) => failed(shared, &e),
        },
        _ => {}
    }
    Some(Step::Marketplaces)
}

async fn from_url(shared: &Shared) -> Option<Step> {
    let source = prompt(shared, "Install from a git URL", "", "https://…, ssh://…, git@host:path or file://…").await.map(|x| x.trim().to_string()).unwrap_or_default();
    if source.is_empty() {
        return Some(Step::Menu);
    }
    let Some(r) = prompt(shared, "Branch, tag or commit", "", "optional: empty for the default branch").await else { return Some(Step::Menu) };
    let trimmed = source.trim_end_matches('/');
    let trimmed = trimmed.strip_suffix(".git").unwrap_or(trimmed);
    let name = trimmed.rsplit(['/', ':']).next().filter(|n| !n.is_empty()).unwrap_or(&source).to_string();
    let mut from = json!({ "source": source });
    if !r.trim().is_empty() {
        from["ref"] = json!(r.trim());
    }
    Some(if install(shared, from, &name).await { Step::Installed } else { Step::Menu })
}
