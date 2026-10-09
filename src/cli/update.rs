// Keeping modisa current: the release manifest for this install's channel, a cached "is there a newer one?" for
// `modisa version` and the TUI's update badge, and `modisa update`, which replaces the binary with the new release
// after checking its SHA-256.
use std::cell::{Cell, RefCell};
use std::io::{IsTerminal, Write as _};
use std::rc::Rc;
use std::time::Duration;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::commands::js_str;
use crate::config::{load_config, Channel};
use crate::core::paths::{self_exe, DIR};
use crate::core::version::{channel, newer, platform, FROM_SOURCE, REPO, VERSION};

// How this modisa got here, judged by where its binary lives. A package manager owns the binary it installed, so
// updating or removing that binary is the manager's job: modisa only says which command. (src/core/install.ts)
#[derive(Clone, Debug, PartialEq)]
pub struct Install {
    pub by: &'static str, // "source" | "script" | "homebrew" | "mise" | "system"
    pub manager: Option<&'static str>,
    pub upgrade: Option<&'static str>,
    pub remove: Option<&'static str>,
}

pub fn installed_by(exe: &str, from_source: bool) -> Install {
    let install = |by, manager, upgrade, remove| Install { by, manager: Some(manager), upgrade: Some(upgrade), remove: Some(remove) };
    if from_source {
        return Install { by: "source", manager: None, upgrade: None, remove: None };
    }
    if ["/Cellar/", "/homebrew/", "/linuxbrew/"].iter().any(|d| exe.contains(d)) {
        return install("homebrew", "Homebrew", "brew upgrade modisa", "brew uninstall modisa");
    }
    if exe.contains("/mise/installs/") {
        return install("mise", "mise", "mise upgrade github:manyeya/modisa", "mise uninstall github:manyeya/modisa");
    }
    // .deb and .rpm packages put it in /usr/bin
    if exe.starts_with("/usr/bin/") || exe.starts_with("/usr/sbin/") {
        return install("system", "your system's package manager", "sudo apt upgrade modisa (or dnf upgrade modisa)", "sudo apt remove modisa (or dnf remove modisa)");
    }
    Install { by: "script", manager: None, upgrade: None, remove: None } // install.sh, or a binary someone put on their PATH by hand
}

// What to run to update this install: its package manager's command, or `modisa update`.
pub fn update_command() -> String {
    installed_by(&self_exe(), FROM_SOURCE).upgrade.unwrap_or("modisa update").into()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Asset {
    pub url: String,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: String,
    pub channel: String,
    pub notes: String,
    pub assets: IndexMap<String, Asset>,
}

const EVERY: f64 = 6.0 * 60.0 * 60.0 * 1000.0;

fn cache() -> String {
    format!("{}/update.json", *DIR)
}

// MODISA_UPDATE_URL points elsewhere (a mirror, a test server) or turns checks "off".
pub fn manifest_url(channel: &str) -> Option<String> {
    match std::env::var("MODISA_UPDATE_URL") {
        Ok(o) if o == "off" => None,
        Ok(o) if !o.is_empty() => Some(o),
        _ if channel == "staging" => Some(format!("https://github.com/{REPO}/releases/download/staging/manifest.json")),
        _ => Some(format!("https://github.com/{REPO}/releases/latest/download/manifest.json")),
    }
}

pub fn parse_manifest(raw: &Value) -> Option<Manifest> {
    let version = raw.get("version")?.as_str()?;
    let entries: Vec<(String, &Value)> = match raw.get("assets")? {
        Value::Object(m) => m.iter().map(|(k, v)| (k.clone(), v)).collect(),
        Value::Array(xs) => xs.iter().enumerate().map(|(i, v)| (i.to_string(), v)).collect(),
        _ => return None,
    };
    let sha = |s: &str| s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    let assets = entries.into_iter().filter_map(|(plat, a)| {
        let (url, sha256) = (a.get("url")?.as_str()?, a.get("sha256")?.as_str()?);
        sha(sha256).then(|| (plat, Asset { url: url.into(), sha256: sha256.into() }))
    });
    let text = |k: &str, or: &str| raw.get(k).filter(|v| !v.is_null()).map_or(or.into(), js_str);
    Some(Manifest { version: version.into(), channel: text("channel", "stable"), notes: text("notes", ""), assets: assets.collect() })
}

// ponytail: curl fetches it (no HTTP client among the crates); none on PATH is no newer release.
async fn fetch_manifest(url: &str) -> Option<Manifest> {
    let out = tokio::process::Command::new("curl").args(["-fsSL", "--max-time", "4", url]).output().await.ok()?;
    if !out.status.success() {
        return None;
    }
    parse_manifest(&serde_json::from_slice(&out.stdout).ok()?)
}

fn now_ms() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_millis() as f64)
}

