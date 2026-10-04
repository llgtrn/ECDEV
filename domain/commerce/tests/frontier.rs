use ecdev_core::frontier::{CrawlLimits, Frontier, UrlPolicy, canonicalize};
use serde_json::json;
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[test]
fn locked_crawlee_queue_oracle() {
    let oracle: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/crawlee-frontier.json")).unwrap();
    assert_eq!(
        oracle["commit_sha"],
        "438e3419626bd070f8984566bfb86ab9355f55d6"
    );
    assert_eq!(
        oracle["oracle"],
        "LOCKED_REQUEST_QUEUE_BACKEND_WITH_PINNED_NATIVE_BINARY"
    );
    assert_eq!(oracle["cases"].as_array().unwrap().len(), 46);
    for case in oracle["cases"].as_array().unwrap() {
        let database = path();
        let mut frontier = Frontier::open(&database).unwrap();
        let mut config = limits();
        config.max_pages = 1000;
        config.max_urls = 1000;
        config.global_concurrency = 100;
        config.per_origin_concurrency = 100;
        config.lease_ms = 180_000;
        config.max_retries = 20;
        config.backoff_ms = 0;
        config.deadline_ms = 1_000_000;
        frontier.create_run("oracle", &config).unwrap();
        let mut now = 1;
        let mut front_rank = 0;
        let mut priorities = BTreeMap::<String, i64>::new();
        let mut leases = BTreeMap::new();
        let mut actual = Vec::new();
        // The donor boolean forefront maps to positive native priority ranks, LIFO.
        // Normal items use priority zero. Canonical URL is an explicit donor uniqueKey.
        for step in case["steps"].as_array().unwrap() {
            now += 10;
            let address = format!(
                "https://queue.example/{}",
                step["key"].as_str().unwrap_or("")
            );
            let result = match step["op"].as_str().unwrap() {
                "add" => {
                    let priority = if step["forefront"] == true {
                        front_rank += 1;
                        front_rank
                    } else {
                        0
                    };
                    let identity =
                        canonicalize(&address, None, &UrlPolicy::default(), now).unwrap();
                    let added = frontier.enqueue("oracle", &identity, 0, priority).unwrap();
                    if added {
                        priorities.insert(address.clone(), priority);
                    }
                    let handled = frontier
                        .captures("oracle")
                        .unwrap()
                        .iter()
                        .any(|p| p["url"] == address);
                    json!({"added":added,"handled":handled})
                }
                "fetch" => match frontier.lease("oracle", now).unwrap() {
                    Some(lease) => {
                        let address = lease.canonical_url.clone();
                        leases.insert(address.clone(), lease);
                        json!(address)
                    }
                    None => serde_json::Value::Null,
                },
                "handle" => json!(
                    frontier
                        .complete(leases.get(&address).unwrap(), now, &json!({"url":address}))
                        .is_ok()
                ),
                "reclaim" => {
                    let current = priorities[&address];
                    let priority = if step["forefront"] == true {
                        if current > 0 {
                            current
                        } else {
                            front_rank += 1;
                            front_rank
                        }
                    } else {
                        0
                    };
                    let success = frontier
                        .reclaim(leases.get(&address).unwrap(), now, Some(priority))
                        .is_ok();
                    if success {
                        priorities.insert(address, priority);
                    }
                    json!(success)
                }
                "advance" => {
                    now += step["millis"].as_i64().unwrap();
                    serde_json::Value::Null
                }
                "reopen" => {
                    frontier = Frontier::open(&database).unwrap();
                    serde_json::Value::Null
                }
                "status" => {
                    let status = frontier.status("oracle").unwrap();
                    let states = &status["states"];
                    let total: u64 = states
                        .as_object()
                        .unwrap()
                        .values()
                        .map(|n| n.as_u64().unwrap())
                        .sum();
                    let handled = states["HANDLED"].as_u64().unwrap();
                    json!({"total":total,"handled":handled,"pending":total-handled,"empty":states["PENDING"]==0&&states["RETRYABLE"]==0,"finished":total==handled})
                }
                other => panic!("unknown oracle operation {other}"),
            };
            actual.push(result);
        }
        assert_eq!(
            json!(actual),
            case["expected"],
            "donor trace {}",
            case["name"]
        );
    }
}

