//! JSON: a reader for legacy registries, a deterministic writer for generated views, and the
//! one-record-per-line dialect of the repository universe (`.ecdev/declared/universe/*.jsonl`,
//! see `universe`), whose lines [`Json::render_line`] writes canonically.

use super::Value;
use std::collections::BTreeMap;

pub fn parse(text: &str) -> Result<Value, String> {
    let mut p = Parser {
        s: text.as_bytes(),
        i: 0,
        depth: 0,
    };
    p.ws();
    let v = p.value()?;
    p.ws();
    if p.i != p.s.len() {
        return Err(p.err("trailing characters"));
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    depth: usize,
}

impl Parser<'_> {
    fn err(&self, m: &str) -> String {
        let line = self.s[..self.i.min(self.s.len())]
            .iter()
            .filter(|c| **c == b'\n')
            .count()
            + 1;
        format!("line {line}: {m}")
    }
    fn ws(&mut self) {
        while matches!(self.s.get(self.i), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }
    fn lit(&mut self, word: &str, v: Value) -> Result<Value, String> {
        if self.s[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(v)
        } else {
            Err(self.err("unexpected token"))
        }
    }
    fn value(&mut self) -> Result<Value, String> {
        if self.depth >= 64 {
            return Err(self.err("JSON nesting exceeds reader budget"));
        }
        self.depth += 1;
        let result = self.value_inner();
        self.depth -= 1;
        result
    }
    fn value_inner(&mut self) -> Result<Value, String> {
        match self.s.get(self.i) {
            Some(b'{') => {
                self.i += 1;
                let mut map = BTreeMap::new();
                self.ws();
                if self.s.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Ok(Value::Table(map));
                }
                loop {
                    self.ws();
                    let k = self.string()?;
                    self.ws();
                    if self.s.get(self.i) != Some(&b':') {
                        return Err(self.err("expected `:`"));
                    }
                    self.i += 1;
                    self.ws();
                    let v = self.value()?;
                    map.insert(k, v);
                    self.ws();
                    match self.s.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Value::Table(map));
                        }
                        _ => return Err(self.err("expected `,` or `}`")),
                    }
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Ok(Value::Array(items));
                }
                loop {
                    self.ws();
                    items.push(self.value()?);
                    self.ws();
                    match self.s.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(Value::Array(items));
                        }
                        _ => return Err(self.err("expected `,` or `]`")),
                    }
                }
            }
            Some(b'"') => self.string().map(Value::Str),
            Some(b't') => self.lit("true", Value::Bool(true)),
            Some(b'f') => self.lit("false", Value::Bool(false)),
            Some(b'n') => self.lit("null", Value::Null),
            Some(_) => {
                let start = self.i;
                while matches!(self.s.get(self.i), Some(c) if c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.' | b'e' | b'E'))
                {
                    self.i += 1;
                }
                let raw = std::str::from_utf8(&self.s[start..self.i]).unwrap_or("");
                if raw.is_empty() {
                    return Err(self.err("unexpected character"));
                }
                Ok(raw
                    .parse::<i64>()
                    .map(Value::Int)
                    .unwrap_or_else(|_| Value::Float(raw.to_string())))
            }
            None => Err(self.err("unexpected end")),
        }
    }
    fn string(&mut self) -> Result<String, String> {
        if self.s.get(self.i) != Some(&b'"') {
            return Err(self.err("expected string"));
        }
        self.i += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            match self.s.get(self.i) {
                None => return Err(self.err("unterminated string")),
                Some(b'"') => {
                    self.i += 1;
                    return Ok(String::from_utf8_lossy(&out).into_owned());
                }
                Some(b'\\') => {
                    self.i += 1;
                    let c = *self.s.get(self.i).ok_or_else(|| self.err("bad escape"))?;
                    self.i += 1;
                    match c {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'/' => out.push(b'/'),
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'u' => {
                            let mut cp = self.hex4()?;
                            if (0xD800..0xDC00).contains(&cp) {
                                if !self.s[self.i..].starts_with(b"\\u") {
                                    return Err(self.err("missing low surrogate"));
                                }
                                self.i += 2;
                                let lo = self.hex4()?;
                                if !(0xDC00..=0xDFFF).contains(&lo) {
                                    return Err(self.err("invalid low surrogate"));
                                }
                                cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                            } else if (0xDC00..=0xDFFF).contains(&cp) {
                                return Err(self.err("unpaired low surrogate"));
                            }
                            let ch = char::from_u32(cp)
                                .ok_or_else(|| self.err("invalid Unicode scalar"))?;
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                        _ => return Err(self.err("bad escape")),
                    }
                }
                Some(&c) => {
                    out.push(c);
                    self.i += 1;
                }
            }
        }
    }
    fn hex4(&mut self) -> Result<u32, String> {
        let h = std::str::from_utf8(self.s.get(self.i..self.i + 4).unwrap_or(b"")).unwrap_or("");
        self.i += 4;
        u32::from_str_radix(h, 16).map_err(|_| self.err("bad \\u escape"))
    }
}

