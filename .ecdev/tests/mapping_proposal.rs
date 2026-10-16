//! Portable proposals preserve source custody; hashes alone cannot admit targets.
mod common;

use ecdev_governance::formats::{json, Value};
use ecdev_governance::migration::{proposal, source, source_view};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
}
fn capture_path() -> PathBuf {
    root().join(".ecdev/tests/fixtures/source-capture-00dd7f0-v1.json")
}
fn capture() -> &'static source::Capture {
    static CAPTURE: OnceLock<source::Capture> = OnceLock::new();
    CAPTURE.get_or_init(|| {
        source::read_export_bound(&capture_path(), Some(proposal::CAPTURE_SHA256)).unwrap()
    })
}
fn draft() -> Value {
    static DRAFT: OnceLock<Value> = OnceLock::new();
    DRAFT
        .get_or_init(|| {
            json::parse_unique(&proposal::generate(capture()).unwrap().render()).unwrap()
        })
        .clone()
}
fn field<'a>(v: &'a mut Value, key: &str) -> &'a mut Value {
    let Value::Table(t) = v else {
        panic!("object expected")
    };
    t.get_mut(key).unwrap()
}
fn array<'a>(v: &'a mut Value, key: &str) -> &'a mut Vec<Value> {
    let Value::Array(a) = field(v, key) else {
        panic!("array expected")
    };
    a
}
fn reject(v: &Value) {
    assert!(proposal::inspect(capture(), v).is_err());
}

#[test]
fn exact_capture_proposal_is_deterministic_complete_and_pending() {
    let first = proposal::generate(capture()).unwrap().render();
    assert_eq!(first, proposal::generate(capture()).unwrap().render());
    let value = json::parse_unique(&first).unwrap();
    let checked =
        json::parse_unique(&proposal::inspect_text(capture(), &first).unwrap().render()).unwrap();
    assert_eq!(checked.str("status"), Some("VALID_PENDING_PROPOSAL"));
    assert_eq!(value.get("target_custody"), Some(&Value::Null));
    assert_eq!(value.items_of("node_mappings").len(), 819);
    assert_eq!(value.items_of("rights_claims").len(), 68);
    assert_eq!(value.items_of("global_identity_candidates").len(), 68);
    assert_eq!(
        value.at("source_semantics.canonical_enrollment"),
        Some(&Value::Str("BLOCKED".into()))
    );
    assert_eq!(
        value.at("source_semantics.canonical_identity_mapping"),
        Some(&Value::Null)
    );
    assert_eq!(
        value.at("source_semantics.active_evidence_records"),
        Some(&Value::Int(582))
    );
    assert_eq!(
        value.at("source_semantics.retired_evidence_records"),
        Some(&Value::Int(8))
    );
    assert_eq!(
        value.at("source_semantics.facts").unwrap().items().len(),
        1861
    );
    let envelope = json::parse_unique(&capture().render().unwrap()).unwrap();
    let semantics = json::parse_unique(&source_view::inspect(capture()).unwrap().render()).unwrap();
    assert_eq!(value.get("source_semantics"), Some(&semantics));
    assert_eq!(
        value.at("source_semantics.source_graph"),
        envelope.get("source_graph")
    );
    let declaration_index = envelope
        .items_of("files")
        .iter()
        .position(|file| file.str("path") == Some(".ecdev/declared/donors.rs"))
        .unwrap();
    for (index, (rights, donor)) in value
        .items_of("rights_claims")
        .iter()
        .zip(semantics.items_of("donors"))
        .enumerate()
    {
        for (claim, original) in [
            ("source_alias", "source_alias"),
            ("source_id", "source_id"),
            ("origin", "origin"),
            ("license_claim", "license"),
            ("provenance", "provenance"),
        ] {
            assert_eq!(rights.get(claim), donor.get(original));
        }
        assert_eq!(
            rights.str("declaration_capture_locator"),
            Some(format!("/files/{declaration_index}").as_str())
        );
        assert_eq!(
            rights.str("source_semantics_locator"),
            Some(format!("/source_semantics/donors/{index}").as_str())
        );
        assert_eq!(
            rights.str("scope"),
            Some("RECORDED_SOURCE_CLAIMS; NO_CURRENT_RIGHTS_AUTHORIZATION")
        );
    }
    let original = envelope.at("source_graph.nodes").unwrap().items();
    for (index, (mapping, node)) in value
        .items_of("node_mappings")
        .iter()
        .zip(original)
        .enumerate()
    {
        assert_eq!(mapping.get("source_metadata"), Some(node));
        assert_eq!(mapping.get("source_id"), node.get("id"));
        assert_eq!(mapping.get("source_namespace"), node.get("namespace"));
        assert_eq!(mapping.get("source_semantic_key"), node.get("semantic_key"));
        assert_eq!(
            mapping.str("capture_locator"),
            Some(format!("/source_graph/nodes/{index}").as_str())
        );
        assert_eq!(mapping.get("proposed_target"), Some(&Value::Null));
    }
    let payloads: Vec<_> = ["declaration", "history", "evidence", "knowledge"]
        .into_iter()
        .flat_map(|lane| value.at(&format!("payloads.{lane}")).unwrap().items())
        .collect();
    assert_eq!(payloads.len(), 590);
    assert_eq!(
        payloads
            .iter()
            .filter_map(|p| p.str("source_path"))
            .collect::<BTreeSet<_>>()
            .len(),
        590
    );
    for payload in payloads {
        let path = payload.str("source_path").unwrap();
        assert_eq!(
            payload.str("sha256"),
            Some(&ecdev_governance::digest::content_digest(&capture().files[path])[7..])
        );
        assert_eq!(
            payload.get("bytes"),
            Some(&Value::Int(capture().files[path].len() as i64))
        );
    }
}

