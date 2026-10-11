//! WHATWG URL parsing and serialisation for ECDEV's acquisition, redirect, origin and robots policy.
//!
//! The basic URL parser follows https://url.spec.whatwg.org/ for the special schemes (http, https,
//! ws, wss, ftp) and for non-special schemes with a host or an opaque path. One form is refused
//! explicitly rather than guessed: the `file` scheme (ECDEV never reads files), as
//! `ParseError::UnsupportedScheme`, never a different URL. Domain names go through UTS #46
//! (crate::international_domain).

use std::{borrow::Cow, fmt, str::FromStr};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    EmptyHost,
    InvalidDomainCharacter,
    InvalidIpv4Address,
    InvalidIpv6Address,
    InvalidPort,
    RelativeUrlWithoutBase,
    RelativeUrlWithCannotBeABaseBase,
    SetHostOnCannotBeABaseUrl,
    InvalidPercentEncodedHost,
    UnsupportedScheme,
    InvalidInternationalDomain,
    Overflow,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for ParseError {}

/// A component the URL's form cannot carry (credentials without a host, path segments of an opaque path).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CannotModify;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Set {
    C0,
    Fragment,
    Query,
    SpecialQuery,
    Path,
    Userinfo,
    PathSegment,
}

fn in_set(c: char, set: Set) -> bool {
    let c0 = (c as u32) < 0x20 || c as u32 > 0x7e;
    let fragment = c0 || matches!(c, ' ' | '"' | '<' | '>' | '`');
    let query = c0 || matches!(c, ' ' | '"' | '#' | '<' | '>');
    let path = query || matches!(c, '?' | '^' | '`' | '{' | '}');
    let userinfo = path || matches!(c, '/' | ':' | ';' | '=' | '@' | '[' | '\\' | ']' | '|');
    match set {
        Set::C0 => c0,
        Set::Fragment => fragment,
        Set::Query => query,
        Set::SpecialQuery => query || c == '\'',
        Set::Path => path,
        Set::Userinfo => userinfo,
        Set::PathSegment => path || matches!(c, '/' | '%'),
    }
}

fn encode_char(c: char, set: Set, out: &mut String) {
    if in_set(c, set) {
        let mut buf = [0u8; 4];
        for b in c.encode_utf8(&mut buf).bytes() {
            out.push('%');
            out.push_str(&format!("{b:02X}"));
        }
    } else {
        out.push(c);
    }
}

fn encode_str(s: &str, set: Set) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        encode_char(c, set, &mut out);
    }
    out
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

pub(crate) fn percent_decode(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if input[i] == b'%'
            && i + 2 < input.len()
            && let (Some(a), Some(b)) = (hex(input[i + 1]), hex(input[i + 2]))
        {
            out.push(a * 16 + b);
            i += 3;
            continue;
        }
        out.push(input[i]);
        i += 1;
    }
    out
}

fn default_port(scheme: &str) -> Option<u16> {
    match scheme {
        "ftp" => Some(21),
        "http" | "ws" => Some(80),
        "https" | "wss" => Some(443),
        _ => None,
    }
}

