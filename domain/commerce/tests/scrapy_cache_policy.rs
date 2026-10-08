//! ECDEV's cache freshness against Scrapy's RFC2616Policy (BSD-3-Clause, scrapy/scrapy at
//! 54f7ed9c), on a grid of 528 response header sets (tools/commerce/scrapy_cache_policy_oracle.py).
//! Compared on the freshness remaining when the response is received. Differences are classified
//! and counted: ECDEV has no heuristic freshness from Last-Modified, applies a 24-hour ceiling
//! and a one-hour default, and treats no-store and no-cache as no freshness.
use ecdev_core::research::{MAX_FRESHNESS_SECONDS, freshness_lifetime};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn classify() -> BTreeMap<String, usize> {
    let f: Value =
        serde_json::from_str(include_str!("fixtures/scrapy-cache-policy-oracle.json")).unwrap();
    assert_eq!(
        f["scrapy_commit"],
        "54f7ed9cccbe19db3fe6cd2252bf3c29326c67d7"
    );
    let mut out: BTreeMap<String, usize> = BTreeMap::new();
    for c in f["cases"].as_array().unwrap() {
        let h = &c["headers"];
        let headers = json!({"cache_control":h.get("Cache-Control"),"expires":h.get("Expires"),"date":h.get("Date"),"age":h.get("Age"),
            "last_modified":h.get("Last-Modified"),"received_at_ms":f["now"].as_i64().unwrap() * 1000});
        let (ours, basis) = freshness_lifetime(&headers);
        let stored = c["scrapy_stored"].as_bool().unwrap();
        // A response Scrapy's policy refuses to store has no cache entry to be fresh.
        let theirs = if stored {
            c["scrapy_remaining"].as_f64().unwrap()
        } else {
            0.0
        };
        let class = if (ours as f64 - theirs).abs() < 1.0 {
            "SAME_REMAINING_FRESHNESS".to_string()
        } else if theirs > ours as f64 && ours == MAX_FRESHNESS_SECONDS {
            "ECDEV_CAPPED_AT_24H".to_string()
        } else if theirs > 0.0 && ours == 0 {
            format!("SCRAPY_FRESH_ECDEV_STALE_{basis}")
        } else if theirs == 0.0 && ours > 0 {
            format!(
                "ECDEV_FRESH_SCRAPY_STALE_{basis}{}",
                if stored {
                    ""
                } else {
                    "_SCRAPY_WOULD_NOT_STORE"
                }
            )
        } else {
            format!("DIFFERENT_LIFETIME_{basis}")
        };
        *out.entry(class).or_default() += 1;
    }
    out
}

#[test]
fn freshness_agrees_with_scrapy_except_where_ecdev_is_bounded_or_stricter() {
    let got = classify();
    let want: BTreeMap<&str, usize> = [
        // The common ground: max-age, Expires minus Date, Age, no-cache, no-store.
        ("SAME_REMAINING_FRESHNESS", 395),
        // Scrapy keeps a one-year max-age (or 90,000 s) fresh; ECDEV never holds a page past
        // 24 hours whatever the server says, because a commerce page changes.
        ("ECDEV_CAPPED_AT_24H", 96),
        // "max-age=abc" is invalid: ECDEV treats it as already stale (revalidate), Scrapy
        // ignores it and falls through to Expires or the heuristic.
        ("SCRAPY_FRESH_ECDEV_STALE_MAX_AGE_INVALID", 13),
        // No freshness information at all: ECDEV holds a capture for its one-hour default;
        // Scrapy stores nothing, or stores it as stale.
        ("ECDEV_FRESH_SCRAPY_STALE_DEFAULT", 8),
        ("ECDEV_FRESH_SCRAPY_STALE_DEFAULT_SCRAPY_WOULD_NOT_STORE", 8),
        // Heuristic freshness: Scrapy keeps a page unchanged for ten days fresh for one tenth
        // of that (a day); ECDEV has no Last-Modified heuristic and uses its default hour.
        ("DIFFERENT_LIFETIME_DEFAULT", 8),
    ]
    .into_iter()
    .collect();
    let got: BTreeMap<&str, usize> = got.iter().map(|(k, v)| (k.as_str(), *v)).collect();
    assert_eq!(got, want);
    assert_eq!(got.values().sum::<usize>(), 528);
}
