//! HTTP/1.1 message syntax (RFC 9110 and RFC 9112): header fields, start lines, body framing
//! (Content-Length, chunked, close-delimited) and the writers. The parser is strict where leniency
//! has been a source of request smuggling: bare line feeds, whitespace before the colon, folded
//! fields, a Transfer-Encoding together with a Content-Length and conflicting lengths are errors.

use std::{
    fmt,
    io::{self, BufRead, Read, Write},
};

/// What went wrong reading or writing a message.
#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    /// The peer closed the connection before sending any byte of a message.
    Closed,
    /// The bytes are not a valid message (a 400 for a server).
    Malformed(&'static str),
    /// A line, header block or body exceeds its limit (431 or 413).
    TooLarge(&'static str),
    /// A framing or version this implementation does not provide (501 or 505).
    Unsupported(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "i/o error: {e}"),
            Error::Closed => f.write_str("connection closed"),
            Error::Malformed(m) => write!(f, "malformed message: {m}"),
            Error::TooLarge(m) => write!(f, "message too large: {m}"),
            Error::Unsupported(m) => write!(f, "unsupported: {m}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Error {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Size limits applied while reading a message.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// The start line and each header line, without the line terminator.
    pub max_line: usize,
    /// The start line and all header lines together.
    pub max_head: usize,
    pub max_fields: usize,
    /// The decoded body.
    pub max_body: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_line: 16 * 1024,
            max_head: 64 * 1024,
            max_fields: 128,
            max_body: 32 * 1024 * 1024,
        }
    }
}

fn is_tchar(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}

/// A header field name or a method: one or more token characters.
pub fn is_token(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(is_tchar)
}

fn valid_value(v: &[u8]) -> bool {
    v.iter()
        .all(|&b| b == b'\t' || (0x20..=0x7e).contains(&b) || b >= 0x80)
}

/// Header fields in the order received or added; names compare case-insensitively.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Headers {
    entries: Vec<(String, Vec<u8>)>,
}

impl Headers {
    pub fn new() -> Headers {
        Headers::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Adds a field after the existing ones. Fails when the name is not a token or the value holds
    /// a control character, so a caller can never put a line break into a message.
    pub fn append(&mut self, name: &str, value: impl AsRef<[u8]>) -> Result<()> {
        if !is_token(name) {
            return Err(Error::Malformed("header name is not a token"));
        }
        let value = trim_ows(value.as_ref());
        if !valid_value(value) {
            return Err(Error::Malformed("header value holds a control character"));
        }
        self.entries.push((name.to_string(), value.to_vec()));
        Ok(())
    }

    /// Replaces every field of that name with one value.
    pub fn set(&mut self, name: &str, value: impl AsRef<[u8]>) -> Result<()> {
        let mut staged = Headers::new();
        staged.append(name, value)?;
        self.remove(name);
        self.entries.extend(staged.entries);
        Ok(())
    }

    pub fn remove(&mut self, name: &str) {
        self.entries.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
    }

    pub fn contains(&self, name: &str) -> bool {
        self.entries
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case(name))
    }

    /// The first value as text, when it is printable ASCII (which is what every field the
    /// application reads is).
    pub fn get(&self, name: &str) -> Option<&str> {
        self.get_bytes(name).and_then(|v| {
            if v.iter().all(|&b| b == b'\t' || (0x20..0x7f).contains(&b)) {
                std::str::from_utf8(v).ok()
            } else {
                None
            }
        })
    }

    pub fn get_bytes(&self, name: &str) -> Option<&[u8]> {
        self.entries
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_slice())
    }

    pub fn get_all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a [u8]> + 'a {
        self.entries
            .iter()
            .filter(move |(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_slice())
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.entries.iter().map(|(n, v)| (n.as_str(), v.as_slice()))
    }

    /// The comma-separated members of every field of that name, trimmed and lowercased.
    pub fn list(&self, name: &str) -> Vec<String> {
        let mut out = Vec::new();
        for v in self.get_all(name) {
            for member in v.split(|&b| b == b',') {
                let m = trim_ows(member);
                if !m.is_empty() {
                    out.push(String::from_utf8_lossy(m).to_ascii_lowercase());
                }
            }
        }
        out
    }
}

