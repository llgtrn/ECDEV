//! The generated corpus behind the native URL parser's parity tests: seeded absolute and relative
//! references, random international hosts, and mutator scenarios. It is deterministic, so a digest of
//! the native outputs over it is a stable fingerprint.

use ecdev_core::web_address::Url;

pub struct Rng(pub u64);
impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    pub fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[(self.next() % items.len() as u64) as usize]
    }
}

pub const SCHEMES: &[&str] = &[
    "http",
    "https",
    "HTTP",
    "ftp",
    "ws",
    "wss",
    "foo",
    "web+x",
    "mailto",
    "javascript",
    "data",
    "blob",
    "a",
];
pub const USERINFO: &[&str] = &[
    "",
    "",
    "",
    "u@",
    "u:p@",
    ":p@",
    "u:@",
    "us er:p ss@",
    "%41:%42@",
    "a@b@",
    "é:ü@",
    "@",
    ":@",
    "a:b:c@",
];
pub const HOSTS: &[&str] = &[
    "example.com",
    "EXAMPLE.com",
    "www.example.co.jp",
    "a.b.c.d",
    "example.com.",
    "xn--n3h.com",
    "xn--bcher-kva.example",
    "bücher.example",
    "例え.テスト",
    "☃.com",
    "faß.de",
    "ǆ.com",
    "x_y.com",
    "a-b.com",
    "-a.com",
    "a..b",
    ".a",
    "a.",
    "[::1]",
    "[2001:db8::ff00:42:8329]",
    "[::ffff:192.0.2.1]",
    "[1:2:3:4:5:6:7:8]",
    "[1::]",
    "[::]",
    "[1:0:0:2:0:0:0:3]",
    "[fe80::1%25eth0]",
    "127.0.0.1",
    "0x7f.1",
    "0177.0.0.1",
    "2130706433",
    "1.2.3",
    "1.2.3.4.5",
    "256.1.1.1",
    "1.1.1.256",
    "0xffffffff",
    "4294967296",
    "1.2.3.4.",
    "%31.%32.%33.%34",
    "ex%41mple.com",
    "ex ample.com",
    "exa<mple.com",
    "exa|mple.com",
    "a%2Fb",
    "a%zzb",
    "localhost",
    "LOCALHOST",
    "",
    "ａｂｃ.com",
    "a\u{00ad}b.com",
    "a\u{200b}b.com",
    "\u{0645}\u{062b}\u{0627}\u{0644}.com",
    "ab\u{0301}.com",
    "\u{0301}ab.com",
    "a\u{200c}b.com",
    "日本語.jp",
    "xn--wgv71a119e.jp",
    "xn--a",
    "xn--",
    "xN--n3h.com",
];
pub const PORTS: &[&str] = &[
    "",
    "",
    "",
    ":80",
    ":443",
    ":8080",
    ":0",
    ":65535",
    ":65536",
    ":",
    ":a",
    ":8o",
    ":0080",
    ":21",
    ":00443",
    ":99999999999",
];
pub const PATHS: &[&str] = &[
    "",
    "/",
    "/a",
    "/a/b/c",
    "/a/./b",
    "/a/../b",
    "/a/b/..",
    "/a/b/.",
    "/./",
    "/../",
    "/a//b",
    "/%2e/a",
    "/%2E%2e/a",
    "/a/%2e%2E",
    "/a b",
    "/a\\b",
    "\\a\\b",
    "/é",
    "/日本",
    "/a%2fb",
    "/a%zz",
    "/a?b",
    "/{}",
    "/a|b",
    "/`",
    "/\"",
    "/<>",
    "/a#b",
    "/;a=b",
    "/a:b",
    "/@",
    "//",
    "///",
    "/..%2f..",
    "/a/b/../../..",
];
pub const QUERIES: &[&str] = &[
    "", "?", "?a=b", "?a=b&c=d", "?a b", "?'", "?é=ü", "?a=%41", "?%", "?a+b=c+d", "?{}", "?\"<>",
    "?a=b#", "?a&&b", "?=x", "?x=", "?#",
];
pub const FRAGMENTS: &[&str] = &[
    "", "#", "#f", "#a b", "#é", "#`", "#a#b", "#%41", "#<>\"", "#{}",
];
pub const RELATIVE: &[&str] = &[
    "",
    "x",
    "/x",
    "//x",
    "//x/y",
    "../x",
    "./x",
    "?q",
    "#f",
    "x?q#f",
    "../../../x",
    "/../x",
    "\\x",
    "\\\\x",
    "//x:99/",
    "http:x",
    "http:/x",
    "http://x",
    "https:x",
    "foo:bar",
    "mailto:a@b",
    "javascript:alert(1)",
    " x ",
    "\tx\n",
    "x y",
    "?",
    "#",
    "x/./y/../z",
    "//",
    "///x",
    "/%2e%2e/x",
    "a:b",
    "1:2",
    "//[::1]/",
    "//u@h/",
    ".",
    "..",
];

