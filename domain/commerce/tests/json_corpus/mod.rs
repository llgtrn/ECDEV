//! Deterministic JSON corpora shared by the parity proof and the differential run that froze it:
//! the JSONTestSuite files, generated documents with random escapes and whitespace, byte-level
//! mutations of them, and floating-point bit patterns.
#![allow(dead_code)]

use ecdev_core::sha256::Sha256;

pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545F4914F6CDD1D)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    pub fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
        &v[self.below(v.len() as u64) as usize]
    }
}

/// (file name, bytes) of the JSONTestSuite parsing files, sorted by name.
pub fn suite() -> Vec<(String, Vec<u8>)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/jsontestsuite");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|p| {
            (
                p.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read(&p).unwrap(),
            )
        })
        .collect()
}

const WS: &[&str] = &["", "", "", " ", "\n", "\t", "\r\n", "  "];

fn string_text(rng: &mut Rng) -> String {
    let pieces: &[&str] = &[
        "a",
        "Z",
        "0",
        " ",
        "key",
        "ünï",
        "日本語",
        "😀",
        "\u{10FFFF}",
        "\u{7f}",
        "\u{a0}",
        "\u{2028}",
        "\u{feff}",
        "\\\"",
        "\\\\",
        "\\/",
        "\\b",
        "\\f",
        "\\n",
        "\\r",
        "\\t",
        "\\u0000",
        "\\u001f",
        "\\u0041",
        "\\u00e9",
        "\\u20AC",
        "\\ud83d\\ude00",
        "\\uD83D\\uDE00",
        "\\u007f",
        "\\uFFFF",
        "'",
        "<",
        ">",
        "&",
        "{",
        "[",
        ",",
        ":",
    ];
    let n = rng.below(6);
    let mut s = String::from("\"");
    for _ in 0..n {
        s.push_str(rng.pick(pieces));
    }
    s.push('"');
    s
}

fn number_text(rng: &mut Rng) -> String {
    let ints: &[&str] = &[
        "0",
        "-0",
        "1",
        "-1",
        "7",
        "10",
        "123456789",
        "9007199254740993",
        "-9223372036854775808",
        "9223372036854775807",
        "18446744073709551615",
        "18446744073709551616",
        "123456789012345678901234567890",
        "-123456789012345678901234567890",
    ];
    let fracs: &[&str] = &[
        "",
        "",
        "",
        ".0",
        ".5",
        ".10",
        ".000",
        ".123456789012345678901234567890",
        ".5E3",
    ];
    let exps: &[&str] = &[
        "", "", "", "e0", "E5", "e-5", "e+10", "e999", "E-999", "e308", "e-324", "e1000000",
    ];
    format!("{}{}{}", rng.pick(ints), rng.pick(fracs), rng.pick(exps))
}

fn value_text(rng: &mut Rng, depth: usize, out: &mut String) {
    let ws = |rng: &mut Rng| *rng.pick(WS);
    let kind = if depth >= 5 {
        rng.below(5)
    } else {
        rng.below(8)
    };
    match kind {
        0 => out.push_str("null"),
        1 => out.push_str("true"),
        2 => out.push_str("false"),
        3 | 4 => {
            if rng.below(2) == 0 {
                out.push_str(&string_text(rng))
            } else {
                out.push_str(&number_text(rng))
            }
        }
        5 | 6 => {
            out.push('[');
            let n = rng.below(5);
            for i in 0..n {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(ws(rng));
                value_text(rng, depth + 1, out);
                out.push_str(ws(rng));
            }
            out.push(']');
        }
        _ => {
            out.push('{');
            let n = rng.below(5);
            for i in 0..n {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(ws(rng));
                out.push_str(&string_text(rng));
                out.push_str(ws(rng));
                out.push(':');
                out.push_str(ws(rng));
                value_text(rng, depth + 1, out);
                out.push_str(ws(rng));
            }
            out.push('}');
        }
    }
}

