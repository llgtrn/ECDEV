use ecdev_core::frontier::{CrawlLimits, Frontier, UrlPolicy, canonicalize};
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

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
        f.enqueue("a", &url(s), 0, 0).unwrap();
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
