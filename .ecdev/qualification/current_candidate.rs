//! Executable current-source handoff candidate. This never installs authority or a lock.
use ecdev_governance as source;
use source::digest::content_digest;
use source::formats::{json, json::Json, Value};
use source::migration::{proposal, source as capture, source_view};
use std::{collections::BTreeSet, io::Write, path::Path};
use ynventa::formats::Value as CentralValue;
use ynventa::migration::ecdev::portable;

fn converted(value: &CentralValue) -> Value {
    match value {
        CentralValue::Null => Value::Null,
        CentralValue::Bool(v) => Value::Bool(*v),
        CentralValue::Int(v) => Value::Int(*v),
        CentralValue::Float(v) => Value::Float(v.clone()),
        CentralValue::Str(v) => Value::Str(v.clone()),
        CentralValue::Array(v) => Value::Array(v.iter().map(converted).collect()),
        CentralValue::Table(v) => {
            Value::Table(v.iter().map(|(k, v)| (k.clone(), converted(v))).collect())
        }
    }
}

fn check(checks: &mut Vec<Json>, name: &str, pass: bool) -> Result<(), String> {
    if !pass {
        return Err(format!("current candidate check failed: {name}"));
    }
    checks.push(Json::obj().with("name", name).with("pass", true));
    Ok(())
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .and_then(|mut file| file.write_all(bytes))
        .map_err(|e| e.to_string())
}

