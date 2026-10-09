// Which modisa this is: the release version baked in at build time (MODISA_BUILD_VERSION), or Cargo.toml's version +
// "-dev" for a developer's build; its update channel; and the platform name release assets use.
pub const VERSION: &str = match option_env!("MODISA_BUILD_VERSION") {
    Some(v) => v,
    None => concat!(env!("CARGO_PKG_VERSION"), "-dev"),
};
pub const FROM_SOURCE: bool = option_env!("MODISA_BUILD_VERSION").is_none();
pub const REPO: &str = "manyeya/modisa";

pub fn channel() -> &'static str {
    if VERSION.contains("-staging") { "staging" } else { "stable" }
}

// darwin-arm64, linux-x64, linux-arm64: the release assets there are
pub fn platform() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("darwin-arm64"),
        ("linux", "x86_64") => Some("linux-x64"),
        ("linux", "aarch64") => Some("linux-arm64"),
        _ => None,
    }
}

// Semver order, prereleases included: 1.2.0-staging.3 < 1.2.0-staging.10 < 1.2.0; -dev sorts like any prerelease.
pub fn compare(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    let split = |v: &str| {
        let v = v.strip_prefix('v').unwrap_or(v);
        let (core, pre) = v.split_once('-').map(|(c, p)| (c, Some(p))).unwrap_or((v, None));
        let nums: Vec<u64> = core.split('.').map(|n| n.parse().unwrap_or(0)).collect();
        (nums, pre.map(|p| p.split('.').map(String::from).collect::<Vec<_>>()).unwrap_or_default())
    };
    let (x, y) = (split(a), split(b));
    for i in 0..3 {
        let (p, q) = (x.0.get(i).copied().unwrap_or(0), y.0.get(i).copied().unwrap_or(0));
        if p != q {
            return p.cmp(&q);
        }
    }
    if x.1.is_empty() || y.1.is_empty() {
        return y.1.len().cmp(&x.1.len()); // a release beats its prereleases
    }
    for i in 0..x.1.len().max(y.1.len()) {
        let (Some(p), Some(q)) = (x.1.get(i), y.1.get(i)) else { return if x.1.get(i).is_none() { Less } else { Greater } };
        let (np, nq) = (p.parse::<u64>().ok(), q.parse::<u64>().ok());
        match (np, nq) {
            (Some(a), Some(b)) if a != b => return a.cmp(&b),
            (Some(_), None) => return Less,
            (None, Some(_)) => return Greater,
            _ if p != q => return p.cmp(q),
            _ => {}
        }
    }
    Equal
}

pub fn newer(candidate: &str, current: &str) -> bool {
    compare(candidate, current).is_gt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_order() {
        let sorted = ["0.1.0-dev", "0.1.0-staging.2", "0.1.0-staging.10", "0.1.0", "0.1.1-staging.1", "0.1.1", "0.2.0", "1.0.0"];
        for w in sorted.windows(2) {
            assert!(compare(w[0], w[1]).is_lt(), "{} < {}", w[0], w[1]);
        }
        assert!(newer("v0.1.1", "0.1.0"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("0.1.0-staging.3", "0.1.0"));
    }
}
