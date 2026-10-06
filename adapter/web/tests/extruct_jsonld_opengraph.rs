//! ECDEV's native JSON-LD and OpenGraph reading against frozen outputs of the pinned extruct
//! (BSD-3-Clause, scrapinghub/extruct at dc3bf7d2). Scripts that are valid JSON must yield the
//! same items; scripts extruct only reads after repairing them (comment/CDATA framing, JavaScript
//! escapes, guessed quotes, trailing junk, HTML-encoded syntax) are refused by ECDEV and
//! recorded as INVALID_JSON_LD, because a repaired value is a guess, not an observed assertion.
use serde_json::Value;

fn oracle() -> Value {
    serde_json::from_str(include_str!("fixtures/extruct-jsonld-opengraph.json")).unwrap()
}

/// extruct's item view of one decoded script: an array contributes its elements, an object
/// itself, a scalar nothing; empty items are dropped.
fn items(value: &Value) -> Vec<Value> {
    let all = match value {
        Value::Array(a) => a.clone(),
        Value::Object(_) => vec![value.clone()],
        _ => vec![],
    };
    all.into_iter()
        .filter(|v| match v {
            Value::Object(o) => !o.is_empty(),
            Value::Array(a) => !a.is_empty(),
            Value::Null | Value::Bool(false) => false,
            Value::String(s) => !s.is_empty(),
            _ => true,
        })
        .collect()
}

/// JSON equality where numbers compare by value: ECDEV keeps the exact source literal (`1e3`),
/// Python's decoder yields a float (`1000.0`).
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same(p, q))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| same(v, w)))
        }
        _ => a == b,
    }
}

fn page(case: &Value) -> Value {
    ecdev_web::extract(case["html"].as_str().unwrap(), "https://shop.example/page").unwrap()
}

#[test]
fn extruct_jsonld_oracle_strict_partition_matches() {
    let mut compared = 0;
    let mut mime_case = 0;
    for case in oracle()["cases"].as_array().unwrap() {
        let strict: Vec<bool> = case["ld_strict_valid"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_bool().unwrap())
            .collect();
        if !strict.iter().all(|v| *v) {
            continue;
        }
        let out = page(case);
        if strict.is_empty() {
            // MIME types are case-insensitive; extruct's XPath matches only the lowercase
            // spelling, ECDEV reads `Application/LD+JSON` too (divergence counted below).
            if !out["structured_data"].as_array().unwrap().is_empty() {
                mime_case += 1;
            }
            continue;
        }
        let got: Vec<Value> = out["structured_data"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|s| items(&s["value"]))
            .collect();
        let got = Value::Array(got);
        assert!(
            same(&got, &case["jsonld"]),
            "{}: {got} != {}",
            case["name"],
            case["jsonld"]
        );
        assert!(
            !out["extraction_errors"]
                .as_array()
                .unwrap()
                .contains(&Value::from("INVALID_JSON_LD")),
            "{}",
            case["name"]
        );
        compared += 1;
    }
    assert!(compared >= 15, "{compared}");
    assert_eq!(
        mime_case, 1,
        "case-variant JSON-LD MIME types in the frozen corpus"
    );
}

#[test]
fn extruct_jsonld_repair_partition_is_refused_not_guessed() {
    let mut refused = 0;
    for case in oracle()["cases"].as_array().unwrap() {
        let strict: Vec<bool> = case["ld_strict_valid"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_bool().unwrap())
            .collect();
        let invalid = strict.iter().filter(|v| !**v).count();
        if invalid == 0 {
            continue;
        }
        let out = page(case);
        let errors = out["extraction_errors"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| *e == "INVALID_JSON_LD")
            .count();
        assert_eq!(errors, invalid, "{}", case["name"]);
        assert_eq!(
            out["structured_data"].as_array().unwrap().len(),
            strict.len() - invalid,
            "{}",
            case["name"]
        );
        refused += invalid;
    }
    assert!(refused >= 10, "{refused}");
}

#[test]
fn extruct_opengraph_oracle_head_property_partition_matches() {
    let mut compared = 0;
    let mut bare = 0;
    for case in oracle()["cases"].as_array().unwrap() {
        if case["opengraph_comparable"] != true {
            continue;
        }
        let out = page(case);
        let got: Vec<Value> = out["page_metadata"]["open_graph"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|o| {
                o["selector"]
                    .as_str()
                    .unwrap()
                    .starts_with("meta[property=")
            })
            .filter(|o| {
                matches!(
                    o["property"].as_str().unwrap().split(':').next(),
                    Some("og" | "product")
                )
            })
            .map(|o| serde_json::json!([o["property"], o["value"]]))
            .collect();
        // A bare `og`/`product` property names no OpenGraph property; extruct keeps it because it
        // only checks the namespace before the first colon. ECDEV requires `prefix:` (divergence
        // counted below).
        let want: Vec<Value> = case["opengraph_og_product"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p[0].as_str().unwrap().contains(':'))
            .cloned()
            .collect();
        bare += case["opengraph_og_product"].as_array().unwrap().len() - want.len();
        assert_eq!(Value::Array(got), Value::Array(want), "{}", case["name"]);
        compared += 1;
    }
    assert!(compared >= 60, "{compared}");
    assert_eq!(
        bare, 1,
        "bare-namespace OpenGraph properties in the frozen corpus"
    );
}
