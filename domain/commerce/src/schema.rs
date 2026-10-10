//! Tool arguments checked against the tool's declared input schema before anything runs. The
//! schemas use a small JSON Schema subset: type, enum, properties, required,
//! additionalProperties, items, min/maxItems, uniqueItems, minimum/maximum, min/maxLength and
//! anchored patterns. Annotation keywords (description, default, format) are not assertions.
//! Patterns are matched by a small backtracking matcher for anchored expressions made of
//! literals, escapes, character classes, groups and the quantifiers ? + * {n} {n,m}; a pattern
//! outside that is a definition error, never silently skipped.

use serde_json::Value;

#[derive(Debug, Clone)]
enum Node {
    Lit(char),
    Class(Vec<(char, char)>),
    Group(Vec<(Node, usize, usize)>),
}

fn parse_seq(
    p: &[char],
    i: &mut usize,
    in_group: bool,
) -> Result<Vec<(Node, usize, usize)>, String> {
    let mut seq = vec![];
    while *i < p.len() {
        let c = p[*i];
        let node = match c {
            ')' if in_group => {
                *i += 1;
                return Ok(seq);
            }
            '(' => {
                *i += 1;
                Node::Group(parse_seq(p, i, true)?)
            }
            '[' => {
                *i += 1;
                let mut ranges = vec![];
                while *i < p.len() && p[*i] != ']' {
                    let a = p[*i];
                    if *i + 2 < p.len() && p[*i + 1] == '-' && p[*i + 2] != ']' {
                        ranges.push((a, p[*i + 2]));
                        *i += 3;
                    } else {
                        ranges.push((a, a));
                        *i += 1;
                    }
                }
                if *i >= p.len() {
                    return Err("UNTERMINATED_CLASS".into());
                }
                *i += 1;
                Node::Class(ranges)
            }
            '\\' => {
                *i += 1;
                let e = *p.get(*i).ok_or("DANGLING_ESCAPE")?;
                *i += 1;
                Node::Lit(e)
            }
            '^' | '$' | '|' | '.' | '*' | '+' | '?' | '{' => {
                return Err(format!("UNSUPPORTED_PATTERN_TOKEN {c}"));
            }
            _ => {
                *i += 1;
                Node::Lit(c)
            }
        };
        let (min, max) = match p.get(*i) {
            Some('?') => (*i += 1, (0, 1)).1,
            Some('+') => (*i += 1, (1, usize::MAX)).1,
            Some('*') => (*i += 1, (0, usize::MAX)).1,
            Some('{') => {
                let end = p[*i..]
                    .iter()
                    .position(|c| *c == '}')
                    .ok_or("UNTERMINATED_REPEAT")?
                    + *i;
                let body: String = p[*i + 1..end].iter().collect();
                *i = end + 1;
                let (a, b) = body.split_once(',').unwrap_or((&body, &body));
                let a: usize = a.parse().map_err(|_| "BAD_REPEAT")?;
                let b: usize = if b.is_empty() {
                    usize::MAX
                } else {
                    b.parse().map_err(|_| "BAD_REPEAT")?
                };
                (a, b)
            }
            _ => (1, 1),
        };
        seq.push((node, min, max));
    }
    if in_group {
        return Err("UNTERMINATED_GROUP".into());
    }
    Ok(seq)
}

/// Parses an anchored pattern ("^...$").
fn compile(pattern: &str) -> Result<Vec<(Node, usize, usize)>, String> {
    let body = pattern
        .strip_prefix('^')
        .and_then(|b| b.strip_suffix('$'))
        .ok_or("PATTERN_MUST_BE_ANCHORED")?;
    let chars: Vec<char> = body.chars().collect();
    parse_seq(&chars, &mut 0, false)
}

fn match_seq(
    seq: &[(Node, usize, usize)],
    s: &[char],
    k: &mut dyn FnMut(usize) -> bool,
    at: usize,
) -> bool {
    let Some((atom, rest)) = seq.split_first() else {
        return k(at);
    };
    rep(atom, 0, rest, s, at, k)
}

