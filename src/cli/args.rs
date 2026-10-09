// Command-line argument parsing: positionals, --flags (valued, boolean or repeated), and -s <session>.
use indexmap::IndexMap;

// A flag is a bare switch (`--json`) or has a value (`--name x`, `--name=x`, which may be empty).
#[derive(Clone, Debug, PartialEq)]
pub enum Flag {
    On,
    Value(String),
}

// `_` in the original: the positionals.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Args {
    pub pos: Vec<String>,
    pub flags: IndexMap<String, Flag>,
    pub lists: IndexMap<String, Vec<String>>,
}

const BOOLEAN: &[&str] = &["json", "follow", "exited", "right", "down", "tab", "focus", "output", "help", "idle", "release", "version", "purge", "yes", "screen", "new-tab", "new-workspace", "on", "off", "toggle", "system", "sound", "takeover", "observe", "ansi"];
// Flags given once per value, which all count: --env A=1 --env B=2. They're in `lists`, not `flags`.
const REPEATABLE: &[&str] = &["env"];
// Flags that take a value in one command though they're switches elsewhere: `agent spawn --tab` opens a tab, while
// `pane move --tab t` names one.
const VALUED: &[(&str, &[&str])] = &[("pane move", &["tab"])];

pub fn parse_args(argv: &[String]) -> Args {
    let mut a = Args::default();
    let mut i = 0;
    while i < argv.len() {
        let x = &argv[i];
        if x == "--" {
            a.pos.extend(argv[i + 1..].iter().cloned());
            break;
        } else if let Some(name) = x.strip_prefix("--") {
            let eq = name.find('='); // the first one: --match=a=b matches a=b
            let k = eq.map_or(name, |e| &name[..e]).to_string();
            let command = a.pos.iter().take(2).cloned().collect::<Vec<_>>().join(" ");
            let valued = VALUED.iter().any(|(c, ks)| *c == command && ks.contains(&k.as_str()));
            let boolean = BOOLEAN.contains(&k.as_str()) && !valued;
            let bare = i + 1 >= argv.len() || argv[i + 1].starts_with("--"); // no value follows
            if REPEATABLE.contains(&k.as_str()) {
                let v = match eq {
                    Some(e) => name[e + 1..].to_string(),
                    None if bare => String::new(),
                    None => {
                        i += 1;
                        argv[i].clone()
                    }
                };
                a.lists.entry(k).or_default().push(v);
            } else if let Some(e) = eq {
                a.flags.insert(k, Flag::Value(name[e + 1..].to_string()));
            } else if boolean || bare {
                a.flags.insert(k, Flag::On);
            } else {
                i += 1;
                a.flags.insert(k, Flag::Value(argv[i].clone()));
            }
        } else if x == "-s" {
            i += 1;
            // a trailing -s names no session (the original sets it to undefined)
            match argv.get(i) {
                Some(s) => a.flags.insert("session".into(), Flag::Value(s.clone())),
                None => a.flags.shift_remove("session"),
            };
        } else {
            a.pos.push(x.clone());
        }
        i += 1;
    }
    a
}

impl Args {
    // Flag value helpers. A flag's value, when it has one (a bare switch has none).
    pub fn str(&self, k: &str) -> Option<&str> {
        match self.flags.get(k) {
            Some(Flag::Value(v)) => Some(v),
            _ => None,
        }
    }

    // A flag's value as a number, the way JavaScript's Number() reads it ("" is 0, "x" is NaN).
    pub fn num(&self, k: &str) -> Option<f64> {
        self.str(k).map(js_number)
    }

    // Whether a flag counts as given, the way JavaScript tests it: a switch, or a value that isn't empty.
    pub fn on(&self, k: &str) -> bool {
        match self.flags.get(k) {
            Some(Flag::On) => true,
            Some(Flag::Value(v)) => !v.is_empty(),
            None => false,
        }
    }

    // A bare switch (the original's `=== true`): --new-tab, not --new-tab=x.
    pub fn switch(&self, k: &str) -> bool {
        self.flags.get(k) == Some(&Flag::On)
    }

    pub fn has(&self, k: &str) -> bool {
        self.flags.contains_key(k)
    }
}

