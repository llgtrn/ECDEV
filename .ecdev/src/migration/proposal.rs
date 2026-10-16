//! Pending mappings of an exact source capture. This module neither reads a target
//! repository nor installs identities, ownership, rights, declarations, or evidence.

use super::{source::Capture, source_view};
use crate::digest::{content_digest, hex, Sha256};
use crate::formats::{json, json::Json, Value};
use std::collections::BTreeSet;

pub const FORMAT: &str = "ecdev-source-mapping-proposal-v1";
pub const CAPTURE_SHA256: &str = "ef930f0e26aab701ef384f9992d0b32abf3d101ac5bcb842ffa4d045bdce9a1d";
pub const MAX_PROPOSAL_BYTES: usize = 16 * 1024 * 1024;
const SCOPE: &str = "PROPOSAL_ONLY; NOT_ENROLLED";
const REASONS: [&str; 2] = [
    "TARGET_CUSTODY_PIN_NOT_PROVIDED",
    "CANONICAL_OWNER_DECISION_NOT_BOUND",
];
const LANES: [&str; 4] = ["declaration", "history", "evidence", "knowledge"];
const ROOT_FIELDS: [&str; 11] = [
    "format",
    "scope",
    "capture_binding",
    "target_custody",
    "unresolved_reasons",
    "node_mappings",
    "payloads",
    "rights_claims",
    "source_semantics",
    "global_identity_candidates",
    "candidate_scope",
];
const NODE_FIELDS: [&str; 7] = [
    "source_id",
    "source_namespace",
    "source_semantic_key",
    "capture_locator",
    "source_metadata",
    "proposed_target",
    "unresolved_reasons",
];
const PAYLOAD_FIELDS: [&str; 5] = [
    "source_path",
    "capture_locator",
    "hex_locator",
    "sha256",
    "bytes",
];
const RIGHTS_FIELDS: [&str; 8] = [
    "source_alias",
    "source_id",
    "origin",
    "license_claim",
    "provenance",
    "declaration_capture_locator",
    "source_semantics_locator",
    "scope",
];
const CANDIDATE_FIELDS: [&str; 7] = [
    "source_id",
    "source_namespace",
    "source_semantic_key",
    "capture_locator",
    "candidate_id",
    "identity_rule",
    "status",
];

fn foreign(value: &Value) -> Json {
    match value {
        Value::Null => Json::Null,
        Value::Bool(v) => Json::Bool(*v),
        Value::Int(v) => Json::Int(*v),
        Value::Float(v) => Json::Number(v.clone()),
        Value::Str(v) => Json::Str(v.clone()),
        Value::Array(v) => Json::Array(v.iter().map(foreign).collect()),
        Value::Table(v) => Json::Object(
            v.iter()
                .map(|(key, value)| (key.clone(), foreign(value)))
                .collect(),
        ),
    }
}

fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .str(key)
        .ok_or_else(|| format!("PROPOSAL_STRING_REQUIRED: {key}"))
}

fn field<'a>(value: &'a Value, key: &str) -> Result<&'a Value, String> {
    value
        .get(key)
        .ok_or_else(|| format!("PROPOSAL_FIELD_REQUIRED: {key}"))
}

fn array<'a>(value: &'a Value, key: &str) -> Result<&'a [Value], String> {
    match field(value, key)? {
        Value::Array(values) => Ok(values),
        _ => Err(format!("PROPOSAL_ARRAY_REQUIRED: {key}")),
    }
}

fn closed(value: &Value, fields: &[&str]) -> Result<(), String> {
    let object = value.table().ok_or("PROPOSAL_OBJECT_REQUIRED")?;
    if let Some(key) = object.keys().find(|key| !fields.contains(&key.as_str())) {
        return Err(format!("UNKNOWN_PROPOSAL_FIELD: {key}"));
    }
    if let Some(key) = fields.iter().find(|key| !object.contains_key(**key)) {
        return Err(format!("PROPOSAL_FIELD_REQUIRED: {key}"));
    }
    Ok(())
}

fn unresolved() -> Json {
    Json::from(REASONS.to_vec())
}

