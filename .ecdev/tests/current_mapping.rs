//! Current source proposals are executable custody candidates, never canonical admission.
use ecdev_governance::compact::{facts, history};
use ecdev_governance::digest::content_digest;
use ecdev_governance::formats::{json, Value};
use ecdev_governance::migration::{proposal, source, source_view};
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::OnceLock;

struct Current {
    capture: source::Capture,
    bytes: String,
    digest: String,
    draft: String,
    value: Value,
}

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
}

fn current() -> &'static Current {
    static CURRENT: OnceLock<Current> = OnceLock::new();
    CURRENT.get_or_init(|| {
        let capture = source::capture(root()).unwrap();
        let bytes = capture.render().unwrap();
        let digest = content_digest(bytes.as_bytes())[7..].to_string();
        let draft = proposal::generate_bound(&capture, &digest)
            .unwrap()
            .render();
        let value = json::parse_unique(&draft).unwrap();
        Current {
            capture,
            bytes,
            digest,
            draft,
            value,
        }
    })
}

fn field<'a>(value: &'a mut Value, key: &str) -> &'a mut Value {
    let Value::Table(fields) = value else {
        panic!("object required")
    };
    fields.get_mut(key).unwrap()
}

fn array<'a>(value: &'a mut Value, key: &str) -> &'a mut Vec<Value> {
    let Value::Array(values) = field(value, key) else {
        panic!("array required")
    };
    values
}

fn reject(value: &Value) {
    let c = current();
    assert!(proposal::inspect_bound(&c.capture, value, &c.digest).is_err());
}