fn trim_ows(mut v: &[u8]) -> &[u8] {
    while let [b' ' | b'\t', rest @ ..] = v {
        v = rest;
    }
    while let [rest @ .., b' ' | b'\t'] = v {
        v = rest;
    }
    v
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestHead {
    pub method: String,
    pub target: String,
    /// The HTTP/1.x minor version, 0 or 1.
    pub minor: u8,
    pub headers: Headers,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResponseHead {
    pub minor: u8,
    pub status: u16,
    pub reason: String,
    pub headers: Headers,
}

/// Reads up to the next CRLF. `Ok(None)` is a clean end of stream before any byte. A line feed that
/// does not follow a carriage return is an error: tolerating it is how smuggling starts.
fn read_line<R: BufRead>(r: &mut R, max: usize) -> Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    loop {
        let buf = r.fill_buf()?;
        if buf.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Err(Error::Malformed("message ends inside a line"))
            };
        }
        match buf.iter().position(|&b| b == b'\n') {
            Some(i) => {
                line.extend_from_slice(&buf[..i]);
                r.consume(i + 1);
                if line.last() != Some(&b'\r') {
                    return Err(Error::Malformed("line feed without carriage return"));
                }
                line.pop();
                if line.len() > max {
                    return Err(Error::TooLarge("line"));
                }
                return Ok(Some(line));
            }
            None => {
                let n = buf.len();
                line.extend_from_slice(buf);
                r.consume(n);
                if line.len() > max + 1 {
                    return Err(Error::TooLarge("line"));
                }
            }
        }
    }
}

fn parse_version(s: &[u8]) -> Result<u8> {
    match s {
        b"HTTP/1.1" => Ok(1),
        b"HTTP/1.0" => Ok(0),
        [b'H', b'T', b'T', b'P', b'/', ..] => Err(Error::Unsupported("HTTP version")),
        _ => Err(Error::Malformed("version")),
    }
}

fn read_fields<R: BufRead>(r: &mut R, limits: &Limits, mut used: usize) -> Result<Headers> {
    let mut headers = Headers::new();
    loop {
        let Some(line) = read_line(r, limits.max_line)? else {
            return Err(Error::Malformed("message ends inside the header block"));
        };
        used += line.len() + 2;
        if used > limits.max_head {
            return Err(Error::TooLarge("header block"));
        }
        if line.is_empty() {
            return Ok(headers);
        }
        if line[0] == b' ' || line[0] == b'\t' {
            return Err(Error::Malformed("folded header field"));
        }
        let Some(colon) = line.iter().position(|&b| b == b':') else {
            return Err(Error::Malformed("header field without a colon"));
        };
        let name =
            std::str::from_utf8(&line[..colon]).map_err(|_| Error::Malformed("header name"))?;
        if !is_token(name) {
            return Err(Error::Malformed("header name is not a token"));
        }
        if headers.len() >= limits.max_fields {
            return Err(Error::TooLarge("header count"));
        }
        headers.append(name, &line[colon + 1..])?;
    }
}

/// Reads a request line and header fields. `Ok(None)` when the peer closed an idle connection.
pub fn read_request_head<R: BufRead>(r: &mut R, limits: &Limits) -> Result<Option<RequestHead>> {
    // A server ignores empty lines before a request line (RFC 9112 section 2.2), a few at most.
    let mut line = None;
    for _ in 0..3 {
        match read_line(r, limits.max_line)? {
            None => return Ok(None),
            Some(l) if l.is_empty() => continue,
            Some(l) => {
                line = Some(l);
                break;
            }
        }
    }
    let line = line.ok_or(Error::Malformed("empty request line"))?;
    let mut parts = line.split(|&b| b == b' ');
    let (Some(method), Some(target), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(Error::Malformed("request line"));
    };
    let method = std::str::from_utf8(method).map_err(|_| Error::Malformed("method"))?;
    if !is_token(method) {
        return Err(Error::Malformed("method is not a token"));
    }
    if target.is_empty() || target.iter().any(|&b| b <= 0x20 || b >= 0x7f) {
        return Err(Error::Malformed("request target"));
    }
    let target = String::from_utf8_lossy(target).into_owned();
    let minor = parse_version(version)?;
    let headers = read_fields(r, limits, line.len() + 2)?;
    Ok(Some(RequestHead {
        method: method.to_string(),
        target,
        minor,
        headers,
    }))
}

