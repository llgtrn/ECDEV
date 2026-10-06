//! The authoritative declaration format: each `.ecdev/declared/*.rs` file is ONE Rust constant
//! expression (struct literals, enum paths, tuple variants, string/integer/bool literals and
//! `&[...]` slices). The same bytes are type-checked by rustc (`include!` in the subsystem's
//! build) and read at run time by this module, so that the governance verifier can read any
//! repository's declarations without compiling them and without repository-specific code.

use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub line: usize,
    pub kind: Kind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// `Name { field: expr, ... }`
    Struct(String, Vec<(String, Expr)>),
    /// `A::B` or a bare identifier such as `None`.
    Path(Vec<String>),
    /// `A::B(expr, ...)` or `Some(expr)`.
    Call(Vec<String>, Vec<Expr>),
    Str(String),
    Int(i64),
    Bool(bool),
    /// `&[...]` (the `&` is optional on input and always written on output).
    List(Vec<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

pub fn parse(text: &str) -> Result<Expr, ParseError> {
    let mut p = P {
        s: text.as_bytes(),
        i: 0,
        line: 1,
    };
    p.trivia()?;
    let e = p.expr()?;
    p.trivia()?;
    if p.i < p.s.len() {
        return Err(p.err("trailing input after the declaration expression"));
    }
    Ok(e)
}

struct P<'a> {
    s: &'a [u8],
    i: usize,
    line: usize,
}