#[test]
fn current_capture_maps_all_source_nodes_payloads_rights_and_records_without_admission() {
    let c = current();
    assert_eq!(c.capture.files.len(), 734);
    assert_ne!(c.digest, proposal::CAPTURE_SHA256);
    assert_eq!(
        proposal::generate_bound(&c.capture, &c.digest)
            .unwrap()
            .render(),
        c.draft
    );
    assert_eq!(
        proposal::generate_bound(&c.capture, &c.digest.to_ascii_uppercase())
            .unwrap()
            .render(),
        c.draft
    );
    let inspection = proposal::inspect_text_bound(&c.capture, &c.draft, &c.digest).unwrap();
    let inspection = json::parse_unique(&inspection.render()).unwrap();
    assert_eq!(inspection.str("status"), Some("VALID_PENDING_PROPOSAL"));
    assert_eq!(inspection.str("scope"), Some("PROPOSAL_ONLY; NOT_ENROLLED"));
    assert_eq!(
        inspection.str("capture_sha256"),
        Some(format!("sha256:{}", c.digest).as_str())
    );
    assert_eq!(c.value.get("target_custody"), Some(&Value::Null));
    assert_eq!(c.value.items_of("global_identity_candidates").len(), 68);
    assert_eq!(
        c.value.at("source_semantics.canonical_enrollment"),
        Some(&Value::Str("BLOCKED".into()))
    );
    for key in ["canonical_identity_mapping", "norl_relevance_mapping"] {
        assert_eq!(
            c.value.at(&format!("source_semantics.{key}")),
            Some(&Value::Null)
        );
    }
    let envelope = json::parse_unique(&c.bytes).unwrap();
    let semantics =
        json::parse_unique(&source_view::inspect(&c.capture).unwrap().render()).unwrap();
    assert_eq!(c.value.get("source_semantics"), Some(&semantics));
    assert_eq!(
        c.value.at("source_semantics.source_graph"),
        envelope.get("source_graph")
    );
    let nodes = envelope.at("source_graph.nodes").unwrap().items();
    let edges = envelope.at("source_graph.edges").unwrap().items();
    assert_eq!((nodes.len(), edges.len()), (828, 1122));
    let mappings = c.value.items_of("node_mappings");
    assert_eq!(mappings.len(), 828);
    let mut mapped_ids = BTreeSet::new();
    for (index, (mapping, node)) in mappings.iter().zip(nodes).enumerate() {
        assert_eq!(mapping.get("source_metadata"), Some(node));
        for (mapped, original) in [
            ("source_id", "id"),
            ("source_namespace", "namespace"),
            ("source_semantic_key", "semantic_key"),
        ] {
            assert_eq!(mapping.get(mapped), node.get(original));
        }
        assert!(mapped_ids.insert(mapping.str("source_id").unwrap()));
        assert_eq!(mapping.get("proposed_target"), Some(&Value::Null));
        assert_eq!(
            mapping.str("capture_locator"),
            Some(format!("/source_graph/nodes/{index}").as_str())
        );
    }
    let payloads: Vec<_> = ["declaration", "history", "evidence", "knowledge"]
        .into_iter()
        .flat_map(|lane| c.value.at(&format!("payloads.{lane}")).unwrap().items())
        .collect();
    assert_eq!(payloads.len(), 734);
    let mut mapped_paths = BTreeSet::new();
    for payload in payloads {
        let path = payload.str("source_path").unwrap();
        assert!(mapped_paths.insert(path));
        let index = envelope
            .items_of("files")
            .iter()
            .position(|file| file.str("path") == Some(path))
            .unwrap();
        assert_eq!(
            payload.str("capture_locator"),
            Some(format!("/files/{index}").as_str())
        );
        assert_eq!(
            payload.str("hex_locator"),
            Some(format!("/files/{index}/hex").as_str())
        );
        assert_eq!(
            payload.str("sha256"),
            Some(&content_digest(&c.capture.files[path])[7..])
        );
        assert_eq!(
            payload.get("bytes"),
            Some(&Value::Int(c.capture.files[path].len() as i64))
        );
    }
    assert_eq!(
        mapped_paths,
        c.capture.files.keys().map(String::as_str).collect()
    );
    let rights = c.value.items_of("rights_claims");
    assert_eq!(rights.len(), 68);
    let mut aliases = BTreeSet::new();
    for (claim, donor) in rights.iter().zip(semantics.items_of("donors")) {
        assert!(aliases.insert(claim.str("source_alias").unwrap()));
        for (mapped, original) in [
            ("source_alias", "source_alias"),
            ("source_id", "source_id"),
            ("origin", "origin"),
            ("license_claim", "license"),
            ("provenance", "provenance"),
        ] {
            assert_eq!(claim.get(mapped), donor.get(original));
        }
        assert_eq!(
            claim.str("scope"),
            Some("RECORDED_SOURCE_CLAIMS; NO_CURRENT_RIGHTS_AUTHORIZATION")
        );
        assert_eq!(donor.get("canonical_effective_state"), Some(&Value::Null));
    }
    let recorded_facts = semantics.items_of("facts");
    assert_eq!(recorded_facts.len(), 1861);
    let original_facts: Vec<_> = c
        .capture
        .files
        .iter()
        .filter(|(path, _)| path.starts_with(".ecdev/knowledge/"))
        .flat_map(|(_, bytes)| facts::decode(bytes).unwrap())
        .collect();
    assert_eq!(original_facts.len(), recorded_facts.len());
    for (original, mapped) in original_facts.iter().zip(recorded_facts) {
        assert_eq!(mapped.str("source_id"), Some(original.id().as_str()));
        assert_eq!(
            mapped.str("sequence_decimal"),
            Some(original.seq.to_string().as_str())
        );
        assert_eq!(mapped.str("value"), Some(original.value.as_str()));
        assert_eq!(mapped.str("subject"), Some(original.subject.as_str()));
    }
    assert_eq!(
        semantics.get("active_evidence_records"),
        Some(&Value::Int(726))
    );
    assert_eq!(
        semantics.get("retired_evidence_records"),
        Some(&Value::Int(8))
    );
    let historical: BTreeSet<_> = c
        .capture
        .files
        .iter()
        .filter(|(path, _)| path.starts_with(".ecdev/history/"))
        .flat_map(|(_, bytes)| history::Batch::decode(bytes).unwrap().registered)
        .collect();
    assert_eq!(
        semantics
            .at("history.registered_donors")
            .unwrap()
            .items()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect::<BTreeSet<_>>(),
        historical
    );
    assert_eq!(c.capture.render().unwrap(), c.bytes);
}