/// A JSON document whose object members keep insertion order, so that output is deterministic
/// and follows the schema's declared field order.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Int(i64),
    /// A pre-rendered number (used for exact ratios).
    Number(String),
    Str(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    pub fn obj() -> Self {
        Json::Object(Vec::new())
    }
    pub fn with(mut self, key: &str, v: impl Into<Json>) -> Self {
        if let Json::Object(m) = &mut self {
            m.push((key.to_string(), v.into()));
        }
        self
    }
    pub fn render(&self) -> String {
        let mut s = String::new();
        self.write(&mut s, 0);
        s.push('\n');
        s
    }
    /// The compact one-line rendering (no whitespace, members in insertion order, no newline):
    /// the canonical form of one JSONL record.
    pub fn render_line(&self) -> String {
        let mut s = String::new();
        self.write_line(&mut s);
        s
    }
    fn write_line(&self, out: &mut String) {
        match self {
            Json::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write_line(out);
                }
                out.push(']');
            }
            Json::Object(members) => {
                out.push('{');
                for (i, (k, v)) in members.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    quote(k, out);
                    out.push(':');
                    v.write_line(out);
                }
                out.push('}');
            }
            other => other.write(out, 0),
        }
    }
    fn write(&self, out: &mut String, indent: usize) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Int(n) => out.push_str(&n.to_string()),
            Json::Number(n) => out.push_str(n),
            Json::Str(s) => quote(s, out),
            Json::Array(items) => {
                if items.is_empty() {
                    out.push_str("[]");
                    return;
                }
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push('\n');
                    out.push_str(&"  ".repeat(indent + 1));
                    item.write(out, indent + 1);
                }
                out.push('\n');
                out.push_str(&"  ".repeat(indent));
                out.push(']');
            }
            Json::Object(members) => {
                if members.is_empty() {
                    out.push_str("{}");
                    return;
                }
                out.push('{');
                for (i, (k, v)) in members.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push('\n');
                    out.push_str(&"  ".repeat(indent + 1));
                    quote(k, out);
                    out.push_str(": ");
                    v.write(out, indent + 1);
                }
                out.push('\n');
                out.push_str(&"  ".repeat(indent));
                out.push('}');
            }
        }
    }
}

fn quote(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

impl From<&str> for Json {
    fn from(s: &str) -> Self {
        Json::Str(s.to_string())
    }
}
impl From<String> for Json {
    fn from(s: String) -> Self {
        Json::Str(s)
    }
}
impl From<&String> for Json {
    fn from(s: &String) -> Self {
        Json::Str(s.clone())
    }
}
impl From<bool> for Json {
    fn from(b: bool) -> Self {
        Json::Bool(b)
    }
}
impl From<u64> for Json {
    fn from(n: u64) -> Self {
        Json::Int(n as i64)
    }
}
impl From<usize> for Json {
    fn from(n: usize) -> Self {
        Json::Int(n as i64)
    }
}
impl From<i64> for Json {
    fn from(n: i64) -> Self {
        Json::Int(n)
    }
}
impl<T: Into<Json>> From<Vec<T>> for Json {
    fn from(v: Vec<T>) -> Self {
        Json::Array(v.into_iter().map(Into::into).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_nesting_is_bounded_before_recursive_descent() {
        assert!(parse(&format!("{}0{}", "[".repeat(32), "]".repeat(32))).is_ok());
        assert!(
            parse(&format!("{}0{}", "[".repeat(10_000), "]".repeat(10_000)))
                .unwrap_err()
                .contains("nesting")
        );
    }

    #[test]
    fn unicode_surrogates_are_validated_without_overflow_or_replacement() {
        assert_eq!(
            parse(r#""\ud800\udc00""#).unwrap().as_str(),
            Some("\u{10000}")
        );
        assert_eq!(
            parse(r#""\udbff\udfff""#).unwrap().as_str(),
            Some("\u{10ffff}")
        );
        for input in [
            r#""\ud800\u0000""#,
            r#""\ud800\ud800""#,
            r#""\ud800""#,
            r#""\udc00""#,
        ] {
            assert!(parse(input).is_err(), "{input}");
        }
        assert!(crate::migration::source::Capture::parse(r#"{"format":"\ud800\u0000"}"#).is_err());
    }

    #[test]
    fn parse_and_render() {
        let v = parse(r#"{"a": [1, 2.5, "xé\n"], "b": {"c": null, "d": true}}"#).unwrap();
        assert_eq!(v.items_of("a").len(), 3);
        assert_eq!(v.items_of("a")[2].as_str(), Some("xé\n"));
        let j = Json::obj().with("z", 1u64).with("a", vec!["q\""]);
        assert_eq!(
            j.render(),
            "{\n  \"z\": 1,\n  \"a\": [\n    \"q\\\"\"\n  ]\n}\n"
        );
        let back = parse(&j.render()).unwrap();
        assert_eq!(back.items_of("a")[0].as_str(), Some("q\""));
    }
}
