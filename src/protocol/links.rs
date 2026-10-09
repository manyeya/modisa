// Link handlers: which URLs a plugin's link entry takes. One module for the manifest check, the server's check on
// plugin.invoke and the TUI's chooser, so all three match the same way under the same limits.
//
// - `pattern`: a URL glob. `*` is any run of characters, everything else is literal; the scheme and host ignore case.
// - `regex`: RE2 syntax, run by the regex crate (RE2's semantics and its linear-time guarantee: no backreferences or
//   lookaround), found anywhere in the URL unless anchored. It sees the URL exactly as captured; `(?i)` makes it
//   ignore case.
//
// Matching one URL against one entry is O(URL length × entry size): URLs are at most LINK.url characters, globs at most
// LINK.source, and a regex's compiled program at most LINK.program instructions (with RE2's own repeat cap of 1000 and
// nesting at most LINK.depth). With at most LINK.per_plugin entries a plugin, a click or an invoke does bounded work.
use std::cell::RefCell;
use std::collections::HashMap;

use regex::Regex;
use serde::{Deserialize, Serialize};

pub struct Limits {
    pub url: usize,
    pub source: usize,
    pub program: usize,
    pub depth: usize,
    pub per_plugin: usize,
}

pub const LINK: Limits = Limits { url: 2048, source: 500, program: 300, depth: 16, per_plugin: 8 };

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LinkEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regex: Option<String>,
    pub action: String,
}

// the scheme and host, lowercased
fn lower_origin(s: &str) -> String {
    let lower = s.to_lowercase();
    let Some(scheme_end) = s.find("://").filter(|&i| i > 0 && s[..i].chars().all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c)) && s.as_bytes()[0].is_ascii_alphabetic()) else { return s.to_string() };
    let host_end = s[scheme_end + 3..].find(['/', '?', '#']).map(|i| i + scheme_end + 3).unwrap_or(s.len());
    if !s.is_char_boundary(host_end) || lower.len() != s.len() {
        return s.to_string();
    }
    format!("{}{}", &lower[..host_end], &s[host_end..])
}