/// Reads a status line and header fields.
pub fn read_response_head<R: BufRead>(r: &mut R, limits: &Limits) -> Result<ResponseHead> {
    let Some(line) = read_line(r, limits.max_line)? else {
        return Err(Error::Closed);
    };
    if line.len() < 12 {
        return Err(Error::Malformed("status line"));
    }
    let minor = parse_version(&line[..8])?;
    if line[8] != b' ' {
        return Err(Error::Malformed("status line"));
    }
    let code = &line[9..12];
    if !code.iter().all(u8::is_ascii_digit) {
        return Err(Error::Malformed("status code"));
    }
    let status = u16::from(code[0] - b'0') * 100
        + u16::from(code[1] - b'0') * 10
        + u16::from(code[2] - b'0');
    if status < 100 {
        return Err(Error::Malformed("status code"));
    }
    let reason = match &line[12..] {
        [] => String::new(),
        [b' ', rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        _ => return Err(Error::Malformed("status line")),
    };
    let headers = read_fields(r, limits, line.len() + 2)?;
    Ok(ResponseHead {
        minor,
        status,
        reason,
        headers,
    })
}

/// How the body of a message is delimited.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Framing {
    None,
    Length(u64),
    Chunked,
    /// Up to the end of the connection (responses only).
    Close,
}

fn content_length(h: &Headers) -> Result<Option<u64>> {
    let mut found: Option<u64> = None;
    for v in h.get_all("content-length") {
        // A list such as "5, 5" is legal when every member agrees (RFC 9110 section 8.6).
        for member in v.split(|&b| b == b',') {
            let m = trim_ows(member);
            if m.is_empty() || !m.iter().all(u8::is_ascii_digit) {
                return Err(Error::Malformed("content-length"));
            }
            let n: u64 = std::str::from_utf8(m)
                .unwrap()
                .parse()
                .map_err(|_| Error::TooLarge("content-length"))?;
            match found {
                Some(prev) if prev != n => {
                    return Err(Error::Malformed("conflicting content-length"));
                }
                _ => found = Some(n),
            }
        }
    }
    Ok(found)
}

fn transfer_framing(h: &Headers) -> Result<Option<Framing>> {
    if !h.contains("transfer-encoding") {
        return Ok(None);
    }
    let codings = h.list("transfer-encoding");
    match codings.as_slice() {
        [only] if only == "chunked" => Ok(Some(Framing::Chunked)),
        [.., last] if last == "chunked" => Err(Error::Unsupported("transfer coding")),
        _ => Err(Error::Malformed(
            "transfer-encoding does not end in chunked",
        )),
    }
}

pub fn request_framing(h: &Headers) -> Result<Framing> {
    if let Some(f) = transfer_framing(h)? {
        if h.contains("content-length") {
            return Err(Error::Malformed(
                "both transfer-encoding and content-length",
            ));
        }
        return Ok(f);
    }
    Ok(match content_length(h)? {
        Some(0) | None => Framing::None,
        Some(n) => Framing::Length(n),
    })
}

pub fn response_framing(request_was_head: bool, status: u16, h: &Headers) -> Result<Framing> {
    if request_was_head || (100..200).contains(&status) || status == 204 || status == 304 {
        return Ok(Framing::None);
    }
    if let Some(f) = transfer_framing(h)? {
        if h.contains("content-length") {
            return Err(Error::Malformed(
                "both transfer-encoding and content-length",
            ));
        }
        return Ok(f);
    }
    Ok(match content_length(h)? {
        Some(n) => Framing::Length(n),
        None => Framing::Close,
    })
}

fn read_exact_limited<R: Read>(r: &mut R, n: u64, out: &mut Vec<u8>, max: usize) -> Result<()> {
    if out.len() as u64 + n > max as u64 {
        return Err(Error::TooLarge("body"));
    }
    let start = out.len();
    out.resize(start + n as usize, 0);
    r.read_exact(&mut out[start..]).map_err(|e| {
        if e.kind() == io::ErrorKind::UnexpectedEof {
            Error::Malformed("body ends early")
        } else {
            Error::Io(e)
        }
    })
}

