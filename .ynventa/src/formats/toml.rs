//! A TOML reader sufficient for Cargo manifests, Cargo.lock and the legacy `.atlas` registries.
//! Ynventa never writes TOML: it is legacy input and third-party manifest format only.

use super::Value;
use std::collections::BTreeMap;

#[derive(Debug)]
pub struct TomlError {
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for TomlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

pub fn parse(text: &str) -> Result<Value, TomlError> {
    let mut p = Parser {
        s: text.as_bytes(),
        i: 0,
        line: 1,
    };
    let mut root = BTreeMap::new();
    // Path of the table currently receiving key/value pairs; `array` marks `[[...]]`.
    let mut current: Vec<String> = Vec::new();
    loop {
        p.skip_ws_nl_comments();
        if p.eof() {
            break;
        }
        if p.peek() == Some(b'[') {
            let array = p.s.get(p.i + 1) == Some(&b'[');
            p.i += if array { 2 } else { 1 };
            let path = p.key_path()?;
            p.skip_ws();
            p.expect(b']')?;
            if array {
                p.expect(b']')?;
            }
            p.end_of_line()?;
            if array {
                let parent = navigate(&mut root, &path[..path.len() - 1], p.line)?;
                let last = path.last().unwrap().clone();
                let entry = parent
                    .entry(last)
                    .or_insert_with(|| Value::Array(Vec::new()));
                match entry {
                    Value::Array(items) => items.push(Value::Table(BTreeMap::new())),
                    _ => return Err(p.err("array of tables conflicts with a value")),
                }
            } else {
                navigate(&mut root, &path, p.line)?;
            }
            current = path;
            continue;
        }
        let key = p.key_path()?;
        p.skip_ws();
        p.expect(b'=')?;
        p.skip_ws();
        let value = p.value()?;
        p.end_of_line()?;
        let line = p.line;
        let table = navigate(&mut root, &current, line)?;
        let table = navigate(table, &key[..key.len() - 1], line)?;
        table.insert(key.last().unwrap().clone(), value);
    }
    Ok(Value::Table(root))
}

/// Walks (creating as needed) to the table at `path`; an array of tables resolves to its last
/// element, as TOML specifies.
fn navigate<'a>(
    mut table: &'a mut BTreeMap<String, Value>,
    path: &[String],
    line: usize,
) -> Result<&'a mut BTreeMap<String, Value>, TomlError> {
    for part in path {
        let entry = table
            .entry(part.clone())
            .or_insert_with(|| Value::Table(BTreeMap::new()));
        table = match entry {
            Value::Table(t) => t,
            Value::Array(items) => match items.last_mut() {
                Some(Value::Table(t)) => t,
                _ => {
                    return Err(TomlError {
                        line,
                        message: format!("`{part}` is not a table"),
                    })
                }
            },
            _ => {
                return Err(TomlError {
                    line,
                    message: format!("`{part}` is not a table"),
                })
            }
        };
    }
    Ok(table)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    line: usize,
}

