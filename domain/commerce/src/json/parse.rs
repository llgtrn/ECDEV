//! A strict JSON parser (RFC 8259): no comments, no trailing commas, no leading zeros, no lone
//! surrogates, no control characters in strings, no bytes outside UTF-8. Numbers keep their text.
//! Nesting is limited so a hostile document cannot exhaust the stack.

use super::{
    error::{Error, Result},
    number::Number,
    value::{Map, Value},
};

/// Containers may nest at most this deep (127 arrays or objects inside each other).
pub const MAX_DEPTH: usize = 127;

pub fn parse(input: &[u8]) -> Result<Value> {
    let mut p = Parser { s: input, i: 0 };
    p.skip_ws();
    let v = p.value(0)?;
    p.skip_ws();
    if p.i < p.s.len() {
        return Err(p.syntax("trailing characters"));
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn position(&self, at: usize) -> (usize, usize) {
        let upto = &self.s[..at.min(self.s.len())];
        let line = upto.iter().filter(|&&b| b == b'\n').count() + 1;
        let column = upto.iter().rev().take_while(|&&b| b != b'\n').count();
        (line, column)
    }

    fn syntax(&self, message: &str) -> Error {
        let (line, column) = self.position(self.i + 1);
        Error::syntax(message, line, column)
    }

    fn eof(&self, message: &str) -> Error {
        let (line, column) = self.position(self.s.len());
        Error::eof(message, line, column)
    }

    fn skip_ws(&mut self) {
        while matches!(self.s.get(self.i), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn value(&mut self, depth: usize) -> Result<Value> {
        match self.peek() {
            None => Err(self.eof("EOF while parsing a value")),
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => Ok(Value::String(self.string()?)),
            Some(b't') => self.literal(b"true", Value::Bool(true)),
            Some(b'f') => self.literal(b"false", Value::Bool(false)),
            Some(b'n') => self.literal(b"null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.syntax("expected value")),
        }
    }

    fn literal(&mut self, word: &[u8], value: Value) -> Result<Value> {
        for (k, &expected) in word.iter().enumerate() {
            match self.s.get(self.i + k) {
                Some(&b) if b == expected => {}
                Some(_) => {
                    self.i += k;
                    return Err(self.syntax("expected ident"));
                }
                None => {
                    self.i = self.s.len();
                    return Err(self.eof("EOF while parsing a value"));
                }
            }
        }
        self.i += word.len();
        Ok(value)
    }

    /// Numbers keep every digit. Their spelling is normalised the one way the document model
    /// has always had it: an exponent is written `e` with an explicit sign (`1E5` is `1e+5`) and
    /// the integer `-0` is `0`; `-0.0` and `-0e0` keep their sign.
    fn number(&mut self) -> Result<Value> {
        let start = self.i;
        if self.peek() == Some(b'-') {
            self.i += 1;
        }
        match self.peek() {
            Some(b'0') => {
                self.i += 1;
                if matches!(self.peek(), Some(b'0'..=b'9')) {
                    return Err(self.syntax("invalid number"));
                }
            }
            Some(b'1'..=b'9') => {
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.i += 1;
                }
            }
            None => return Err(self.eof("EOF while parsing a value")),
            Some(_) => return Err(self.syntax("invalid number")),
        }
        if self.peek() == Some(b'.') {
            self.i += 1;
            match self.peek() {
                Some(b'0'..=b'9') => {}
                None => return Err(self.eof("EOF while parsing a value")),
                Some(_) => return Err(self.syntax("invalid number")),
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.i += 1;
            }
        }
        let mantissa_end = self.i;
        let mut exponent: Option<(bool, usize, usize)> = None;
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.i += 1;
            let negative = self.peek() == Some(b'-');
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.i += 1;
            }
            let digits_start = self.i;
            match self.peek() {
                Some(b'0'..=b'9') => {}
                None => return Err(self.eof("EOF while parsing a value")),
                Some(_) => return Err(self.syntax("invalid number")),
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.i += 1;
            }
            exponent = Some((negative, digits_start, self.i));
        }
        let mantissa =
            std::str::from_utf8(&self.s[start..mantissa_end]).expect("number bytes are ASCII");
        let text = match exponent {
            Some((negative, from, to)) => {
                let digits =
                    std::str::from_utf8(&self.s[from..to]).expect("number bytes are ASCII");
                format!("{mantissa}e{}{digits}", if negative { '-' } else { '+' })
            }
            None if mantissa == "-0" => "0".to_string(),
            None => mantissa.to_string(),
        };
        Ok(Value::Number(Number::from_valid_text(&text)))
    }

    fn hex4(&mut self) -> Result<u32> {
        let mut n = 0u32;
        for _ in 0..4 {
            let Some(b) = self.peek() else {
                return Err(self.eof("EOF while parsing a string"));
            };
            let d = match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'f' => b - b'a' + 10,
                b'A'..=b'F' => b - b'A' + 10,
                _ => return Err(self.syntax("invalid escape")),
            };
            n = n * 16 + u32::from(d);
            self.i += 1;
        }
        Ok(n)
    }

    fn string(&mut self) -> Result<String> {
        self.i += 1; // opening quote
        let mut buf: Vec<u8> = Vec::new();
        loop {
            let run_start = self.i;
            while let Some(&b) = self.s.get(self.i) {
                if b == b'"' || b == b'\\' || b < 0x20 {
                    break;
                }
                self.i += 1;
            }
            buf.extend_from_slice(&self.s[run_start..self.i]);
            match self.peek() {
                None => return Err(self.eof("EOF while parsing a string")),
                Some(b'"') => {
                    self.i += 1;
                    return String::from_utf8(buf)
                        .map_err(|_| self.syntax("invalid unicode code point"));
                }
                Some(b'\\') => {
                    self.i += 1;
                    let Some(e) = self.peek() else {
                        return Err(self.eof("EOF while parsing a string"));
                    };
                    self.i += 1;
                    match e {
                        b'"' => buf.push(b'"'),
                        b'\\' => buf.push(b'\\'),
                        b'/' => buf.push(b'/'),
                        b'b' => buf.push(0x08),
                        b'f' => buf.push(0x0c),
                        b'n' => buf.push(b'\n'),
                        b'r' => buf.push(b'\r'),
                        b't' => buf.push(b'\t'),
                        b'u' => {
                            let first = self.hex4()?;
                            let code = match first {
                                0xD800..=0xDBFF => {
                                    if self.s.get(self.i) == Some(&b'\\')
                                        && self.s.get(self.i + 1) == Some(&b'u')
                                    {
                                        self.i += 2;
                                        let second = self.hex4()?;
                                        if !(0xDC00..=0xDFFF).contains(&second) {
                                            return Err(
                                                self.syntax("lone leading surrogate in hex escape")
                                            );
                                        }
                                        0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00)
                                    } else if self.i >= self.s.len() {
                                        return Err(self.eof("EOF while parsing a string"));
                                    } else {
                                        return Err(self.syntax("unexpected end of hex escape"));
                                    }
                                }
                                0xDC00..=0xDFFF => {
                                    return Err(self.syntax("lone leading surrogate in hex escape"));
                                }
                                other => other,
                            };
                            let c = char::from_u32(code).expect("surrogates were handled");
                            let mut utf8 = [0u8; 4];
                            buf.extend_from_slice(c.encode_utf8(&mut utf8).as_bytes());
                        }
                        _ => {
                            self.i -= 1;
                            return Err(self.syntax("invalid escape"));
                        }
                    }
                }
                Some(_) => {
                    return Err(self.syntax(
                        "control character (\\u0000-\\u001F) found while parsing a string",
                    ));
                }
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Value> {
        if depth >= MAX_DEPTH {
            return Err(self.syntax("recursion limit exceeded"));
        }
        self.i += 1;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.i += 1;
            return Ok(Value::Array(items));
        }
        loop {
            self.skip_ws();
            items.push(self.value(depth + 1)?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.i += 1;
                    self.skip_ws();
                    if self.peek() == Some(b']') {
                        return Err(self.syntax("trailing comma"));
                    }
                }
                Some(b']') => {
                    self.i += 1;
                    return Ok(Value::Array(items));
                }
                None => return Err(self.eof("EOF while parsing a list")),
                Some(_) => return Err(self.syntax("expected `,` or `]`")),
            }
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value> {
        if depth >= MAX_DEPTH {
            return Err(self.syntax("recursion limit exceeded"));
        }
        self.i += 1;
        let mut members = Map::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.i += 1;
            return Ok(Value::Object(members));
        }
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'"') => {}
                None => return Err(self.eof("EOF while parsing an object")),
                Some(_) => return Err(self.syntax("key must be a string")),
            }
            let key = self.string()?;
            self.skip_ws();
            match self.peek() {
                Some(b':') => self.i += 1,
                None => return Err(self.eof("EOF while parsing an object")),
                Some(_) => return Err(self.syntax("expected `:`")),
            }
            self.skip_ws();
            let value = self.value(depth + 1)?;
            members.insert(key, value);
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.i += 1;
                    self.skip_ws();
                    if self.peek() == Some(b'}') {
                        return Err(self.syntax("trailing comma"));
                    }
                }
                Some(b'}') => {
                    self.i += 1;
                    return Ok(Value::Object(members));
                }
                None => return Err(self.eof("EOF while parsing an object")),
                Some(_) => return Err(self.syntax("expected `,` or `}`")),
            }
        }
    }
}