pub const GENERATED: usize = 20000;
pub const MUTATED: usize = 60000;
pub const FLOATS: usize = 400000;

/// Valid documents: random structure, whitespace, escape spellings and number spellings.
pub fn generated() -> Vec<Vec<u8>> {
    let mut rng = Rng(0x1234_5678_9ABC_DEF1);
    (0..GENERATED)
        .map(|_| {
            let mut s = String::new();
            s.push_str(rng.pick(WS));
            value_text(&mut rng, 0, &mut s);
            s.push_str(rng.pick(WS));
            s.into_bytes()
        })
        .collect()
}

/// Generated documents with a few random byte edits: mostly invalid, sometimes still valid.
pub fn mutated() -> Vec<Vec<u8>> {
    let base = generated();
    let mut rng = Rng(0x0BAD_C0DE_F00D_1234);
    let junk: &[u8] = b"{}[],:\"\\ -+.eE0123456789tfnu/\x00\x1f\x7f\xc3\xff\x80\xed\xa0";
    (0..MUTATED)
        .map(|i| {
            let mut doc = base[i % base.len()].clone();
            for _ in 0..1 + rng.below(3) {
                if doc.is_empty() {
                    break;
                }
                let at = rng.below(doc.len() as u64) as usize;
                match rng.below(5) {
                    0 => {
                        doc.remove(at);
                    }
                    1 => doc.insert(at, *rng.pick(junk)),
                    2 => doc[at] = *rng.pick(junk),
                    3 => doc.truncate(at),
                    _ => {
                        let b = doc[at];
                        doc.insert(at, b)
                    }
                }
            }
            doc
        })
        .collect()
}

/// Nesting right around the limit, arrays, objects and a mix.
pub fn nesting() -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for depth in [1usize, 2, 100, 125, 126, 127, 128, 129, 130, 200, 1000] {
        out.push(format!("{}{}", "[".repeat(depth), "]".repeat(depth)).into_bytes());
        out.push(format!("{}1{}", "[".repeat(depth), "]".repeat(depth)).into_bytes());
        out.push(
            format!(
                "{}{}",
                "{\"a\":".repeat(depth),
                "1".to_string() + &"}".repeat(depth)
            )
            .into_bytes(),
        );
        let mix: String = (0..depth)
            .map(|i| if i % 2 == 0 { "[" } else { "{\"k\":" })
            .collect();
        let close: String = (0..depth)
            .rev()
            .map(|i| if i % 2 == 0 { "]" } else { "}" })
            .collect();
        out.push(format!("{mix}0{close}").into_bytes());
    }
    out
}

/// Finite f64 values from random bit patterns, plus the edge cases.
pub fn floats() -> Vec<f64> {
    let mut rng = Rng(0xF10A_7000_0000_0001);
    let mut out = vec![
        0.0,
        -0.0,
        1.0,
        -1.0,
        0.1,
        0.2,
        0.3,
        1e15,
        1e16,
        1e17,
        1e21,
        1e22,
        1e23,
        1e-5,
        1e-6,
        1e-7,
        123456789.0,
        1.0 / 3.0,
        2.0 / 3.0,
        f64::MAX,
        f64::MIN,
        f64::MIN_POSITIVE,
        f64::EPSILON,
        5e-324,
        9007199254740992.0,
        9007199254740993.0,
        4.35,
        0.5,
        100.0,
        1e100,
        1.5e-10,
    ];
    while out.len() < FLOATS {
        let f = f64::from_bits(rng.next());
        if f.is_finite() {
            out.push(f);
        }
        // Decimal-looking values: what prices and rates look like.
        let cents = rng.below(100_000_000) as f64 / 100.0;
        out.push(cents);
        out.push(cents / 7.0);
    }
    out.truncate(FLOATS);
    out
}

