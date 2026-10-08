//! A tool result is read by a client with a context window, not only by a program.
//! Live trend captures reached 1.1 to 1.9 MB (343 posts, their clusters, entity
//! recurrence and per-post series), which no client can read. At the tool boundary an
//! oversize result keeps its scalars and shortens its long lists, and says exactly what
//! it dropped and how to read the rest. Nothing is dropped silently, and the frozen
//! snapshot behind it is untouched.
use serde_json::{Map, Value, json};

pub const MAX_RESULT_BYTES: usize = 256 * 1024;
const MIN_ITEMS: usize = 3;
const MIN_REPEATED: usize = 8;
pub const PAGE_LIMIT_MAX: usize = 500;

fn size(v: &Value) -> usize {
    serde_json::to_vec(v).map_or(usize::MAX, |b| b.len())
}

/// Replace a nested list that repeats a top-level list element for element with a
/// reference to it. Live, one 343-id evidence list was repeated 47 times (every score
/// component and every recurring entity), 40 to 55% of the result. Nothing is lost.
fn dedupe(value: &mut Value) -> Map<String, Value> {
    let mut replaced = Map::new();
    let Some(obj) = value.as_object_mut() else {
        return replaced;
    };
    let tops: Vec<(String, Vec<Value>)> = obj
        .iter()
        .filter_map(|(k, v)| {
            Some((
                k.clone(),
                v.as_array().filter(|a| a.len() >= MIN_REPEATED)?.clone(),
            ))
        })
        .collect();
    fn walk(
        v: &mut Value,
        tops: &[(String, Vec<Value>)],
        skip: &str,
        replaced: &mut Map<String, Value>,
    ) {
        match v {
            Value::Array(a) => {
                if let Some((name, _)) = tops.iter().find(|(n, t)| n != skip && t == a) {
                    let n = a.len();
                    *replaced.entry(name.clone()).or_insert(json!(0)) =
                        json!(replaced.get(name).and_then(Value::as_u64).unwrap_or(0) + 1);
                    *v = json!({"same_as":name,"count":n});
                } else {
                    for x in a {
                        walk(x, tops, skip, replaced);
                    }
                }
            }
            Value::Object(o) => {
                for x in o.values_mut() {
                    walk(x, tops, skip, replaced);
                }
            }
            _ => {}
        }
    }
    for (k, v) in obj.iter_mut() {
        match v {
            // A top-level list is a source of references, not a reference itself.
            Value::Array(a) => {
                for x in a {
                    walk(x, &tops, k, &mut replaced);
                }
            }
            other => walk(other, &tops, k, &mut replaced),
        }
    }
    replaced
}

/// Shorten every top-level list to a common length until the result fits `max` bytes.
/// `next` is a ready-made call that reads the full lists in pages.
pub fn bound(mut value: Value, max: usize, next: Option<Value>) -> Value {
    let original = size(&value);
    if original <= max {
        return value;
    }
    let repeats = dedupe(&mut value);
    if size(&value) <= max {
        if let Some(o) = value.as_object_mut() {
            o.insert("output_bound".into(), json!({"state":"REPEATED_LISTS_REFERENCED","max_bytes":max,"original_bytes":original,"referenced":repeats,"note":"A list that repeated a top-level list element for element is shown as {same_as, count}; nothing else changed."}));
        }
        return value;
    }
    let Some(obj) = value.as_object_mut() else {
        return value;
    };
    let totals: Map<String, Value> = obj
        .iter()
        .filter_map(|(k, v)| Some((k.clone(), json!(v.as_array()?.len()))))
        .collect();
    let mut keep = totals.values().filter_map(Value::as_u64).max().unwrap_or(0) as usize;
    let full = obj.clone();
    let mut fits = false;
    while keep > MIN_ITEMS {
        keep = (keep / 2).max(MIN_ITEMS);
        for (k, v) in full.iter() {
            if let Some(a) = v.as_array() {
                obj.insert(
                    k.clone(),
                    Value::Array(a.iter().take(keep).cloned().collect()),
                );
            }
        }
        if size(&Value::Object(obj.clone())) <= max {
            fits = true;
            break;
        }
    }
    let truncated: Map<String, Value> = totals
        .iter()
        .filter(|(_, n)| n.as_u64().unwrap_or(0) as usize > keep)
        .map(|(k, n)| (k.clone(), json!({"total":n,"returned":keep})))
        .collect();
    obj.insert(
        "output_bound".into(),
        json!({"state":if fits {"LISTS_SHORTENED_TO_FIT"} else {"LISTS_SHORTENED_STILL_OVER_BOUND"},"max_bytes":max,"original_bytes":original,"lists_kept_first":keep,"truncated":truncated,"referenced":repeats,"read_the_rest":next,"note":"Only list lengths changed; every value kept is complete. A shortened list is a prefix in the original order, not a ranking by this response."}),
    );
    value
}