// The release to move to, if there's a newer one for this install. Cached for 6h; never fails.
pub async fn check_for_update(force: bool) -> Option<Manifest> {
    let cfg = load_config();
    if !force && !cfg.update.check {
        return None;
    }
    if FROM_SOURCE && std::env::var("MODISA_UPDATE_URL").map_or(true, |u| u.is_empty()) {
        return None; // a checkout updates with git pull
    }
    let url = manifest_url(if channel() == "staging" || cfg.update.channel == Channel::Staging { "staging" } else { "stable" })?;
    let cached: Option<Value> = std::fs::read(cache()).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let fresh = |c: &Value| c["url"] == url.as_str() && c["at"].as_f64().is_some_and(|at| now_ms() - at < EVERY);
    let mut m = cached.as_ref().filter(|c| !force && fresh(c)).and_then(|c| parse_manifest(&c["manifest"]));
    if m.is_none() {
        m = fetch_manifest(&url).await;
        if let Some(m) = &m {
            let _ = std::fs::create_dir_all(&*DIR).and_then(|_| std::fs::write(cache(), json!({ "url": url, "at": now_ms() as u64, "manifest": m }).to_string()));
        }
    }
    m.filter(|m| newer(&m.version, VERSION))
}

pub async fn run_version() -> i32 {
    let staging = if channel() == "staging" { " (staging)" } else { "" };
    let source = if FROM_SOURCE { " (from source)" } else { "" };
    outln!("modisa {VERSION}{staging}{source}");
    if let Some(m) = check_for_update(false).await {
        outln!("update available: {} — run `{}`", m.version, update_command());
    }
    0
}

// ---------- terminal flair (the original's cli/flair.ts) ----------
// The wordmark in the brand gradient, spinners and a download bar. Only on a terminal that takes colour: plain lines
// when the output is piped, NO_COLOR is set or TERM is dumb; MODISA_FANCY=1 forces it. install.sh draws the same
// wordmark.
pub fn fancy() -> bool {
    let set = |k: &str| std::env::var(k).is_ok_and(|v| !v.is_empty());
    std::env::var("MODISA_FANCY").as_deref() == Ok("1") || (std::io::stdout().is_terminal() && !set("NO_COLOR") && std::env::var("TERM").is_ok_and(|t| t != "dumb"))
}

const ESC: &str = "\x1b[";
const RESET: &str = "\x1b[0m";
const FROM: [f64; 3] = [94.0, 231.0, 239.0]; // the ion theme's focus cyan
const TO: [f64; 3] = [179.0, 154.0, 255.0]; // and its accent violet

fn rgb(r: f64, g: f64, b: f64) -> String {
    if std::env::var("COLORTERM").is_ok_and(|c| { let c = c.to_lowercase(); c.contains("truecolor") || c.contains("24bit") }) {
        return format!("{ESC}38;2;{r};{g};{b}m");
    }
    let q = |v: f64| (v / 255.0 * 5.0).round();
    format!("{ESC}38;5;{}m", 16.0 + 36.0 * q(r) + 6.0 * q(g) + q(b))
}

// the brand gradient's colour at t, from 0 (cyan) to 1 (violet)
fn paint(t: f64) -> String {
    let k = t.clamp(0.0, 1.0);
    let c: Vec<f64> = (0..3).map(|i| (FROM[i] + (TO[i] - FROM[i]) * k).round()).collect();
    rgb(c[0], c[1], c[2])
}
fn green() -> String {
    rgb(165.0, 239.0, 181.0)
}
fn red() -> String {
    rgb(255.0, 127.0, 150.0)
}
fn dim(s: &str) -> String {
    format!("{ESC}2m{s}{RESET}")
}

// text in the gradient, left to right
fn gradient(text: &str) -> String {
    use unicode_segmentation::UnicodeSegmentation;
    let chars: Vec<&str> = text.graphemes(true).collect();
    let last = chars.len().saturating_sub(1).max(1) as f64;
    chars.iter().enumerate().map(|(i, c)| if *c == " " { c.to_string() } else { format!("{}{c}", paint(i as f64 / last)) }).collect::<String>() + RESET
}

const WORDMARK: [&str; 3] = [
    "█▄ ▄█ ▄▀▀▀▄ █▀▀▀▄ ▀▀█▀▀ ▄▀▀▀▀ ▄▀▀▀▄",
    "█ ▀ █ █   █ █   █   █    ▀▀▀▄ █▀▀▀█",
    "█   █ ▀▄▄▄▀ █▄▄▄▀ ▄▄█▄▄ ▄▄▄▄▀ █   █",
];

