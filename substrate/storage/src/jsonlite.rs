//! `json_extract(document, path)`: a strict JSON reader and the path forms the queries use
//! (`$`, `.key`, `."quoted key"`, `[n]`, `[#-n]`).

use crate::{
    error::{Error, ErrorKind, Result},
    value::Value,
};

enum J {
    Null,
    Bool(bool),
    Num(String),
    Str(String, String),
    Arr(Vec<J>),
    Obj(Vec<(String, String, J)>),
}

struct P<'a> {
    b: &'a [u8],
    i: usize,
    depth: usize,
}

fn bad() -> Error {
    Error::new(ErrorKind::Syntax, "malformed JSON")
}

impl P<'_> {
    fn ws(&mut self) {
        while matches!(self.b.get(self.i), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }
    fn value(&mut self) -> Result<J> {
        self.ws();
        match self.b.get(self.i).copied().ok_or_else(bad)? {
            b'{' => {
                self.depth += 1;
                if self.depth > 1000 {
                    return Err(bad());
                }
                self.i += 1;
                let mut members = Vec::new();
                self.ws();
                if self.b.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    self.depth -= 1;
                    return Ok(J::Obj(members));
                }
                loop {
                    self.ws();
                    if self.b.get(self.i) != Some(&b'"') {
                        return Err(bad());
                    }
                    let (k, raw_k) = self.string()?;
                    self.ws();
                    if self.b.get(self.i) != Some(&b':') {
                        return Err(bad());
                    }
                    self.i += 1;
                    members.push((k, raw_k, self.value()?));
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            self.depth -= 1;
                            return Ok(J::Obj(members));
                        }
                        _ => return Err(bad()),
                    }
                }
            }
            b'[' => {
                self.depth += 1;
                if self.depth > 1000 {
                    return Err(bad());
                }
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.b.get(self.i) == Some(&b']') {
                    self.i += 1;
                    self.depth -= 1;
                    return Ok(J::Arr(items));
                }
                loop {
                    items.push(self.value()?);
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            self.depth -= 1;
                            return Ok(J::Arr(items));
                        }
                        _ => return Err(bad()),
                    }
                }
            }
            b'"' => {
                let (v, raw) = self.string()?;
                Ok(J::Str(v, raw))
            }
            b't' => self.word("true", J::Bool(true)),
            b'f' => self.word("false", J::Bool(false)),
            b'n' => self.word("null", J::Null),
            b'-' | b'0'..=b'9' => {
                let start = self.i;
                if self.b[self.i] == b'-' {
                    self.i += 1;
                }
                let digits = self.i;
                while self.b.get(self.i).is_some_and(u8::is_ascii_digit) {
                    self.i += 1;
                }
                if self.i == digits || (self.b[digits] == b'0' && self.i - digits > 1) {
                    return Err(bad());
                }
                if self.b.get(self.i) == Some(&b'.') {
                    self.i += 1;
                    let f = self.i;
                    while self.b.get(self.i).is_some_and(u8::is_ascii_digit) {
                        self.i += 1;
                    }
                    if self.i == f {
                        return Err(bad());
                    }
                }
                if matches!(self.b.get(self.i), Some(b'e' | b'E')) {
                    self.i += 1;
                    if matches!(self.b.get(self.i), Some(b'+' | b'-')) {
                        self.i += 1;
                    }
                    let f = self.i;
                    while self.b.get(self.i).is_some_and(u8::is_ascii_digit) {
                        self.i += 1;
                    }
                    if self.i == f {
                        return Err(bad());
                    }
                }
                Ok(J::Num(
                    String::from_utf8_lossy(&self.b[start..self.i]).into_owned(),
                ))
            }
            _ => Err(bad()),
        }
    }
    fn word(&mut self, w: &str, v: J) -> Result<J> {
        if self.b[self.i..].starts_with(w.as_bytes()) {
            self.i += w.len();
            Ok(v)
        } else {
            Err(bad())
        }
    }
    fn string(&mut self) -> Result<(String, String)> {
        let start = self.i;
        self.i += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let c = *self.b.get(self.i).ok_or_else(bad)?;
            self.i += 1;
            match c {
                b'"' => {
                    let v = String::from_utf8(out).map_err(|_| bad())?;
                    return Ok((
                        v,
                        String::from_utf8_lossy(&self.b[start..self.i]).into_owned(),
                    ));
                }
                b'\\' => {
                    let e = *self.b.get(self.i).ok_or_else(bad)?;
                    self.i += 1;
                    match e {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'u' => {
                            let hex = |s: &mut Self| -> Result<u32> {
                                let h = s.b.get(s.i..s.i + 4).ok_or_else(bad)?;
                                s.i += 4;
                                u32::from_str_radix(std::str::from_utf8(h).map_err(|_| bad())?, 16)
                                    .map_err(|_| bad())
                            };
                            let first = hex(self)?;
                            let code = if (0xD800..0xDC00).contains(&first) {
                                if self.b.get(self.i..self.i + 2) != Some(b"\\u") {
                                    return Err(bad());
                                }
                                self.i += 2;
                                let second = hex(self)?;
                                if !(0xDC00..0xE000).contains(&second) {
                                    return Err(bad());
                                }
                                0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00)
                            } else {
                                first
                            };
                            let ch = char::from_u32(code).ok_or_else(bad)?;
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                        _ => return Err(bad()),
                    }
                }
                c if c < 0x20 => return Err(bad()),
                c => out.push(c),
            }
        }
    }
}

