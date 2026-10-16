//! Bounded source-custody interoperability; no source admission or proof execution.
use ecdev_governance as source;
use source::compact::{codec::Decoder, facts, history};
use source::digest::content_digest;
use source::formats::{json, json::Json, rust as source_rust, Value};
use source::migration::{source as capture, source_view};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::Path;
use ynventa::formats::{rust as central_rust, Value as CentralValue};
use ynventa::migration::ecdev::{self as central, portable};

const ENGINE_COMMIT: &str = "217fd75c53795d31fa0a2a989a04ca49bdacfa80";
const SCHEMA: &str = "sha256:4e3b2d05c3970ed4fa2a06b5a2430a5289301cf1e455789c3f2f88ed0cff0abc";
const ENGINE: &str = "sha256:7fc475276e0d24d42a452ddd8370df79e53129863bc84ed5c21082cf65d6915d";
const FIXTURE: &str = "c751083d31323cf190a687452233277459d84967a6d5daf04bffb830f03c654d";

fn check(checks: &mut Vec<Json>, name: &str, pass: bool) -> Result<(), String> {
    checks.push(Json::obj().with("name", name).with("pass", pass));
    if pass {
        Ok(())
    } else {
        Err(format!("custody check failed: {name}"))
    }
}

fn source_value(value: &CentralValue) -> Value {
    match value {
        CentralValue::Null => Value::Null,
        CentralValue::Bool(v) => Value::Bool(*v),
        CentralValue::Int(v) => Value::Int(*v),
        CentralValue::Float(v) => Value::Float(v.clone()),
        CentralValue::Str(v) => Value::Str(v.clone()),
        CentralValue::Array(v) => Value::Array(v.iter().map(source_value).collect()),
        CentralValue::Table(v) => Value::Table(
            v.iter()
                .map(|(key, value)| (key.clone(), source_value(value)))
                .collect(),
        ),
    }
}

fn central_expr(expr: &source_rust::Expr) -> central_rust::Expr {
    use central_rust::Kind as C;
    use source_rust::Kind as S;
    central_rust::Expr {
        line: expr.line,
        kind: match &expr.kind {
            S::Struct(name, fields) => C::Struct(
                name.clone(),
                fields
                    .iter()
                    .map(|(key, expr)| (key.clone(), central_expr(expr)))
                    .collect(),
            ),
            S::Path(v) => C::Path(v.clone()),
            S::Call(name, args) => C::Call(name.clone(), args.iter().map(central_expr).collect()),
            S::Str(v) => C::Str(v.clone()),
            S::Int(v) => C::Int(*v),
            S::Bool(v) => C::Bool(*v),
            S::List(v) => C::List(v.iter().map(central_expr).collect()),
        },
    }
}