fn write(s: &str) {
    let mut out = std::io::stdout();
    let _ = out.write_all(s.as_bytes());
    let _ = out.flush();
}

thread_local! {
    static HIDDEN: Cell<bool> = const { Cell::new(false) };
}
pub fn show_cursor() {
    if HIDDEN.replace(false) {
        write(&format!("{ESC}?25h"));
    }
}
// run_update shows it again however the command ends
fn hide_cursor() {
    if !HIDDEN.replace(true) {
        write(&format!("{ESC}?25l"));
    }
}

// the wordmark, one row at a time, with a line under it
async fn banner(subtitle: &str) {
    hide_cursor();
    write("\n");
    for row in WORDMARK {
        write(&format!("  {}\n", gradient(row)));
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    write(&format!("\n  {}\n\n", dim(subtitle)));
}

const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

// a spinner in front of `label` until done() or fail(); update() sets what follows the label (a bar, a byte count)
struct Spinner {
    suffix: Rc<RefCell<String>>,
    task: tokio::task::JoinHandle<()>,
}

fn draw(frame: usize, label: &str, suffix: &str) {
    let frame = frame % FRAMES.len();
    write(&format!("\r{ESC}K  {}{}{RESET} {label}{suffix}", paint(frame as f64 / (FRAMES.len() - 1) as f64), FRAMES[frame]));
}

fn spinner(label: &str) -> Spinner {
    hide_cursor();
    let suffix = Rc::new(RefCell::new(String::new()));
    draw(0, label, "");
    let (l, s) = (label.to_string(), suffix.clone());
    let task = tokio::task::spawn_local(async move {
        for i in 1.. {
            tokio::time::sleep(Duration::from_millis(80)).await;
            draw(i, &l, &s.borrow());
        }
    });
    Spinner { suffix, task }
}

impl Spinner {
    fn update(&self, s: String) {
        *self.suffix.borrow_mut() = s;
    }
    fn end(&self, mark: String, text: &str) {
        self.task.abort();
        write(&format!("\r{ESC}K  {mark}{RESET} {text}\n"));
    }
    fn done(&self, text: &str) {
        self.end(format!("{}✓", green()), text);
    }
    fn fail(&self, text: &str) {
        self.end(format!("{}✗", red()), text);
    }
}

// a bar `width` cells wide, filled to `fraction` in the gradient
fn bar(fraction: f64, width: usize) -> String {
    let filled = (fraction.clamp(0.0, 1.0) * width as f64).round() as usize;
    let out: String = (0..filled).map(|k| format!("{}━", paint(k as f64 / (width - 1) as f64))).collect();
    format!("{out}{RESET}{}", dim(&"━".repeat(width - filled)))
}

fn megabytes(n: u64) -> String {
    format!("{:.1} MB", n as f64 / 1_048_576.0)
}

// ---------- modisa update ----------

// SHA-256, hex: what a release's checksum is.
// ponytail: written out here (FIPS 180-4) rather than taking a crate for one digest
pub fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut compress = |block: &[u8]| {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([block[4 * i], block[4 * i + 1], block[4 * i + 2], block[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            (hh, g, f, e, d, c, b, a) = (g, f, e, d.wrapping_add(t1), c, b, a, t1.wrapping_add(t2));
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    };
    let full = data.len() / 64 * 64;
    for block in data[..full].chunks(64) {
        compress(block);
    }
    let mut tail = data[full..].to_vec();
    tail.push(0x80);
    while tail.len() % 64 != 56 {
        tail.push(0);
    }
    tail.extend((data.len() as u64 * 8).to_be_bytes());
    for block in tail.chunks(64) {
        compress(block);
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}

// The last response's status and content-length in a header dump (redirects each leave a block of their own).
fn response_head(headers: &str) -> (u16, u64) {
    let (mut status, mut length) = (0, 0);
    for line in headers.lines() {
        if line.starts_with("HTTP/") {
            status = line.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
            length = 0;
        } else if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                length = v.trim().parse().unwrap_or(0);
            }
        }
    }
    (status, length)
}

// The release file, read as it streams in, telling `progress` how much has arrived (and of how much, when known).
// ponytail: curl fetches it (no HTTP client among the crates), its headers dumped beside it for the length and status
async fn download(url: &str, mut progress: impl FnMut(u64, u64)) -> Result<Vec<u8>, String> {
    use tokio::io::AsyncReadExt;
    let mut b = [0u8; 6];
    let _ = getrandom::fill(&mut b);
    let headers = std::env::temp_dir().join(format!("modisa-update-{}-{}.headers", std::process::id(), b.iter().map(|x| format!("{x:02x}")).collect::<String>()));
    let mut child = tokio::process::Command::new("curl")
        .args(["-sS", "-L", "-D"])
        .arg(&headers)
        .args(["--url", url])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("download failed: {e}"))?;
    let mut stdout = child.stdout.take().unwrap();
    let (mut bytes, mut total, mut buf) = (Vec::new(), 0u64, vec![0u8; 64 * 1024]);
    loop {
        let n = stdout.read(&mut buf).await.map_err(|e| format!("download failed: {e}"))?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&buf[..n]);
        if total == 0 {
            total = response_head(&std::fs::read_to_string(&headers).unwrap_or_default()).1;
        }
        progress(bytes.len() as u64, total);
    }
    let out = child.wait_with_output().await.map_err(|e| format!("download failed: {e}"))?;
    let (status, _) = response_head(&std::fs::read_to_string(&headers).unwrap_or_default());
    let _ = std::fs::remove_file(&headers);
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let err = err.strip_prefix("curl: ").map(|e| e.split_once(") ").map_or(e, |(_, m)| m)).unwrap_or(&err).to_string();
        return Err(format!("download failed: {err}"));
    }
    if !(200..300).contains(&status) {
        return Err(format!("download failed: {status}"));
    }
    Ok(bytes)
}