fn is_special(scheme: &str) -> bool {
    matches!(scheme, "ftp" | "file" | "http" | "https" | "ws" | "wss")
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum HostRepr {
    Domain(String),
    Ipv4(u32),
    Ipv6([u16; 8]),
    Opaque(String),
    Empty,
}

impl HostRepr {
    fn serialize(&self) -> String {
        match self {
            HostRepr::Domain(d) | HostRepr::Opaque(d) => d.clone(),
            HostRepr::Empty => String::new(),
            HostRepr::Ipv4(a) => format!(
                "{}.{}.{}.{}",
                a >> 24,
                (a >> 16) & 255,
                (a >> 8) & 255,
                a & 255
            ),
            HostRepr::Ipv6(p) => format!("[{}]", serialize_ipv6(p)),
        }
    }
}

fn serialize_ipv6(pieces: &[u16; 8]) -> String {
    // The first longest run of two or more zero pieces is compressed.
    let (mut best, mut best_len, mut i) = (None, 0, 0);
    while i < 8 {
        if pieces[i] == 0 {
            let start = i;
            while i < 8 && pieces[i] == 0 {
                i += 1;
            }
            if i - start > best_len && i - start > 1 {
                best = Some(start);
                best_len = i - start;
            }
        } else {
            i += 1;
        }
    }
    let mut out = String::new();
    let mut ignore0 = false;
    for (i, piece) in pieces.iter().enumerate() {
        if ignore0 && *piece == 0 {
            continue;
        }
        ignore0 = false;
        if best == Some(i) {
            out.push_str(if i == 0 { "::" } else { ":" });
            ignore0 = true;
            continue;
        }
        out.push_str(&format!("{piece:x}"));
        if i != 7 {
            out.push(':');
        }
    }
    out
}

fn parse_ipv6(input: &str) -> Result<[u16; 8], ParseError> {
    let err = ParseError::InvalidIpv6Address;
    let c: Vec<char> = input.chars().collect();
    let mut address = [0u16; 8];
    let (mut piece, mut compress, mut p) = (0usize, None::<usize>, 0usize);
    if c.first() == Some(&':') {
        if c.get(1) != Some(&':') {
            return Err(err);
        }
        p += 2;
        piece += 1;
        compress = Some(piece);
    }
    while p < c.len() {
        if piece == 8 {
            return Err(err);
        }
        if c[p] == ':' {
            if compress.is_some() {
                return Err(err);
            }
            p += 1;
            piece += 1;
            compress = Some(piece);
            continue;
        }
        let (mut value, mut length) = (0u32, 0);
        while length < 4 && p < c.len() && c[p].is_ascii_hexdigit() {
            value = value * 16 + c[p].to_digit(16).unwrap();
            p += 1;
            length += 1;
        }
        if p < c.len() && c[p] == '.' {
            if length == 0 {
                return Err(err);
            }
            p -= length;
            if piece > 6 {
                return Err(err);
            }
            let mut seen = 0;
            while p < c.len() {
                let mut v4 = None::<u32>;
                if seen > 0 {
                    if c[p] == '.' && seen < 4 {
                        p += 1;
                    } else {
                        return Err(err);
                    }
                }
                if p >= c.len() || !c[p].is_ascii_digit() {
                    return Err(err);
                }
                while p < c.len() && c[p].is_ascii_digit() {
                    let n = c[p].to_digit(10).unwrap();
                    v4 = match v4 {
                        None => Some(n),
                        Some(0) => return Err(err),
                        Some(v) => Some(v * 10 + n),
                    };
                    if v4.unwrap() > 255 {
                        return Err(err);
                    }
                    p += 1;
                }
                address[piece] = (address[piece] as u32 * 0x100 + v4.unwrap()) as u16;
                seen += 1;
                if seen == 2 || seen == 4 {
                    piece += 1;
                }
            }
            if seen != 4 {
                return Err(err);
            }
            break;
        } else if p < c.len() && c[p] == ':' {
            p += 1;
            if p >= c.len() {
                return Err(err);
            }
        } else if p < c.len() {
            return Err(err);
        }
        address[piece] = value as u16;
        piece += 1;
    }
    if let Some(compress) = compress {
        let mut swaps = piece - compress;
        piece = 7;
        while piece != 0 && swaps > 0 {
            address.swap(piece, compress + swaps - 1);
            piece -= 1;
            swaps -= 1;
        }
    } else if piece != 8 {
        return Err(err);
    }
    Ok(address)
}

fn ends_in_a_number(host: &str) -> bool {
    let mut parts: Vec<&str> = host.split('.').collect();
    if parts.last() == Some(&"") {
        if parts.len() == 1 {
            return false;
        }
        parts.pop();
    }
    let last = parts.last().copied().unwrap_or("");
    if !last.is_empty() && last.bytes().all(|b| b.is_ascii_digit()) {
        return true;
    }
    parse_ipv4_number(last).is_some()
}

fn parse_ipv4_number(input: &str) -> Option<u64> {
    if input.is_empty() {
        return None;
    }
    let (digits, radix) =
        if input.len() >= 2 && (input.starts_with("0x") || input.starts_with("0X")) {
            (&input[2..], 16)
        } else if input.len() >= 2 && input.starts_with('0') {
            (&input[1..], 8)
        } else {
            (input, 10)
        };
    if digits.is_empty() {
        return Some(0);
    }
    // from_str_radix would accept a leading "+"; the standard allows digits only.
    if !digits.chars().all(|c| c.is_digit(radix)) {
        return None;
    }
    // Digits too large for u64 are still "a number"; they overflow the address range later.
    Some(u64::from_str_radix(digits, radix).unwrap_or(u64::MAX))
}

fn parse_ipv4(input: &str) -> Result<u32, ParseError> {
    let err = ParseError::InvalidIpv4Address;
    let mut parts: Vec<&str> = input.split('.').collect();
    if parts.last() == Some(&"") && parts.len() > 1 {
        parts.pop();
    }
    if parts.len() > 4 {
        return Err(err);
    }
    let mut numbers = vec![];
    for part in &parts {
        numbers.push(parse_ipv4_number(part).ok_or(err.clone())?);
    }
    if numbers[..numbers.len() - 1].iter().any(|n| *n > 255) {
        return Err(err);
    }
    if *numbers.last().unwrap() >= 256u64.pow(5 - numbers.len() as u32) {
        return Err(err);
    }
    let mut ipv4 = *numbers.last().unwrap();
    for (i, n) in numbers[..numbers.len() - 1].iter().enumerate() {
        ipv4 += n * 256u64.pow(3 - i as u32);
    }
    Ok(ipv4 as u32)
}

fn domain_to_ascii(domain: &str) -> Result<String, ParseError> {
    crate::international_domain::domain_to_ascii(domain)
        .ok_or(ParseError::InvalidInternationalDomain)
}

fn forbidden_host_char(c: char) -> bool {
    matches!(
        c,
        '\0' | '\t'
            | '\n'
            | '\r'
            | ' '
            | '#'
            | '/'
            | ':'
            | '<'
            | '>'
            | '?'
            | '@'
            | '['
            | '\\'
            | ']'
            | '^'
            | '|'
    )
}

fn forbidden_domain_char(c: char) -> bool {
    forbidden_host_char(c) || (c as u32) <= 0x1f || c == '%' || c as u32 == 0x7f
}

fn parse_host(input: &str, special: bool) -> Result<HostRepr, ParseError> {
    if let Some(rest) = input.strip_prefix('[') {
        let inner = rest
            .strip_suffix(']')
            .ok_or(ParseError::InvalidIpv6Address)?;
        return Ok(HostRepr::Ipv6(parse_ipv6(inner)?));
    }
    if !special {
        if input.chars().any(forbidden_host_char) {
            return Err(ParseError::InvalidDomainCharacter);
        }
        return Ok(HostRepr::Opaque(encode_str(input, Set::C0)));
    }
    let decoded = String::from_utf8(percent_decode(input.as_bytes()))
        .map_err(|_| ParseError::InvalidPercentEncodedHost)?;
    let ascii = domain_to_ascii(&decoded)?;
    if ascii.is_empty() {
        return Err(ParseError::EmptyHost);
    }
    if ascii.chars().any(forbidden_domain_char) {
        return Err(ParseError::InvalidDomainCharacter);
    }
    if ends_in_a_number(&ascii) {
        return Ok(HostRepr::Ipv4(parse_ipv4(&ascii)?));
    }
    Ok(HostRepr::Domain(ascii))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    SchemeStart,
    Scheme,
    NoScheme,
    SpecialRelativeOrAuthority,
    PathOrAuthority,
    Relative,
    RelativeSlash,
    SpecialAuthoritySlashes,
    SpecialAuthorityIgnoreSlashes,
    Authority,
    Host,
    Port,
    PathStart,
    Path,
    OpaquePath,
    Query,
    Fragment,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Record {
    scheme: String,
    username: String,
    password: String,
    host: Option<HostRepr>,
    port: Option<u16>,
    opaque: Option<String>,
    path: Vec<String>,
    query: Option<String>,
    fragment: Option<String>,
}

impl Record {
    fn special(&self) -> bool {
        is_special(&self.scheme)
    }
    fn includes_credentials(&self) -> bool {
        !self.username.is_empty() || !self.password.is_empty()
    }
    fn cannot_have_username_password_port(&self) -> bool {
        matches!(self.host, None | Some(HostRepr::Empty)) || self.scheme == "file"
    }
    fn shorten_path(&mut self) {
        self.path.pop();
    }
}

fn is_single_dot(s: &str) -> bool {
    s == "." || s.eq_ignore_ascii_case("%2e")
}

fn is_double_dot(s: &str) -> bool {
    matches!(
        s.to_ascii_lowercase().as_str(),
        ".." | ".%2e" | "%2e." | "%2e%2e"
    )
}

fn parse_record(input: &str, base: Option<&Record>) -> Result<Record, ParseError> {
    let trimmed = input.trim_matches(|c: char| (c as u32) <= 0x20);
    let chars: Vec<char> = trimmed
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    let mut url = Record {
        scheme: String::new(),
        username: String::new(),
        password: String::new(),
        host: None,
        port: None,
        opaque: None,
        path: vec![],
        query: None,
        fragment: None,
    };
    let mut state = State::SchemeStart;
    let mut buffer = String::new();
    let (mut at_sign, mut inside_brackets, mut password_token_seen) = (false, false, false);
    let mut p = 0usize;
    loop {
        let c = chars.get(p).copied();
        match state {
            State::SchemeStart => {
                if let Some(ch) = c.filter(char::is_ascii_alphabetic) {
                    buffer.push(ch.to_ascii_lowercase());
                    state = State::Scheme;
                } else {
                    state = State::NoScheme;
                    continue;
                }
            }
            State::Scheme => {
                if let Some(ch) =
                    c.filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.'))
                {
                    buffer.push(ch.to_ascii_lowercase());
                } else if c == Some(':') {
                    url.scheme = std::mem::take(&mut buffer);
                    if url.scheme == "file" {
                        return Err(ParseError::UnsupportedScheme);
                    }
                    if url.special() {
                        if base.is_some_and(|b| b.scheme == url.scheme) {
                            state = State::SpecialRelativeOrAuthority;
                        } else {
                            state = State::SpecialAuthoritySlashes;
                        }
                    } else if chars.get(p + 1) == Some(&'/') {
                        state = State::PathOrAuthority;
                        p += 1;
                    } else {
                        url.opaque = Some(String::new());
                        state = State::OpaquePath;
                    }
                } else {
                    buffer.clear();
                    state = State::NoScheme;
                    p = 0;
                    continue;
                }
            }
            State::NoScheme => {
                let Some(b) = base else {
                    return Err(ParseError::RelativeUrlWithoutBase);
                };
                if b.opaque.is_some() && c != Some('#') {
                    return Err(ParseError::RelativeUrlWithCannotBeABaseBase);
                }
                if b.opaque.is_some() {
                    url.scheme = b.scheme.clone();
                    url.opaque = b.opaque.clone();
                    url.query = b.query.clone();
                    url.fragment = Some(String::new());
                    state = State::Fragment;
                } else if b.scheme != "file" {
                    state = State::Relative;
                    continue;
                } else {
                    return Err(ParseError::UnsupportedScheme);
                }
            }
            State::SpecialRelativeOrAuthority => {
                if c == Some('/') && chars.get(p + 1) == Some(&'/') {
                    state = State::SpecialAuthorityIgnoreSlashes;
                    p += 1;
                } else {
                    state = State::Relative;
                    continue;
                }
            }
            State::PathOrAuthority => {
                if c == Some('/') {
                    state = State::Authority;
                } else {
                    state = State::Path;
                    continue;
                }
            }
            State::Relative => {
                let b = base.expect("relative state needs a base");
                url.scheme = b.scheme.clone();
                if c == Some('/') || (url.special() && c == Some('\\')) {
                    state = State::RelativeSlash;
                } else {
                    url.username = b.username.clone();
                    url.password = b.password.clone();
                    url.host = b.host.clone();
                    url.port = b.port;
                    url.path = b.path.clone();
                    url.query = b.query.clone();
                    if c == Some('?') {
                        url.query = Some(String::new());
                        state = State::Query;
                    } else if c == Some('#') {
                        url.fragment = Some(String::new());
                        state = State::Fragment;
                    } else if c.is_some() {
                        url.query = None;
                        url.shorten_path();
                        state = State::Path;
                        continue;
                    }
                }
            }
            State::RelativeSlash => {
                if url.special() && matches!(c, Some('/') | Some('\\')) {
                    state = State::SpecialAuthorityIgnoreSlashes;
                } else if c == Some('/') {
                    state = State::Authority;
                } else {
                    let b = base.expect("relative slash state needs a base");
                    url.username = b.username.clone();
                    url.password = b.password.clone();
                    url.host = b.host.clone();
                    url.port = b.port;
                    state = State::Path;
                    continue;
                }
            }
            State::SpecialAuthoritySlashes => {
                if c == Some('/') && chars.get(p + 1) == Some(&'/') {
                    state = State::SpecialAuthorityIgnoreSlashes;
                    p += 1;
                } else {
                    state = State::SpecialAuthorityIgnoreSlashes;
                    continue;
                }
            }
            State::SpecialAuthorityIgnoreSlashes => {
                if !matches!(c, Some('/') | Some('\\')) {
                    state = State::Authority;
                    continue;
                }
            }
            State::Authority => {
                if c == Some('@') {
                    if at_sign {
                        buffer.insert_str(0, "%40");
                    }
                    at_sign = true;
                    for ch in buffer.chars() {
                        if ch == ':' && !password_token_seen {
                            password_token_seen = true;
                            continue;
                        }
                        let target = if password_token_seen {
                            &mut url.password
                        } else {
                            &mut url.username
                        };
                        encode_char(ch, Set::Userinfo, target);
                    }
                    buffer.clear();
                } else if c.is_none()
                    || matches!(c, Some('/') | Some('?') | Some('#'))
                    || (url.special() && c == Some('\\'))
                {
                    if at_sign && buffer.is_empty() {
                        return Err(ParseError::EmptyHost);
                    }
                    p -= buffer.chars().count();
                    buffer.clear();
                    state = State::Host;
                    continue;
                } else {
                    buffer.push(c.unwrap());
                }
            }
            State::Host => {
                if c == Some(':') && !inside_brackets {
                    if buffer.is_empty() {
                        return Err(ParseError::EmptyHost);
                    }
                    url.host = Some(parse_host(&buffer, url.special())?);
                    buffer.clear();
                    state = State::Port;
                } else if c.is_none()
                    || matches!(c, Some('/') | Some('?') | Some('#'))
                    || (url.special() && c == Some('\\'))
                {
                    if url.special() && buffer.is_empty() {
                        return Err(ParseError::EmptyHost);
                    }
                    if buffer.is_empty() && (url.includes_credentials() || url.port.is_some()) {
                        return Err(ParseError::EmptyHost);
                    }
                    url.host = Some(if buffer.is_empty() {
                        HostRepr::Empty
                    } else {
                        parse_host(&buffer, url.special())?
                    });
                    buffer.clear();
                    state = State::PathStart;
                    continue;
                } else {
                    if c == Some('[') {
                        inside_brackets = true;
                    }
                    if c == Some(']') {
                        inside_brackets = false;
                    }
                    buffer.push(c.unwrap());
                }
            }
            State::Port => {
                if let Some(ch) = c.filter(char::is_ascii_digit) {
                    buffer.push(ch);
                } else if c.is_none()
                    || matches!(c, Some('/') | Some('?') | Some('#'))
                    || (url.special() && c == Some('\\'))
                {
                    if !buffer.is_empty() {
                        let port: u32 = buffer.parse().map_err(|_| ParseError::InvalidPort)?;
                        if port > 65535 {
                            return Err(ParseError::InvalidPort);
                        }
                        url.port = if Some(port as u16) == default_port(&url.scheme) {
                            None
                        } else {
                            Some(port as u16)
                        };
                        buffer.clear();
                    }
                    state = State::PathStart;
                    continue;
                } else {
                    return Err(ParseError::InvalidPort);
                }
            }
            State::PathStart => {
                if url.special() {
                    state = State::Path;
                    if c != Some('/') && c != Some('\\') {
                        continue;
                    }
                } else if c == Some('?') {
                    url.query = Some(String::new());
                    state = State::Query;
                } else if c == Some('#') {
                    url.fragment = Some(String::new());
                    state = State::Fragment;
                } else if c.is_some() {
                    state = State::Path;
                    if c != Some('/') {
                        continue;
                    }
                } else if url.host.is_none() {
                    url.path.push(String::new());
                }
            }
            State::Path => {
                let end = c.is_none()
                    || c == Some('/')
                    || (url.special() && c == Some('\\'))
                    || matches!(c, Some('?') | Some('#'));
                if end {
                    let slash = c == Some('/') || (url.special() && c == Some('\\'));
                    if is_double_dot(&buffer) {
                        url.shorten_path();
                        if !slash {
                            url.path.push(String::new());
                        }
                    } else if is_single_dot(&buffer) && !slash {
                        url.path.push(String::new());
                    } else if !is_single_dot(&buffer) {
                        url.path.push(std::mem::take(&mut buffer));
                    }
                    buffer.clear();
                    if c == Some('?') {
                        url.query = Some(String::new());
                        state = State::Query;
                    }
                    if c == Some('#') {
                        url.fragment = Some(String::new());
                        state = State::Fragment;
                    }
                } else {
                    encode_char(c.unwrap(), Set::Path, &mut buffer);
                }
            }
            State::OpaquePath => {
                if c == Some('?') {
                    url.query = Some(String::new());
                    state = State::Query;
                } else if c == Some('#') {
                    url.fragment = Some(String::new());
                    state = State::Fragment;
                } else if let Some(ch) = c {
                    // A space just before a query or fragment would be lost when they are removed.
                    let trailing = ch == ' ' && matches!(chars.get(p + 1), Some('?') | Some('#'));
                    if trailing {
                        url.opaque.as_mut().unwrap().push_str("%20");
                    } else {
                        encode_char(ch, Set::C0, url.opaque.as_mut().unwrap());
                    }
                }
            }
            State::Query => {
                if c == Some('#') || c.is_none() {
                    let set = if url.special() {
                        Set::SpecialQuery
                    } else {
                        Set::Query
                    };
                    let q = url.query.get_or_insert_with(String::new);
                    for ch in buffer.chars() {
                        encode_char(ch, set, q);
                    }
                    buffer.clear();
                    if c == Some('#') {
                        url.fragment = Some(String::new());
                        state = State::Fragment;
                    }
                } else if let Some(ch) = c {
                    buffer.push(ch);
                }
            }
            State::Fragment => {
                if let Some(ch) = c {
                    encode_char(
                        ch,
                        Set::Fragment,
                        url.fragment.get_or_insert_with(String::new),
                    );
                }
            }
        }
        if c.is_none() {
            break;
        }
        p += 1;
    }
    Ok(url)
}

fn serialize_path(r: &Record) -> String {
    match &r.opaque {
        Some(o) => o.clone(),
        None => r.path.iter().fold(String::new(), |mut acc, seg| {
            acc.push('/');
            acc.push_str(seg);
            acc
        }),
    }
}

fn serialize_href(r: &Record) -> String {
    let mut out = format!("{}:", r.scheme);
    if let Some(host) = &r.host {
        out.push_str("//");
        if r.includes_credentials() {
            out.push_str(&r.username);
            if !r.password.is_empty() {
                out.push(':');
                out.push_str(&r.password);
            }
            out.push('@');
        }
        out.push_str(&host.serialize());
        if let Some(port) = r.port {
            out.push_str(&format!(":{port}"));
        }
    } else if r.opaque.is_none() && r.path.len() > 1 && r.path[0].is_empty() {
        out.push_str("/.");
    }
    out.push_str(&serialize_path(r));
    if let Some(q) = &r.query {
        out.push('?');
        out.push_str(q);
    }
    if let Some(f) = &r.fragment {
        out.push('#');
        out.push_str(f);
    }
    out
}

/// A host as policy code sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Host<S = String> {
    Domain(S),
    Ipv4(std::net::Ipv4Addr),
    Ipv6(std::net::Ipv6Addr),
}

/// The origin of a URL: a (scheme, host, port) tuple for the special network schemes, else opaque.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Origin {
    Opaque,
    Tuple(String, Host<String>, u16),
}

