//! ECDEV's URL identity against Scrapy's request fingerprint, on rewritten real URLs.
//! The fixture is frozen donor behaviour (tools/commerce/scrapy_url_identity_oracle.py, Scrapy
//! 2.19.0 at 54f7ed9c, w3lib 2.5.0); ECDEV's side is computed here. Divergence is pinned, not
//! hidden: where the two differ, the table below says which is stricter and why.
use ecdev_core::frontier::{UrlPolicy, canonicalize};
use serde_json::Value;
use std::collections::BTreeMap;

fn same(a: &str, b: &str, policy: &UrlPolicy) -> bool {
    match (
        canonicalize(a, None, policy, 0),
        canonicalize(b, None, policy, 0),
    ) {
        (Ok(x), Ok(y)) => x.identity_hash == y.identity_hash,
        // A URL ECDEV refuses has no identity, so it never equals another.
        _ => false,
    }
}

/// kind -> (cases, scrapy_same, ecdev_default_same, ecdev_sorted_same, ecdev_sorted_untracked_same)
fn measure() -> BTreeMap<String, [usize; 5]> {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/scrapy-url-identity-oracle.json")).unwrap();
    let default = UrlPolicy::default();
    let sorted = UrlPolicy {
        sort_query: true,
        ..UrlPolicy::default()
    };
    let untracked = UrlPolicy {
        sort_query: true,
        tracking_parameters: ["utm_source".to_string()].into(),
        ..UrlPolicy::default()
    };
    let mut out: BTreeMap<String, [usize; 5]> = BTreeMap::new();
    for c in fixture["cases"].as_array().unwrap() {
        let (a, b) = (c["a"].as_str().unwrap(), c["b"].as_str().unwrap());
        let row = out
            .entry(c["kind"].as_str().unwrap().to_string())
            .or_default();
        row[0] += 1;
        row[1] += c["scrapy_same"].as_bool().unwrap() as usize;
        row[2] += same(a, b, &default) as usize;
        row[3] += same(a, b, &sorted) as usize;
        row[4] += same(a, b, &untracked) as usize;
    }
    out
}

#[test]
fn ecdev_identity_agrees_with_scrapy_except_where_it_is_stricter_or_more_correct() {
    let m = measure();
    let row = |k: &str| m[k];
    // [cases, scrapy_same, ecdev_default_same, ecdev_sorted_same, ecdev_sorted_untracked_same]
    // Writing a URL differently without changing what it names: ECDEV and Scrapy merge these.
    for kind in [
        "host_case",
        "scheme_case",
        "dot_segment",
        "empty_path",
        "fragment_added",
        "escaped_unreserved_letter",
        "percent_hex_case",
    ] {
        let r = row(kind);
        assert_eq!((r[1], r[2], r[3]), (r[0], r[0], r[0]), "{kind}");
    }
    // Different resources stay different in both systems, under every ECDEV policy.
    for kind in [
        "path_case",
        "parameter_value_changed",
        "parameter_name_changed",
        "repeated_slash",
    ] {
        let r = row(kind);
        assert_eq!((r[1], r[2], r[3], r[4]), (0, 0, 0, 0), "{kind}");
    }
    // An explicit default port names the same origin. ECDEV (WHATWG URL) merges it; Scrapy
    // fingerprints ":443" apart from the bare URL and fetches the page twice.
    let r = row("default_port");
    assert_eq!((r[1], r[2]), (0, r[0]));
    // Query order and a bare "?" are significant to ECDEV by default (a site may treat repeated
    // or ordered parameters as meaningful), merged by Scrapy always; ECDEV merges them when a
    // policy asks. No production caller asks: measured, it would merge nothing (see the
    // study record: 0 of 1,703 page links and 0 of 646 post links were affected).
    for kind in ["query_order", "empty_query_marker"] {
        let r = row(kind);
        assert_eq!((r[1], r[2], r[3]), (r[0], 0, r[0]), "{kind}");
    }
    // Tracking parameters: Scrapy never removes them; ECDEV removes only those a policy names.
    let r = row("tracking_parameter_added");
    assert_eq!((r[1], r[2], r[3], r[4]), (0, 0, 0, r[0]));
    assert!(m.values().map(|r| r[0]).sum::<usize>() >= 500);
}