#[test]
fn reclaim_is_fenced_and_cannot_evade_retry_exhaustion() {
    let mut f = Frontier::open(&path()).unwrap();
    let mut config = limits();
    config.max_retries = 1;
    f.create_run("reclaim", &config).unwrap();
    f.enqueue("reclaim", &url("https://example.org/a"), 0, 0)
        .unwrap();
    f.enqueue("reclaim", &url("https://example.org/b"), 0, 0)
        .unwrap();
    let a = f.lease("reclaim", 2).unwrap().unwrap();
    f.reclaim(&a, 3, None).unwrap();
    assert!(f.reclaim(&a, 3, Some(100)).is_err());
    let b = f.lease("reclaim", 4).unwrap().unwrap();
    assert!(b.canonical_url.ends_with("/b"));
    f.complete(&b, 5, &json!({})).unwrap();
    let retry = f.lease("reclaim", 6).unwrap().unwrap();
    assert_eq!(retry.attempts, 2);
    f.reclaim(&retry, 7, None).unwrap();
    assert_eq!(f.status("reclaim").unwrap()["states"]["FAILED"], 1);
    assert!(f.lease("reclaim", 8).unwrap().is_none());
}

#[test]
fn origin_status_counts_partition_the_durable_frontier() {
    let database = path();
    let mut frontier = Frontier::open(&database).unwrap();
    frontier.create_run("origins", &limits()).unwrap();
    for address in [
        "https://a.example/1",
        "https://a.example/2",
        "https://b.example/1",
    ] {
        frontier.enqueue("origins", &url(address), 0, 0).unwrap();
    }
    let lease = frontier.lease("origins", 2).unwrap().unwrap();
    frontier.complete(&lease, 3, &json!({})).unwrap();
    let first = frontier.status("origins").unwrap();
    assert_eq!(first["origins"].as_array().unwrap().len(), 2);
    assert_eq!(first["origins"][0]["origin"], "https://a.example");
    assert_eq!(first["origins"][0]["url_count"], 2);
    assert_eq!(first["origins"][0]["states"]["HANDLED"], 1);
    assert_eq!(first["origins"][0]["states"]["PENDING"], 1);
    assert_eq!(first["origins"][1]["url_count"], 1);
    drop(frontier);
    let reopened = Frontier::open(&database).unwrap();
    assert_eq!(
        reopened.status("origins").unwrap()["origins"],
        first["origins"]
    );
    for state in first["states"].as_object().unwrap().keys() {
        let sum: u64 = first["origins"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| o["states"][state].as_u64().unwrap())
            .sum();
        assert_eq!(sum, first["states"][state].as_u64().unwrap());
    }
}

fn limits() -> CrawlLimits {
    CrawlLimits {
        max_pages: 50,
        max_urls: 100,
        max_depth: 3,
        global_concurrency: 2,
        per_origin_concurrency: 1,
        origin_interval_ms: 0,
        lease_ms: 100,
        max_retries: 2,
        backoff_ms: 20,
        max_backoff_ms: 200,
        deadline_ms: 100_000,
    }
}
fn path() -> PathBuf {
    std::env::temp_dir().join(format!("ecdev-frontier-{}.sqlite", uuid::Uuid::new_v4()))
}
fn url(s: &str) -> ecdev_core::frontier::UrlIdentity {
    canonicalize(s, None, &UrlPolicy::default(), 1).unwrap()
}

#[test]
fn identity_preserves_product_parameters_and_normalizes_safe_variants() {
    let a = url("HTTP://EXAMPLE.COM:80/a/../b/%7e?q=red%2fblue#reviews");
    assert_eq!(a.canonical_url, "http://example.com/b/~?q=red%2Fblue");
    assert_eq!(
        a.identity_hash,
        url("http://example.com/b/~?q=red%2Fblue").identity_hash
    );
    assert_ne!(
        url("https://example.com/?sku=1").identity_hash,
        url("https://example.com/?sku=2").identity_hash
    );
    assert_ne!(
        url("https://example.com/a//b").identity_hash,
        url("https://example.com/a/b").identity_hash
    );
    assert_ne!(
        url("https://example.com/?x=1&x=2").identity_hash,
        url("https://example.com/?x=2&x=1").identity_hash
    );
    assert_eq!(
        url("https://例え.テスト/").canonical_url,
        "https://xn--r8jz45g.xn--zckzah/"
    );
    let policy = UrlPolicy {
        sort_query: true,
        tracking_parameters: ["utm_source".into(), "sku".into()].into(),
        identity_parameters: ["sku".into()].into(),
        ..Default::default()
    };
    let b = canonicalize(
        "../p?z=2&utm_source=ad&sku=7&a=1",
        Some("https://shop.example/category/a/"),
        &policy,
        2,
    )
    .unwrap();
    assert_eq!(
        b.canonical_url,
        "https://shop.example/category/p?a=1&sku=7&z=2"
    );
    assert_eq!(
        b.source_page.as_deref(),
        Some("https://shop.example/category/a/")
    );
    assert!(canonicalize("https://user:password@shop.example/", None, &policy, 2).is_err());
}