impl P<'_> {
    fn err(&self, m: &str) -> ParseError {
        ParseError {
            line: self.line,
            message: m.to_string(),
        }
    }
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }
    fn trivia(&mut self) -> Result<(), ParseError> {
        loop {
            match self.peek() {
                Some(b'\n') => {
                    self.line += 1;
                    self.i += 1;
                }
                Some(b' ' | b'\t' | b'\r') => self.i += 1,
                Some(b'/') if self.s.get(self.i + 1) == Some(&b'/') => {
                    while !matches!(self.peek(), None | Some(b'\n')) {
                        self.i += 1;
                    }
                }
                Some(b'/') if self.s.get(self.i + 1) == Some(&b'*') => {
                    self.i += 2;
                    loop {
                        match self.peek() {
                            None => return Err(self.err("unterminated block comment")),
                            Some(b'*') if self.s.get(self.i + 1) == Some(&b'/') => {
                                self.i += 2;
                                break;
                            }
                            Some(b'\n') => {
                                self.line += 1;
                                self.i += 1;
                            }
                            _ => self.i += 1,
                        }
                    }
                }
                _ => return Ok(()),
            }
        }
    }
    fn eat(&mut self, c: u8) -> Result<(), ParseError> {
        self.trivia()?;
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            Err(self.err(&format!("expected `{}`", c as char)))
        }
    }
    fn ident(&mut self) -> Result<String, ParseError> {
        self.trivia()?;
        let start = self.i;
        while matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || c == b'_') {
            self.i += 1;
        }
        if start == self.i || self.s[start].is_ascii_digit() {
            return Err(self.err("expected an identifier"));
        }
        Ok(String::from_utf8_lossy(&self.s[start..self.i]).into_owned())
    }
    fn expr(&mut self) -> Result<Expr, ParseError> {
        self.trivia()?;
        let line = self.line;
        let kind = match self.peek() {
            Some(b'&') => {
                self.i += 1;
                self.trivia()?;
                if self.peek() != Some(b'[') {
                    return Err(self.err("expected `[` after `&`"));
                }
                self.list()?
            }
            Some(b'[') => self.list()?,
            Some(b'"') => Kind::Str(self.string()?),
            Some(b'r') if matches!(self.s.get(self.i + 1), Some(b'"' | b'#')) => {
                Kind::Str(self.raw_string()?)
            }
            Some(c) if c.is_ascii_digit() || c == b'-' => {
                let start = self.i;
                self.i += 1;
                while matches!(self.peek(), Some(c) if c.is_ascii_digit() || c == b'_') {
                    self.i += 1;
                }
                let digits: String = String::from_utf8_lossy(&self.s[start..self.i])
                    .chars()
                    .filter(|c| *c != '_')
                    .collect();
                Kind::Int(digits.parse().map_err(|_| self.err("bad integer"))?)
            }
            Some(c) if c.is_ascii_alphabetic() || c == b'_' => {
                let mut path = vec![self.ident()?];
                loop {
                    self.trivia()?;
                    if self.s[self.i..].starts_with(b"::") {
                        self.i += 2;
                        path.push(self.ident()?);
                    } else {
                        break;
                    }
                }
                self.trivia()?;
                match self.peek() {
                    Some(b'{') if path.len() == 1 => {
                        self.i += 1;
                        let mut fields = Vec::new();
                        loop {
                            self.trivia()?;
                            if self.peek() == Some(b'}') {
                                self.i += 1;
                                break;
                            }
                            let name = self.ident()?;
                            self.eat(b':')?;
                            let value = self.expr()?;
                            if fields.iter().any(|(n, _): &(String, Expr)| *n == name) {
                                return Err(self.err(&format!("duplicate field `{name}`")));
                            }
                            fields.push((name, value));
                            self.trivia()?;
                            match self.peek() {
                                Some(b',') => self.i += 1,
                                Some(b'}') => {}
                                _ => return Err(self.err("expected `,` or `}`")),
                            }
                        }
                        Kind::Struct(path.pop().unwrap(), fields)
                    }
                    Some(b'(') => {
                        self.i += 1;
                        let mut args = Vec::new();
                        loop {
                            self.trivia()?;
                            if self.peek() == Some(b')') {
                                self.i += 1;
                                break;
                            }
                            args.push(self.expr()?);
                            self.trivia()?;
                            match self.peek() {
                                Some(b',') => self.i += 1,
                                Some(b')') => {}
                                _ => return Err(self.err("expected `,` or `)`")),
                            }
                        }
                        Kind::Call(path, args)
                    }
                    _ => match path.as_slice() {
                        [w] if w == "true" => Kind::Bool(true),
                        [w] if w == "false" => Kind::Bool(false),
                        _ => Kind::Path(path),
                    },
                }
            }
            _ => return Err(self.err("expected an expression")),
        };
        Ok(Expr { line, kind })
    }
    fn list(&mut self) -> Result<Kind, ParseError> {
        self.eat(b'[')?;
        let mut items = Vec::new();
        loop {
            self.trivia()?;
            if self.peek() == Some(b']') {
                self.i += 1;
                return Ok(Kind::List(items));
            }
            items.push(self.expr()?);
            self.trivia()?;
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b']') => {}
                _ => return Err(self.err("expected `,` or `]`")),
            }
        }
    }
    fn string(&mut self) -> Result<String, ParseError> {
        self.i += 1;
        let mut out = String::new();
        let mut buf: Vec<u8> = Vec::new();
        loop {
            match self.peek() {
                None => return Err(self.err("unterminated string")),
                Some(b'"') => {
                    self.i += 1;
                    out.push_str(&String::from_utf8_lossy(&buf));
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.i += 1;
                    let c = self.peek().ok_or_else(|| self.err("bad escape"))?;
                    self.i += 1;
                    match c {
                        b'n' => buf.push(b'\n'),
                        b't' => buf.push(b'\t'),
                        b'r' => buf.push(b'\r'),
                        b'0' => buf.push(0),
                        b'\\' => buf.push(b'\\'),
                        b'"' => buf.push(b'"'),
                        b'\'' => buf.push(b'\''),
                        b'\n' => {
                            self.line += 1;
                            while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
                                if self.peek() == Some(b'\n') {
                                    self.line += 1;
                                }
                                self.i += 1;
                            }
                        }
                        b'u' => {
                            self.eat(b'{')?;
                            let start = self.i;
                            while matches!(self.peek(), Some(c) if c.is_ascii_hexdigit()) {
                                self.i += 1;
                            }
                            let hex = String::from_utf8_lossy(&self.s[start..self.i]).into_owned();
                            self.eat(b'}')?;
                            let ch = u32::from_str_radix(&hex, 16)
                                .ok()
                                .and_then(char::from_u32)
                                .ok_or_else(|| self.err("bad unicode escape"))?;
                            let mut b = [0u8; 4];
                            buf.extend_from_slice(ch.encode_utf8(&mut b).as_bytes());
                        }
                        _ => return Err(self.err("unsupported escape")),
                    }
                }
                Some(c) => {
                    if c == b'\n' {
                        self.line += 1;
                    }
                    buf.push(c);
                    self.i += 1;
                }
            }
        }
    }
    fn raw_string(&mut self) -> Result<String, ParseError> {
        self.i += 1;
        let mut hashes = 0;
        while self.peek() == Some(b'#') {
            hashes += 1;
            self.i += 1;
        }
        if self.peek() != Some(b'"') {
            return Err(self.err("bad raw string"));
        }
        self.i += 1;
        let start = self.i;
        let mut close = vec![b'"'];
        close.extend(std::iter::repeat_n(b'#', hashes));
        loop {
            if self.i >= self.s.len() {
                return Err(self.err("unterminated raw string"));
            }
            if self.s[self.i..].starts_with(&close) {
                let s = String::from_utf8_lossy(&self.s[start..self.i]).into_owned();
                self.i += close.len();
                return Ok(s);
            }
            if self.s[self.i] == b'\n' {
                self.line += 1;
            }
            self.i += 1;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Construction helpers and canonical rendering.

pub fn st(name: &str, fields: Vec<(&str, Expr)>) -> Expr {
    e(Kind::Struct(
        name.to_string(),
        fields
            .into_iter()
            .map(|(n, v)| (n.to_string(), v))
            .collect(),
    ))
}
pub fn path(p: &str) -> Expr {
    e(Kind::Path(p.split("::").map(str::to_string).collect()))
}
pub fn call(p: &str, args: Vec<Expr>) -> Expr {
    e(Kind::Call(
        p.split("::").map(str::to_string).collect(),
        args,
    ))
}
pub fn s(v: &str) -> Expr {
    e(Kind::Str(v.to_string()))
}
pub fn int(v: i64) -> Expr {
    e(Kind::Int(v))
}
pub fn boolean(v: bool) -> Expr {
    e(Kind::Bool(v))
}
pub fn list(items: Vec<Expr>) -> Expr {
    e(Kind::List(items))
}
pub fn strs(items: &[String]) -> Expr {
    list(items.iter().map(|v| s(v)).collect())
}
pub fn opt(v: &Option<String>) -> Expr {
    match v {
        Some(v) => call("Some", vec![s(v)]),
        None => path("None"),
    }
}
fn e(kind: Kind) -> Expr {
    Expr { line: 0, kind }
}

/// Renders an expression canonically: deterministic, rustfmt-compatible layout.
pub fn render(expr: &Expr) -> String {
    let mut out = String::new();
    write_expr(expr, 0, &mut out);
    out.push('\n');
    out
}

fn quote(v: &str) -> String {
    let mut q = String::from("\"");
    for c in v.chars() {
        match c {
            '"' => q.push_str("\\\""),
            '\\' => q.push_str("\\\\"),
            '\n' => q.push_str("\\n"),
            '\t' => q.push_str("\\t"),
            '\r' => q.push_str("\\r"),
            '\0' => q.push_str("\\0"),
            c if (c as u32) < 0x20 => {
                let _ = write!(q, "\\u{{{:x}}}", c as u32);
            }
            c => q.push(c),
        }
    }
    q.push('"');
    q
}

/// Single-line rendering when the expression has no struct inside it.
fn inline(expr: &Expr) -> Option<String> {
    Some(match &expr.kind {
        Kind::Struct(..) => return None,
        Kind::Path(p) => p.join("::"),
        Kind::Call(p, args) => {
            let parts: Option<Vec<String>> = args.iter().map(inline).collect();
            format!("{}({})", p.join("::"), parts?.join(", "))
        }
        Kind::Str(v) => quote(v),
        Kind::Int(n) => n.to_string(),
        Kind::Bool(b) => b.to_string(),
        Kind::List(items) => {
            let parts: Option<Vec<String>> = items.iter().map(inline).collect();
            format!("&[{}]", parts?.join(", "))
        }
    })
}

const WIDTH: usize = 100;

fn write_expr(expr: &Expr, indent: usize, out: &mut String) {
    let pad = "    ".repeat(indent + 1);
    let close = "    ".repeat(indent);
    match &expr.kind {
        Kind::Struct(name, fields) => {
            out.push_str(name);
            out.push_str(" {\n");
            for (n, v) in fields {
                out.push_str(&pad);
                out.push_str(n);
                out.push_str(": ");
                let col = pad.len() + n.len() + 2;
                match inline(v) {
                    Some(line) if col + line.len() < WIDTH => out.push_str(&line),
                    _ => write_expr(v, indent + 1, out),
                }
                out.push_str(",\n");
            }
            out.push_str(&close);
            out.push('}');
        }
        Kind::List(items) => {
            if items.is_empty() {
                out.push_str("&[]");
                return;
            }
            out.push_str("&[\n");
            for item in items {
                out.push_str(&pad);
                match inline(item) {
                    Some(line) if pad.len() + line.len() < WIDTH => out.push_str(&line),
                    _ => write_expr(item, indent + 1, out),
                }
                out.push_str(",\n");
            }
            out.push_str(&close);
            out.push(']');
        }
        Kind::Call(p, args) => match inline(expr) {
            Some(line) => out.push_str(&line),
            None => {
                out.push_str(&p.join("::"));
                out.push_str("(\n");
                for a in args {
                    out.push_str(&pad);
                    write_expr(a, indent + 1, out);
                    out.push_str(",\n");
                }
                out.push_str(&close);
                out.push(')');
            }
        },
        _ => out.push_str(&inline(expr).unwrap_or_default()),
    }
}

/// Structural equality ignoring source lines.
pub fn same(a: &Expr, b: &Expr) -> bool {
    match (&a.kind, &b.kind) {
        (Kind::Struct(n1, f1), Kind::Struct(n2, f2)) => {
            n1 == n2
                && f1.len() == f2.len()
                && f1
                    .iter()
                    .zip(f2)
                    .all(|((x, v), (y, w))| x == y && same(v, w))
        }
        (Kind::Call(p1, a1), Kind::Call(p2, a2)) => {
            p1 == p2 && a1.len() == a2.len() && a1.iter().zip(a2).all(|(x, y)| same(x, y))
        }
        (Kind::List(a1), Kind::List(a2)) => {
            a1.len() == a2.len() && a1.iter().zip(a2).all(|(x, y)| same(x, y))
        }
        (x, y) => x == y,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let src = r##"// header comment
Repository {
    namespace: "demo", /* inline */
    count: 3,
    flag: true,
    kind: NodeKind::Kernel,
    maybe: None,
    other: Some("x"),
    blocked: Exception::Blocked("needs \"quotes\"\n"),
    raw: r#"a"b"#,
    items: &[Node { key: "a", tags: &["x", "y"] }, Node { key: "b", tags: [] }],
}"##;
        let e = parse(src).unwrap();
        let text = render(&e);
        let again = parse(&text).unwrap();
        assert!(same(&e, &again), "{text}");
        assert_eq!(render(&again), text);
        match &e.kind {
            Kind::Struct(name, fields) => {
                assert_eq!(name, "Repository");
                assert_eq!(
                    fields[6].1.kind,
                    Kind::Call(
                        vec!["Exception".into(), "Blocked".into()],
                        vec![Expr {
                            line: 9,
                            kind: Kind::Str("needs \"quotes\"\n".into())
                        }]
                    )
                );
                assert_eq!(fields[7].1.kind, Kind::Str("a\"b".into()));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn errors_carry_lines() {
        let err = parse("A {\n  x: 1,\n  x: 2,\n}").unwrap_err();
        assert_eq!(err.line, 3);
        assert!(parse("A { x: }").is_err());
        assert!(parse("A {} B {}").is_err());
    }
}
