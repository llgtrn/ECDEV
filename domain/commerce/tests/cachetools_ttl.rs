//! ECDEV's fetch-cache freshness against frozen decisions of the pinned cachetools TTLCache
//! (MIT, tkem/cachetools at 3c082c65): 160 deterministic set/get sequences with an injected
//! timer. ECDEV's cache is persistent and unbounded (no size eviction, deliberately) and adds a
//! stale-if-error window cachetools does not have; the freshness rule itself must agree.
use ecdev_core::research::{
    FETCH_TTL_SECONDS, MAX_FRESHNESS_SECONDS, STALE_IF_ERROR_SECONDS, cache_fresh,
    freshness_lifetime, stale_usable,
};
use serde_json::Value;
use std::collections::BTreeMap;

#[test]
fn cachetools_ttl_freshness_oracle() {
    let o: Value =
        serde_json::from_str(include_str!("fixtures/cachetools-ttl-oracle.json")).unwrap();
    assert_eq!(o["commit_sha"], "3c082c654c2804b9354e4b62dbd2994f1aac464d");
    let (mut gets, mut hits, mut at_expiry) = (0, 0, 0);
    for seq in o["sequences"].as_array().unwrap() {
        let ttl = seq["ttl"].as_u64().unwrap();
        let mut expires: BTreeMap<&str, u64> = BTreeMap::new();
        for e in seq["events"].as_array().unwrap() {
            let key = e["key"].as_str().unwrap();
            let at = e["at"].as_u64().unwrap();
            if e["op"] == "set" {
                expires.insert(key, at + ttl);
                continue;
            }
            let ours = expires.get(key).is_some_and(|x| cache_fresh(*x, at));
            assert_eq!(ours, e["hit"].as_bool().unwrap(), "{seq}");
            gets += 1;
            hits += ours as u32;
            at_expiry += (expires.get(key) == Some(&at)) as u32;
        }
    }
    // Pinned corpus: every get agrees, and expiry-instant reads are exercised.
    assert_eq!((gets, hits, at_expiry), (884, 72, 50));
}

#[test]
fn freshness_and_stale_window_regressions() {
    assert!(cache_fresh(100, 99));
    assert!(!cache_fresh(100, 100), "stale at the expiry instant");
    assert!(stale_usable(100, 100 + STALE_IF_ERROR_SECONDS));
    assert!(!stale_usable(100, 101 + STALE_IF_ERROR_SECONDS));
    assert!(stale_usable(100, 50), "a fresh entry is usable too");
}

#[test]
fn per_entry_freshness_follows_the_response_headers() {
    let f = |h: Value| freshness_lifetime(&h);
    let date = "Sun, 06 Nov 1994 08:49:37 GMT";
    assert_eq!(f(serde_json::json!({})), (FETCH_TTL_SECONDS, "DEFAULT"));
    assert_eq!(
        f(serde_json::json!({"cache_control":"public, max-age=600"})),
        (600, "MAX_AGE")
    );
    assert_eq!(
        f(serde_json::json!({"cache_control":"max-age=600","age":"100"})),
        (500, "MAX_AGE")
    );
    assert_eq!(
        f(serde_json::json!({"cache_control":"Max-Age=\"60\""})),
        (60, "MAX_AGE")
    );
    assert_eq!(
        f(serde_json::json!({"cache_control":"max-age=31536000"})),
        (MAX_FRESHNESS_SECONDS, "MAX_AGE")
    );
    assert_eq!(
        f(serde_json::json!({"cache_control":"max-age=600, no-cache"})),
        (0, "NO_CACHE_REVALIDATE")
    );
    assert_eq!(
        f(serde_json::json!({"cache_control":"no-store","expires":date})),
        (0, "NO_STORE")
    );
    assert_eq!(
        f(serde_json::json!({"cache_control":"max-age=soon"})),
        (0, "MAX_AGE_INVALID")
    );
    // max-age wins over Expires.
    assert_eq!(
        f(
            serde_json::json!({"cache_control":"max-age=5","expires":"Sun, 06 Nov 1994 09:49:37 GMT","date":date})
        ),
        (5, "MAX_AGE")
    );
    assert_eq!(
        f(serde_json::json!({"expires":"Sun, 06 Nov 1994 09:49:37 GMT","date":date})),
        (3600, "EXPIRES")
    );
    assert_eq!(
        f(serde_json::json!({"expires":"Sun, 06 Nov 1994 07:49:37 GMT","date":date})),
        (0, "EXPIRES")
    );
    // An invalid Expires, including "0", means already stale.
    assert_eq!(
        f(serde_json::json!({"expires":"0","date":date})),
        (0, "EXPIRES_INVALID")
    );
    assert_eq!(
        f(serde_json::json!({"expires":"tomorrow","date":date})),
        (0, "EXPIRES_INVALID")
    );
    // Without a Date header the receipt time stands in.
    assert_eq!(
        f(
            serde_json::json!({"expires":"Sun, 06 Nov 1994 08:59:37 GMT","received_at_ms":784111777000_i64})
        ),
        (600, "EXPIRES")
    );
    assert_eq!(
        f(serde_json::json!({"expires":date})),
        (0, "EXPIRES_WITHOUT_DATE")
    );
}