/// Greedy repetition of one atom with backtracking: n copies matched so far, then the rest.
fn rep(
    atom: &(Node, usize, usize),
    n: usize,
    rest: &[(Node, usize, usize)],
    s: &[char],
    at: usize,
    k: &mut dyn FnMut(usize) -> bool,
) -> bool {
    let (node, min, max) = atom;
    if n < *max && n < s.len() + 1 {
        let advanced = match node {
            Node::Lit(c) => (s.get(at) == Some(c))
                .then_some(at + 1)
                .map(|next| rep(atom, n + 1, rest, s, next, k)),
            Node::Class(r) => s
                .get(at)
                .filter(|c| r.iter().any(|(a, b)| a <= *c && *c <= b))
                .map(|_| rep(atom, n + 1, rest, s, at + 1, k)),
            Node::Group(g) => {
                let mut found = false;
                let ok = match_seq(
                    g,
                    s,
                    &mut |next| {
                        if next == at {
                            return false;
                        }
                        found = rep(atom, n + 1, rest, s, next, k);
                        found
                    },
                    at,
                );
                Some(ok && found)
            }
        };
        if advanced == Some(true) {
            return true;
        }
    }
    n >= *min && match_seq(rest, s, k, at)
}

pub fn pattern_matches(pattern: &str, s: &str) -> Result<bool, String> {
    let seq = compile(pattern)?;
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    Ok(match_seq(&seq, &chars, &mut |end| end == n, 0))
}

fn kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// The first way `value` breaks `schema`, as "SCHEMA_VIOLATION <path>: <reason>".
pub fn validate(schema: &Value, value: &Value, path: &str) -> Result<(), String> {
    let fail = |why: String| Err(format!("SCHEMA_VIOLATION {path}: {why}"));
    if let Some(t) = schema["type"].as_str() {
        let k = kind(value);
        if !(k == t || (t == "number" && k == "integer")) {
            return fail(format!("expected {t}, got {k}"));
        }
    }
    if let Some(e) = schema["enum"].as_array()
        && !e.contains(value)
    {
        return fail("not one of the allowed values".into());
    }
    match value {
        Value::Object(o) => {
            for r in schema["required"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                if !o.contains_key(r) {
                    return fail(format!("missing required {r}"));
                }
            }
            let props = schema["properties"].as_object();
            for (k, v) in o {
                match props.and_then(|p| p.get(k)) {
                    Some(s) => validate(s, v, &format!("{path}.{k}"))?,
                    None => match &schema["additionalProperties"] {
                        Value::Bool(false) => return fail(format!("unknown property {k}")),
                        s @ Value::Object(_) => validate(s, v, &format!("{path}.{k}"))?,
                        _ => {}
                    },
                }
            }
        }
        Value::Array(a) => {
            if schema["minItems"]
                .as_u64()
                .is_some_and(|m| (a.len() as u64) < m)
            {
                return fail("too few items".into());
            }
            if schema["maxItems"]
                .as_u64()
                .is_some_and(|m| (a.len() as u64) > m)
            {
                return fail("too many items".into());
            }
            if schema["uniqueItems"] == true
                && a.iter().enumerate().any(|(i, x)| a[..i].contains(x))
            {
                return fail("duplicate items".into());
            }
            if schema["items"].is_object() {
                for (i, x) in a.iter().enumerate() {
                    validate(&schema["items"], x, &format!("{path}[{i}]"))?;
                }
            }
        }
        Value::String(s) => {
            let n = s.chars().count() as u64;
            if schema["minLength"].as_u64().is_some_and(|m| n < m) {
                return fail("too short".into());
            }
            if schema["maxLength"].as_u64().is_some_and(|m| n > m) {
                return fail("too long".into());
            }
            if let Some(p) = schema["pattern"].as_str()
                && !pattern_matches(p, s)?
            {
                return fail("does not match the pattern".into());
            }
        }
        Value::Number(_) => {
            let x = value.as_f64().unwrap_or(f64::NAN);
            if schema["minimum"].as_f64().is_some_and(|m| x < m) {
                return fail("below the minimum".into());
            }
            if schema["maximum"].as_f64().is_some_and(|m| x > m) {
                return fail("above the maximum".into());
            }
        }
        _ => {}
    }
    Ok(())
}