/// Absolute references, relative references and path-only references against generated bases.
pub fn reference_cases() -> Vec<(String, Option<String>)> {
    let mut rng = Rng(0x2545f4914f6cdd1d);
    let mut cases = vec![];
    for _ in 0..400_000 {
        let scheme = rng.pick(SCHEMES);
        let special = matches!(
            scheme.to_ascii_lowercase().as_str(),
            "http" | "https" | "ftp" | "ws" | "wss"
        );
        let mut path = rng.pick(PATHS);
        while !special && path.contains('\\') {
            path = rng.pick(PATHS);
        }
        let absolute = format!(
            "{}://{}{}{}{}{}{}",
            scheme,
            rng.pick(USERINFO),
            rng.pick(HOSTS),
            rng.pick(PORTS),
            path,
            rng.pick(QUERIES),
            rng.pick(FRAGMENTS)
        );
        cases.push((absolute, None));
        let base_scheme = rng.pick(&["http", "https", "foo", "ftp"]);
        let mut base_path = rng.pick(PATHS);
        while base_scheme == "foo" && base_path.contains('\\') {
            base_path = rng.pick(PATHS);
        }
        let base = format!(
            "{}://{}{}{}{}{}",
            base_scheme,
            rng.pick(HOSTS),
            rng.pick(PORTS),
            base_path,
            rng.pick(QUERIES),
            rng.pick(FRAGMENTS)
        );
        cases.push((rng.pick(RELATIVE).to_string(), Some(base.clone())));
        let mixed = format!(
            "{}{}{}",
            rng.pick(PATHS),
            rng.pick(QUERIES),
            rng.pick(FRAGMENTS)
        );
        cases.push((mixed, Some(base)));
    }
    cases
}

pub fn native_outcome(input: &str, base: Option<&str>) -> String {
    let result = match base {
        Some(b) => Url::parse(b).and_then(|b| b.join(input)),
        None => Url::parse(input),
    };
    match result {
        Ok(u) => summary(&u),
        Err(_) => "ERR".into(),
    }
}

pub fn summary(u: &Url) -> String {
    format!(
        "{}|{:?}|{:?}|{}|{:?}|{:?}|{}|{}|{:?}|{:?}|{}|{:?}",
        u.as_str(),
        u.host_str(),
        u.port(),
        u.path(),
        u.query(),
        u.fragment(),
        u.username(),
        u.password().unwrap_or(""),
        u.port_or_known_default(),
        u.domain(),
        u.origin().ascii_serialization(),
        u.query_pairs()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>(),
    )
}