impl Origin {
    pub fn ascii_serialization(&self) -> String {
        match self {
            Origin::Opaque => "null".into(),
            Origin::Tuple(scheme, host, port) => {
                let host = match host {
                    Host::Domain(d) => d.clone(),
                    Host::Ipv4(a) => a.to_string(),
                    Host::Ipv6(a) => format!("[{}]", serialize_ipv6(&a.segments())),
                };
                if Some(*port) == default_port(scheme) {
                    format!("{scheme}://{host}")
                } else {
                    format!("{scheme}://{host}:{port}")
                }
            }
        }
    }
    pub fn is_tuple(&self) -> bool {
        matches!(self, Origin::Tuple(..))
    }
}

#[derive(Clone)]
pub struct Url {
    record: Record,
    href: String,
    path_text: String,
    host_text: Option<String>,
}

impl Url {
    fn from_record(record: Record) -> Url {
        let href = serialize_href(&record);
        let path_text = serialize_path(&record);
        let host_text = record.host.as_ref().map(HostRepr::serialize);
        Url {
            record,
            href,
            path_text,
            host_text,
        }
    }

    fn refresh(&mut self) {
        self.href = serialize_href(&self.record);
        self.path_text = serialize_path(&self.record);
        self.host_text = self.record.host.as_ref().map(HostRepr::serialize);
    }