#[test]
fn persistence_priority_handled_and_deduplication() {
    let p = path();
    let mut f = Frontier::open(&p).unwrap();
    f.create_run("a", &limits()).unwrap();
    assert!(
        f.enqueue("a", &url("https://shop.example/a#fragment"), 0, 0)
            .unwrap()
    );
    assert!(
        !f.enqueue("a", &url("https://SHOP.example:443/a"), 0, 999)
            .unwrap()
    );
    f.enqueue("a", &url("https://other.example/priority"), 0, 10)
        .unwrap();
    drop(f);
    let mut f = Frontier::open(&p).unwrap();
    let l = f.lease("a", 2).unwrap().unwrap();
    assert_eq!(l.canonical_url, "https://other.example/priority");
    f.complete(&l, 3, &json!({"raw_hash":"observed-capture"}))
        .unwrap();
    let a = f.lease("a", 4).unwrap().unwrap();
    assert_eq!(a.canonical_url, "https://shop.example/a");
    f.complete(&a, 5, &json!({})).unwrap();
    assert!(f.lease("a", 6).unwrap().is_none());
    assert_eq!(f.status("a").unwrap()["states"]["HANDLED"], 2);
    drop(f);
    fs::remove_file(p).unwrap();
}

#[test]
fn retries_obey_backoff_retry_after_and_exhaustion() {
    let p = path();
    let mut f = Frontier::open(&p).unwrap();
    f.create_run("a", &limits()).unwrap();
    f.enqueue("a", &url("https://shop.example/"), 0, 0).unwrap();
    let a = f.lease("a", 2).unwrap().unwrap();
    f.fail(&a, 3, "HTTP_429", true, Some(80)).unwrap();
    assert!(f.lease("a", 82).unwrap().is_none());
    let b = f.lease("a", 83).unwrap().unwrap();
    f.fail(&b, 84, "HTTP_503", true, None).unwrap();
    assert!(f.lease("a", 123).unwrap().is_none());
    let c = f.lease("a", 124).unwrap().unwrap();
    f.fail(&c, 125, "HTTP_503", true, None).unwrap();
    assert!(f.lease("a", 1000).unwrap().is_none());
    assert_eq!(f.status("a").unwrap()["states"]["FAILED"], 1);
    assert_eq!(f.status("a").unwrap()["retry_events"], 2);
    drop(f);
    fs::remove_file(p).unwrap();
}

#[test]
fn independent_workers_fenced_origin_global_limits_cancellation_and_deadline() {
    let p = path();
    let mut f = Frontier::open(&p).unwrap();
    f.create_run("a", &limits()).unwrap();
    for s in [
        "https://a.example/1",
        "https://a.example/2",
        "https://b.example/1",
        "https://c.example/1",
    ] {
        // Pin the lease whose fencing is inspected; equal-priority retries now join the tail.
        let priority = i64::from(s == "https://a.example/1");
        f.enqueue("a", &url(s), 0, priority).unwrap();
    }
    let mut other = Frontier::open(&p).unwrap();
    let a = f.lease("a", 2).unwrap().unwrap();
    let b = other.lease("a", 3).unwrap().unwrap();
    assert!(b.canonical_url.starts_with("https://b.example"));
    assert!(f.lease("a", 4).unwrap().is_none());
    let recovered = other.lease("a", 102).unwrap().unwrap();
    assert_eq!(recovered.identity_hash, a.identity_hash);
    assert!(f.complete(&a, 103, &json!({"stale":true})).is_err());
    other.cancel("a", 104).unwrap();
    assert!(f.complete(&recovered, 105, &json!({})).is_err());
    assert_eq!(f.status("a").unwrap()["states"]["CANCELLED"], 4);
    f.create_run("deadline", &limits()).unwrap();
    f.enqueue("deadline", &url("https://a.example/"), 0, 0)
        .unwrap();
    assert!(f.lease("deadline", 100_000).unwrap().is_none());
    assert_eq!(f.status("deadline").unwrap()["states"]["CANCELLED"], 1);
    drop(other);
    drop(f);
    fs::remove_file(p).unwrap();
}