#[test]
fn node_coverage_duplicates_metadata_ids_keys_and_locators_cannot_drift() {
    for mutation in [
        "missing",
        "extra",
        "duplicate",
        "reorder",
        "id",
        "key",
        "locator",
        "metadata",
    ] {
        let mut v = draft();
        let mappings = array(&mut v, "node_mappings");
        match mutation {
            "missing" => {
                mappings.pop();
            }
            "extra" => mappings.push(mappings[0].clone()),
            "duplicate" => mappings[1] = mappings[0].clone(),
            "reorder" => mappings.swap(0, 1),
            "id" => {
                *field(&mut mappings[0], "source_id") =
                    Value::Str(format!("ec1:{}", "0".repeat(32)))
            }
            "key" => {
                *field(&mut mappings[0], "source_semantic_key") = Value::Str("invented-key".into())
            }
            "locator" => {
                *field(&mut mappings[0], "capture_locator") =
                    Value::Str("/source_graph/nodes/1".into())
            }
            _ => {
                let Value::Table(metadata) = field(&mut mappings[0], "source_metadata") else {
                    panic!()
                };
                metadata.insert("unreviewed_owner".into(), Value::Str("self".into()));
            }
        }
        reject(&v);
    }
}

#[test]
fn payload_and_original_history_knowledge_rights_cannot_be_removed_or_reclassified() {
    for lane in ["declaration", "history", "evidence", "knowledge"] {
        let mut v = draft();
        let payloads = field(&mut v, "payloads");
        array(payloads, lane).pop();
        reject(&v);
    }
    for mutation in ["duplicate", "hash", "hex", "rights", "fact", "proof"] {
        let mut v = draft();
        match mutation {
            "duplicate" => {
                let evidence = array(field(&mut v, "payloads"), "evidence");
                evidence[1] = evidence[0].clone();
            }
            "hash" => {
                *field(
                    &mut array(field(&mut v, "payloads"), "history")[0],
                    "sha256",
                ) = Value::Str("0".repeat(64))
            }
            "hex" => {
                *field(
                    &mut array(field(&mut v, "payloads"), "evidence")[0],
                    "hex_locator",
                ) = Value::Str("/files/0/hex".into())
            }
            "rights" => {
                array(&mut v, "rights_claims").pop();
            }
            "fact" => {
                let facts = array(field(&mut v, "source_semantics"), "facts");
                facts.pop();
            }
            _ => {
                *field(field(&mut v, "source_semantics"), "active_evidence_records") =
                    Value::Int(0);
            }
        }
        reject(&v);
    }
}

#[test]
fn deterministic_hash_candidates_cannot_change_identity_or_become_target_assignments() {
    for mutation in [
        "id",
        "namespace",
        "key",
        "duplicate",
        "source",
        "status",
        "missing",
    ] {
        let mut v = draft();
        let candidates = array(&mut v, "global_identity_candidates");
        match mutation {
            "id" => {
                *field(&mut candidates[0], "candidate_id") =
                    Value::Str(format!("yn1:{}", "0".repeat(32)))
            }
            "namespace" => {
                *field(&mut candidates[0], "source_namespace") =
                    Value::Str("invented-target".into())
            }
            "key" => {
                *field(&mut candidates[0], "source_semantic_key") = Value::Str("another/key".into())
            }
            "duplicate" => candidates[1] = candidates[0].clone(),
            "source" => *field(&mut candidates[0], "source_id") = Value::Str("ec1:invalid".into()),
            "status" => *field(&mut candidates[0], "status") = Value::Str("ADMITTED".into()),
            _ => {
                candidates.pop();
            }
        }
        reject(&v);
    }
}

