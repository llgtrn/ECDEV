//! Offline Web capture provenance through the maintained re-extraction request route.
use ecdev_core::{Engine, json};
use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn identical_web_capture_bytes_preserve_each_source_context() {
    let root = std::env::temp_dir().join(format!(
        "ecdev-reextract-source-context-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let engine = Engine::open(&root)
        .unwrap()
        .with_provider(Arc::new(ecdev_web::Web::default()));
    let html = r#"<meta property="og:type" content="product"><meta property="og:title" content="Cup"><meta property="product:price:amount" content="2,980"><meta property="product:price:currency" content="JPY">"#;
    // Identical bytes at distinct URLs remain distinct source captures.
    let shared = engine.research(json!({"market":"PUBLIC_WEB","query":"Shared bytes fixture","sources":[{"url":"https://example.org/a","fixture_html":html},{"url":"https://example.org/b","fixture_html":html}],"max_pages":2})).unwrap();
    assert_eq!(shared["mode"], "FIXTURE");
    assert_eq!(shared["network_calls"], 0);
    let observations = shared["observations"].as_array().unwrap();
    assert_eq!(observations.len(), 2);
    assert_eq!(observations[0]["raw_hash"], observations[1]["raw_hash"]);
    assert_ne!(
        observations[0]["external_source"],
        observations[1]["external_source"]
    );
    let shared_id = shared["run_id"].as_str().unwrap().to_string();
    let reextracted = engine
        .call(
            "ecdev.research.reextract",
            json!({"run_ids":[shared_id,shared_id]}),
        )
        .unwrap();
    assert_eq!(reextracted["network"], "NONE");
    assert_eq!(reextracted["captures"].as_array().unwrap().len(), 2);
    for source in ["https://example.org/a", "https://example.org/b"] {
        let series = reextracted["series"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["source"] == source && s["field"] == "price_minor")
            .unwrap();
        assert_eq!(series["points"].as_array().unwrap().len(), 1);
        assert_eq!(series["points"][0]["value"], 2980);
    }
    drop(engine);
    std::fs::remove_dir_all(root).unwrap();
}