impl Parser<'_> {
    fn err(&self, message: &str) -> TomlError {
        TomlError {
            line: self.line,
            message: message.to_string(),
        }
    }
    fn eof(&self) -> bool {
        self.i >= self.s.len()
    }
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }
    fn expect(&mut self, c: u8) -> Result<(), TomlError> {
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            Err(self.err(&format!("expected `{}`", c as char)))
        }
    }
    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t')) {
            self.i += 1;
        }
    }
    fn skip_comment(&mut self) {
        if self.peek() == Some(b'#') {
            while !matches!(self.peek(), None | Some(b'\n')) {
                self.i += 1;
            }
        }
    }
    fn skip_ws_nl_comments(&mut self) {
        loop {
            self.skip_ws();
            self.skip_comment();
            match self.peek() {
                Some(b'\n') => {
                    self.line += 1;
                    self.i += 1;
                }
                Some(b'\r') => self.i += 1,
                _ => break,
            }
        }
    }
    fn end_of_line(&mut self) -> Result<(), TomlError> {
        self.skip_ws();
        self.skip_comment();
        match self.peek() {
            None => Ok(()),
            Some(b'\n') | Some(b'\r') => Ok(()),
            _ => Err(self.err("expected end of line")),
        }
    }
    fn key_path(&mut self) -> Result<Vec<String>, TomlError> {
        let mut parts = Vec::new();
        loop {
            self.skip_ws();
            let part = match self.peek() {
                Some(b'"') => self.basic_string()?,
                Some(b'\'') => self.literal_string()?,
                _ => {
                    let start = self.i;
                    while matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
                    {
                        self.i += 1;
                    }
                    if start == self.i {
                        return Err(self.err("expected a key"));
                    }
                    String::from_utf8_lossy(&self.s[start..self.i]).into_owned()
                }
            };
            parts.push(part);
            self.skip_ws();
            if self.peek() == Some(b'.') {
                self.i += 1;
            } else {
                return Ok(parts);
            }
        }
    }
    fn value(&mut self) -> Result<Value, TomlError> {
        match self.peek() {
            Some(b'"') => {
                if self.s[self.i..].starts_with(b"\"\"\"") {
                    self.multiline_basic().map(Value::Str)
                } else {
                    self.basic_string().map(Value::Str)
                }
            }
            Some(b'\'') => {
                if self.s[self.i..].starts_with(b"'''") {
                    self.multiline_literal().map(Value::Str)
                } else {
                    self.literal_string().map(Value::Str)
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_ws_nl_comments();
                    if self.peek() == Some(b']') {
                        self.i += 1;
                        return Ok(Value::Array(items));
                    }
                    items.push(self.value()?);
                    self.skip_ws_nl_comments();
                    match self.peek() {
                        Some(b',') => self.i += 1,
                        Some(b']') => {}
                        _ => return Err(self.err("expected `,` or `]` in array")),
                    }
                }
            }
            Some(b'{') => {
                self.i += 1;
                let mut table = BTreeMap::new();
                loop {
                    self.skip_ws();
                    if self.peek() == Some(b'}') {
                        self.i += 1;
                        return Ok(Value::Table(table));
                    }
                    let key = self.key_path()?;
                    self.skip_ws();
                    self.expect(b'=')?;
                    self.skip_ws();
                    let v = self.value()?;
                    let line = self.line;
                    let t = navigate(&mut table, &key[..key.len() - 1], line)?;
                    t.insert(key.last().unwrap().clone(), v);
                    self.skip_ws();
                    match self.peek() {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {}
                        _ => return Err(self.err("expected `,` or `}` in inline table")),
                    }
                }
            }
            Some(_) => self.scalar(),
            None => Err(self.err("expected a value")),
        }
    }
    fn scalar(&mut self) -> Result<Value, TomlError> {
        let start = self.i;
        while matches!(self.peek(), Some(c) if !matches!(c, b',' | b']' | b'}' | b'\n' | b'\r' | b'#'))
        {
            self.i += 1;
        }
        let raw = String::from_utf8_lossy(&self.s[start..self.i])
            .trim()
            .to_string();
        if raw == "true" {
            return Ok(Value::Bool(true));
        }
        if raw == "false" {
            return Ok(Value::Bool(false));
        }
        let digits: String = raw.chars().filter(|c| *c != '_').collect();
        if let Ok(n) = digits.parse::<i64>() {
            return Ok(Value::Int(n));
        }
        if let Some(hex) = digits.strip_prefix("0x") {
            if let Ok(n) = i64::from_str_radix(hex, 16) {
                return Ok(Value::Int(n));
            }
        }
        if digits.parse::<f64>().is_ok()
            || matches!(digits.as_str(), "inf" | "+inf" | "-inf" | "nan")
        {
            return Ok(Value::Float(digits));
        }
        if raw.is_empty() {
            return Err(self.err("expected a value"));
        }
        // Dates and times are kept verbatim.
        Ok(Value::Str(raw))
    }
    fn basic_string(&mut self) -> Result<String, TomlError> {
        self.expect(b'"')?;
        let mut out = Vec::new();
        loop {
            match self.peek() {
                None | Some(b'\n') => return Err(self.err("unterminated string")),
                Some(b'"') => {
                    self.i += 1;
                    return Ok(String::from_utf8_lossy(&out).into_owned());
                }
                Some(b'\\') => {
                    self.i += 1;
                    self.escape(&mut out)?;
                }
                Some(c) => {
                    out.push(c);
                    self.i += 1;
                }
            }
        }
    }
    fn escape(&mut self, out: &mut Vec<u8>) -> Result<(), TomlError> {
        let c = self.peek().ok_or_else(|| self.err("bad escape"))?;
        self.i += 1;
        match c {
            b'n' => out.push(b'\n'),
            b't' => out.push(b'\t'),
            b'r' => out.push(b'\r'),
            b'b' => out.push(8),
            b'f' => out.push(12),
            b'"' => out.push(b'"'),
            b'\\' => out.push(b'\\'),
            b'u' | b'U' => {
                let n = if c == b'u' { 4 } else { 8 };
                let hex = String::from_utf8_lossy(&self.s[self.i..(self.i + n).min(self.s.len())])
                    .into_owned();
                self.i += n;
                let ch = u32::from_str_radix(&hex, 16)
                    .ok()
                    .and_then(char::from_u32)
                    .ok_or_else(|| self.err("bad unicode escape"))?;
                let mut buf = [0u8; 4];
                out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
            }
            b'\n' | b' ' | b'\t' | b'\r' => {
                // Line-ending backslash in multi-line strings: trim following whitespace.
                if c == b'\n' {
                    self.line += 1;
                }
                while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
                    if self.peek() == Some(b'\n') {
                        self.line += 1;
                    }
                    self.i += 1;
                }
            }
            _ => return Err(self.err("bad escape")),
        }
        Ok(())
    }
    fn literal_string(&mut self) -> Result<String, TomlError> {
        self.expect(b'\'')?;
        let start = self.i;
        while !matches!(self.peek(), None | Some(b'\'') | Some(b'\n')) {
            self.i += 1;
        }
        let s = String::from_utf8_lossy(&self.s[start..self.i]).into_owned();
        self.expect(b'\'')?;
        Ok(s)
    }
    fn multiline_basic(&mut self) -> Result<String, TomlError> {
        self.i += 3;
        self.skip_first_newline();
        let mut out = Vec::new();
        loop {
            if self.s[self.i..].starts_with(b"\"\"\"") {
                self.i += 3;
                // Up to two further quotes belong to the content.
                while self.peek() == Some(b'"') {
                    out.push(b'"');
                    self.i += 1;
                }
                return Ok(String::from_utf8_lossy(&out).into_owned());
            }
            match self.peek() {
                None => return Err(self.err("unterminated multi-line string")),
                Some(b'\\') => {
                    self.i += 1;
                    self.escape(&mut out)?;
                }
                Some(c) => {
                    if c == b'\n' {
                        self.line += 1;
                    }
                    out.push(c);
                    self.i += 1;
                }
            }
        }
    }
    fn multiline_literal(&mut self) -> Result<String, TomlError> {
        self.i += 3;
        self.skip_first_newline();
        let start = self.i;
        loop {
            if self.s[self.i..].starts_with(b"'''") {
                let s = String::from_utf8_lossy(&self.s[start..self.i]).into_owned();
                self.i += 3;
                return Ok(s);
            }
            match self.peek() {
                None => return Err(self.err("unterminated multi-line string")),
                Some(b'\n') => {
                    self.line += 1;
                    self.i += 1;
                }
                _ => self.i += 1,
            }
        }
    }
    fn skip_first_newline(&mut self) {
        if self.s[self.i..].starts_with(b"\r\n") {
            self.i += 2;
            self.line += 1;
        } else if self.peek() == Some(b'\n') {
            self.i += 1;
            self.line += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_manifest_shapes() {
        let v = parse(
            r#"
# comment
[package]
name = "x" # trailing
version = "0.1.0"

[dependencies]
serde = { version = "1", features = ["derive"] }
core = { path = "../core" }
plain = "1.2"

[target.'cfg(unix)'.dependencies]
libc = "0.2"

[[bin]]
name = "a"
[[bin]]
name = "b"

[workspace]
members = [
    "core", # the kernel
    "tools/census",
]
"#,
        )
        .unwrap();
        assert_eq!(v.get("package").and_then(|p| p.str("name")), Some("x"));
        let deps = v.get("dependencies").unwrap();
        assert_eq!(
            deps.get("core").and_then(|c| c.str("path")),
            Some("../core")
        );
        assert_eq!(deps.str("plain"), Some("1.2"));
        assert_eq!(
            v.get("target")
                .and_then(|t| t.get("cfg(unix)"))
                .and_then(|t| t.get("dependencies"))
                .and_then(|d| d.str("libc")),
            Some("0.2")
        );
        assert_eq!(v.get("bin").unwrap().items().len(), 2);
        assert_eq!(
            v.get("workspace").unwrap().strings("members"),
            vec!["core".to_string(), "tools/census".to_string()]
        );
    }

    #[test]
    fn strings_and_scalars() {
        let v = parse(
            "a = \"\"\"\nline1\nline2\"\"\"\nb = 'lit\\n'\nc = 1_000\nd = 2026-09-30\ne = 1.5\nf = \"\\u00e9\"\n",
        )
        .unwrap();
        assert_eq!(v.str("a"), Some("line1\nline2"));
        assert_eq!(v.str("b"), Some("lit\\n"));
        assert_eq!(v.get("c"), Some(&Value::Int(1000)));
        assert_eq!(v.str("d"), Some("2026-09-30"));
        assert_eq!(v.get("e"), Some(&Value::Float("1.5".into())));
        assert_eq!(v.str("f"), Some("é"));
    }
}