/// Reads a whole body and returns it decoded.
pub fn read_body<R: BufRead>(r: &mut R, framing: Framing, max: usize) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    match framing {
        Framing::None => {}
        Framing::Length(n) => {
            if n > max as u64 {
                return Err(Error::TooLarge("body"));
            }
            // Allocate as the bytes arrive: a lying length must not reserve memory up front.
            let mut remaining = n;
            let mut chunk = vec![0u8; 64 * 1024];
            while remaining > 0 {
                let want = remaining.min(chunk.len() as u64) as usize;
                let got = r.read(&mut chunk[..want])?;
                if got == 0 {
                    return Err(Error::Malformed("body ends early"));
                }
                body.extend_from_slice(&chunk[..got]);
                remaining -= got as u64;
            }
        }
        Framing::Chunked => loop {
            let Some(line) = read_line(r, 1024)? else {
                return Err(Error::Malformed("body ends early"));
            };
            let size_text = line.split(|&b| b == b';').next().unwrap_or(&[]);
            let size_text = trim_ows(size_text);
            if size_text.is_empty()
                || size_text.len() > 16
                || !size_text.iter().all(u8::is_ascii_hexdigit)
            {
                return Err(Error::Malformed("chunk size"));
            }
            let size = u64::from_str_radix(std::str::from_utf8(size_text).unwrap(), 16)
                .map_err(|_| Error::Malformed("chunk size"))?;
            if size == 0 {
                // Trailer fields are read and discarded.
                let mut used = 0;
                loop {
                    let Some(t) = read_line(r, 8192)? else {
                        return Err(Error::Malformed("body ends early"));
                    };
                    if t.is_empty() {
                        return Ok(body);
                    }
                    used += t.len();
                    if used > 64 * 1024 {
                        return Err(Error::TooLarge("trailer"));
                    }
                }
            }
            if size > max as u64 || body.len() as u64 + size > max as u64 {
                return Err(Error::TooLarge("body"));
            }
            read_exact_limited(r, size, &mut body, max)?;
            let mut crlf = [0u8; 2];
            r.read_exact(&mut crlf)
                .map_err(|_| Error::Malformed("body ends early"))?;
            if &crlf != b"\r\n" {
                return Err(Error::Malformed("chunk is not followed by CRLF"));
            }
        },
        Framing::Close => {
            let mut limited = r.take(max as u64 + 1);
            limited.read_to_end(&mut body)?;
            if body.len() > max {
                return Err(Error::TooLarge("body"));
            }
        }
    }
    Ok(body)
}

pub fn reason_phrase(status: u16) -> &'static str {
    match status {
        100 => "Continue",
        101 => "Switching Protocols",
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        206 => "Partial Content",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        408 => "Request Timeout",
        409 => "Conflict",
        411 => "Length Required",
        413 => "Content Too Large",
        414 => "URI Too Long",
        415 => "Unsupported Media Type",
        422 => "Unprocessable Content",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        505 => "HTTP Version Not Supported",
        _ => "",
    }
}

fn write_fields<W: Write>(w: &mut W, headers: &Headers) -> io::Result<()> {
    for (name, value) in headers.iter() {
        w.write_all(name.as_bytes())?;
        w.write_all(b": ")?;
        w.write_all(value)?;
        w.write_all(b"\r\n")?;
    }
    w.write_all(b"\r\n")
}

pub fn write_request_head<W: Write>(
    w: &mut W,
    method: &str,
    target: &str,
    headers: &Headers,
) -> io::Result<()> {
    write!(w, "{method} {target} HTTP/1.1\r\n")?;
    write_fields(w, headers)
}

pub fn write_response_head<W: Write>(
    w: &mut W,
    minor: u8,
    status: u16,
    headers: &Headers,
) -> io::Result<()> {
    write!(w, "HTTP/1.{minor} {status} {}\r\n", reason_phrase(status))?;
    write_fields(w, headers)
}

/// Writes chunks of a `Transfer-Encoding: chunked` body, flushing each so a stream reaches the
/// peer as it is produced.
pub struct ChunkedWriter<W: Write> {
    inner: W,
}

impl<W: Write> ChunkedWriter<W> {
    pub fn new(inner: W) -> ChunkedWriter<W> {
        ChunkedWriter { inner }
    }

    pub fn chunk(&mut self, data: &[u8]) -> io::Result<()> {
        if data.is_empty() {
            return Ok(());
        }
        write!(self.inner, "{:x}\r\n", data.len())?;
        self.inner.write_all(data)?;
        self.inner.write_all(b"\r\n")?;
        self.inner.flush()
    }