#[test]
fn retry_after_origin_cooldown_survives_reopen_and_keeps_other_origins_available() {
    let p = path();
    let mut f = Frontier::open(&p).unwrap();
    f.create_run("a", &limits()).unwrap();
    f.enqueue("a", &url("https://shop.example/a"), 0, 1000)
        .unwrap();
    f.enqueue("a", &url("https://shop.example/b"), 0, 500)
        .unwrap();
    f.enqueue("a", &url("https://other.example/a"), 0, 100)
        .unwrap();
    let failed = f.lease("a", 2).unwrap().unwrap();
    f.fail(&failed, 3, "HTTP_STATUS_429", true, Some(80))
        .unwrap();
    // Reusing an expired token cannot replace the valid cooldown with a longer one.
    assert!(
        f.fail(&failed, 4, "HTTP_STATUS_429", true, Some(5000))
            .is_err()
    );
    drop(f);
    let mut f = Frontier::open(&p).unwrap();
    let other = f.lease("a", 4).unwrap().unwrap();
    assert_eq!(other.canonical_url, "https://other.example/a");
    f.complete(&other, 5, &json!({})).unwrap();
    assert!(f.lease("a", 82).unwrap().is_none());
    let retried = f.lease("a", 83).unwrap().unwrap();
    assert_eq!(retried.canonical_url, "https://shop.example/a");
    f.complete_with_origin_cooldown(
        &retried,
        84,
        &json!({"stale_cache_fallback":true}),
        Some(150),
    )
    .unwrap();
    drop(f);
    let mut f = Frontier::open(&p).unwrap();
    assert!(f.lease("a", 149).unwrap().is_none());
    assert_eq!(
        f.lease("a", 150).unwrap().unwrap().canonical_url,
        "https://shop.example/b"
    );
}

#[test]
fn depth_url_and_attempt_budgets_and_throttle_are_durable() {
    let p = path();
    let mut f = Frontier::open(&p).unwrap();
    let mut bounds = limits();
    bounds.max_pages = 1;
    bounds.max_urls = 2;
    bounds.max_depth = 1;
    bounds.origin_interval_ms = 500;
    f.create_run("a", &bounds).unwrap();
    assert!(
        !f.enqueue("a", &url("https://a.example/deep"), 2, 0)
            .unwrap()
    );
    assert!(f.enqueue("a", &url("https://a.example/1"), 1, 0).unwrap());
    assert!(f.enqueue("a", &url("https://a.example/2"), 1, 0).unwrap());
    assert!(!f.enqueue("a", &url("https://a.example/3"), 1, 0).unwrap());
    let a = f.lease("a", 2).unwrap().unwrap();
    f.complete(&a, 3, &json!({})).unwrap();
    drop(f);
    let mut f = Frontier::open(&p).unwrap();
    assert!(f.lease("a", 1000).unwrap().is_none());
    assert_eq!(f.status("a").unwrap()["acquisition_attempts"], 1);
    bounds.max_pages = 50;
    bounds.max_urls = 100;
    f.create_run("throttle", &bounds).unwrap();
    for s in ["https://a.example/1", "https://a.example/2"] {
        f.enqueue("throttle", &url(s), 0, 0).unwrap();
    }
    let a = f.lease("throttle", 2).unwrap().unwrap();
    f.complete(&a, 3, &json!({})).unwrap();
    assert!(f.lease("throttle", 501).unwrap().is_none());
    assert!(f.lease("throttle", 502).unwrap().is_some());
    drop(f);
    fs::remove_file(p).unwrap();
}

// Subprocess entry point: the parent kills this process after the lease transaction commits.
#[test]
fn crash_child() {
    let Some(p) = std::env::var_os("ECDEV_CRASH_DB") else {
        return;
    };
    let mut f = Frontier::open(std::path::Path::new(&p)).unwrap();
    f.lease("crash", 2).unwrap().unwrap();
    fs::write(
        std::path::Path::new(&p).with_extension("ready"),
        b"LEASE_COMMITTED",
    )
    .unwrap();
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}

#[test]
fn deliberate_process_interruption_recovers_expired_lease() {
    let p = path();
    let f = Frontier::open(&p).unwrap();
    f.create_run("crash", &limits()).unwrap();
    drop(f);
    let mut f = Frontier::open(&p).unwrap();
    f.enqueue("crash", &url("https://shop.example/crash"), 0, 0)
        .unwrap();
    drop(f);
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "crash_child", "--nocapture"])
        .env("ECDEV_CRASH_DB", &p)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let start = Instant::now();
    let ready = p.with_extension("ready");
    while !ready.exists() && start.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(10));
    }
    let was_ready = ready.exists();
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(was_ready, "child did not commit lease before kill");
    let mut f = Frontier::open(&p).unwrap();
    assert_eq!(f.status("crash").unwrap()["states"]["LEASED"], 1);
    assert!(f.lease("crash", 101).unwrap().is_none());
    let recovered = f.lease("crash", 102).unwrap().unwrap();
    assert_eq!(recovered.attempts, 2);
    f.complete(
        &recovered,
        103,
        &json!({"recovered_after_process_kill":true}),
    )
    .unwrap();
    assert_eq!(f.status("crash").unwrap()["states"]["HANDLED"], 1);
    drop(f);
    fs::remove_file(p).unwrap();
    fs::remove_file(ready).unwrap();
}