    pub fn parse(input: &str) -> Result<Url, ParseError> {
        parse_record(input, None).map(Url::from_record)
    }

    pub fn join(&self, input: &str) -> Result<Url, ParseError> {
        parse_record(input, Some(&self.record)).map(Url::from_record)
    }

    pub fn as_str(&self) -> &str {
        &self.href
    }

    pub fn scheme(&self) -> &str {
        &self.record.scheme
    }

    pub fn is_special(&self) -> bool {
        self.record.special()
    }

    pub fn cannot_be_a_base(&self) -> bool {
        self.record.opaque.is_some()
    }

    pub fn has_host(&self) -> bool {
        self.record.host.is_some()
    }

    pub fn host_str(&self) -> Option<&str> {
        self.host_text.as_deref().filter(|h| !h.is_empty())
    }

    pub fn host(&self) -> Option<Host<&str>> {
        Some(match self.record.host.as_ref()? {
            HostRepr::Ipv4(a) => Host::Ipv4(std::net::Ipv4Addr::from(*a)),
            HostRepr::Ipv6(p) => Host::Ipv6(std::net::Ipv6Addr::from(*p)),
            HostRepr::Domain(_) | HostRepr::Opaque(_) => Host::Domain(self.host_text.as_deref()?),
            HostRepr::Empty => return None,
        })
    }