fn run() -> Result<Json, String> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 3 {
        return Err(
            "expected source root, existing external output and caller capture SHA-256".into(),
        );
    }
    let root = std::fs::canonicalize(&args[0]).map_err(|e| e.to_string())?;
    let output = std::fs::canonicalize(&args[1]).map_err(|e| e.to_string())?;
    let expected = args[2].to_str().ok_or("non-UTF8 capture digest")?;
    let engine_root =
        std::fs::canonicalize(ynventa::protocol::engine_repository()).map_err(|e| e.to_string())?;
    if !output.is_dir()
        || output.starts_with(&root)
        || root.starts_with(&output)
        || output.starts_with(&engine_root)
        || engine_root.starts_with(&output)
    {
        return Err("candidate output must be disjoint from source and engine".into());
    }
    if root.join(ynventa::protocol::LOCK_FILE).exists() {
        return Err("maintained source must retain its protective source-only boundary".into());
    }
    let capture_path = output.join("committed-source-capture-v1.json");
    let local = capture::read_export_bound(&capture_path, Some(expected))?;
    let raw = std::fs::read(&capture_path).map_err(|e| e.to_string())?;
    let central = portable::parse_bound(&raw, expected)?;
    let draft = proposal::generate_bound(&local, expected)?.render();
    let value = json::parse_unique(&draft)?;
    let inspected = proposal::inspect_text_bound(&local, &draft, expected)?.render();
    let inspection = json::parse_unique(&inspected)?;
    let mut checks = Vec::new();
    check(
        &mut checks,
        "caller_bound_current_mapping_roundtrip",
        inspection.str("status") == Some("VALID_PENDING_PROPOSAL")
            && inspection.str("capture_sha256") == Some(content_digest(&raw).as_str())
            && proposal::generate(&local).is_err()
            && central.original_bytes == raw,
    )?;

    let graph = converted(&central.source_graph.original);
    let nodes = graph.items_of("nodes");
    let mapped = value.items_of("node_mappings");
    let ids: BTreeSet<_> = mapped.iter().filter_map(|n| n.str("source_id")).collect();
    check(
        &mut checks,
        "all_source_node_ids_and_metadata_preserved",
        mapped.len() == nodes.len()
            && ids.len() == nodes.len()
            && mapped.iter().zip(nodes).all(|(mapped, node)| {
                mapped.get("source_metadata") == Some(node)
                    && mapped.str("source_id") == node.str("id")
                    && mapped
                        .str("source_id")
                        .is_some_and(|id| id.starts_with("ec1:"))
                    && mapped.get("proposed_target") == Some(&Value::Null)
            }),
    )?;

    let lanes = value.get("payloads").ok_or("payload lanes absent")?;
    let payloads: Vec<_> = ["declaration", "history", "evidence", "knowledge"]
        .iter()
        .flat_map(|lane| lanes.items_of(lane))
        .collect();
    let paths: BTreeSet<_> = payloads
        .iter()
        .filter_map(|p| p.str("source_path"))
        .collect();
    check(
        &mut checks,
        "all_four_payload_lanes_match_central_bytes",
        payloads.len() == local.files.len()
            && paths.len() == local.files.len()
            && payloads.iter().all(|p| {
                p.str("source_path").is_some_and(|path| {
                    local
                        .files
                        .get(path)
                        .zip(central.snapshot.files.get(path))
                        .is_some_and(|(bytes, file)| {
                            *bytes == file.bytes
                                && p.str("sha256") == Some(&content_digest(bytes)[7..])
                                && p.get("bytes") == Some(&Value::Int(bytes.len() as i64))
                        })
                })
            }),
    )?;

    let semantics = json::parse_unique(&source_view::inspect(&local)?.render())?;
    let rights = value.items_of("rights_claims");
    check(
        &mut checks,
        "all_rights_and_foreign_semantics_preserved",
        value.get("source_semantics") == Some(&semantics)
            && value.get("target_custody") == Some(&Value::Null)
            && rights.len() == semantics.items_of("donors").len()
            && rights
                .iter()
                .zip(semantics.items_of("donors"))
                .all(|(claim, donor)| {
                    claim.get("source_alias") == donor.get("source_alias")
                        && claim.get("source_id") == donor.get("source_id")
                        && claim.get("license_claim") == donor.get("license")
                        && claim.get("origin") == donor.get("origin")
                        && claim.get("provenance") == donor.get("provenance")
                }),
    )?;
    let candidates = value.items_of("global_identity_candidates");
    check(
        &mut checks,
        "current_global_hash_candidates_match_published_rule",
        candidates.len() == rights.len()
            && candidates.iter().all(|candidate| {
                candidate.str("candidate_id")
                    == Some(
                        ynventa::graph::NodeId::of(
                            candidate.str("source_namespace").unwrap_or(""),
                            candidate.str("source_semantic_key").unwrap_or(""),
                        )
                        .to_string()
                        .as_str(),
                    )
            }),
    )?;
    check(
        &mut checks,
        "stale_capture_pin_refused",
        proposal::generate_bound(&local, &"0".repeat(64)).is_err(),
    )?;
    let mut self_grant = value.clone();
    if let Value::Table(fields) = &mut self_grant {
        fields.insert(
            "target_custody".into(),
            Value::Str("source-self-grant".into()),
        );
    }
    check(
        &mut checks,
        "target_self_grant_refused",
        proposal::inspect_bound(&local, &self_grant, expected).is_err(),
    )?;
    let lock = ynventa::protocol::current_lock();
    let parsed = ynventa::protocol::Lock::parse(&lock.render()).ok_or("engine lock roundtrip")?;
    let refusal = ynventa::assess(&root)
        .err()
        .ok_or("source unexpectedly admitted")?;
    // A matching lock does not constitute the missing authority handoff. Exercise that claim
    // on an inert negative fixture in the external output, never in the maintained source.
    let negative = output.join("negative-matching-lock-fixture");
    std::fs::create_dir(&negative).map_err(|e| e.to_string())?;
    for (path, bytes) in &local.files {
        let destination = negative.join(path);
        std::fs::create_dir_all(destination.parent().ok_or("negative payload parent")?)
            .map_err(|e| e.to_string())?;
        write_new(&destination, bytes)?;
    }
    let negative_lock = negative.join(ynventa::protocol::LOCK_FILE);
    std::fs::create_dir_all(negative_lock.parent().ok_or("negative lock parent")?)
        .map_err(|e| e.to_string())?;
    write_new(&negative_lock, lock.render().as_bytes())?;
    let locked_refusal = ynventa::assess(&negative)
        .err()
        .ok_or("matching lock unexpectedly admitted source fixture")?;
    let negative_source = source::assess(&negative)?;
    check(
        &mut checks,
        "matching_protocol_lock_does_not_enroll_source",
        parsed.mismatch(&lock).is_none()
            && ynventa::protocol::shard("ecdev").is_none()
            && refusal.contains("ECDEV migration preflight")
            && refusal.contains("BLOCKED:")
            && locked_refusal.contains("ECDEV migration preflight")
            && locked_refusal.contains("BLOCKED:")
            && !source::conformance::source_migration_violations(&negative_source).is_empty()
            && !root.join(ynventa::protocol::LOCK_FILE).exists(),
    )?;
    let registered_ids: Vec<String> = ynventa::protocol::SHARDS
        .iter()
        .map(|shard| shard.id.to_string())
        .collect();
    let mut expected_ids = registered_ids.clone();
    expected_ids.push("ecdev".into());
    expected_ids.sort();
    expected_ids.dedup();
    write_new(
        &output.join("source-eight-mapping-candidate-v1.json"),
        draft.as_bytes(),
    )?;
    write_new(
        &output.join("source-eight-mapping-inspection-v1.json"),
        inspected.as_bytes(),
    )?;
    Ok(Json::obj()
        .with("format", "ecdev-source-eight-candidate-v1").with("status", "PASS")
        .with("source_id", "ecdev").with("source_repository", "llgtrn/ECDEV")
        .with("scope", "CURRENT_SOURCE_MAPPING_AND_CUSTODY_CANDIDATE; NO_ENROLLMENT_OR_CAPABILITY_ADMISSION")
        .with("capture_sha256", content_digest(&raw))
        .with("mapping_sha256", content_digest(draft.as_bytes()))
        .with("inspection_sha256", content_digest(inspected.as_bytes()))
        .with("source_nodes", mapped.len()).with("payloads", payloads.len())
        .with("facts", semantics.items_of("facts").len()).with("rights_claims", rights.len())
        .with("canonical_enrollment", "BLOCKED").with("registry_binding", "PENDING")
        .with("registered_sources", ynventa::protocol::SHARDS.len()).with("source_registered", false)
        .with("source_lock_installed", false).with("current_rights_authorization", Json::Null)
        .with("target_mapping", Json::Null).with("target_custody", Json::Null).with("proposed_target", Json::Null)
        .with("tests_passed", checks.len()).with("tests_failed", 0usize).with("checks", checks)
        .with("ordinary_assessment_refusal", refusal)
        .with("negative_matching_lock_fixture_refusal", locked_refusal)
        .with("current_engine_lock_diagnostic_only", lock.render())
        .with("candidate_registry_expectation", Json::obj()
            .with("current_registered_source_ids", registered_ids)
            .with("required_closed_source_ids", expected_ids)
            .with("source_id", "ecdev").with("source_repository", "llgtrn/ECDEV")
            .with("canonical_source_owner", Json::Null)
            .with("registry_entry_accepted", false)
            .with("new_common_lock", Json::Null))
        .with("source_grammar", "Portable ecdev-source-capture-v1 and caller-bound mapping accepted; canonical declaration/identity/codec adapter absent")
        .with("migration_readiness", "All four original lanes retained; commit membership independently checked by runner; owner and target bindings unresolved")
        .with("actual_native_integration", "Separate source caller proofs; not reverified or admitted by this custody candidate")
        .with("source_retirement", "Not performed; executable .ecdev authority and source data remain in place")
        .with("required_engine_changes", vec![
            "Explicit lossless ECDEV source admission adapter and qualified authority handoff; preserve ec1/f1/ECG as original source identities and records",
            "Authorized target/owner mappings and rights distinct from source registration and bounded capability admission",
            "Eighth canonical registry entry plus a new verified schema/engine/protocol/epoch lock contract for all eight consumers",
            "Ordinary assessment and capsule production for admitted source; exact eight-source identity and lock closure, not found-checkout success",
            "Protective source-only refusal remains until the qualified handoff is accepted; portable Pending must remain distinct from enrollment",
        ]))
}

fn main() {
    match run() {
        Ok(report) => println!("{}", report.render()),
        Err(error) => {
            eprintln!("current candidate refused: {error}");
            std::process::exit(1);
        }
    }
}