/// Random international hosts: scripts, combining marks, joiners, bidi controls, fullwidth forms,
/// unassigned code points and random xn-- labels.
pub fn international_hosts() -> Vec<String> {
    let ranges: &[(u32, u32)] = &[
        (0x41, 0x7a),
        (0xa0, 0x24f),
        (0x370, 0x3ff),
        (0x400, 0x4ff),
        (0x590, 0x5ff),
        (0x600, 0x6ff),
        (0x700, 0x74f),
        (0x900, 0x97f),
        (0xe00, 0xe7f),
        (0x1100, 0x11ff),
        (0x1e00, 0x1fff),
        (0x2000, 0x206f),
        (0x2070, 0x20cf),
        (0x2100, 0x218f),
        (0x2460, 0x24ff),
        (0x3000, 0x30ff),
        (0x3130, 0x318f),
        (0x3200, 0x33ff),
        (0x3400, 0x4dbf),
        (0x4e00, 0x9fff),
        (0xac00, 0xd7a3),
        (0xf900, 0xfaff),
        (0xfb00, 0xfdff),
        (0xfe00, 0xfe6f),
        (0xff00, 0xffef),
        (0x1f300, 0x1f6ff),
        (0x20000, 0x2a6df),
        (0x300, 0x36f),
        (0x1ab0, 0x1aff),
        (0x200b, 0x200f),
        (0x202a, 0x202e),
        (0x2060, 0x2064),
        (0xe0000, 0xe007f),
        (0x10000, 0x1007f),
        (0x1d400, 0x1d7ff),
    ];
    let mut rng = Rng(0x9e3779b97f4a7c15);
    let mut hosts = vec![];
    for _ in 0..600_000 {
        let labels = 1 + rng.next() % 3;
        let mut host = String::new();
        for l in 0..labels {
            if l > 0 {
                host.push(if rng.next().is_multiple_of(12) {
                    '\u{3002}'
                } else {
                    '.'
                });
            }
            let n = 1 + rng.next() % 5;
            if rng.next().is_multiple_of(9) {
                host.push_str("xn--");
                for _ in 0..(1 + rng.next() % 8) {
                    host.push(
                        b"abcdefghijklmnopqrstuvwxyz0123456789-"[(rng.next() % 37) as usize]
                            as char,
                    );
                }
                continue;
            }
            for _ in 0..n {
                let (lo, hi) = ranges[(rng.next() % ranges.len() as u64) as usize];
                let cp = lo + (rng.next() % (hi - lo + 1) as u64) as u32;
                if let Some(c) = char::from_u32(cp) {
                    host.push(c);
                }
            }
        }
        hosts.push(host);
    }
    hosts
}

/// One scripted sequence of mutator calls on a base URL.
pub enum Op {
    SetQuery(&'static str),
    SetFragment(&'static str),
    SetPath(&'static str),
    SetUsername(&'static str),
    ClearQuery,
    ClearFragment,
    AppendPairs(usize),
    ClearPairs,
    Segments,
}

pub const PAIRS: &[(&str, &str)] = &[
    ("k", "v"),
    ("a b", "é&=+"),
    ("", ""),
    ("ünï", "cødé"),
    ("x", "a*b-c.d_e~f"),
];

pub fn mutator_scenarios() -> Vec<(&'static str, Vec<Op>)> {
    let bases = [
        "https://example.com/a/b?x=1#f",
        "http://u:p@host:8080/p/q/",
        "https://example.com",
        "https://example.com/?",
        "http://[::1]:81/x",
        "ftp://h/a;b",
        "https://例え.テスト/日本?語=値",
        "foo://h/p?q#f",
        "https://example.com/a%2Fb/c",
    ];
    let queries = ["", "a=b", "a b", "é", "'", "\"<>#", "a=1&b=2", "%41"];
    let fragments = ["", "f", "a b", "é", "`", "#x", "<>\""];
    let paths = [
        "/",
        "/a",
        "/a/b/../c",
        "/a b",
        "/é",
        "/a?b",
        "/a#b",
        "/./x",
        "x/y",
        "",
        "//",
    ];
    let users = ["", "u", "u@v", "é", "a:b", "a b"];
    let mut out = vec![];
    for base in bases {
        for q in queries {
            for f in fragments {
                for p in paths {
                    for user in users {
                        out.push((
                            base,
                            vec![
                                Op::SetQuery(q),
                                Op::SetFragment(f),
                                Op::SetPath(p),
                                Op::SetUsername(user),
                                Op::ClearQuery,
                                Op::ClearFragment,
                            ],
                        ));
                    }
                }
            }
        }
        for n in 0..=PAIRS.len() {
            out.push((base, vec![Op::AppendPairs(n), Op::ClearPairs]));
        }
        out.push((base, vec![Op::Segments]));
    }
    out
}
