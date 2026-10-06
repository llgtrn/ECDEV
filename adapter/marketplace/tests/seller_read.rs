//! `ecdev.seller.read` through the engine: official layer only, fixtures labeled, raw capture
//! persisted by hash, restricted domain and missing authorization refused before any IO.
use ecdev_core::service::Engine;
use ecdev_marketplace::Amazon;
use serde_json::{Value, json};
use std::sync::Arc;

fn cases() -> Value {
    serde_json::from_str(include_str!("fixtures/official-read-responses.json")).unwrap()
}

fn engine(name: &str) -> (Engine, std::path::PathBuf) {
    let root =
        std::env::temp_dir().join(format!("ecdev-seller-read-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let engine = Engine::open(&root).unwrap().with_provider(Arc::new(Amazon));
    (engine, root)
}

#[test]
fn every_locked_read_example_flows_through_the_engine_as_labeled_fixture_evidence() {
    let (engine, root) = engine("fixtures");
    for case in cases()["cases"].as_array().unwrap() {
        let operation = case["operation"].as_str().unwrap();
        let mut args = case["query"].clone();
        args["market"] = case["market"].clone();
        args["fixture_responses"] = json!({operation: case["response"].clone()});
        let run = engine.call("ecdev.seller.read", args).unwrap();
        assert_eq!(run["source_layer"], "OFFICIAL_SP_API", "{operation}");
        assert_eq!(run["run_kind"], "OFFICIAL_SELLER_READ");
        assert_eq!(run["mode"], "FIXTURE");
        assert_eq!(run["network_calls"], 0);
        assert_eq!(run["cost_minor"], 0);
        assert_eq!(run["new_live_acquisition"], false);
        assert_eq!(run["result"]["official_live_validation"], "UNAVAILABLE");
        let record = &run["result"]["records"][0];
        assert_eq!(record["status"], "COMPLETE", "{operation}: {record}");
        assert!(record.get("raw_body").is_none(), "raw body is not inlined");
        let hash = record["raw_capture_sha256"].as_str().unwrap();
        let raw = std::fs::read(root.join(format!(".ecdev-data/raw/{hash}.json"))).unwrap();
        assert_eq!(
            raw,
            case["response"]["raw_body"].as_str().unwrap().as_bytes()
        );
        for field in record["normalized"]["fields"].as_array().unwrap() {
            assert_eq!(field["source_layer"], "OFFICIAL_SP_API");
            assert_eq!(field["observation_mode"], "FIXTURE");
            assert!(matches!(
                field["evidence_state"].as_str(),
                Some("OBSERVED" | "UNKNOWN" | "WITHHELD")
            ));
        }
        let persisted = engine
            .call("ecdev.runs.inspect", json!({"run_id":run["run_id"]}))
            .unwrap();
        assert_eq!(persisted["run_kind"], "OFFICIAL_SELLER_READ");
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn restricted_unknown_and_non_official_reads_are_refused_before_io() {
    let (engine, root) = engine("refused");
    for (args, error) in [
        (
            json!({"market":"AMAZON_US","operation":"GET_ORDERS"}),
            "RESTRICTED_DOMAIN_DISABLED",
        ),
        (
            json!({"market":"AMAZON_US","operation":"ORDER_BUYER_INFO"}),
            "RESTRICTED_DOMAIN_DISABLED",
        ),
        (
            json!({"market":"AMAZON_US","operation":"CREATE_RESTRICTED_DATA_TOKEN"}),
            "RESTRICTED_DOMAIN_DISABLED",
        ),
        (
            json!({"market":"AMAZON_US","operation":"PUT_LISTINGS_ITEM"}),
            "UNSUPPORTED_SELLER_READ_OPERATION",
        ),
        (
            json!({"market":"AMAZON_DE","operation":"MARKETPLACE_PARTICIPATIONS"}),
            "INVALID_SELLER_READ_MARKET",
        ),
        (
            json!({"market":"AMAZON_US","operation":"MARKETPLACE_PARTICIPATIONS","evidence_layer":"KEEPA"}),
            "SELLER_READ_IS_OFFICIAL_SP_API_ONLY",
        ),
    ] {
        assert_eq!(engine.call("ecdev.seller.read", args).unwrap_err(), error);
    }
    assert!(!root.join(".ecdev-data/raw").exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_live_read_without_authorization_is_plan_only_with_zero_requests() {
    let (engine, root) = engine("unauthorized");
    let run = engine
        .call(
            "ecdev.seller.read",
            json!({"market":"AMAZON_JP","operation":"MARKETPLACE_PARTICIPATIONS"}),
        )
        .unwrap();
    assert_eq!(run["mode"], "PLAN_ONLY");
    assert_eq!(run["status"], "UNAVAILABLE");
    assert_eq!(run["network_calls"], 0);
    assert_eq!(run["cost_minor"], 0);
    assert_eq!(run["new_live_acquisition"], false);
    assert_eq!(run["fallback_providers"], json!([]));
    let _ = std::fs::remove_dir_all(root);
}
