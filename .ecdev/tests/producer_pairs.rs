//! Real retained captures with equal coverage are distinct producer inputs.
use ecdev_governance::digest::content_digest;
use ecdev_governance::formats::json;
use ecdev_governance::migration::{proposal, source};
use std::path::Path;

const CURRENT_CAPTURE: &str = "441fa612fdb5e9a2dd468a7da87730ff0ec53d6fcaa4b014337bcac393b1a909";
const CURRENT_PROPOSAL: &str = "d499b6b7f5d01cfc4f1a9083faeca057f2a3d9511d070c3f7e101eee590e6f29";
const EARLIER_CAPTURE: &str = "c751083d31323cf190a687452233277459d84967a6d5daf04bffb830f03c654d";
const HISTORICAL_PROPOSAL: &str =
    "f2933fd99d4e2737dd3abb4233d7d8c634409a09597c2a01783016930745b056";
const DRIFT: &str = "PROPOSAL_SOURCE_BINDING_OR_COVERAGE_DRIFT";

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
}

fn read(path: &str, expected: &str) -> String {
    let raw = std::fs::read_to_string(root().join(path)).unwrap();
    assert_eq!(content_digest(raw.as_bytes()), format!("sha256:{expected}"));
    raw
}

fn current() -> (source::Capture, String) {
    let raw = read(
        "research/commerce/governance/source-eight-capture-db92bf27-v1.json",
        CURRENT_CAPTURE,
    );
    let draft = read(
        "research/commerce/governance/source-eight-mapping-db92bf27-v1.json",
        CURRENT_PROPOSAL,
    );
    (source::Capture::parse(&raw).unwrap(), draft)
}

#[test]
fn published_current_artifact_pair_matches_the_actual_caller_bound_producer() {
    let (capture, draft) = current();
    assert_eq!(capture.files.len(), 734);
    assert_eq!(
        proposal::generate_bound(&capture, CURRENT_CAPTURE)
            .unwrap()
            .render(),
        draft
    );
    let result = proposal::inspect_text_bound(&capture, &draft, CURRENT_CAPTURE).unwrap();
    let result = json::parse_unique(&result.render()).unwrap();
    assert_eq!(result.str("status"), Some("VALID_PENDING_PROPOSAL"));
    assert_eq!(result.items_of("unresolved_reasons").len(), 2);
    assert!(proposal::generate(&capture).is_err());
    assert!(proposal::inspect_text(&capture, &draft).is_err());
}

#[test]
fn equal_size_historical_captures_with_identical_payloads_cannot_exchange_proposals() {
    let earlier = read(
        ".ecdev/tests/fixtures/source-capture-v1.json",
        EARLIER_CAPTURE,
    );
    let historical = read(
        ".ecdev/tests/fixtures/source-capture-00dd7f0-v1.json",
        proposal::CAPTURE_SHA256,
    );
    assert_eq!(earlier.len(), historical.len());
    let earlier = source::Capture::parse(&earlier).unwrap();
    let historical = source::Capture::parse(&historical).unwrap();
    assert_eq!(earlier.files.len(), 590);
    assert_eq!(earlier.files, historical.files);
    assert_ne!(earlier.observed_head, historical.observed_head);
    assert_ne!(earlier.source_schema, historical.source_schema);
    let retained = read(
        "research/commerce/governance/source-mapping-proposal-v1.json",
        HISTORICAL_PROPOSAL,
    );
    assert_eq!(proposal::generate(&historical).unwrap().render(), retained);
    assert!(proposal::inspect_text(&historical, &retained).is_ok());
    assert_eq!(
        proposal::inspect_text_bound(&earlier, &retained, EARLIER_CAPTURE).unwrap_err(),
        DRIFT
    );
    let earlier_draft = proposal::generate_bound(&earlier, EARLIER_CAPTURE)
        .unwrap()
        .render();
    assert!(proposal::inspect_text_bound(&earlier, &earlier_draft, EARLIER_CAPTURE).is_ok());
    assert_eq!(
        proposal::inspect_text_bound(&historical, &earlier_draft, proposal::CAPTURE_SHA256)
            .unwrap_err(),
        DRIFT
    );
    assert!(proposal::generate(&earlier).is_err());
}

#[test]
fn changing_only_observed_head_requires_regeneration_without_rebinding_published_evidence() {
    let (capture, retained) = current();
    let mut changed = capture.clone();
    changed.observed_head = "f".repeat(40);
    assert_ne!(changed.observed_head, capture.observed_head);
    assert_eq!(changed.files, capture.files);
    assert_eq!(
        changed.source_graph().unwrap(),
        capture.source_graph().unwrap()
    );
    let changed_raw = changed.render().unwrap();
    let changed_pin = content_digest(changed_raw.as_bytes())[7..].to_string();
    assert_ne!(changed_pin, CURRENT_CAPTURE);
    assert!(proposal::generate_bound(&changed, CURRENT_CAPTURE).is_err());
    assert_eq!(
        proposal::inspect_text_bound(&changed, &retained, &changed_pin).unwrap_err(),
        DRIFT
    );
    let regenerated = proposal::generate_bound(&changed, &changed_pin)
        .unwrap()
        .render();
    assert!(proposal::inspect_text_bound(&changed, &regenerated, &changed_pin).is_ok());
    assert!(proposal::inspect_text_bound(&capture, &retained, CURRENT_CAPTURE).is_ok());
}