    pub fn domain(&self) -> Option<&str> {
        match self.record.host.as_ref()? {
            HostRepr::Domain(d) | HostRepr::Opaque(d) => Some(d),
            _ => None,
        }
    }

    pub fn port(&self) -> Option<u16> {
        self.record.port
    }

    pub fn port_or_known_default(&self) -> Option<u16> {
        self.record
            .port
            .or_else(|| default_port(&self.record.scheme))
    }

    pub fn username(&self) -> &str {
        &self.record.username
    }

    pub fn password(&self) -> Option<&str> {
        (!self.record.password.is_empty()).then_some(self.record.password.as_str())
    }

    pub fn path(&self) -> &str {
        &self.path_text
    }

    pub fn path_segments(&self) -> Option<std::str::Split<'_, char>> {
        let path = self.path_text.strip_prefix('/')?;
        Some(path.split('/'))
    }

    pub fn query(&self) -> Option<&str> {
        self.record.query.as_deref()
    }

    pub fn fragment(&self) -> Option<&str> {
        self.record.fragment.as_deref()
    }

    pub fn origin(&self) -> Origin {
        if !matches!(
            self.record.scheme.as_str(),
            "http" | "https" | "ws" | "wss" | "ftp"
        ) {
            return Origin::Opaque;
        }
        match (self.host(), self.port_or_known_default()) {
            (Some(host), Some(port)) => Origin::Tuple(
                self.record.scheme.clone(),
                match host {
                    Host::Domain(d) => Host::Domain(d.to_string()),
                    Host::Ipv4(a) => Host::Ipv4(a),
                    Host::Ipv6(a) => Host::Ipv6(a),
                },
                port,
            ),
            _ => Origin::Opaque,
        }
    }

    pub fn query_pairs(&self) -> QueryPairsIter<'_> {
        let pairs: Vec<(Cow<'_, str>, Cow<'_, str>)> = self
            .record
            .query
            .as_deref()
            .unwrap_or("")
            .split('&')
            .filter(|s| !s.is_empty())
            .map(|pair| {
                let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
                (form_decode(k), form_decode(v))
            })
            .collect();
        QueryPairsIter {
            inner: pairs.into_iter(),
        }
    }

    pub fn query_pairs_mut(&mut self) -> QueryPairs<'_> {
        let existing = self.record.query.clone().unwrap_or_default();
        QueryPairs {
            url: self,
            buffer: existing,
        }
    }

    pub fn set_query(&mut self, query: Option<&str>) {
        let set = if self.record.special() {
            Set::SpecialQuery
        } else {
            Set::Query
        };
        self.record.query = query.map(|q| encode_str(q, set));
        self.refresh();
    }

    pub fn set_fragment(&mut self, fragment: Option<&str>) {
        self.record.fragment = fragment.map(|f| encode_str(f, Set::Fragment));
        self.refresh();
    }

    pub fn set_path(&mut self, path: &str) {
        if self.record.opaque.is_some() {
            return;
        }
        if path.is_empty() && !self.record.special() {
            self.record.path.clear();
            self.refresh();
            return;
        }
        let guarded = path.replace('?', "%3F").replace('#', "%23");
        let scratch = format!(
            "{}://h/{}",
            self.record.scheme,
            guarded.strip_prefix('/').unwrap_or(&guarded)
        );
        if let Ok(parsed) = parse_record(&scratch, None) {
            self.record.path = parsed.path;
            self.refresh();
        }
    }

    pub fn set_password(&mut self, password: Option<&str>) -> Result<(), CannotModify> {
        if self.record.cannot_have_username_password_port() {
            return Err(CannotModify);
        }
        self.record.password = password
            .map(|p| encode_str(p, Set::Userinfo))
            .unwrap_or_default();
        self.refresh();
        Ok(())
    }

    pub fn path_segments_mut(&mut self) -> Result<PathSegmentsMut<'_>, CannotModify> {
        if self.record.opaque.is_some() {
            return Err(CannotModify);
        }
        Ok(PathSegmentsMut { url: self })
    }

    pub fn set_username(&mut self, username: &str) -> Result<(), CannotModify> {
        if self.record.cannot_have_username_password_port() {
            return Err(CannotModify);
        }
        self.record.username = encode_str(username, Set::Userinfo);
        self.refresh();
        Ok(())
    }
}