// What `modisa update` shows: plain lines, or on a colour terminal the wordmark, spinners and a download bar.
struct Ui {
    fancy: bool,
}

impl Ui {
    async fn banner(&self, subtitle: &str) {
        if self.fancy {
            banner(subtitle).await;
        }
    }

    // `work` under a spinner; `verdict` is what the line says once it's done (None: it failed)
    async fn step<T>(&self, label: &str, work: impl std::future::Future<Output = T>, verdict: impl Fn(&T) -> Option<String>) -> T {
        if !self.fancy {
            return work.await;
        }
        let s = spinner(label);
        let result = work.await;
        match verdict(&result) {
            Some(said) => s.done(&said),
            None => s.fail(label),
        }
        result
    }

    async fn download(&self, label: &str, url: &str) -> Result<Vec<u8>, String> {
        if !self.fancy {
            outln!("{label}…");
            return download(url, |_, _| {}).await;
        }
        let s = spinner(label);
        let got = download(url, |got, total| {
            s.update(if total > 0 {
                let f = got as f64 / total as f64;
                format!("  {} {:>3}%  {}", bar(f, 24), (f * 100.0).floor(), dim(&format!("{} / {}", megabytes(got), megabytes(total))))
            } else {
                format!("  {}", dim(&megabytes(got)))
            })
        })
        .await;
        match &got {
            Ok(bytes) => s.done(&format!("{label}  {}", dim(&megabytes(bytes.len() as u64)))),
            Err(_) => s.fail(label),
        }
        got
    }

    fn say(&self, message: &str) {
        if self.fancy { outln!("  {}", dim(message)) } else { outln!("{message}") }
    }

    fn fail(&self, message: &str) {
        if self.fancy { errln!("  {}error:{RESET} {message}", red()) } else { errln!("{message}") }
    }

    fn finale(&self, main: &str, detail: &str, notes: &[&str]) {
        if !self.fancy {
            outln!("{main}. {detail}");
            if !notes.is_empty() {
                outln!("\n{}", notes.join("\n"));
            }
            return;
        }
        outln!("\n  {}\n  {}", gradient(&format!("✓ {main}")), dim(detail));
        if !notes.is_empty() {
            outln!("\n{}", notes.iter().map(|n| format!("  {}", dim(n))).collect::<Vec<_>>().join("\n"));
        }
        outln!();
    }
}

pub async fn run_update() -> i32 {
    let code = update(&Ui { fancy: fancy() }).await;
    show_cursor();
    code
}