#[test]
fn absent_target_custody_cannot_be_filled_by_a_source_self_grant() {
    let mut target = std::collections::BTreeMap::new();
    target.insert("namespace".into(), Value::Str("proposed-target".into()));
    target.insert(
        "semantic_key".into(),
        Value::Str("source-chosen-key".into()),
    );
    target.insert("owner".into(), Value::Str("source-chosen-owner".into()));
    for top in [true, false] {
        let mut v = draft();
        if top {
            *field(&mut v, "target_custody") = Value::Table(target.clone());
        } else {
            *field(&mut array(&mut v, "node_mappings")[0], "proposed_target") =
                Value::Table(target.clone());
        }
        assert!(proposal::inspect(capture(), &v)
            .unwrap_err()
            .contains("TARGET_CUSTODY_REQUIRED"));
    }
    for name in [
        "enrolled",
        "admitted",
        "current_rights_authorized",
        "owner_decision",
    ] {
        let mut v = draft();
        let Value::Table(table) = &mut v else {
            panic!()
        };
        table.insert(name.into(), Value::Bool(true));
        reject(&v);
    }
}

#[test]
fn original_pin_version_and_decoded_duplicate_keys_are_mandatory() {
    let old = source::read_export_bound(
        &root().join(".ecdev/tests/fixtures/source-capture-v1.json"),
        None,
    )
    .unwrap();
    assert!(proposal::generate(&old)
        .unwrap_err()
        .contains("SOURCE_CAPTURE_PIN_MISMATCH"));
    let original = proposal::generate(capture()).unwrap().render();
    for raw in [
        original.replace(proposal::FORMAT, "ecdev-source-mapping-proposal-v999"),
        original.replacen("{", "{\"scope\":\"duplicate\",", 1),
        original.replacen("{", "{\"\\u0073cope\":\"duplicate\",", 1),
        original[..original.len() / 2].into(),
    ] {
        assert!(proposal::inspect_text(capture(), &raw).is_err());
    }
    assert!(
        proposal::inspect_text(capture(), &" ".repeat(proposal::MAX_PROPOSAL_BYTES + 1)).is_err()
    );
}

#[test]
fn public_cli_requires_digest_and_refuses_mutations_and_nonregular_proposal_files() {
    let r = common::Repo::new("mapping-cli");
    let fixture = capture_path().display().to_string();
    let text = proposal::generate(capture()).unwrap().render();
    r.write("proposal.json", &text);
    let proposal_path = r.root.join("proposal.json").display().to_string();
    let before = r.read("proposal.json");
    for flags in [
        vec!["--record"],
        vec!["--write"],
        vec!["--force"],
        vec!["--out", "other.json"],
        vec!["extra"],
    ] {
        let mut args = vec![
            "migrate",
            "propose-mapping",
            "--from",
            fixture.as_str(),
            "--expect-sha256",
            proposal::CAPTURE_SHA256,
        ];
        args.extend(flags);
        assert_eq!(r.cli(&args).0, 2);
    }
    assert_eq!(
        r.cli(&["migrate", "propose-mapping", "--from", &fixture]).0,
        2
    );
    let generate_args = [
        "migrate",
        "propose-mapping",
        "--from",
        &fixture,
        "--expect-sha256",
        proposal::CAPTURE_SHA256,
        "--json",
    ];
    let (code, output) = r.cli(&generate_args);
    assert_eq!(code, 0, "{output}");
    assert_eq!(json::parse_unique(&output).unwrap(), draft());
    for duplicate in ["--from", "--expect-sha256"] {
        let mut args = generate_args.to_vec();
        args.extend([
            duplicate,
            if duplicate == "--from" {
                &fixture
            } else {
                proposal::CAPTURE_SHA256
            },
        ]);
        assert_eq!(r.cli(&args).0, 2);
    }
    let args = [
        "migrate",
        "inspect-proposal",
        "--from",
        &proposal_path,
        "--capture",
        &fixture,
        "--expect-sha256",
        proposal::CAPTURE_SHA256,
        "--json",
    ];
    let (code, output) = r.cli(&args);
    assert_eq!(code, 0, "{output}");
    assert_eq!(
        json::parse_unique(&output).unwrap().str("status"),
        Some("VALID_PENDING_PROPOSAL")
    );
    for duplicate in ["--from", "--expect-sha256", "--capture"] {
        let mut duplicated = args.to_vec();
        let argument = match duplicate {
            "--from" => proposal_path.as_str(),
            "--capture" => fixture.as_str(),
            _ => proposal::CAPTURE_SHA256,
        };
        duplicated.extend([duplicate, argument]);
        assert_eq!(r.cli(&duplicated).0, 2);
    }
    r.write("bad.json", "{}");
    assert_eq!(
        r.cli(&[
            "migrate",
            "inspect-proposal",
            "--from",
            r.root.join("bad.json").to_str().unwrap(),
            "--capture",
            &fixture,
            "--expect-sha256",
            proposal::CAPTURE_SHA256
        ])
        .0,
        2
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(r.root.join("proposal.json"), r.root.join("linked.json"))
            .unwrap();
        assert_eq!(
            r.cli(&[
                "migrate",
                "inspect-proposal",
                "--from",
                r.root.join("linked.json").to_str().unwrap(),
                "--capture",
                &fixture,
                "--expect-sha256",
                proposal::CAPTURE_SHA256
            ])
            .0,
            2
        );
    }
    assert_eq!(r.read("proposal.json"), before);
    assert!(!r.exists("other.json"));
}
