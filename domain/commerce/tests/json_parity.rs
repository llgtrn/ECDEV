//! The native JSON parser, writer and derives against frozen results of the retired serde_json
//! (arbitrary_precision) and serde derives; see `fixtures/json-differential-digests.json`.
mod json_corpus;
use ecdev_core::json::{self, FromJson, ToJson, Value};
use json_corpus::*;

fn fixture() -> Value {
    json::from_str(include_str!("fixtures/json-differential-digests.json")).unwrap()
}

fn untyped(name: &str, docs: Vec<(String, Vec<u8>)>) {
    let f = fixture();
    let section = &f["untyped"][name];
    assert_eq!(
        section["documents"].as_u64().unwrap() as usize,
        docs.len(),
        "{name}"
    );
    let (mut accepted, mut rejected) = (0, 0);
    let rows: Vec<(String, String)> = docs
        .iter()
        .map(|(id, bytes)| {
            let r = native_result(bytes);
            if r == "err" {
                rejected += 1
            } else {
                accepted += 1
            }
            (id.clone(), r)
        })
        .collect();
    assert_eq!(
        (accepted, rejected),
        (
            section["accepted"].as_u64().unwrap(),
            section["rejected"].as_u64().unwrap()
        ),
        "{name}"
    );
    assert_eq!(
        digest(rows.iter().map(|(id, r)| (id.clone(), r.as_bytes()))),
        section["digest"],
        "{name}"
    );
}

#[test]
fn the_jsontestsuite_reproduces_the_frozen_results() {
    untyped("suite", suite());
}

#[test]
fn generated_documents_reproduce_the_frozen_results() {
    untyped(
        "generated",
        generated()
            .into_iter()
            .enumerate()
            .map(|(i, d)| (i.to_string(), d))
            .collect(),
    );
}

#[test]
fn mutated_documents_reproduce_the_frozen_results() {
    untyped(
        "mutated",
        mutated()
            .into_iter()
            .enumerate()
            .map(|(i, d)| (i.to_string(), d))
            .collect(),
    );
}

#[test]
fn nesting_limit_reproduces_the_frozen_results() {
    untyped(
        "nesting",
        nesting()
            .into_iter()
            .enumerate()
            .map(|(i, d)| (i.to_string(), d))
            .collect(),
    );
}

#[test]
fn floats_print_as_the_frozen_text() {
    let f = fixture();
    let section = &f["untyped"]["floats"];
    let floats = floats();
    assert_eq!(
        section["documents"].as_u64().unwrap() as usize,
        floats.len()
    );
    let rows: Vec<(String, String)> = floats
        .iter()
        .enumerate()
        .map(|(i, v)| (i.to_string(), Value::from(*v).to_string()))
        .collect();
    assert_eq!(
        digest(rows.iter().map(|(id, r)| (id.clone(), r.as_bytes()))),
        section["digest"]
    );
}

fn fallback_limit() -> u32 {
    7
}

#[derive(Debug, Clone, PartialEq, ToJson, FromJson)]
#[json(deny_unknown_fields)]
struct Inner {
    id: String,
    #[json(default)]
    tags: Vec<String>,
    score: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, ToJson, FromJson)]
#[json(rename_all = "SCREAMING_SNAKE_CASE")]
enum Market {
    AmazonJp,
    Rakuten,
    Other,
}

#[derive(Debug, Clone, PartialEq, ToJson, FromJson)]
#[json(tag = "kind", deny_unknown_fields)]
enum Shock {
    Nothing,
    Price { at: u32, bps: i64 },
    Stock { at: u32, units: Option<u32> },
}

#[derive(Debug, Clone, PartialEq, ToJson, FromJson)]
#[json(deny_unknown_fields)]
struct Outer {
    name: String,
    market: Market,
    #[json(default = "fallback_limit")]
    limit: u32,
    small: Option<u8>,
    signed: Option<i16>,
    big: Option<u64>,
    ratio: Option<f64>,
    flag: Option<bool>,
    inner: Option<Inner>,
    items: Vec<Inner>,
    shocks: Option<Vec<Shock>>,
    pair: Option<(String, u32)>,
    table: Option<std::collections::BTreeMap<String, u32>>,
    #[json(default, skip_if = "Option::is_none")]
    note: Option<String>,
    raw: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Default, ToJson, FromJson)]
#[json(default)]
struct Loose {
    a: u32,
    b: String,
    c: Vec<u8>,
}

fn row<N: FromJson + ToJson>(doc: &Value) -> Result<String, ()> {
    json::from_value::<N>(doc.clone())
        .map(|v| format!("ok:{}", v.to_json()))
        .map_err(|_| ())
}

fn typed(name: &str, docs: &[(usize, &Value)], decode: fn(&Value) -> Result<String, ()>) {
    let f = fixture();
    let section = &f["typed"][name];
    assert_eq!(
        section["documents"].as_u64().unwrap() as usize,
        docs.len(),
        "{name}"
    );
    let stricter: Vec<usize> = section["stricter_than_serde"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as usize)
        .collect();
    let mut rows = Vec::new();
    for (index, doc) in docs {
        let r = decode(doc);
        if stricter.contains(index) {
            assert!(
                r.is_err(),
                "{name} {index}: serde accepts, the native derive must refuse"
            );
        } else {
            rows.push((index.to_string(), r.unwrap_or_else(|()| "err".into())));
        }
    }
    assert_eq!(
        digest(rows.iter().map(|(id, r)| (id.clone(), r.as_bytes()))),
        section["digest"],
        "{name}"
    );
}

#[test]
fn derived_types_reproduce_the_frozen_serde_results() {
    let all = typed_documents();
    let pick = |name: &str| -> Vec<(usize, &Value)> {
        all.iter()
            .enumerate()
            .filter(|(_, (t, _))| *t == name)
            .map(|(i, (_, v))| (i, v))
            .collect()
    };
    typed("outer", &pick("outer"), row::<Outer>);
    typed("loose", &pick("loose"), row::<Loose>);
    typed("shock", &pick("shock"), row::<Shock>);
    typed("market", &pick("market"), row::<Market>);
}