    pub fn finish(mut self) -> io::Result<W> {
        self.inner.write_all(b"0\r\n\r\n")?;
        self.inner.flush()?;
        Ok(self.inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn head(text: &str) -> Result<Option<RequestHead>> {
        read_request_head(
            &mut Cursor::new(text.as_bytes().to_vec()),
            &Limits::default(),
        )
    }

    #[test]
    fn a_plain_request_parses() {
        let h = head("GET /a/b?c=d HTTP/1.1\r\nHost: x\r\nAccept:  text/html \r\nX-Dup: 1\r\nx-dup: 2\r\n\r\n").unwrap().unwrap();
        assert_eq!(
            (h.method.as_str(), h.target.as_str(), h.minor),
            ("GET", "/a/b?c=d", 1)
        );
        assert_eq!(h.headers.get("accept"), Some("text/html"));
        assert_eq!(h.headers.get_all("X-DUP").count(), 2);
    }

    #[test]
    fn smuggling_shapes_are_refused() {
        for bad in [
            "GET / HTTP/1.1\nHost: x\r\n\r\n",
            "GET / HTTP/1.1\r\nHost: x\nA: b\r\n\r\n",
            "GET / HTTP/1.1\r\nHost : x\r\n\r\n",
            "GET / HTTP/1.1\r\nHost: x\r\n folded\r\n\r\n",
            "GET / HTTP/1.1\r\nHost: x\r\nBad Name: y\r\n\r\n",
            "GET / HTTP/1.1\r\nHost: x\r\nA: b\x00c\r\n\r\n",
            "GET /  HTTP/1.1\r\nHost: x\r\n\r\n",
            "GET / HTTP/1.1 \r\nHost: x\r\n\r\n",
            "GET /a b HTTP/1.1\r\n\r\n",
            "G(T / HTTP/1.1\r\n\r\n",
            "GET / HTTP/9.9\r\n\r\n",
            "GET / FTP/1.1\r\n\r\n",
        ] {
            assert!(head(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn framing_follows_rfc_9112() {
        let mut h = Headers::new();
        assert_eq!(request_framing(&h).unwrap(), Framing::None);
        h.append("Content-Length", "12").unwrap();
        assert_eq!(request_framing(&h).unwrap(), Framing::Length(12));
        h.append("Content-Length", "12").unwrap();
        assert_eq!(request_framing(&h).unwrap(), Framing::Length(12));
        h.append("Content-Length", "13").unwrap();
        assert!(request_framing(&h).is_err());
        let mut h = Headers::new();
        h.append("Content-Length", "+5").unwrap();
        assert!(request_framing(&h).is_err());
        let mut h = Headers::new();
        h.append("Transfer-Encoding", "chunked").unwrap();
        assert_eq!(request_framing(&h).unwrap(), Framing::Chunked);
        h.append("Content-Length", "5").unwrap();
        assert!(request_framing(&h).is_err());
        let mut h = Headers::new();
        h.append("Transfer-Encoding", "gzip, chunked").unwrap();
        assert!(matches!(request_framing(&h), Err(Error::Unsupported(_))));
        let mut h = Headers::new();
        h.append("Transfer-Encoding", "chunked, gzip").unwrap();
        assert!(request_framing(&h).is_err());
        assert_eq!(
            response_framing(true, 200, &Headers::new()).unwrap(),
            Framing::None
        );
        assert_eq!(
            response_framing(false, 204, &Headers::new()).unwrap(),
            Framing::None
        );
        assert_eq!(
            response_framing(false, 200, &Headers::new()).unwrap(),
            Framing::Close
        );
    }

    #[test]
    fn chunked_bodies_decode_with_extensions_and_trailers() {
        let wire = b"4;ext=1\r\nWiki\r\n5\r\npedia\r\nE\r\n in\r\n\r\nchunks.\r\n0\r\nTrailer: x\r\n\r\nNEXT";
        let mut r = Cursor::new(wire.to_vec());
        let body = read_body(&mut r, Framing::Chunked, 1000).unwrap();
        assert_eq!(body, b"Wikipedia in\r\n\r\nchunks.");
        let mut rest = Vec::new();
        r.read_to_end(&mut rest).unwrap();
        assert_eq!(rest, b"NEXT");
        for bad in [
            "zz\r\n",
            "5\r\nabcde0\r\n\r\n",
            "5\r\nabc",
            "\r\n",
            "-1\r\n",
            "11111111111111111\r\n",
        ] {
            assert!(
                read_body(
                    &mut Cursor::new(bad.as_bytes().to_vec()),
                    Framing::Chunked,
                    1000
                )
                .is_err(),
                "{bad:?}"
            );
        }
        assert!(matches!(
            read_body(
                &mut Cursor::new(b"5\r\nabcde\r\n0\r\n\r\n".to_vec()),
                Framing::Chunked,
                4
            ),
            Err(Error::TooLarge(_))
        ));
    }

    #[test]
    fn length_bodies_stop_at_the_length_and_resist_lying_lengths() {
        let mut r = Cursor::new(b"hello world".to_vec());
        assert_eq!(
            read_body(&mut r, Framing::Length(5), 100).unwrap(),
            b"hello"
        );
        assert!(read_body(&mut Cursor::new(b"abc".to_vec()), Framing::Length(10), 100).is_err());
        assert!(matches!(
            read_body(&mut Cursor::new(Vec::new()), Framing::Length(u64::MAX), 100),
            Err(Error::TooLarge(_))
        ));
        assert_eq!(
            read_body(&mut Cursor::new(b"abc".to_vec()), Framing::Close, 100).unwrap(),
            b"abc"
        );
        assert!(matches!(
            read_body(&mut Cursor::new(vec![0u8; 20]), Framing::Close, 10),
            Err(Error::TooLarge(_))
        ));
    }

    #[test]
    fn limits_apply_to_lines_blocks_and_counts() {
        let long = format!("GET /{} HTTP/1.1\r\n\r\n", "a".repeat(20000));
        assert!(matches!(head(&long), Err(Error::TooLarge(_))));
        let many: String = (0..200).map(|i| format!("H{i}: v\r\n")).collect();
        assert!(matches!(
            head(&format!("GET / HTTP/1.1\r\n{many}\r\n")),
            Err(Error::TooLarge(_))
        ));
        let wide: String = (0..10)
            .map(|i| format!("H{i}: {}\r\n", "v".repeat(10000)))
            .collect();
        assert!(matches!(
            head(&format!("GET / HTTP/1.1\r\n{wide}\r\n")),
            Err(Error::TooLarge(_))
        ));
    }

    #[test]
    fn responses_parse_with_and_without_a_reason() {
        let mut r = Cursor::new(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi".to_vec());
        let h = read_response_head(&mut r, &Limits::default()).unwrap();
        assert_eq!((h.status, h.reason.as_str(), h.minor), (200, "OK", 1));
        let h = read_response_head(
            &mut Cursor::new(b"HTTP/1.0 404\r\n\r\n".to_vec()),
            &Limits::default(),
        )
        .unwrap();
        assert_eq!((h.status, h.reason.as_str(), h.minor), (404, "", 0));
        for bad in [
            "HTTP/1.1 20 OK\r\n\r\n",
            "HTTP/1.1 2000 OK\r\n\r\n",
            "HTTP/1.1 200OK\r\n\r\n",
            "HTTP/2 200 OK\r\n\r\n",
            "ICY 200 OK\r\n\r\n",
            "HTTP/1.1 099 x\r\n\r\n",
        ] {
            assert!(
                read_response_head(
                    &mut Cursor::new(bad.as_bytes().to_vec()),
                    &Limits::default()
                )
                .is_err(),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_header_value_cannot_carry_a_line_break() {
        let mut h = Headers::new();
        assert!(h.append("X", "a\r\nInjected: 1").is_err());
        assert!(h.append("X", "a\nb").is_err());
        assert!(h.append("X\r\n", "a").is_err());
        assert!(h.append("", "a").is_err());
        h.append("X", "  tab\tinside ok  ").unwrap();
        assert_eq!(h.get("x"), Some("tab\tinside ok"));
    }

    #[test]
    fn chunked_writer_round_trips_through_the_reader() {
        let mut w = ChunkedWriter::new(Vec::new());
        w.chunk(b"abc").unwrap();
        w.chunk(b"").unwrap();
        w.chunk(&[b'x'; 300]).unwrap();
        let wire = w.finish().unwrap();
        let body = read_body(&mut Cursor::new(wire), Framing::Chunked, 1000).unwrap();
        assert_eq!(body.len(), 303);
        assert_eq!(&body[..3], b"abc");
    }

    #[test]
    fn mutated_heads_never_panic() {
        let seed = b"POST /x?y=z HTTP/1.1\r\nHost: a\r\nContent-Length: 4\r\nTransfer-Encoding: chunked\r\n\r\n";
        let mut s = 0x1234_5678_9abc_def0u64;
        for _ in 0..20000 {
            let mut m = seed.to_vec();
            for _ in 0..(1 + s % 4) {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                let i = (s as usize) % m.len();
                match (s >> 40) % 3 {
                    0 => m[i] = (s >> 32) as u8,
                    1 => {
                        m.remove(i);
                    }
                    _ => m.insert(i, (s >> 24) as u8),
                }
            }
            let _ = read_request_head(&mut Cursor::new(m.clone()), &Limits::default());
            let _ = read_body(&mut Cursor::new(m), Framing::Chunked, 100);
        }
    }
}