/// Project a result to the requested top-level `fields`, paging each list among them.
pub fn project(
    value: &Value,
    fields: &[String],
    offset: usize,
    limit: usize,
) -> Result<Value, String> {
    let obj = value.as_object().ok_or("RESULT_NOT_AN_OBJECT")?;
    let mut out = Map::new();
    let mut paging = Map::new();
    for f in fields {
        let v = obj.get(f).ok_or_else(|| format!("UNKNOWN_FIELD {f}"))?;
        if let Some(a) = v.as_array() {
            let page: Vec<_> = a.iter().skip(offset).take(limit).cloned().collect();
            paging.insert(
                f.clone(),
                json!({"total":a.len(),"offset":offset,"returned":page.len(),"next_offset":if offset + page.len() < a.len() {json!(offset + page.len())} else {Value::Null}}),
            );
            out.insert(f.clone(), Value::Array(page));
        } else {
            out.insert(f.clone(), v.clone());
        }
    }
    out.insert("paging".into(), Value::Object(paging));
    for key in ["snapshot_id", "run_id", "query", "capture_mode"] {
        if let Some(v) = obj.get(key) {
            out.entry(key.to_owned()).or_insert_with(|| v.clone());
        }
    }
    Ok(Value::Object(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn big(n: usize) -> Value {
        json!({"snapshot_id":"s1","count":n,"posts":(0..n).map(|i|json!({"i":i,"text":"x".repeat(200)})).collect::<Vec<_>>(),"small":[1,2,3,4,5,6]})
    }
    #[test]
    fn a_result_within_the_bound_is_untouched() {
        let v = big(10);
        assert_eq!(bound(v.clone(), MAX_RESULT_BYTES, None), v);
    }
    #[test]
    fn an_oversize_list_is_shortened_and_the_loss_is_named() {
        let v = big(5000);
        let out = bound(v.clone(), 64 * 1024, Some(json!({"tool":"t"})));
        assert!(size(&out) <= 64 * 1024);
        assert_eq!(out["output_bound"]["state"], "LISTS_SHORTENED_TO_FIT");
        let kept = out["posts"].as_array().unwrap().len();
        assert!((MIN_ITEMS..5000).contains(&kept));
        assert_eq!(out["output_bound"]["truncated"]["posts"]["total"], 5000);
        assert_eq!(out["output_bound"]["truncated"]["posts"]["returned"], kept);
        assert_eq!(out["posts"][0], v["posts"][0]);
        assert_eq!(out["count"], 5000);
        assert_eq!(out["output_bound"]["read_the_rest"]["tool"], "t");
    }
    #[test]
    fn a_list_already_short_is_not_reported_as_truncated() {
        let out = bound(big(5000), 64 * 1024, None);
        assert!(out["output_bound"]["truncated"].get("small").is_none());
        assert!(out["small"].as_array().unwrap().len() >= 3);
    }
    #[test]
    fn a_repeated_list_becomes_a_reference_before_anything_is_cut() {
        let ids: Vec<_> = (0..400).map(|i| json!(format!("id-{i:030}"))).collect();
        let v = json!({"evidence_ids":ids,"metrics":(0..30).map(|_|json!({"evidence_ids":ids,"x":1})).collect::<Vec<_>>(),"short":[1,2,3]});
        let out = bound(v.clone(), 64 * 1024, None);
        assert_eq!(out["output_bound"]["state"], "REPEATED_LISTS_REFERENCED");
        assert_eq!(out["evidence_ids"], v["evidence_ids"]);
        assert_eq!(out["metrics"].as_array().unwrap().len(), 30);
        assert_eq!(
            out["metrics"][7]["evidence_ids"],
            json!({"same_as":"evidence_ids","count":400})
        );
        assert_eq!(out["output_bound"]["referenced"]["evidence_ids"], 30);
        assert_eq!(out["short"], json!([1, 2, 3]));
    }
    #[test]
    fn scalars_alone_over_the_bound_are_reported_not_hidden() {
        let v = json!({"blob":"y".repeat(100_000),"list":[1,2,3,4,5,6,7,8]});
        let out = bound(v, 1000, None);
        assert_eq!(
            out["output_bound"]["state"],
            "LISTS_SHORTENED_STILL_OVER_BOUND"
        );
        assert_eq!(out["blob"].as_str().unwrap().len(), 100_000);
    }
    #[test]
    fn a_projection_pages_lists_and_reports_the_next_offset() {
        let v = big(10);
        let p = project(&v, &["posts".into()], 4, 3).unwrap();
        assert_eq!(p["posts"].as_array().unwrap().len(), 3);
        assert_eq!(p["posts"][0]["i"], 4);
        assert_eq!(p["paging"]["posts"]["next_offset"], 7);
        let last = project(&v, &["posts".into()], 9, 3).unwrap();
        assert_eq!(last["paging"]["posts"]["next_offset"], Value::Null);
        assert_eq!(p["snapshot_id"], "s1");
        assert_eq!(
            project(&v, &["nope".into()], 0, 1).unwrap_err(),
            "UNKNOWN_FIELD nope"
        );
    }
}