async fn update(ui: &Ui) -> i32 {
    if FROM_SOURCE {
        ui.say(&format!("modisa {VERSION} runs from source: update it with git pull"));
        return 0;
    }
    // replacing a binary a package manager owns would leave the manager's records wrong
    let how = installed_by(&self_exe(), FROM_SOURCE);
    if let Some(upgrade) = how.upgrade {
        ui.fail(&format!("modisa was installed with {}: update it with `{upgrade}`, then `modisa restart`", how.manager.unwrap_or_default()));
        return 1;
    }
    let Some(plat) = platform() else {
        ui.fail("there's no modisa release for this platform; run it from source");
        return 1;
    };
    ui.banner(&format!("modisa {VERSION} · {plat}")).await;
    let found = ui.step("looking for a newer release", check_for_update(true), |m| Some(m.as_ref().map_or("no newer release".into(), |m| format!("found modisa {}", m.version)))).await;
    let Some(m) = found else {
        ui.say(&format!("modisa {VERSION} is up to date"));
        return 0;
    };
    let Some(asset) = m.assets.get(plat) else {
        ui.fail(&format!("modisa {} has no build for {plat}", m.version));
        return 1;
    };
    let bytes = match ui.download(&format!("downloading modisa {} for {plat}", m.version), &asset.url).await {
        Ok(b) => b,
        Err(e) => {
            ui.fail(&e);
            return 1;
        }
    };
    let matches = ui.step("verifying its SHA-256 checksum", async { sha256_hex(&bytes) == asset.sha256 }, |ok| ok.then(|| "checksum verified".to_string())).await;
    if !matches {
        ui.fail("the download doesn't match its published checksum; nothing was changed");
        return 1;
    }
    let target = self_exe();
    let tmp = format!("{target}.update-{}", now_ms() as u64); // same directory, so the rename is atomic
    let replace = async {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(&tmp, &bytes)?;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
        std::fs::rename(&tmp, &target)
    };
    let moved = ui.step(&format!("replacing {target}"), replace, |r| r.is_ok().then(|| format!("replaced {target}"))).await;
    if let Err(e) = moved {
        let _ = std::fs::remove_file(&tmp);
        ui.fail(&format!("couldn't replace {target}: {e}"));
        return 1;
    }
    let notes: Vec<&str> = m.notes.trim().split('\n').filter(|l| !l.is_empty()).take(8).collect();
    ui.finale(&format!("updated modisa {VERSION} → {}", m.version), "Run `modisa restart` to load it into running sessions.", &notes);
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installs_are_told_apart_by_where_the_binary_is() {
        assert_eq!(installed_by("/opt/homebrew/Cellar/modisa/1.0/bin/modisa", false).upgrade, Some("brew upgrade modisa"));
        assert_eq!(installed_by("/home/me/.local/share/mise/installs/modisa/1/modisa", false).by, "mise");
        assert_eq!(installed_by("/usr/bin/modisa", false).by, "system");
        assert_eq!(installed_by("/home/me/.local/bin/modisa", false).by, "script");
        assert_eq!(installed_by("/usr/bin/modisa", true).by, "source");
    }

    #[test]
    fn a_manifest_keeps_only_assets_with_a_checksum() {
        let sha = "a".repeat(64);
        let m = parse_manifest(&json!({ "version": "1.2.3", "assets": { "darwin-arm64": { "url": "u", "sha256": sha }, "linux-x64": { "url": "u", "sha256": "nope" } } })).unwrap();
        assert_eq!((m.channel.as_str(), m.notes.as_str(), m.assets.len()), ("stable", "", 1));
        assert!(parse_manifest(&json!({ "version": 1, "assets": {} })).is_none());
        assert!(parse_manifest(&json!({ "version": "1", "assets": null })).is_none());
    }

    #[test]
    fn release_manifests_keep_only_well_formed_assets() {
        let sha = "a".repeat(64);
        let m = parse_manifest(&json!({ "version": "0.2.0", "assets": { "darwin-arm64": { "url": "https://x/y", "sha256": sha }, "linux-x64": { "url": "https://x/z", "sha256": "nope" } } })).unwrap();
        assert_eq!(serde_json::to_value(&m).unwrap(), json!({ "version": "0.2.0", "channel": "stable", "notes": "", "assets": { "darwin-arm64": { "url": "https://x/y", "sha256": sha } } }));
        assert!(parse_manifest(&json!({ "assets": {} })).is_none());
        assert!(parse_manifest(&Value::Null).is_none());
    }

    #[test]
    fn sha256_matches_the_standard_vectors() {
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"), "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1");
        assert_eq!(sha256_hex(&vec![b'a'; 1_000_000]), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
    }

    #[test]
    fn a_header_dump_says_the_final_status_and_length() {
        let dump = "HTTP/1.1 302 Found\r\nLocation: x\r\nContent-Length: 5\r\n\r\nHTTP/2 200\r\ncontent-length: 1234\r\n\r\n";
        assert_eq!(response_head(dump), (200, 1234));
        assert_eq!(megabytes(1_572_864), "1.5 MB");
        assert!(bar(0.5, 24).matches('━').count() == 24);
    }
}