fn form_decode(s: &str) -> Cow<'_, str> {
    if !s.bytes().any(|b| b == b'+' || b == b'%') {
        return Cow::Borrowed(s);
    }
    let replaced: Vec<u8> = s
        .bytes()
        .map(|b| if b == b'+' { b' ' } else { b })
        .collect();
    Cow::Owned(String::from_utf8_lossy(&percent_decode(&replaced)).into_owned())
}

fn form_encode(s: &str, out: &mut String) {
    for b in s.bytes() {
        match b {
            b' ' => out.push('+'),
            b'*' | b'-' | b'.' | b'_' | b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
}

/// The decoded name-value pairs of a query string.
pub struct QueryPairsIter<'a> {
    inner: std::vec::IntoIter<(Cow<'a, str>, Cow<'a, str>)>,
}

impl<'a> Iterator for QueryPairsIter<'a> {
    type Item = (Cow<'a, str>, Cow<'a, str>);
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

impl QueryPairsIter<'_> {
    pub fn into_owned(self) -> impl Iterator<Item = (String, String)> {
        self.inner.map(|(k, v)| (k.into_owned(), v.into_owned()))
    }
}

/// Edits the segments of a hierarchical URL's path; the URL is re-serialised when it is dropped.
pub struct PathSegmentsMut<'a> {
    url: &'a mut Url,
}