/// Every pattern a schema declares, for checking that each one compiles.
pub fn patterns(schema: &Value, out: &mut Vec<String>) {
    match schema {
        Value::Object(o) => {
            if let Some(p) = o.get("pattern").and_then(Value::as_str) {
                out.push(p.to_string());
            }
            for v in o.values() {
                patterns(v, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|v| patterns(v, out)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_patterns_in_use_match_as_regular_expressions_would() {
        let cases: &[(&str, &str, bool)] = &[
            ("^[A-Z0-9]{10}$", "B07N4M94X4", true),
            ("^[A-Z0-9]{10}$", "b07n4m94x4", false),
            ("^[A-Z0-9]{10}$", "B07N4M94X", false),
            ("^[0-9]+(\\.[0-9]+)?$", "2980", true),
            ("^[0-9]+(\\.[0-9]+)?$", "19.99", true),
            ("^[0-9]+(\\.[0-9]+)?$", "19.", false),
            ("^[0-9]+(\\.[0-9]+)?$", ".5", false),
            ("^[0-9]+(\\.[0-9]+)?$", "1.2.3", false),
            ("^[0-9]{8,14}$", "4901234567894", true),
            ("^[0-9]{8,14}$", "1234567", false),
            ("^[A-Z]{2}$", "JP", true),
            ("^[A-Z]{2}$", "jp", false),
            ("^[A-Z]{3}$", "JPY", true),
        ];
        for (p, s, want) in cases {
            assert_eq!(pattern_matches(p, s).unwrap(), *want, "{p} {s}");
        }
        assert!(pattern_matches("[a]", "a").is_err(), "unanchored");
        assert!(
            pattern_matches("^a|b$", "a").is_err(),
            "alternation unsupported"
        );
    }

    #[test]
    fn arguments_outside_the_schema_are_refused() {
        let s = json!({"type":"object","properties":{"q":{"type":"string","maxLength":3},"n":{"type":"integer","minimum":1,"maximum":5},"tags":{"type":"array","items":{"enum":["a","b"]},"maxItems":2,"uniqueItems":true}},"required":["q"],"additionalProperties":false});
        assert!(validate(&s, &json!({"q":"abc","n":5,"tags":["a","b"]}), "args").is_ok());
        for bad in [
            json!({}),
            json!({"q":"abcd"}),
            json!({"q":"a","n":0}),
            json!({"q":"a","n":1.5}),
            json!({"q":"a","x":1}),
            json!({"q":"a","tags":["c"]}),
            json!({"q":"a","tags":["a","a"]}),
            json!({"q":3}),
        ] {
            assert!(validate(&s, &bad, "args").is_err(), "{bad}");
        }
        assert!(
            validate(&json!({}), &json!({"anything":[1]}), "v").is_ok(),
            "an empty schema accepts all"
        );
    }

    #[test]
    fn every_declared_tool_schema_is_enforceable() {
        let mut found = vec![];
        for tool in crate::service::tool_definitions() {
            patterns(&tool["inputSchema"], &mut found);
        }
        assert!(!found.is_empty());
        for p in found {
            assert!(compile(&p).is_ok(), "{p}");
        }
    }

    #[test]
    fn unknown_arguments_are_refused_before_a_tool_runs() {
        let root = std::env::temp_dir().join(format!(
            "ecdev-schema-{}",
            crate::identifier::Uuid::new_v4()
        ));
        let engine = crate::Engine::open(&root).unwrap();
        // The slip a live run made: an unknown field inside a source.
        let e = engine
            .call(
                "ecdev.trend.discover",
                json!({"query":"m","sources":[{"platform":"MASTODON_TAG","query_note":"x"}]}),
            )
            .unwrap_err();
        assert!(
            e.starts_with("SCHEMA_VIOLATION arguments.sources[0]"),
            "{e}"
        );
        assert!(
            engine
                .call("ecdev.listing.search", json!({"query":"m","results":51}))
                .unwrap_err()
                .starts_with("SCHEMA_VIOLATION")
        );
        // A tool's own policy reason is kept where it says more.
        assert_eq!(
            engine
                .call(
                    "ecdev.seller.read",
                    json!({"market":"AMAZON_US","operation":"ORDERS"})
                )
                .unwrap_err(),
            "RESTRICTED_DOMAIN_DISABLED"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