// the greedy wildcard match: at most pattern × URL steps
pub fn url_matches(glob: &str, link: &str) -> bool {
    let p: Vec<char> = lower_origin(glob).chars().collect();
    let u: Vec<char> = lower_origin(link).chars().collect();
    let (mut pi, mut ui, mut star, mut resume) = (0usize, 0usize, None::<usize>, 0usize);
    while ui < u.len() {
        if pi < p.len() && p[pi] != '*' && p[pi] == u[ui] {
            pi += 1;
            ui += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            pi += 1;
            resume = ui;
        } else if let Some(s) = star {
            pi = s + 1;
            resume += 1;
            ui = resume;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

// how deep a regex's groups nest, skipping escapes and character classes
// ponytail: a `]` first in a class is taken as its end; the compiled-size limit still bounds such a pattern
fn nesting(source: &str) -> usize {
    let (mut deepest, mut open, mut in_class) = (0usize, 0i64, false);
    let mut chars = source.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            chars.next();
        } else if in_class {
            in_class = c != ']';
        } else if c == '[' {
            in_class = true;
        } else if c == '(' {
            open += 1;
            deepest = deepest.max(open as usize);
        } else if c == ')' {
            open -= 1;
        }
    }
    deepest
}

// RE2's repetition rules, which the regex crate doesn't share: a count is at most 1000, and nested repetitions multiply
// to at most 1000.
fn repeats(source: &str) -> Result<(), String> {
    use regex_syntax::ast::{Ast, RepetitionKind, RepetitionRange};
    let ast = regex_syntax::ast::parse::Parser::new().parse(source).map_err(|e| e.kind().to_string())?;
    fn walk(a: &Ast, outer: u64, source: &str) -> Result<(), String> {
        match a {
            Ast::Repetition(r) => {
                let count = match &r.op.kind {
                    RepetitionKind::Range(RepetitionRange::Exactly(n)) | RepetitionKind::Range(RepetitionRange::AtLeast(n)) => *n as u64,
                    RepetitionKind::Range(RepetitionRange::Bounded(n, m)) => (*n).max(*m) as u64,
                    _ => 1,
                };
                if count > 1000 {
                    return Err(format!("invalid repeat count: `{}`", &source[r.op.span.start.offset..r.op.span.end.offset]));
                }
                let nested = outer * count.max(1);
                if nested > 1000 && outer > 1 {
                    return Err("invalid nested repetition operator".into());
                }
                walk(&r.ast, nested, source)
            }
            Ast::Group(g) => walk(&g.ast, outer, source),
            Ast::Alternation(alt) => alt.asts.iter().try_for_each(|x| walk(x, outer, source)),
            Ast::Concat(c) => c.asts.iter().try_for_each(|x| walk(x, outer, source)),
            _ => Ok(()),
        }
    }
    walk(&ast, 1, source)
}

thread_local! {
    // ponytail: never evicted; its entries come from manifests, a few per plugin
    static COMPILED: RefCell<HashMap<String, Regex>> = RefCell::new(HashMap::new());
}

pub fn glob_problem(pattern: &str) -> Option<String> {
    (!(pattern.starts_with("http://") || pattern.starts_with("https://"))).then(|| "a link pattern is a URL glob starting with http:// or https://, like https://github.com/*/pull/*".to_string())
}

// why a regex can't be a link's, with what to change; None when it can
pub fn regex_problem(source: &str) -> Option<String> {
    if source.chars().count() > LINK.source {
        return Some(format!("a link regex is at most {} characters", LINK.source));
    }
    let depth = nesting(source);
    if depth > LINK.depth {
        return Some(format!("its groups nest {depth} deep; the limit is {}", LINK.depth));
    }
    let unsupported = |why: String| Some(format!("not a supported regular expression (RE2 syntax: no backreferences or lookaround): {why}"));
    if let Err(why) = repeats(source) {
        return unsupported(why);
    }
    let size = match regex_automata::nfa::thompson::NFA::new(source) {
        Ok(nfa) => nfa.states().len(),
        Err(e) => return unsupported(e.to_string().lines().last().unwrap_or("").trim().to_string()),
    };
    if size > LINK.program {
        return Some(format!("it compiles to {size} instructions; the limit is {}: simplify it, or split it into several links", LINK.program));
    }
    None
}

fn test_regex(source: &str, url: &str) -> bool {
    COMPILED.with(|c| {
        let mut c = c.borrow_mut();
        if !c.contains_key(source) {
            match Regex::new(source) {
                Ok(re) => c.insert(source.to_string(), re),
                Err(_) => return false,
            };
        }
        c[source].is_match(url)
    })
}

// Does this entry take this URL? Only http(s) URLs within the length limit are taken, and an entry that breaks its
// limits takes nothing.
pub fn link_matches(entry: &LinkEntry, url: &str) -> bool {
    let lower = url.get(..8).unwrap_or(url).to_lowercase();
    if url.chars().count() > LINK.url || !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return false;
    }
    if let Some(p) = &entry.pattern {
        return p.chars().count() <= LINK.source && glob_problem(p).is_none() && url_matches(p, url);
    }
    entry.regex.as_ref().is_some_and(|r| regex_problem(r).is_none() && test_regex(r, url))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn re(r: &str) -> LinkEntry {
        LinkEntry { pattern: None, regex: Some(r.into()), action: "open".into() }
    }

    #[test]
    fn matches_links() {
        let pr = re(r"^https://github\.com/[^/]+/[^/]+/pull/\d+$");
        assert!(link_matches(&pr, "https://github.com/o/r/pull/12"));
        assert!(!link_matches(&pr, "https://github.com/o/r/pull/12/files"));
        assert!(!link_matches(&pr, "https://GitHub.com/o/r/pull/12"));
        assert!(link_matches(&re(r"(?i)^https://github\.com/"), "https://GitHub.com/o"));
        assert!(link_matches(&re(r"/issues/\d+"), "https://x.dev/a/issues/3"));
        assert!(!link_matches(&re(".*"), "ftp://x.dev/"));
        assert!(!link_matches(&re(".*"), &format!("https://x.dev/{}", "a".repeat(LINK.url))));
        let glob = LinkEntry { pattern: Some("https://github.com/*/pull/*".into()), regex: None, action: "open".into() };
        assert!(link_matches(&glob, "https://GITHUB.com/o/r/pull/1"));
        assert!(!link_matches(&glob, "https://github.com/o/r/issues/1"));
    }

    #[test]
    fn refuses_what_re2_would() {
        let refused = |r: &str| regex_problem(r).unwrap_or_default();
        assert!(refused(r"(a)\1").contains("RE2 syntax: no backreferences or lookaround"));
        assert!(refused("(?=x)").contains("RE2 syntax: no backreferences or lookaround"));
        assert!(refused("(?<=x)a").contains("RE2 syntax: no backreferences or lookaround"));
        assert!(refused("a{1001}").contains("invalid repeat count"));
        assert!(refused("a{1000}{1000}").contains("invalid nested repetition"));
        assert!(regex::Regex::new(r"it compiles to \d+ instructions; the limit is 300").unwrap().is_match(&refused("[a-z]{1000}")));
        assert!(refused(&format!("{}a{}", "(".repeat(17), ")".repeat(17))).contains("its groups nest 17 deep; the limit is 16"));
        assert_eq!(regex_problem(r"^https://github\.com/[^/]+/[^/]+/pull/\d+$"), None);
    }
}