fn strings(value: Option<&Value>) -> BTreeSet<String> {
    value
        .map(Value::items)
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn measured(value: Option<&Value>) -> Result<usize, String> {
    match value {
        Some(Value::Int(n)) if *n >= 0 => usize::try_from(*n).map_err(|e| e.to_string()),
        _ => Err("missing measured source count".into()),
    }
}

fn compare(
    label: &str,
    original: &[u8],
    local: &capture::Capture,
    projected: &portable::SourceProjection,
    checks: &mut Vec<Json>,
) -> Result<Json, String> {
    let prefix = |name: &str| format!("{label}.{name}");
    let snapshot = &projected.snapshot;
    check(
        checks,
        &prefix("original_envelope_bytes"),
        projected.original_bytes == original,
    )?;
    check(
        checks,
        &prefix("envelope_digest"),
        projected.envelope_digest == content_digest(original),
    )?;
    check(
        checks,
        &prefix("registry_pending"),
        projected.registry_binding == portable::RegistryBinding::Pending,
    )?;
    check(
        checks,
        &prefix("all_original_file_bytes_and_digests"),
        local.files.len() == snapshot.files.len()
            && local.files.iter().all(|(path, bytes)| {
                snapshot.files.get(path).is_some_and(|file| {
                    file.path == *path
                        && file.bytes == *bytes
                        && file.digest == content_digest(bytes)
                })
            }),
    )?;
    for (name, expr) in &snapshot.declarations {
        let path = format!(".ecdev/declared/{name}");
        let text = std::str::from_utf8(&local.files[&path]).map_err(|e| e.to_string())?;
        let source_ast = source_rust::parse(text).map_err(|e| e.to_string())?;
        check(
            checks,
            &prefix(&format!("full_declaration_ast.{name}")),
            central_expr(&source_ast) == *expr,
        )?;
    }
    let view = json::parse(&source_view::inspect(local)?.render())?;
    check(
        checks,
        &prefix("original_graph_metadata_and_ec1_ids"),
        view.get("source_graph") == Some(&source_value(&projected.source_graph.original))
            && projected
                .source_graph
                .nodes
                .iter()
                .all(|n| n.id.starts_with("ec1:")),
    )?;
    let declaration = local.declaration()?;
    check(
        checks,
        &prefix("source_metadata"),
        projected.source_id == declaration.repository.id
            && projected.source_repository == declaration.repository.origin
            && projected.observed_head == local.observed_head
            && projected.source_graph.schema_claim == local.source_schema,
    )?;
    let mut source_facts = Vec::new();
    let mut registered = BTreeSet::new();
    let mut historical_caps = BTreeSet::new();
    let mut floor = false;
    for (path, bytes) in &local.files {
        if path.starts_with(".ecdev/knowledge/") {
            source_facts.extend(facts::decode(bytes).map_err(|e| e.to_string())?);
        } else if path.starts_with(".ecdev/history/") {
            let batch = history::Batch::decode(bytes).map_err(|e| e.to_string())?;
            registered.extend(batch.registered);
            historical_caps.extend(batch.capabilities);
            floor |= Decoder::tag_of(bytes) == Some(history::HISTORY_TAG);
        }
    }
    check(
        checks,
        &prefix("f1_fact_values_sequences_provenance_and_supersession"),
        source_facts.len() == snapshot.facts.len()
            && source_facts.iter().zip(&snapshot.facts).all(|(s, c)| {
                s.id() == c.id
                    && s.kind.wire() == c.kind
                    && s.subject == c.subject
                    && s.key == c.key
                    && s.value == c.value
                    && s.provenance.iter().cloned().collect::<Vec<_>>() == c.provenance
                    && s.seq == c.seq
                    && s.superseded == c.superseded
            }),
    )?;
    check(
        checks,
        &prefix("historical_denominator_and_same_donor_absorption"),
        registered == snapshot.history_donors
            && historical_caps == snapshot.history_capabilities
            && floor == snapshot.capability_floor_recorded
            && strings(view.at("history.declared_capabilities")) == snapshot.declared_capabilities
            && strings(view.at("history.absorbed_same_donor_keys"))
                == snapshot.absorbed_capabilities
            && match view.at("history.known_capabilities") {
                Some(Value::Null) => snapshot.known_capabilities().is_none(),
                values => snapshot.known_capabilities() == Some(strings(values)),
            },
    )?;
    check(
        checks,
        &prefix("active_and_retired_evidence_are_separate"),
        measured(view.get("active_evidence_records"))? == snapshot.evidence_records
            && measured(view.get("retired_evidence_records"))? == snapshot.retired_evidence_records,
    )?;
    check(
        checks,
        &prefix("no_canonical_mapping_or_effective_verdict"),
        view.str("canonical_enrollment") == Some("BLOCKED")
            && view.get("canonical_identity_mapping") == Some(&Value::Null)
            && view.get("norl_relevance_mapping") == Some(&Value::Null)
            && view
                .items_of("donors")
                .iter()
                .all(|d| d.get("canonical_effective_state") == Some(&Value::Null)),
    )?;
    let mut adoption = BTreeMap::<String, usize>::new();
    let mut knowledge = BTreeMap::<String, usize>::new();
    for donor in view.items_of("donors") {
        for capability in donor.items_of("capabilities") {
            *adoption
                .entry(
                    capability
                        .at("adoption.variant")
                        .and_then(Value::as_str)
                        .unwrap_or("UNKNOWN")
                        .into(),
                )
                .or_default() += 1;
            *knowledge
                .entry(
                    capability
                        .at("knowledge.variant")
                        .and_then(Value::as_str)
                        .unwrap_or("UNKNOWN")
                        .into(),
                )
                .or_default() += 1;
        }
    }
    let histogram = |values: BTreeMap<String, usize>| {
        Json::Object(
            values
                .into_iter()
                .map(|(key, n)| (key, Json::from(n)))
                .collect(),
        )
    };
    Ok(Json::obj()
        .with("sha256", &projected.envelope_digest)
        .with("bytes", original.len())
        .with("observed_head", &projected.observed_head)
        .with(
            "producer_schema_claim",
            &projected.source_graph.schema_claim,
        )
        .with("files", snapshot.files.len())
        .with(
            "raw_bytes",
            local.files.values().map(Vec::len).sum::<usize>(),
        )
        .with("graph_nodes", projected.source_graph.nodes.len())
        .with("graph_edges", projected.source_graph.edges.len())
        .with("declared_nodes", snapshot.node_keys.len())
        .with("declared_donors", snapshot.donor_keys.len())
        .with(
            "declared_capabilities",
            snapshot.declared_capabilities.len(),
        )
        .with("historical_donors", snapshot.history_donors.len())
        .with(
            "historical_capabilities",
            snapshot.history_capabilities.len(),
        )
        .with(
            "capability_floor_recorded",
            snapshot.capability_floor_recorded,
        )
        .with("facts", snapshot.facts.len())
        .with("active_evidence_records", snapshot.evidence_records)
        .with(
            "retired_evidence_records",
            snapshot.retired_evidence_records,
        )
        .with("adoption_variants", histogram(adoption))
        .with("knowledge_variants", histogram(knowledge))
        .with("registry_binding", "PENDING")
        .with("proof_freshness", "NOT_REVERIFIED"))
}

fn replace_once(input: &str, old: &str, new: &str) -> Result<String, String> {
    if !input.contains(old) || old == new {
        return Err(format!("negative fixture mutation did not apply: {old}"));
    }
    Ok(input.replacen(old, new, 1))
}

fn probe(
    name: &str,
    input: &str,
    central_expected: bool,
    source_expected: bool,
) -> Result<Json, String> {
    let digest = content_digest(input.as_bytes());
    let c = portable::parse_bound(input.as_bytes(), &digest[7..]);
    let s = capture::Capture::parse(input);
    if c.is_ok() != central_expected || s.is_ok() != source_expected {
        return Err(format!("{name}: unexpected reader outcomes: central={}, source={}; central error={:?}; source error={:?}", c.is_ok(), s.is_ok(), c.as_ref().err(), s.as_ref().err()));
    }
    if let Ok(projection) = &c {
        if projection.registry_binding != portable::RegistryBinding::Pending
            || projection.original_bytes != input.as_bytes()
        {
            return Err(format!(
                "{name}: accepted custody input changed bytes or registry status"
            ));
        }
    }
    Ok(Json::obj()
        .with("name", name)
        .with("pass", true)
        .with("central_accepts", c.is_ok())
        .with("source_accepts", s.is_ok())
        .with("central_error", c.err().map_or(Json::Null, Json::from))
        .with("source_error", s.err().map_or(Json::Null, Json::from))
        .with(
            "meaning",
            if central_expected != source_expected {
                "DOCUMENTED_READER_SCOPE_DIFFERENCE; NO_ADMISSION"
            } else {
                "SOURCE_CUSTODY_ONLY"
            },
        ))
}

fn run() -> Result<Json, String> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("expected source root and existing external output directory".into());
    }
    let root = std::fs::canonicalize(Path::new(&args[0])).map_err(|e| e.to_string())?;
    let output = std::fs::canonicalize(Path::new(&args[1])).map_err(|e| e.to_string())?;
    let engine_root = std::fs::canonicalize(ynventa::protocol::engine_repository())
        .map_err(|e| format!("engine reference root: {e}"))?;
    if !output.is_dir()
        || output.starts_with(&root)
        || root.starts_with(&output)
        || output.starts_with(&engine_root)
        || engine_root.starts_with(&output)
    {
        return Err("output directory must already exist and be disjoint from source and engine reference trees".into());
    }
    let mut checks = Vec::new();
    let lock = ynventa::protocol::current_lock();
    check(
        &mut checks,
        "pinned_engine_contract",
        lock.protocol_version == 1
            && lock.compatibility_epoch == 1
            && lock.schema_digest == SCHEMA
            && lock.engine_digest == ENGINE,
    )?;
    check(
        &mut checks,
        "seven_registered_sources_exclude_ecdev",
        ynventa::protocol::SHARDS.len() == 7 && ynventa::protocol::shard("ecdev").is_none(),
    )?;
    let mapping_capture = capture::read_export_bound(
        &root.join(".ecdev/tests/fixtures/source-capture-00dd7f0-v1.json"),
        Some(source::migration::proposal::CAPTURE_SHA256),
    )?;
    let mapping = source::migration::proposal::generate(&mapping_capture)?;
    let mapping_value = json::parse_unique(&mapping.render())?;
    check(
        &mut checks,
        "all_source_mapping_hash_candidates_match_published_central_identity_rule",
        mapping_value.items_of("global_identity_candidates").len() == 68
            && mapping_value
                .items_of("global_identity_candidates")
                .iter()
                .all(|c| {
                    c.str("candidate_id")
                        == Some(
                            ynventa::graph::NodeId::of(
                                c.str("source_namespace").unwrap_or(""),
                                c.str("source_semantic_key").unwrap_or(""),
                            )
                            .to_string()
                            .as_str(),
                        )
                }),
    )?;
    check(
        &mut checks,
        "mapping_proposal_inspection_remains_pending_without_target_custody",
        json::parse(
            &source::migration::proposal::inspect_text(&mapping_capture, &mapping.render())?
                .render(),
        )?
        .str("status")
            == Some("VALID_PENDING_PROPOSAL"),
    )?;
    let refusal = match ynventa::assess(&root) {
        Ok(_) => return Err("custody source unexpectedly received canonical assessment".into()),
        Err(refusal) => refusal,
    };
    check(
        &mut checks,
        "ordinary_canonical_assessment_still_refuses_source_enrollment",
        refusal.contains("ECDEV migration preflight") && refusal.contains("BLOCKED:"),
    )?;

    let fixture_path = root.join(".ecdev/tests/fixtures/source-capture-v1.json");
    let fixture_bytes = std::fs::read(&fixture_path).map_err(|e| e.to_string())?;
    check(
        &mut checks,
        "immutable_fixture_digest",
        content_digest(&fixture_bytes) == format!("sha256:{FIXTURE}"),
    )?;
    let fixture_text = std::str::from_utf8(&fixture_bytes).map_err(|e| e.to_string())?;
    let fixture_source = capture::Capture::parse(fixture_text)?;
    let fixture_central = portable::parse_bound(&fixture_bytes, FIXTURE)?;
    check(
        &mut checks,
        "fixture_file_and_memory_entrypoints_agree",
        portable::read_bound(&fixture_path, FIXTURE)? == fixture_central,
    )?;
    let fixture_report = compare(
        "fixture",
        &fixture_bytes,
        &fixture_source,
        &fixture_central,
        &mut checks,
    )?;
    check(
        &mut checks,
        "immutable_fixture_lane_counts",
        fixture_central.snapshot.files.len() == 590
            && fixture_central.snapshot.facts.len() == 1861
            && fixture_central.snapshot.evidence_records == 582
            && fixture_central.snapshot.retired_evidence_records == 8,
    )?;

    let fresh_source = capture::capture_committed(&root)?;
    let fresh_bytes = fresh_source.render()?.into_bytes();
    let fresh_digest = content_digest(&fresh_bytes);
    let fresh_path = output.join("committed-source-capture-v1.json");
    let mut fresh_file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&fresh_path)
        .map_err(|e| format!("fresh capture output: {e}"))?;
    fresh_file
        .write_all(&fresh_bytes)
        .map_err(|e| e.to_string())?;
    drop(fresh_file);
    check(
        &mut checks,
        "source_bound_file_roundtrip",
        capture::read_export_bound(&fresh_path, Some(&fresh_digest[7..]))? == fresh_source,
    )?;
    let fresh_central = portable::parse_bound(&fresh_bytes, &fresh_digest[7..])?;
    check(
        &mut checks,
        "fresh_file_and_memory_entrypoints_agree",
        portable::read_bound(&fresh_path, &fresh_digest[7..])? == fresh_central,
    )?;
    let directory = central::read(&root)?.ok_or("source directory snapshot absent")?;
    check(
        &mut checks,
        "full_committed_portable_and_current_directory_snapshots_agree",
        fresh_central.snapshot == directory,
    )?;
    let fresh_report = compare(
        "fresh",
        &fresh_bytes,
        &fresh_source,
        &fresh_central,
        &mut checks,
    )?;
    check(
        &mut checks,
        "source_and_central_digest_algorithms_agree",
        ynventa::digest::content_digest(&fresh_bytes) == fresh_digest,
    )?;
    check(
        &mut checks,
        "missing_expected_digest_refused",
        portable::parse_bound(&fixture_bytes, "").is_err(),
    )?;
    check(
        &mut checks,
        "wrong_expected_digest_refused",
        portable::parse_bound(&fixture_bytes, &"0".repeat(64)).is_err(),
    )?;
    check(
        &mut checks,
        "prefixed_expected_digest_refused",
        portable::parse_bound(&fixture_bytes, &format!("sha256:{FIXTURE}")).is_err(),
    )?;
    let mut tampered = fixture_bytes.clone();
    tampered[0] ^= 1;
    check(
        &mut checks,
        "raw_tamper_under_original_digest_refused",
        portable::parse_bound(&tampered, FIXTURE).is_err(),
    )?;
    check(
        &mut checks,
        "source_wrong_expected_digest_refused",
        capture::read_export_bound(&fresh_path, Some(&"0".repeat(64))).is_err(),
    )?;

    let mut probes = Vec::new();
    for (name, text) in [
        (
            "truncated_json",
            fixture_text[..fixture_text.len() / 2].to_string(),
        ),
        (
            "duplicate_root_key",
            replace_once(fixture_text, "{\n", "{\n  \"format\": \"duplicate\",\n")?,
        ),
        (
            "unknown_root_field",
            replace_once(fixture_text, "{\n", "{\n  \"unknown\": null,\n")?,
        ),
        (
            "unsupported_format_version",
            replace_once(
                fixture_text,
                "ecdev-source-capture-v1",
                "ecdev-source-capture-v9",
            )?,
        ),
        (
            "unsupported_binding",
            replace_once(fixture_text, "HEAD_OBSERVED_ONLY", "HEAD_CERTIFIED")?,
        ),
        (
            "source_identity_drift",
            replace_once(
                fixture_text,
                "\"source_id\": \"ecdev\"",
                "\"source_id\": \"other\"",
            )?,
        ),
        (
            "source_origin_mismatch",
            replace_once(
                fixture_text,
                "\"source_repository\": \"llgtrn/ECDEV\"",
                "\"source_repository\": \"unverified/other\"",
            )?,
        ),
        (
            "invalid_observed_head",
            replace_once(fixture_text, &fixture_source.observed_head, "bad-head")?,
        ),
        (
            "unknown_graph_field",
            replace_once(
                fixture_text,
                "\"source_graph\": {",
                "\"source_graph\": {\n    \"unknown\": null,",
            )?,
        ),
        (
            "invalid_graph_identity_prefix",
            replace_once(fixture_text, "ec1:", "ec9:")?,
        ),
        (
            "per_file_digest_drift",
            replace_once(fixture_text, "\"sha256\": \"", "\"sha256\": \"00")?,
        ),
        (
            "unsafe_source_path",
            replace_once(
                fixture_text,
                "\"path\": \".ecdev/",
                "\"path\": \"../.ecdev/",
            )?,
        ),
    ] {
        probes.push(probe(name, &text, false, false)?);
    }
    let node = &fixture_central.source_graph.nodes[0];
    let name = node
        .metadata
        .str("name")
        .ok_or("source graph name absent")?;
    let name_drift = replace_once(
        fixture_text,
        &format!("\"name\": {}", Json::from(name).render_line()),
        &format!(
            "\"name\": {}",
            Json::from(format!("{name} producer-metadata-only")).render_line()
        ),
    )?;
    probes.push(probe(
        "producer_graph_name_not_independently_rebuilt",
        &name_drift,
        true,
        false,
    )?);
    let replacement_id = format!("ec1:{}", "0".repeat(32));
    if fixture_central
        .source_graph
        .nodes
        .iter()
        .any(|n| n.id == replacement_id)
    {
        return Err("differential identity mutation collides with source graph".into());
    }
    let id_drift = fixture_text.replace(&node.id, &replacement_id);
    probes.push(probe(
        "producer_ec1_hash_not_independently_recomputed",
        &id_drift,
        true,
        false,
    )?);
    probes.push(probe(
        "noncanonical_whitespace_is_custody_compatible",
        &(fixture_text.to_string() + " \n"),
        true,
        false,
    )?);
    let schema_claim = replace_once(
        fixture_text,
        &fixture_source.source_schema,
        &format!("sha256:{}", "a".repeat(64)),
    )?;
    probes.push(probe(
        "arbitrary_valid_producer_schema_claim_stays_unverified",
        &schema_claim,
        true,
        true,
    )?);
    let observed = replace_once(fixture_text, &fixture_source.observed_head, &"a".repeat(40))?;
    probes.push(probe(
        "valid_observed_head_is_not_git_membership_proof",
        &observed,
        true,
        true,
    )?);

    let count = checks.len() + probes.len();
    Ok(Json::obj()
        .with("format", "ecdev-central-intake-qualification-v1")
        .with("status", "PASS")
        .with("scope", "READ_ONLY_SOURCE_CUSTODY_INTEROPERABILITY; NO_CAPTURED_DONOR_PROGRAM_OR_DECLARED_PROOF_EXECUTED")
        .with("engine_commit", ENGINE_COMMIT)
        .with("protocol_version", lock.protocol_version as u64)
        .with("compatibility_epoch", lock.compatibility_epoch as u64)
        .with("central_schema", lock.schema_digest)
        .with("central_engine", lock.engine_digest)
        .with("registered_sources", ynventa::protocol::SHARDS.len())
        .with("source_registered", false)
        .with("fixture", fixture_report)
        .with("fresh", fresh_report)
        .with("fresh_artifact", "committed-source-capture-v1.json")
        .with("tests_passed", count)
        .with("tests_failed", 0usize)
        .with("checks", checks)
        .with("reader_probes", probes)
        .with("preserved_unknowns", vec![
            "REGISTRY_BINDING_PENDING; NO_CANONICAL_IDENTITY_OR_OWNERSHIP_MAPPING",
            "SOURCE_HEAD_OBSERVED_ONLY; ENVELOPE_HAS_NO_GIT_MEMBERSHIP_PROOF",
            "PRODUCER_SCHEMA_AND_GRAPH_ARE_CUSTODIED_SOURCE_CLAIMS",
            "NO_PROOF_FRESHNESS_OR_CURRENT_RIGHTS_REVERIFICATION",
            "NO_EXECUTED_ACCEPTANCE_PRODUCER; NO_NATIVE_HARDWARE_CERTIFICATION",
            "NO_OUTGOING_OBSERVER_TRANSITION_QUALIFIED_BY_THIS_INTAKE",
            "DONOR_CLASSIFICATION_AND_AUDITED_OSS_COUNTS_REMAIN_UNKNOWN",
        ]))
}

fn main() {
    match run() {
        Ok(report) => print!("{}", report.render()),
        Err(error) => {
            print!(
                "{}",
                Json::obj()
                    .with("format", "ecdev-central-intake-qualification-v1")
                    .with("status", "FAIL")
                    .with("error", error)
                    .render()
            );
            std::process::exit(1);
        }
    }
}