impl PathSegmentsMut<'_> {
    pub fn pop_if_empty(&mut self) -> &mut Self {
        if self.url.record.path.last().is_some_and(String::is_empty) {
            self.url.record.path.pop();
        }
        self
    }

    pub fn push(&mut self, segment: &str) -> &mut Self {
        // Navigation segments are data nowhere in a pushed path: "." and ".." are dropped.
        if segment != "." && segment != ".." {
            self.url
                .record
                .path
                .push(encode_str(segment, Set::PathSegment));
        }
        self
    }

    pub fn clear(&mut self) -> &mut Self {
        self.url.record.path.clear();
        self
    }
}

impl Drop for PathSegmentsMut<'_> {
    fn drop(&mut self) {
        if self.url.record.path.is_empty() {
            self.url.record.path.push(String::new());
        }
        self.url.refresh();
    }
}

/// Appends application/x-www-form-urlencoded pairs to a URL's query when dropped.
pub struct QueryPairs<'a> {
    url: &'a mut Url,
    buffer: String,
}

impl QueryPairs<'_> {
    pub fn append_pair(&mut self, key: &str, value: &str) -> &mut Self {
        if !self.buffer.is_empty() {
            self.buffer.push('&');
        }
        form_encode(key, &mut self.buffer);
        self.buffer.push('=');
        form_encode(value, &mut self.buffer);
        self
    }

    pub fn extend_pairs<I, K, V>(&mut self, pairs: I) -> &mut Self
    where
        I: IntoIterator,
        I::Item: std::borrow::Borrow<(K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        for pair in pairs {
            let (k, v): &(K, V) = std::borrow::Borrow::borrow(&pair);
            self.append_pair(k.as_ref(), v.as_ref());
        }
        self
    }

    pub fn clear(&mut self) -> &mut Self {
        self.buffer.clear();
        self
    }
}

impl Drop for QueryPairs<'_> {
    fn drop(&mut self) {
        self.url.record.query = Some(std::mem::take(&mut self.buffer));
        self.url.refresh();
    }
}

impl fmt::Display for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.href)
    }
}

impl fmt::Debug for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Url({:?})", self.href)
    }
}

impl PartialEq for Url {
    fn eq(&self, other: &Url) -> bool {
        self.href == other.href
    }
}

impl Eq for Url {}

impl std::hash::Hash for Url {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.href.hash(state)
    }
}