/// A digest over (id, text) pairs, each separated unambiguously.
pub fn digest<'a>(parts: impl IntoIterator<Item = (String, &'a [u8])>) -> String {
    let mut h = Sha256::new();
    for (id, body) in parts {
        h.update(id.as_bytes());
        h.update(b"\n");
        h.update((body.len() as u64).to_le_bytes());
        h.update(body);
        h.update(b"\x00");
    }
    format!("{:x}", h.finalize())
}

// ---- typed documents ---------------------------------------------------------------------------

use ecdev_core::json;
use ecdev_core::json::Value;

pub const TYPED_OUTER: usize = 30000;
pub const TYPED_LOOSE: usize = 5000;
pub const TYPED_SHOCK: usize = 5000;

fn typed_inner(rng: &mut Rng) -> Value {
    let strings = ["", "x", "日本", "a\"b"];
    let mut m = json::Map::new();
    m.insert("id".into(), json!(*rng.pick(&strings)));
    if rng.below(2) == 0 {
        m.insert("tags".into(), json!(["t", "u"]));
    }
    if rng.below(2) == 0 {
        m.insert("score".into(), json!(rng.below(1000) as f64 / 8.0));
    }
    Value::Object(m)
}

fn typed_instance(rng: &mut Rng) -> Value {
    let strings = ["", "x", "日本", "a\"b"];
    let mut m = json::Map::new();
    m.insert("name".into(), json!(*rng.pick(&strings)));
    m.insert(
        "market".into(),
        json!(*rng.pick(&["AMAZON_JP", "RAKUTEN", "OTHER"])),
    );
    if rng.below(2) == 0 {
        m.insert("limit".into(), json!(rng.below(5000)));
    }
    if rng.below(2) == 0 {
        m.insert("small".into(), json!(rng.below(300)));
    }
    if rng.below(2) == 0 {
        m.insert("signed".into(), json!(rng.below(70000) as i64 - 35000));
    }
    if rng.below(2) == 0 {
        m.insert("big".into(), json!(rng.next()));
    }
    if rng.below(2) == 0 {
        m.insert("ratio".into(), json!(rng.below(100000) as f64 / 3.0));
    }
    if rng.below(2) == 0 {
        m.insert("flag".into(), json!(rng.below(2) == 0));
    }
    if rng.below(2) == 0 {
        m.insert("inner".into(), typed_inner(rng));
    }
    m.insert(
        "items".into(),
        Value::Array((0..rng.below(3)).map(|_| typed_inner(rng)).collect()),
    );
    if rng.below(2) == 0 {
        m.insert(
            "shocks".into(),
            json!([{"kind":"Nothing"},{"kind":"Price","at":3,"bps":-250},{"kind":"Stock","at":9}]),
        );
    }
    if rng.below(2) == 0 {
        m.insert("pair".into(), json!(["p", 5]));
    }
    if rng.below(2) == 0 {
        m.insert("table".into(), json!({"a": 1, "b": 2}));
    }
    if rng.below(2) == 0 {
        m.insert("note".into(), json!("n"));
    }
    if rng.below(2) == 0 {
        m.insert("raw".into(), json!({"z": [1, 2.50, "x", null], "a": 1e3}));
    }
    Value::Object(m)
}

