// Wire protocol: JSON-RPC 2.0, newline-delimited, over a unix socket (or ssh stdio for --remote). Requests get
// responses; the server pushes events as notifications ({method, params}, no id).
//
// The TypeScript original validated params with zod schemas; here each handler reads its params through `Params`,
// which checks the same constraints and reports a failure the same way: `invalid params: <path> <message>`.
use indexmap::IndexMap;
use serde_json::{Map, Value};

use super::conn::{fail, RpcError, RpcResult};

// The protocol version: bumped when a request, result or event changes incompatibly. Plugins declare the one they speak.
pub const PROTOCOL: u32 = 1;

// The protocol as JSON Schema (`protocol.describe`, `modisa plugin schema`), as the TypeScript server generated it from
// its zod schemas.
// ponytail: a captured copy; regenerate it when a request, result or event changes
pub const DESCRIBE: &str = include_str!("describe.json");

pub fn invalid(path: &str, message: impl AsRef<str>) -> RpcError {
    fail("invalid_params", format!("invalid params: {path} {}", message.as_ref()))
}

fn kind(v: Option<&Value>) -> &'static str {
    match v {
        None => "undefined",
        Some(Value::Null) => "null",
        Some(Value::Bool(_)) => "boolean",
        Some(Value::Number(_)) => "number",
        Some(Value::String(_)) => "string",
        Some(Value::Array(_)) => "array",
        Some(Value::Object(_)) => "object",
    }
}

fn expected(path: &str, want: &str, got: Option<&Value>) -> RpcError {
    invalid(path, format!("Invalid input: expected {want}, received {}", kind(got)))
}

pub struct Params<'a> {
    m: &'a Map<String, Value>,
    prefix: String,
}

static EMPTY: std::sync::LazyLock<Map<String, Value>> = std::sync::LazyLock::new(Map::new);

impl<'a> Params<'a> {
    pub fn new(v: &'a Value) -> RpcResult<Params<'a>> {
        match v {
            Value::Object(m) => Ok(Params { m, prefix: String::new() }),
            Value::Null => Ok(Params { m: &EMPTY, prefix: String::new() }),
            other => Err(invalid("", format!("Invalid input: expected object, received {}", kind(Some(other))))),
        }
    }

    fn path(&self, k: &str) -> String {
        format!("{}{k}", self.prefix)
    }

    // given at all: like zod's .optional(), null is a value (and the wrong type), not an absent one
    pub fn raw(&self, k: &str) -> Option<&'a Value> {
        self.m.get(k)
    }

    pub fn has(&self, k: &str) -> bool {
        self.raw(k).is_some()
    }