// JavaScript's Number(string): whitespace around it ignored, "" is 0, 0x/0o/0b prefixes, Infinity; anything else that
// isn't a decimal number is NaN.
pub fn js_number(s: &str) -> f64 {
    let t = s.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    if t.is_empty() {
        return 0.0;
    }
    let radix = |digits: &str, r: u32| {
        if digits.is_empty() {
            return f64::NAN;
        }
        digits.chars().try_fold(0f64, |n, c| c.to_digit(r).map(|d| n * r as f64 + d as f64)).unwrap_or(f64::NAN)
    };
    match t.get(..2) {
        Some("0x" | "0X") => return radix(&t[2..], 16),
        Some("0o" | "0O") => return radix(&t[2..], 8),
        Some("0b" | "0B") => return radix(&t[2..], 2),
        _ => {}
    }
    match t {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    // Rust's parser also takes "inf" and "nan", which JavaScript doesn't
    if t.bytes().all(|b| b.is_ascii_digit() || b"+-.eE".contains(&b)) {
        t.parse().unwrap_or(f64::NAN)
    } else {
        f64::NAN
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(argv: &[&str]) -> Args {
        parse_args(&argv.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }
    fn args(pos: &[&str], flags: &[(&str, Flag)], lists: &[(&str, &[&str])]) -> Args {
        Args {
            pos: pos.iter().map(|s| s.to_string()).collect(),
            flags: flags.iter().map(|(k, v)| (k.to_string(), v.clone())).collect(),
            lists: lists.iter().map(|(k, v)| (k.to_string(), v.iter().map(|s| s.to_string()).collect())).collect(),
        }
    }
    fn val(s: &str) -> Flag {
        Flag::Value(s.into())
    }

    #[test]
    fn a_flag_value_keeps_everything_after_the_first_equals() {
        assert_eq!(parse(&["wait", "p1", "--match=a=b"]).str("match"), Some("a=b"));
        assert_eq!(parse(&["--name=", "x"]), args(&["x"], &[("name", val(""))], &[]));
    }

    #[test]
    fn switches_never_take_the_next_word_as_their_value() {
        for flag in ["screen", "new-tab", "new-workspace", "on", "off", "toggle", "takeover", "observe"] {
            assert_eq!(parse(&["pane", "x", &format!("--{flag}"), "p1"]), args(&["pane", "x", "p1"], &[(flag, Flag::On)], &[]));
        }
    }

    #[test]
    fn tab_is_a_switch_for_agent_spawn_but_names_a_tab_for_pane_move() {
        assert_eq!(parse(&["agent", "spawn", "codex", "--tab", "--name", "x"]).flags, args(&[], &[("tab", Flag::On), ("name", val("x"))], &[]).flags);
        assert_eq!(parse(&["agent", "spawn", "--tab", "codex"]), args(&["agent", "spawn", "codex"], &[("tab", Flag::On)], &[]));
        assert_eq!(parse(&["-s", "s", "pane", "move", "p3", "--tab", "t2", "--focus"]), args(&["pane", "move", "p3"], &[("session", val("s")), ("tab", val("t2")), ("focus", Flag::On)], &[]));
        assert_eq!(parse(&["pane", "move", "--tab", "t2", "p3"]).pos, ["pane", "move", "p3"]);
    }

    #[test]
    fn env_adds_up_in_order_with_either_spelling() {
        assert_eq!(parse(&["pane", "split", "--env", "A=1", "--env=B=x=y", "--down", "--env", "A=2", "echo"]), args(&["pane", "split", "echo"], &[("down", Flag::On)], &[("env", &["A=1", "B=x=y", "A=2"])]));
        // with no value it's still there, empty, for the command to refuse
        assert_eq!(parse(&["pane", "split", "--env", "--down"]).lists["env"], [""]);
        assert_eq!(parse(&["pane", "split", "--env"]).lists["env"], [""]);
    }

    #[test]
    fn everything_after_a_double_dash_is_positional() {
        assert_eq!(parse(&["pane", "split", "--", "ls", "--all", "-s"]).pos, ["pane", "split", "ls", "--all", "-s"]);
        assert_eq!(parse(&["-s"]), Args::default());
    }

    #[test]
    fn numbers_read_like_javascript() {
        assert_eq!(js_number(""), 0.0);
        assert_eq!(js_number(" 12 "), 12.0);
        assert_eq!(js_number("0x10"), 16.0);
        assert_eq!(js_number("1e3"), 1000.0);
        assert_eq!(js_number("-Infinity"), f64::NEG_INFINITY);
        for nan in ["abc", "inf", "nan", "1e", "-", "0x", "1_000"] {
            assert!(js_number(nan).is_nan(), "{nan}");
        }
        let a = parse(&["pane", "read", "--lines", "abc", "--ratio", "0.3", "--json"]);
        assert!(a.num("lines").unwrap().is_nan());
        assert_eq!(a.num("ratio"), Some(0.3));
        assert_eq!(a.num("json"), None);
    }
}