#[test]
fn capture_digest_is_external_mandatory_and_not_a_proposal_self_assertion() {
    let c = current();
    for invalid in [
        "".into(),
        "0".repeat(63),
        "g".repeat(64),
        format!("sha256:{}", c.digest),
    ] {
        assert!(proposal::generate_bound(&c.capture, &invalid)
            .unwrap_err()
            .contains("PIN_INVALID"));
        assert!(proposal::inspect_bound(&c.capture, &c.value, &invalid).is_err());
    }
    let wrong = "0".repeat(64);
    assert!(proposal::generate_bound(&c.capture, &wrong)
        .unwrap_err()
        .contains("PIN_MISMATCH"));
    assert!(proposal::inspect_bound(&c.capture, &c.value, &wrong).is_err());
    let mut changed = c.capture.clone();
    changed.observed_head = if changed.observed_head == "a".repeat(40) {
        "b".repeat(40)
    } else {
        "a".repeat(40)
    };
    let changed_bytes = changed.render().unwrap();
    let changed_digest = content_digest(changed_bytes.as_bytes())[7..].to_string();
    assert_ne!(changed_digest, c.digest);
    assert!(proposal::generate_bound(&changed, &c.digest).is_err());
    assert!(proposal::inspect_bound(&changed, &c.value, &c.digest).is_err());
    let new_draft = proposal::generate_bound(&changed, &changed_digest)
        .unwrap()
        .render();
    assert!(proposal::inspect_text_bound(&changed, &new_draft, &changed_digest).is_ok());
    assert!(proposal::inspect_text_bound(&changed, &c.draft, &changed_digest).is_err());
    let mut self_asserted = json::parse_unique(&new_draft).unwrap();
    *field(field(&mut self_asserted, "capture_binding"), "sha256") =
        Value::Str(format!("sha256:{}", c.digest));
    assert!(proposal::inspect_bound(&changed, &self_asserted, &changed_digest).is_err());
    assert!(proposal::inspect_bound(&changed, &self_asserted, &c.digest).is_err());
}

#[test]
fn current_mapping_cannot_grant_targets_rekey_records_drop_payloads_or_rewrite_rights() {
    for mutation in [
        "target",
        "node-target",
        "id",
        "key",
        "coverage",
        "rights",
        "fact",
        "count",
    ] {
        let mut value = current().value.clone();
        match mutation {
            "target" => *field(&mut value, "target_custody") = Value::Str("self-approved".into()),
            "node-target" => {
                *field(
                    &mut array(&mut value, "node_mappings")[0],
                    "proposed_target",
                ) = Value::Str("self-owned".into())
            }
            "id" => {
                *field(&mut array(&mut value, "node_mappings")[0], "source_id") =
                    Value::Str(format!("ec1:{}", "0".repeat(32)))
            }
            "key" => {
                *field(
                    &mut array(&mut value, "node_mappings")[0],
                    "source_semantic_key",
                ) = Value::Str("invented-owner".into())
            }
            "coverage" => {
                array(field(&mut value, "payloads"), "evidence").pop();
            }
            "rights" => {
                *field(&mut array(&mut value, "rights_claims")[0], "license_claim") =
                    Value::Str("unrestricted".into())
            }
            "fact" => {
                *field(
                    &mut array(field(&mut value, "source_semantics"), "facts")[0],
                    "source_id",
                ) = Value::Str(format!("f1:{}", "0".repeat(32)))
            }
            _ => {
                *field(
                    field(&mut value, "source_semantics"),
                    "active_evidence_records",
                ) = Value::Int(0)
            }
        }
        reject(&value);
    }
    let c = current();
    for text in [
        c.draft.replacen('{', "{\"scope\":\"duplicate\",", 1),
        c.draft.replacen('{', "{\"\\u0073cope\":\"duplicate\",", 1),
        c.draft
            .replacen("\"sha256\":", "\"sha256\":\"duplicate\",\"sha256\":", 1),
    ] {
        assert_ne!(text, c.draft);
        assert!(proposal::inspect_text_bound(&c.capture, &text, &c.digest).is_err());
    }
    assert!(proposal::inspect_text_bound(
        &c.capture,
        &" ".repeat(proposal::MAX_PROPOSAL_BYTES + 1),
        &c.digest
    )
    .is_err());
}

#[test]
fn historical_pinned_wrappers_and_retained_proposal_remain_byte_exact() {
    let bytes = include_str!("fixtures/source-capture-00dd7f0-v1.json");
    let capture = source::Capture::parse(bytes).unwrap();
    assert_eq!(capture.files.len(), 590);
    assert_eq!(capture.render().unwrap(), bytes);
    let retained =
        include_str!("../../research/commerce/governance/source-mapping-proposal-v1.json");
    assert_eq!(proposal::generate(&capture).unwrap().render(), retained);
    assert_eq!(
        proposal::generate_bound(&capture, proposal::CAPTURE_SHA256)
            .unwrap()
            .render(),
        retained
    );
    assert_eq!(
        proposal::inspect_text(&capture, retained).unwrap(),
        proposal::inspect_text_bound(&capture, retained, proposal::CAPTURE_SHA256).unwrap()
    );
    let current = current();
    assert!(proposal::generate(&current.capture).is_err());
    assert!(proposal::inspect(&current.capture, &current.value).is_err());
    assert!(proposal::inspect_text(&current.capture, &current.draft).is_err());
}