    pub fn nested(&self, k: &str) -> RpcResult<Option<Params<'a>>> {
        match self.raw(k) {
            None => Ok(None),
            Some(Value::Object(m)) => Ok(Some(Params { m, prefix: format!("{}.", self.path(k)) })),
            other => Err(expected(&self.path(k), "object", other)),
        }
    }

    pub fn opt_str(&self, k: &str) -> RpcResult<Option<String>> {
        match self.raw(k) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            other => Err(expected(&self.path(k), "string", other)),
        }
    }

    // a string of at least `min` characters (and at most `max`), when given
    pub fn opt_len(&self, k: &str, min: usize, max: Option<usize>) -> RpcResult<Option<String>> {
        let Some(s) = self.opt_str(k)? else { return Ok(None) };
        let n = s.chars().count();
        if n < min {
            return Err(invalid(&self.path(k), format!("Too small: expected string to have >={min} characters")));
        }
        if let Some(max) = max.filter(|&m| n > m) {
            return Err(invalid(&self.path(k), format!("Too big: expected string to have <={max} characters")));
        }
        Ok(Some(s))
    }

    pub fn str(&self, k: &str) -> RpcResult<String> {
        self.opt_str(k)?.ok_or_else(|| expected(&self.path(k), "string", None))
    }

    pub fn len(&self, k: &str, min: usize, max: Option<usize>) -> RpcResult<String> {
        self.opt_len(k, min, max)?.ok_or_else(|| expected(&self.path(k), "string", None))
    }

    pub fn opt_bool(&self, k: &str) -> RpcResult<Option<bool>> {
        match self.raw(k) {
            None => Ok(None),
            Some(Value::Bool(b)) => Ok(Some(*b)),
            other => Err(expected(&self.path(k), "boolean", other)),
        }
    }

    pub fn flag(&self, k: &str) -> RpcResult<bool> {
        Ok(self.opt_bool(k)?.unwrap_or(false))
    }

    pub fn opt_num(&self, k: &str, min: Option<f64>, max: Option<f64>, int: bool) -> RpcResult<Option<f64>> {
        let n = match self.raw(k) {
            None => return Ok(None),
            Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
            other => return Err(expected(&self.path(k), "number", other)),
        };
        if int && n.fract() != 0.0 {
            return Err(expected(&self.path(k), "int", self.raw(k)));
        }
        if let Some(min) = min.filter(|&m| n < m) {
            return Err(invalid(&self.path(k), format!("Too small: expected number to be >={min}")));
        }
        if let Some(max) = max.filter(|&m| n > m) {
            return Err(invalid(&self.path(k), format!("Too big: expected number to be <={max}")));
        }
        Ok(Some(n))
    }

    pub fn num_or(&self, k: &str, default: f64, min: Option<f64>, max: Option<f64>, int: bool) -> RpcResult<f64> {
        Ok(self.opt_num(k, min, max, int)?.unwrap_or(default))
    }

    pub fn opt_enum(&self, k: &str, options: &[&str]) -> RpcResult<Option<String>> {
        match self.raw(k) {
            None => Ok(None),
            Some(Value::String(s)) if options.contains(&s.as_str()) => Ok(Some(s.clone())),
            _ => Err(invalid(&self.path(k), format!("Invalid option: expected one of {}", options.iter().map(|o| format!("\"{o}\"")).collect::<Vec<_>>().join("|")))),
        }
    }

    pub fn enum_or(&self, k: &str, options: &[&str], default: &str) -> RpcResult<String> {
        Ok(self.opt_enum(k, options)?.unwrap_or_else(|| default.to_string()))
    }

    pub fn str_list(&self, k: &str, min_items: usize, min_len: usize) -> RpcResult<Option<Vec<String>>> {
        let list = match self.raw(k) {
            None => return Ok(None),
            Some(Value::Array(a)) => a,
            other => return Err(expected(&self.path(k), "array", other)),
        };
        if list.len() < min_items {
            return Err(invalid(&self.path(k), format!("Too small: expected array to have >={min_items} items")));
        }
        list.iter()
            .enumerate()
            .map(|(i, v)| match v {
                Value::String(s) if s.chars().count() >= min_len => Ok(s.clone()),
                Value::String(_) => Err(invalid(&format!("{}.{i}", self.path(k)), format!("Too small: expected string to have >={min_len} characters"))),
                other => Err(expected(&format!("{}.{i}", self.path(k)), "string", Some(other))),
            })
            .collect::<RpcResult<Vec<_>>>()
            .map(Some)
    }

    // A record of anything (an action's params): kept as given.
    pub fn record(&self, k: &str) -> RpcResult<Option<Map<String, Value>>> {
        match self.raw(k) {
            None => Ok(None),
            Some(Value::Object(m)) => Ok(Some(m.clone())),
            other => Err(expected(&self.path(k), "record", other)),
        }
    }

    // Variables a new pane gets on top of the server's environment. Saved with the session (a restart keeps them), so
    // they're on disk. MODISA_ ones are modisa's: MODISA_PANE_ID is always the pane's own.
    pub fn env(&self, k: &str) -> RpcResult<Option<IndexMap<String, String>>> {
        let m = match self.raw(k) {
            None => return Ok(None),
            Some(Value::Object(m)) => m,
            other => return Err(expected(&self.path(k), "record", other)),
        };
        let mut out = IndexMap::new();
        for (name, v) in m {
            let path = format!("{}.{name}", self.path(k));
            let valid = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            if !valid {
                return Err(invalid(&path, "isn't a variable name (letters, digits and _)"));
            }
            if name.starts_with("MODISA_") {
                return Err(invalid(&path, "is modisa's: MODISA_ variables can't be set"));
            }
            match v {
                Value::String(s) => out.insert(name.clone(), s.clone()),
                other => return Err(expected(&path, "string", Some(other))),
            };
        }
        Ok(Some(out))
    }

    // checked after the fields, like a zod refine: its message has no path
    pub fn refine(&self, ok: bool, message: &str) -> RpcResult<()> {
        if ok { Ok(()) } else { Err(fail("invalid_params", format!("invalid params:  {message}"))) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validates_like_zod() {
        let v = json!({ "target": "", "env": { "MODISA_PANE_ID": "x" }, "dir": "up", "ratio": 2 });
        let p = Params::new(&v).unwrap();
        assert_eq!(p.opt_len("target", 1, None).unwrap_err().message, "invalid params: target Too small: expected string to have >=1 characters");
        assert_eq!(p.env("env").unwrap_err().message, "invalid params: env.MODISA_PANE_ID is modisa's: MODISA_ variables can't be set");
        assert_eq!(p.env("nope").unwrap(), None);
        assert!(p.opt_enum("dir", &["right", "down"]).is_err());
        assert!(p.num_or("ratio", 0.5, Some(0.1), Some(0.9), false).is_err());
        assert_eq!(Params::new(&json!({ "ratio": null })).unwrap().opt_num("ratio", None, None, false).unwrap_err().message, "invalid params: ratio Invalid input: expected number, received null");
        let bad = json!({ "env": { "1UP": "x" } });
        assert_eq!(Params::new(&bad).unwrap().env("env").unwrap_err().message, "invalid params: env.1UP isn't a variable name (letters, digits and _)");
    }
}