impl PartialOrd for Url {
    fn partial_cmp(&self, other: &Url) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Url {
    fn cmp(&self, other: &Url) -> std::cmp::Ordering {
        self.href.cmp(&other.href)
    }
}

impl FromStr for Url {
    type Err = ParseError;
    fn from_str(input: &str) -> Result<Url, ParseError> {
        Url::parse(input)
    }
}

impl AsRef<str> for Url {
    fn as_ref(&self) -> &str {
        &self.href
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(input: &str) -> Option<String> {
        Url::parse(input)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
    }

    #[test]
    fn hosts_that_name_loopback_in_disguise_serialize_to_their_address() {
        for (input, canonical) in [
            ("http://2130706433/", "127.0.0.1"),
            ("http://0x7f.1/", "127.0.0.1"),
            ("http://0177.0.0.1/", "127.0.0.1"),
            ("http://127.1/", "127.0.0.1"),
            ("http://[::ffff:7f00:1]/", "[::ffff:7f00:1]"),
            ("http://[0:0:0:0:0:0:0:1]/", "[::1]"),
            ("http://LOCALHOST/", "localhost"),
            ("http://%31%32%37.0.0.1/", "127.0.0.1"),
        ] {
            assert_eq!(host(input).as_deref(), Some(canonical), "{input}");
        }
        assert!(
            Url::parse("http://256.1.1.1/").is_err(),
            "an out-of-range part is an error, never a domain"
        );
        assert!(Url::parse("http://1.2.3.4.5/").is_err());
    }

    #[test]
    fn credentials_and_backslashes_cannot_disguise_the_host() {
        assert_eq!(host("http://a@b@c/").as_deref(), Some("c"));
        assert_eq!(
            host("http://good.example\\@evil.example/").as_deref(),
            Some("good.example")
        );
        assert_eq!(
            host("http://evil.example#@good.example/").as_deref(),
            Some("evil.example")
        );
        assert_eq!(
            host("http://evil.example?@good.example/").as_deref(),
            Some("evil.example")
        );
        assert!(
            Url::parse("http://user@/").is_err(),
            "credentials with no host"
        );
        assert!(Url::parse("http://exa mple.com/").is_err());
        assert!(Url::parse("http://exa%00mple.com/").is_err());
    }

    #[test]
    fn relative_references_resolve_as_the_standard_says() {
        let base = Url::parse("https://example.com/a/b/c?q#f").unwrap();
        for (reference, expected) in [
            ("d", "https://example.com/a/b/d"),
            ("../d", "https://example.com/a/d"),
            ("/d", "https://example.com/d"),
            ("//other.example/d", "https://other.example/d"),
            ("?x", "https://example.com/a/b/c?x"),
            ("#g", "https://example.com/a/b/c?q#g"),
            ("", "https://example.com/a/b/c?q"),
            ("../../../../d", "https://example.com/d"),
            ("https:d", "https://example.com/a/b/d"),
            ("http:d", "http://d/"),
        ] {
            assert_eq!(
                base.join(reference).unwrap().as_str(),
                expected,
                "{reference:?}"
            );
        }
        assert!(Url::parse("/just/a/path").is_err());
        assert_eq!(Url::parse("mailto:a@b.example").unwrap().scheme(), "mailto");
        assert!(Url::parse("mailto:a@b.example").unwrap().cannot_be_a_base());
        assert!(matches!(
            Url::parse("file:///etc/passwd"),
            Err(ParseError::UnsupportedScheme)
        ));
    }

    #[test]
    fn origins_compare_by_scheme_host_and_effective_port() {
        let origin = |s: &str| Url::parse(s).unwrap().origin();
        assert_eq!(
            origin("https://Example.com:443/a"),
            origin("https://example.com/b")
        );
        assert_ne!(
            origin("https://example.com/"),
            origin("http://example.com/")
        );
        assert_ne!(
            origin("https://example.com/"),
            origin("https://example.com:8443/")
        );
        assert_eq!(
            origin("https://example.com:8443/").ascii_serialization(),
            "https://example.com:8443"
        );
        assert_eq!(origin("mailto:a@b").ascii_serialization(), "null");
        assert!(!origin("mailto:a@b").is_tuple());
    }

    #[test]
    fn query_pairs_decode_and_encode_as_forms() {
        let mut u = Url::parse("https://example.com/?a=1+2&b=%C3%A9&c&=d").unwrap();
        let pairs: Vec<(String, String)> = u.query_pairs().into_owned().collect();
        assert_eq!(
            pairs,
            [
                ("a".into(), "1 2".into()),
                ("b".into(), "é".into()),
                ("c".into(), "".into()),
                ("".into(), "d".into())
            ]
        );
        u.query_pairs_mut().clear().append_pair("k y", "v&=é");
        assert_eq!(u.query(), Some("k+y=v%26%3D%C3%A9"));
    }

    #[test]
    fn international_hosts_become_punycode_and_refuse_confusables_the_standard_refuses() {
        assert_eq!(
            host("https://bücher.example/").as_deref(),
            Some("xn--bcher-kva.example")
        );
        assert_eq!(
            host("https://例え.テスト/").as_deref(),
            Some("xn--r8jz45g.xn--zckzah")
        );
        assert_eq!(
            host("https://faß.de/").as_deref(),
            Some("xn--fa-hia.de"),
            "nontransitional: sharp s is kept"
        );
        assert_eq!(host("https://EXAMPLE.COM/").as_deref(), Some("example.com"));
        assert!(
            Url::parse("https://a\u{200d}b.example/").is_err(),
            "a joiner without context"
        );
        assert!(Url::parse("https://xn--a/").is_err(), "invalid Punycode");
        assert!(
            Url::parse("https://\u{0301}a.example/").is_err(),
            "a label cannot begin with a combining mark"
        );
    }
}