fn render(j: &J, out: &mut String) {
    match j {
        J::Null => out.push_str("null"),
        J::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        J::Num(n) => out.push_str(n),
        J::Str(_, raw) => out.push_str(raw),
        J::Arr(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                render(x, out);
            }
            out.push(']');
        }
        J::Obj(o) => {
            out.push('{');
            for (i, (_, raw_k, v)) in o.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(raw_k);
                out.push(':');
                render(v, out);
            }
            out.push('}');
        }
    }
}

enum Step {
    Key(String),
    Index(usize),
    FromEnd(usize),
}

fn path_steps(p: &str) -> Result<Vec<Step>> {
    let b = p.as_bytes();
    if b.first() != Some(&b'$') {
        return Err(Error::new(
            ErrorKind::Syntax,
            format!("JSON path error near '{p}'"),
        ));
    }
    let mut i = 1;
    let mut steps = Vec::new();
    while i < b.len() {
        match b[i] {
            b'.' => {
                i += 1;
                if b.get(i) == Some(&b'"') {
                    let end = p[i + 1..]
                        .find('"')
                        .ok_or_else(|| Error::syntax("JSON path error"))?
                        + i
                        + 1;
                    steps.push(Step::Key(p[i + 1..end].to_string()));
                    i = end + 1;
                } else {
                    let start = i;
                    while i < b.len() && b[i] != b'.' && b[i] != b'[' {
                        i += 1;
                    }
                    if i == start {
                        return Err(Error::syntax("JSON path error"));
                    }
                    steps.push(Step::Key(p[start..i].to_string()));
                }
            }
            b'[' => {
                let end = p[i..]
                    .find(']')
                    .ok_or_else(|| Error::syntax("JSON path error"))?
                    + i;
                let inner = &p[i + 1..end];
                if let Some(rest) = inner.strip_prefix('#') {
                    let n: usize = if rest.is_empty() {
                        0
                    } else {
                        rest.strip_prefix('-')
                            .and_then(|d| d.parse().ok())
                            .ok_or_else(|| Error::syntax("JSON path error"))?
                    };
                    steps.push(Step::FromEnd(n));
                } else {
                    steps.push(Step::Index(
                        inner
                            .parse()
                            .map_err(|_| Error::syntax("JSON path error"))?,
                    ));
                }
                i = end + 1;
            }
            _ => return Err(Error::syntax("JSON path error")),
        }
    }
    Ok(steps)
}

pub fn json_extract(args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        return Err(Error::syntax("json_extract needs a document and a path"));
    }
    let (Some(doc), Some(path)) = (args[0].to_text(), args[1].to_text()) else {
        return Ok(Value::Null);
    };
    let mut p = P {
        b: doc.as_bytes(),
        i: 0,
        depth: 0,
    };
    let root = p.value()?;
    p.ws();
    if p.i != p.b.len() {
        return Err(bad());
    }
    let mut cur = &root;
    for step in path_steps(&path)? {
        cur = match (step, cur) {
            (Step::Key(k), J::Obj(members)) => match members.iter().find(|(n, _, _)| *n == k) {
                Some((_, _, v)) => v,
                None => return Ok(Value::Null),
            },
            (Step::Index(n), J::Arr(a)) => match a.get(n) {
                Some(v) => v,
                None => return Ok(Value::Null),
            },
            (Step::FromEnd(n), J::Arr(a)) => match a.len().checked_sub(n).and_then(|i| a.get(i)) {
                Some(v) => v,
                None => return Ok(Value::Null),
            },
            _ => return Ok(Value::Null),
        };
    }
    Ok(match cur {
        J::Null => Value::Null,
        J::Bool(b) => Value::Integer(i64::from(*b)),
        J::Num(n) => match n.parse::<i64>() {
            Ok(i) => Value::Integer(i),
            Err(_) => Value::Real(n.parse::<f64>().unwrap_or(0.0)),
        },
        J::Str(v, _) => Value::Text(v.clone()),
        other => {
            let mut s = String::new();
            render(other, &mut s);
            Value::Text(s)
        }
    })
}