fn candidate_id(namespace: &str, semantic_key: &str) -> String {
    let mut h = Sha256::new();
    h.field(b"ynventa.node-id.v1");
    h.field(namespace.as_bytes());
    h.field(semantic_key.as_bytes());
    format!("yn1:{}", hex(&h.finish()[..16]))
}

fn lane(path: &str) -> Result<usize, String> {
    ["declared", "history", "evidence", "knowledge"]
        .iter()
        .position(|name| path.starts_with(&format!(".ecdev/{name}/")))
        .ok_or_else(|| format!("UNSUPPORTED_SOURCE_PAYLOAD_LANE: {path}"))
}

/// Produce a deterministic draft for the explicitly pinned caller artifact. Every
/// locator indexes that capture's canonical envelope, not a reconstructed checkout.
pub fn generate(capture: &Capture) -> Result<Json, String> {
    generate_bound(capture, CAPTURE_SHA256)
}

/// Produce a pending draft for the exact canonical capture bytes selected by the caller.
/// The digest is integrity input, not a source registration, target owner or rights grant.
pub fn generate_bound(capture: &Capture, expected_sha256: &str) -> Result<Json, String> {
    if expected_sha256.len() != 64 || !expected_sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("SOURCE_CAPTURE_PIN_INVALID".into());
    }
    let bytes = capture.render()?;
    let digest = content_digest(bytes.as_bytes());
    if !digest[7..].eq_ignore_ascii_case(expected_sha256) {
        return Err("SOURCE_CAPTURE_PIN_MISMATCH".into());
    }
    let envelope = json::parse_unique(&bytes)?;
    let graph = field(&envelope, "source_graph")?;
    let nodes = array(graph, "nodes")?;
    let files = array(&envelope, "files")?;
    let semantics = source_view::inspect(capture)?;
    let semantic_value = json::parse_unique(&semantics.render())?;
    let mut ids = BTreeSet::new();
    let mut keys = BTreeSet::new();
    let mut mappings = Vec::new();
    let mut candidates = Vec::new();
    for (index, node) in nodes.iter().enumerate() {
        let id = text(node, "id")?;
        let namespace = text(node, "namespace")?;
        let key = text(node, "semantic_key")?;
        if !ids.insert(id) || !keys.insert((namespace, key)) {
            return Err("DUPLICATE_SOURCE_GRAPH_IDENTITY".into());
        }
        let locator = format!("/source_graph/nodes/{index}");
        mappings.push(
            Json::obj()
                .with("source_id", id)
                .with("source_namespace", namespace)
                .with("source_semantic_key", key)
                .with("capture_locator", &locator)
                .with("source_metadata", foreign(node))
                .with("proposed_target", Json::Null)
                .with("unresolved_reasons", unresolved()),
        );
        if namespace == "oss" {
            candidates.push(
                Json::obj()
                    .with("source_id", id)
                    .with("source_namespace", namespace)
                    .with("source_semantic_key", key)
                    .with("capture_locator", locator)
                    .with("candidate_id", candidate_id(namespace, key))
                    .with(
                        "identity_rule",
                        "ynventa.node-id.v1; SHA256; u64 big-endian field lengths; first 16 bytes",
                    )
                    .with(
                        "status",
                        "HASH_CANDIDATE_ONLY; TARGET_EXISTENCE_AND_OWNERSHIP_NOT_CHECKED",
                    ),
            );
        }
    }
    let mut payloads: [Vec<Json>; 4] = std::array::from_fn(|_| Vec::new());
    let mut donor_declaration = None;
    for (index, file) in files.iter().enumerate() {
        let path = text(file, "path")?;
        let locator = format!("/files/{index}");
        if path == ".ecdev/declared/donors.rs" {
            donor_declaration = Some(locator.clone());
        }
        payloads[lane(path)?].push(
            Json::obj()
                .with("source_path", path)
                .with("capture_locator", locator)
                .with("hex_locator", format!("/files/{index}/hex"))
                .with("sha256", text(file, "sha256")?)
                .with("bytes", capture.files[path].len()),
        );
    }
    let donor_declaration = donor_declaration.ok_or("SOURCE_DONOR_DECLARATION_MISSING")?;
    let rights = array(&semantic_value, "donors")?
        .iter()
        .enumerate()
        .map(|(index, donor)| {
            Ok(Json::obj()
                .with("source_alias", text(donor, "source_alias")?)
                .with("source_id", text(donor, "source_id")?)
                .with("origin", text(donor, "origin")?)
                .with("license_claim", text(donor, "license")?)
                .with("provenance", foreign(field(donor, "provenance")?))
                .with("declaration_capture_locator", &donor_declaration)
                .with(
                    "source_semantics_locator",
                    format!("/source_semantics/donors/{index}"),
                )
                .with(
                    "scope",
                    "RECORDED_SOURCE_CLAIMS; NO_CURRENT_RIGHTS_AUTHORIZATION",
                ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let payloads = Json::Object(
        LANES
            .iter()
            .zip(payloads)
            .map(|(name, values)| (name.to_string(), Json::from(values)))
            .collect(),
    );
    Ok(Json::obj()
        .with("format", FORMAT)
        .with("scope", SCOPE)
        .with("capture_binding", Json::obj()
            .with("format", text(&envelope, "format")?)
            .with("sha256", digest)
            .with("bytes", bytes.len())
            .with("source_repository", text(&envelope, "source_repository")?)
            .with("source_id", text(&envelope, "source_id")?)
            .with("observed_head", &capture.observed_head)
            .with("producer_schema_claim", &capture.source_schema)
            .with("scope", "EXACT_CAPTURE_BYTES; HEAD_OBSERVED_ONLY; NOT_REVERIFIED"))
        .with("target_custody", Json::Null)
        .with("unresolved_reasons", unresolved())
        .with("node_mappings", mappings)
        .with("payloads", payloads)
        .with("rights_claims", rights)
        .with("source_semantics", semantics)
        .with("global_identity_candidates", candidates)
        .with("candidate_scope", "SOURCE_NAMESPACE_OSS_ONLY; NO_OSS_INCLUSION_REVIEW_OR_ADOPTION; NO_TARGET_EXISTENCE_OR_RIGHTS; ORIGINAL_EC1_IDS_UNCHANGED"))
}

/// Inspect an already parsed proposal without changing either source or target.
/// Wire callers must use `json::parse_unique`: a Value cannot retain duplicate-key
/// provenance after a permissive legacy parse has already discarded it.
pub fn inspect(capture: &Capture, proposal: &Value) -> Result<Json, String> {
    inspect_bound(capture, proposal, CAPTURE_SHA256)
}

/// Check a proposal against caller-selected capture bytes, never its self-asserted binding.
/// Exact nested equality preserves pending ownership, rights and original source metadata.
pub fn inspect_bound(
    capture: &Capture,
    proposal: &Value,
    expected_sha256: &str,
) -> Result<Json, String> {
    let generated = generate_bound(capture, expected_sha256)?;
    let expected = json::parse_unique(&generated.render())?;
    closed(proposal, &ROOT_FIELDS)?;
    if field(proposal, "target_custody")? != &Value::Null {
        return Err("TARGET_CUSTODY_REQUIRED".into());
    }
    let mappings = array(proposal, "node_mappings")?;
    if mappings.len() != array(&expected, "node_mappings")?.len() {
        return Err("SOURCE_NODE_COVERAGE_MISMATCH".into());
    }
    let mut source_ids = BTreeSet::new();
    let mut source_keys = BTreeSet::new();
    let mut node_locators = BTreeSet::new();
    for mapping in mappings {
        closed(mapping, &NODE_FIELDS)?;
        if field(mapping, "proposed_target")? != &Value::Null {
            return Err("TARGET_CUSTODY_REQUIRED".into());
        }
        if !source_ids.insert(text(mapping, "source_id")?)
            || !source_keys.insert((
                text(mapping, "source_namespace")?,
                text(mapping, "source_semantic_key")?,
            ))
            || !node_locators.insert(text(mapping, "capture_locator")?)
        {
            return Err("DUPLICATE_SOURCE_NODE_MAPPING".into());
        }
    }
    let payloads = field(proposal, "payloads")?;
    let expected_payloads = field(&expected, "payloads")?;
    closed(payloads, &LANES)?;
    let mut paths = BTreeSet::new();
    let mut payload_locators = BTreeSet::new();
    let mut hex_locators = BTreeSet::new();
    let mut payload_count = 0usize;
    for (index, name) in LANES.iter().enumerate() {
        let values = array(payloads, name)?;
        if values.len() != array(expected_payloads, name)?.len() {
            return Err(format!("SOURCE_PAYLOAD_COVERAGE_MISMATCH: {name}"));
        }
        for payload in values {
            closed(payload, &PAYLOAD_FIELDS)?;
            let path = text(payload, "source_path")?;
            if lane(path)? != index
                || !paths.insert(path)
                || !payload_locators.insert(text(payload, "capture_locator")?)
                || !hex_locators.insert(text(payload, "hex_locator")?)
            {
                return Err("DUPLICATE_OR_WRONG_LANE_SOURCE_PAYLOAD".into());
            }
            payload_count += 1;
        }
    }
    let rights = array(proposal, "rights_claims")?;
    if rights.len() != array(&expected, "rights_claims")?.len() {
        return Err("SOURCE_RIGHTS_CLAIM_COVERAGE_MISMATCH".into());
    }
    let mut aliases = BTreeSet::new();
    for claim in rights {
        closed(claim, &RIGHTS_FIELDS)?;
        if !aliases.insert(text(claim, "source_alias")?) {
            return Err("DUPLICATE_SOURCE_RIGHTS_CLAIM".into());
        }
    }
    let candidates = array(proposal, "global_identity_candidates")?;
    if candidates.len() != array(&expected, "global_identity_candidates")?.len() {
        return Err("SOURCE_IDENTITY_CANDIDATE_COVERAGE_MISMATCH".into());
    }
    let mut candidate_ids = BTreeSet::new();
    let mut candidate_sources = BTreeSet::new();
    for candidate in candidates {
        closed(candidate, &CANDIDATE_FIELDS)?;
        let namespace = text(candidate, "source_namespace")?;
        let key = text(candidate, "source_semantic_key")?;
        let id = text(candidate, "candidate_id")?;
        if namespace != "oss"
            || id != candidate_id(namespace, key)
            || !candidate_ids.insert(id)
            || !candidate_sources.insert(text(candidate, "source_id")?)
        {
            return Err("SOURCE_IDENTITY_CANDIDATE_DRIFT_OR_DUPLICATE".into());
        }
    }
    // Equality covers every nested field and original position, including all
    // source metadata, graph edges, ECG/f1 claims, rights and history locators.
    if proposal != &expected {
        return Err("PROPOSAL_SOURCE_BINDING_OR_COVERAGE_DRIFT".into());
    }
    Ok(Json::obj()
        .with("format", "ecdev-source-mapping-proposal-inspection-v1")
        .with("status", "VALID_PENDING_PROPOSAL")
        .with("scope", SCOPE)
        .with(
            "capture_sha256",
            text(field(&expected, "capture_binding")?, "sha256")?,
        )
        .with(
            "canonical_proposal_sha256",
            content_digest(generated.render().as_bytes()),
        )
        .with("source_nodes", mappings.len())
        .with("payloads", payload_count)
        .with("rights_claims", rights.len())
        .with("global_identity_candidates", candidates.len())
        .with("target_custody", Json::Null)
        .with("unresolved_reasons", unresolved()))
}

/// Bounded portable-text entrypoint. Duplicate decoded keys are refused before
/// conversion to a value tree can discard their original occurrence.
pub fn inspect_text(capture: &Capture, proposal: &str) -> Result<Json, String> {
    inspect_text_bound(capture, proposal, CAPTURE_SHA256)
}

/// Strict bounded portable-text checker with an independently supplied capture digest.
pub fn inspect_text_bound(
    capture: &Capture,
    proposal: &str,
    expected_sha256: &str,
) -> Result<Json, String> {
    if proposal.len() > MAX_PROPOSAL_BYTES {
        return Err("PROPOSAL_READER_BUDGET_EXCEEDED".into());
    }
    let value = json::parse_unique(proposal).map_err(|e| format!("PROPOSAL_JSON: {e}"))?;
    inspect_bound(capture, &value, expected_sha256)
}