/// Replaces, removes or adds something at a random place.
fn typed_mutate(rng: &mut Rng, v: &mut Value) {
    let replacements = [
        json!(null),
        json!(true),
        json!(-1),
        json!(256),
        json!(70000),
        json!(1.5),
        json!(3.0),
        json!(1e30),
        json!(18446744073709551615u64),
        json!(18446744073709551616.0),
        json!("str"),
        json!("AMAZON_JP"),
        json!("nope"),
        json!([]),
        json!([1, 2]),
        json!({}),
        json!({"kind": "Nothing"}),
        json!({"kind": "Price", "at": 1}),
        json!({"kind": "Zap"}),
    ];
    let mut pointer = String::new();
    let mut cursor = v.clone();
    for _ in 0..rng.below(3) {
        let next = match &cursor {
            Value::Object(m) if !m.is_empty() => {
                let k = m
                    .keys()
                    .nth(rng.below(m.len() as u64) as usize)
                    .unwrap()
                    .clone();
                pointer.push('/');
                pointer.push_str(&k);
                m.get(&k).cloned()
            }
            Value::Array(a) if !a.is_empty() => {
                let i = rng.below(a.len() as u64) as usize;
                pointer.push_str(&format!("/{i}"));
                a.get(i).cloned()
            }
            _ => None,
        };
        match next {
            Some(n) => cursor = n,
            None => break,
        }
    }
    let target = v.pointer_mut(&pointer).unwrap();
    match target {
        Value::Object(m) if !m.is_empty() => match rng.below(3) {
            0 => {
                let k = m
                    .keys()
                    .nth(rng.below(m.len() as u64) as usize)
                    .unwrap()
                    .clone();
                m.remove(&k);
            }
            1 => {
                m.insert("unknown_field".into(), json!(1));
            }
            _ => {
                let k = m
                    .keys()
                    .nth(rng.below(m.len() as u64) as usize)
                    .unwrap()
                    .clone();
                m.insert(k, rng.pick(&replacements).clone());
            }
        },
        other => *other = rng.pick(&replacements).clone(),
    }
}

/// Documents for the typed proofs: (type name, document). Valid instances with up to two random
/// mutations each.
pub fn typed_documents() -> Vec<(&'static str, Value)> {
    let mut rng = Rng(0x00C0_FFEE_0000_0042);
    let mut docs = Vec::new();
    for _ in 0..TYPED_OUTER {
        let mut v = typed_instance(&mut rng);
        for _ in 0..rng.below(3) {
            typed_mutate(&mut rng, &mut v);
        }
        docs.push(("outer", v));
    }
    for _ in 0..TYPED_LOOSE {
        let mut m = json::Map::new();
        for (k, v) in [
            ("a", json!(rng.below(9))),
            ("b", json!("s")),
            ("c", json!([1, 2])),
            ("d", json!(1)),
        ] {
            if rng.below(2) == 0 {
                m.insert(k.into(), v);
            }
        }
        let mut v = Value::Object(m);
        if rng.below(2) == 0 {
            typed_mutate(&mut rng, &mut v);
        }
        docs.push(("loose", v));
    }
    for _ in 0..TYPED_SHOCK {
        let mut v = json!({"kind": *rng.pick(&["Nothing", "Price", "Stock", "Zap"]), "at": rng.below(10), "bps": -5, "units": 3});
        for _ in 0..rng.below(2) {
            typed_mutate(&mut rng, &mut v);
        }
        docs.push(("shock", v));
    }
    for s in [
        "AMAZON_JP",
        "RAKUTEN",
        "OTHER",
        "amazon_jp",
        "AmazonJp",
        "",
        "X",
    ] {
        docs.push(("market", json!(s)));
    }
    for v in [json!(1), json!(null), json!([]), json!({})] {
        docs.push(("market", v));
    }
    docs
}

// ---- native results in the digest format -------------------------------------------------------

fn native_profile(v: &Value, out: &mut String) {
    match v {
        Value::Number(n) => out.push_str(&format!(
            "[{}|{:?}|{:?}|{:?}|{}|{}|{}]",
            n,
            n.as_i64(),
            n.as_u64(),
            n.as_f64(),
            n.is_i64(),
            n.is_u64(),
            n.is_f64()
        )),
        Value::Array(a) => a.iter().for_each(|x| native_profile(x, out)),
        Value::Object(m) => m.values().for_each(|x| native_profile(x, out)),
        _ => {}
    }
}

/// `err`, or `ok:` compact text, pretty text and every number's accessors, one per line.
pub fn native_result(bytes: &[u8]) -> String {
    match json::parse_slice(bytes) {
        Ok(v) => {
            let mut p = String::new();
            native_profile(&v, &mut p);
            format!("ok:{}\n{}\n{}", v, format_args!("{v:#}"), p)
        }
        Err(_) => "err".into(),
    }
}
