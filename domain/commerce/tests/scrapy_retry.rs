//! Which statuses ECDEV retries against Scrapy's RetryMiddleware defaults (BSD-3-Clause,
//! scrapy/scrapy at 54f7ed9c), status by status from 100 to 599 (tools/commerce/scrapy_retry_oracle.py).
use ecdev_core::research::retryable_failure;
use serde_json::Value;

#[test]
fn retry_decisions_differ_from_scrapy_in_named_statuses_only() {
    let f: Value = serde_json::from_str(include_str!("fixtures/scrapy-retry-oracle.json")).unwrap();
    let (mut both, mut scrapy_only, mut ecdev_only) = (vec![], vec![], vec![]);
    for (status, v) in f["statuses"].as_object().unwrap() {
        let code: u16 = status.parse().unwrap();
        match (
            v["retried"].as_bool().unwrap(),
            retryable_failure(Some(code), "HTTP"),
        ) {
            (true, true) => both.push(code),
            (true, false) => scrapy_only.push(code),
            (false, true) => ecdev_only.push(code),
            _ => {}
        }
    }
    both.sort();
    ecdev_only.sort();
    assert_eq!(both, [408, 429, 500, 502, 503, 504, 522, 524]);
    // Scrapy retries nothing ECDEV refuses.
    assert!(scrapy_only.is_empty(), "{scrapy_only:?}");
    // ECDEV also retries the other 5xx that can pass (a 500-series error outside Scrapy's list),
    // never the three that cannot: 501, 505, 511.
    let want: Vec<u16> = (500..=599)
        .filter(|c| ![500, 502, 503, 504, 522, 524, 501, 505, 511].contains(c))
        .collect();
    assert_eq!(ecdev_only, want);
    for permanent in [501, 505, 511] {
        assert!(!retryable_failure(Some(permanent), "HTTP"));
    }
    // Network failures are retried whatever the status field says, as in Scrapy's exceptions.
    assert!(
        retryable_failure(None, "FETCH_NETWORK_ERROR") && retryable_failure(None, "DNS_FAILED")
    );
    assert!(
        !retryable_failure(Some(404), "HTTP_STATUS_404") && !retryable_failure(Some(403), "HTTP")
    );
    assert!(!retryable_failure(None, "ROBOTS_DENIED_OR_UNKNOWN"));
}
