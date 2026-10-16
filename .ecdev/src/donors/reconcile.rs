//! Explicit reviews of retained source evidence. These decisions never feed lifecycle,
//! rights authorization, canonical enrollment, or the existing active denominator.

use super::audit;
use crate::declare::{self, Declaration};
use crate::digest::content_digest;
use crate::formats::{json, json::Json, Value};
use crate::graph::{Graph, NodeId};
use crate::universe::{identity::global_donor_key, Record, Universe};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Component, Path};

pub const FORMAT: &str = "ecdev-source-donor-reconciliation-v1";
pub const REVIEWS: &str = "research/commerce/governance/source-review-decisions-v1.json";
const BASIS: &str = "REVIEWED_RETAINED_SOFTWARE_SOURCE_AND_DECLARED_ROOT_TERMS";
const SCOPE: &str = "RETAINED_SOURCE_ONLY; NO_CURRENT_RIGHTS_OR_ENROLLMENT";

#[derive(Default)]
struct Inputs(BTreeMap<String, Option<Vec<u8>>>);

impl Inputs {
    fn read(&mut self, root: &Path, path: &str) -> Result<Option<Vec<u8>>, String> {
        if !path.starts_with("research/commerce/")
            || path.contains('\\')
            || Path::new(path)
                .components()
                .any(|p| !matches!(p, Component::Normal(_)))
        {
            return Err(format!("unsafe source review path: {path}"));
        }
        let mut current = root.to_path_buf();
        for part in Path::new(path).components() {
            current.push(part);
            if std::fs::symlink_metadata(&current).is_ok_and(|m| m.file_type().is_symlink()) {
                return Err(format!("source review symlink refused: {path}"));
            }
        }
        let bytes = match std::fs::symlink_metadata(root.join(path)) {
            Ok(m) if m.is_file() && m.len() <= 32 * 1024 * 1024 => {
                let mut bytes = Vec::new();
                std::fs::File::open(root.join(path))
                    .and_then(|file| file.take(32 * 1024 * 1024 + 1).read_to_end(&mut bytes))
                    .map_err(|e| format!("{path}: {e}"))?;
                if bytes.len() > 32 * 1024 * 1024 {
                    return Err(format!("source review read exceeds budget: {path}"));
                }
                Some(bytes)
            }
            Ok(_) => {
                return Err(format!(
                    "source review input not a bounded regular file: {path}"
                ))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(format!("{path}: {e}")),
        };
        self.0.insert(path.into(), bytes.clone());
        if self.0.len() > 1024
            || self.0.values().flatten().map(Vec::len).sum::<usize>() > 128 * 1024 * 1024
        {
            return Err("source review inputs exceed budget".into());
        }
        Ok(bytes)
    }

    fn required(&self, path: &str) -> Result<&[u8], String> {
        self.0
            .get(path)
            .and_then(Option::as_deref)
            .ok_or_else(|| format!("missing bound review evidence: {path}"))
    }

    fn value(&self, path: &str) -> Result<Value, String> {
        json::parse_unique(std::str::from_utf8(self.required(path)?).map_err(|e| e.to_string())?)
            .map_err(|e| format!("{path}: {e}"))
    }

    fn check_stable(&self, root: &Path) -> Result<(), String> {
        let mut after = Self::default();
        for path in self.0.keys() {
            after.read(root, path)?;
        }
        if self.0 != after.0 {
            return Err("SOURCE_REVIEW_INPUTS_CHANGED".into());
        }
        Ok(())
    }

    fn report(&self) -> Vec<Json> {
        self.0
            .iter()
            .map(|(path, bytes)| {
                let row = Json::obj().with("path", path);
                match bytes {
                    Some(b) => row
                        .with("state", "PRESENT")
                        .with("sha256", content_digest(b))
                        .with("bytes", b.len()),
                    None => row.with("state", "MISSING"),
                }
            })
            .collect()
    }
}

fn closed(value: &Value, fields: &[&str]) -> Result<(), String> {
    let table = value.table().ok_or("expected review object")?;
    if table.len() != fields.len() || fields.iter().any(|f| !table.contains_key(*f)) {
        return Err("missing or unknown source review field".into());
    }
    Ok(())
}

fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .str(key)
        .ok_or_else(|| format!("expected source review string: {key}"))
}

fn rows<'a>(value: &'a Value, key: &str) -> Result<&'a [Value], String> {
    match value.get(key) {
        Some(Value::Array(v)) => Ok(v),
        _ => Err(format!("expected source review array: {key}")),
    }
}

fn equal(value: &Value, key: &str, expected: &str) -> Result<(), String> {
    if text(value, key)? != expected {
        return Err(format!("source review {key} mismatch"));
    }
    Ok(())
}

fn census_paths(d: &crate::declare::Donor) -> Vec<String> {
    d.provenance
        .iter()
        .filter(|p| p.ends_with("/identity.json"))
        .cloned()
        .collect()
}

fn relative(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\\')
        && Path::new(path)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

fn hash(value: &str, length: usize) -> bool {
    value.len() == length && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn validate_review(
    review: &Value,
    d: &Declaration,
    u: &Universe,
    inputs: &Inputs,
) -> Result<(String, Json), String> {
    closed(
        review,
        &[
            "source_alias",
            "source_id",
            "origin",
            "global_origin",
            "upstream_revision",
            "source_kind",
            "oss_inclusion",
            "declared_license",
            "basis",
            "record",
            "retained_root_license",
            "evidence",
            "limitations",
        ],
    )?;
    let alias = text(review, "source_alias")?;
    let donor = d
        .donors
        .iter()
        .find(|donor| donor.key == alias)
        .ok_or("review names undeclared donor")?;
    let (namespace, key) = Graph::donor_id(&d.repository.id, &donor.key, &donor.origin);
    equal(
        review,
        "source_id",
        &NodeId::of(&namespace, &key).to_string(),
    )?;
    equal(review, "origin", &donor.origin)?;
    equal(
        review,
        "global_origin",
        &global_donor_key(&donor.origin).ok_or("review origin unresolved")?,
    )?;
    equal(review, "declared_license", &donor.license)?;
    equal(review, "source_kind", "OSS_REPOSITORY")?;
    equal(review, "basis", BASIS)?;
    if review.get("oss_inclusion") != Some(&Value::Bool(true)) {
        return Err("unsupported OSS review decision".into());
    }
    let revision = text(review, "upstream_revision")?;
    if revision.len() != 40 || !revision.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("review needs full upstream revision".into());
    }
    let limitations = rows(review, "limitations")?;
    if limitations.is_empty()
        || limitations
            .iter()
            .any(|v| v.as_str().is_none_or(str::is_empty))
    {
        return Err("review limitations required".into());
    }

    let selector = review
        .get("record")
        .ok_or("missing source record selector")?;
    closed(selector, &["source", "record_id", "legacy_sha256"])?;
    let records: Vec<_> = u
        .records()
        .filter(|(s, r)| {
            s.source == text(selector, "source").unwrap_or("")
                && r.id == text(selector, "record_id").unwrap_or("")
        })
        .collect();
    if records.len() != 1 {
        return Err("review requires exactly one original source record".into());
    }
    let record = records[0].1;
    equal(
        selector,
        "legacy_sha256",
        &content_digest(record.legacy.as_bytes()),
    )?;
    if record.global.as_deref() != Some(text(review, "global_origin")?) {
        return Err("review source record identity mismatch".into());
    }
    let legacy = json::parse_unique(&record.legacy)?;
    equal(&legacy, "donor_id", alias)?;
    equal(&legacy, "commit_sha", revision)?;
    equal(&legacy, "license", &donor.license)?;
    check_kinds(&legacy)?;

    let identities = census_paths(donor);
    if identities.len() != 1 {
        return Err("review needs one explicit census identity provenance".into());
    }
    let directory = identities[0]
        .strip_suffix("identity.json")
        .ok_or("bad identity locator")?;
    let required: BTreeSet<_> = [
        "identity.json",
        "license.json",
        "files.jsonl",
        "tree.json",
        "summary.json",
    ]
    .iter()
    .map(|name| format!("{directory}{name}"))
    .chain(["research/commerce/donor-lifecycle-review.json".to_string()])
    .collect();
    let mut evidenced = BTreeSet::new();
    for evidence in rows(review, "evidence")? {
        closed(evidence, &["path", "sha256"])?;
        let path = text(evidence, "path")?;
        if !evidenced.insert(path.to_string()) {
            return Err("duplicate review evidence path".into());
        }
        equal(evidence, "sha256", &content_digest(inputs.required(path)?))?;
    }
    if evidenced != required {
        return Err("review evidence set is not the exact required receipt closure".into());
    }
    let identity = inputs.value(&identities[0])?;
    equal(&identity, "donor_id", alias)?;
    equal(&identity, "commit_sha", revision)?;
    equal(&identity, "license", &donor.license)?;
    check_kinds(&identity)?;
    for field in ["repository_url", "resolved_remote_url"] {
        if global_donor_key(text(&identity, field)?).as_deref()
            != Some(text(review, "global_origin")?)
        {
            return Err(format!("census {field} conflicts with reviewed identity"));
        }
    }
    if text(&identity, "remote_status")? != "VERIFIED_REMOTE" {
        return Err("recorded remote observation missing".into());
    }
    let summary = inputs.value(&format!("{directory}summary.json"))?;
    equal(&summary, "donor_id", alias)?;
    equal(&summary, "commit_sha", revision)?;
    let tree = inputs.value(&format!("{directory}tree.json"))?;
    if !rows(&tree, "submodules")?.is_empty() {
        return Err("submodule custody needs a separate reviewed profile".into());
    }
    let tree_rows = rows(&tree, "files")?;
    let tree_paths: BTreeSet<_> = tree_rows.iter().filter_map(Value::as_str).collect();
    if tree_paths.len() != tree_rows.len()
        || tree_paths.len() > 16_384
        || tree_paths.iter().any(|p| !relative(p))
    {
        return Err("duplicate or unsupported tree path".into());
    }
    let raw = std::str::from_utf8(inputs.required(&format!("{directory}files.jsonl"))?)
        .map_err(|e| e.to_string())?;
    let lines: Vec<_> = raw.lines().filter(|s| !s.is_empty()).take(16_385).collect();
    if lines.len() > 16_384 {
        return Err("source inventory line budget exceeded".into());
    }
    let inventory: Vec<_> = lines
        .into_iter()
        .map(json::parse_unique)
        .collect::<Result<_, _>>()?;
    let mut paths = BTreeSet::new();
    for file in &inventory {
        if !paths.insert(text(file, "path")?) {
            return Err("duplicate source inventory path".into());
        }
        if !relative(text(file, "path")?)
            || !matches!(file.str("git_mode"), Some("100644" | "100755"))
            || !file.str("sha256").is_some_and(|s| hash(s, 64))
            || !file.str("blob_hash").is_some_and(|s| hash(s, 40))
            || !matches!(file.get("bytes"), Some(Value::Int(n)) if *n >= 0)
        {
            return Err("unsupported source inventory path or mode".into());
        }
    }
    if paths != tree_paths
        || summary.get("total_files") != Some(&Value::Int(inventory.len() as i64))
        || identity.get("file_count") != Some(&Value::Int(inventory.len() as i64))
    {
        return Err("review inventory/tree/summary coverage differs".into());
    }
    let source_files = inventory
        .iter()
        .filter(|r| r.str("classification") == Some("FIRST_PARTY_SOURCE"))
        .count();
    if source_files == 0
        || summary.get("first_party_source_files") != Some(&Value::Int(source_files as i64))
    {
        return Err("review has no agreeing software source inventory".into());
    }
    let parsed = inventory
        .iter()
        .filter(|r| {
            r.str("classification") == Some("FIRST_PARTY_SOURCE")
                && r.str("parse_status") == Some("PARSED")
        })
        .count();
    let mut classifications = BTreeMap::<String, i64>::new();
    for file in &inventory {
        *classifications
            .entry(text(file, "classification")?.to_string())
            .or_default() += 1;
    }
    let expected_counts = Value::Table(
        classifications
            .into_iter()
            .map(|(key, n)| (key, Value::Int(n)))
            .collect(),
    );
    if summary.get("classification_counts") != Some(&expected_counts)
        || summary.get("source_parsed") != Some(&Value::Int(parsed as i64))
        || parsed != source_files
        || summary.get("parse_unknown") != Some(&Value::Int(0))
        || summary.get("unknown_files") != Some(&Value::Int(0))
        || summary.get("classified_files") != Some(&Value::Int(inventory.len() as i64))
        || inventory
            .iter()
            .any(|r| r.str("classification") == Some("UNKNOWN"))
    {
        return Err("review closed source census counts or parse assertions disagree".into());
    }

    let root_license = review
        .get("retained_root_license")
        .ok_or("missing retained license review")?;
    closed(root_license, &["path", "sha256", "blob", "bytes"])?;
    let license_path = text(root_license, "path")?;
    if !relative(license_path) || license_path.contains('/') {
        return Err("root-only license profile requires one safe root filename".into());
    }
    let license_hash = text(root_license, "sha256")?;
    if !license_hash
        .strip_prefix("sha256:")
        .is_some_and(|s| hash(s, 64))
        || !hash(text(root_license, "blob")?, 40)
    {
        return Err("malformed retained license digest".into());
    }
    let identity_paths = rows(&identity, "license_paths")?;
    let identity_hashes = rows(&identity, "license_hashes")?;
    if identity_paths.len() != 1
        || identity_hashes.len() != 1
        || identity_paths[0].as_str() != Some(license_path)
        || identity_hashes[0].as_str() != license_hash.strip_prefix("sha256:")
    {
        return Err("root-only review cannot narrow or invent recorded license coverage".into());
    }
    let licenses = inputs.value(&format!("{directory}license.json"))?;
    let license_files = rows(&licenses, "files")?;
    if license_files.len() != 1 {
        return Err("component license terms need a separate reviewed profile".into());
    }
    let file = &license_files[0];
    equal(file, "source_path", license_path)?;
    let retained_text = text(file, "text")?;
    if retained_text.is_empty() || content_digest(retained_text.as_bytes()) != license_hash {
        return Err("retained license text mismatch".into());
    }
    equal(file, "sha256", &license_hash[7..])?;
    equal(file, "blob_hash", text(root_license, "blob")?)?;
    let license_row = inventory
        .iter()
        .find(|row| row.str("path") == Some(license_path))
        .ok_or("license absent from inventory")?;
    equal(license_row, "sha256", &license_hash[7..])?;
    equal(license_row, "blob_hash", text(root_license, "blob")?)?;
    if license_row.get("bytes") != root_license.get("bytes")
        || root_license.get("bytes") != Some(&Value::Int(retained_text.len() as i64))
    {
        return Err("retained license byte length mismatch".into());
    }
    let lifecycle = inputs.value("research/commerce/donor-lifecycle-review.json")?;
    let historical: Vec<_> = rows(&lifecycle, "donors")?
        .iter()
        .filter(|v| v.str("donor_id") == Some(alias))
        .collect();
    if historical.len() != 1 {
        return Err("missing or duplicate historical source review".into());
    }
    equal(historical[0], "commit_sha", revision)?;
    equal(historical[0], "license", &donor.license)?;
    let historical_licenses = rows(historical[0], "license_files")?;
    if historical_licenses.len() != 1
        || historical_licenses[0].str("path") != Some(license_path)
        || historical_licenses[0].str("sha256") != license_hash.strip_prefix("sha256:")
    {
        return Err("historical license review conflicts with retained evidence".into());
    }

    // Other records for the SAME exact origin can contradict the decision; never
    // silently select only a convenient record or join by a similar name.
    for (_, other) in u.records() {
        let same_alias = json::parse_unique(&other.legacy)?.str("donor_id") == Some(alias);
        if same_alias || other.global == record.global {
            check_record(
                other,
                alias,
                revision,
                &donor.license,
                text(review, "global_origin")?,
            )?;
        }
    }
    Ok((
        alias.into(),
        Json::obj()
            .with("status", "SOURCE_REVIEWED_OSS_INCLUSION")
            .with("reason_codes", Vec::<String>::new())
            .with("source_kind", "OSS_REPOSITORY")
            .with("oss_inclusion", true)
            .with("basis", BASIS)
            .with("upstream_revision", revision)
            .with("global_origin", text(review, "global_origin")?)
            .with("original_record_id", &record.id)
            .with("original_provenance", &record.provenance)
            .with("retained_root_license", audit::foreign(root_license))
            .with("declared_license", &donor.license)
            .with(
                "historical_license_receipt_status",
                licenses.str("status").unwrap_or("MISSING"),
            )
            .with(
                "retained_terms",
                "EXISTING_DECLARED_TERMS_WITH_REVIEWED_CAPTURED_BYTES; NO_INFERRED_LICENSE",
            )
            .with("first_party_source_files", source_files)
            .with("inventory_files", inventory.len())
            .with(
                "current_copying_authorization",
                "UNKNOWN; NOT_GRANTED_BY_SOURCE_REVIEW",
            )
            .with(
                "upstream_commit_membership",
                "RETAINED_RECEIPT_ASSERTION; NOT_FRESHLY_ATTESTED",
            )
            .with("canonical_registry_binding", "PENDING")
            .with(
                "limitations",
                audit::foreign(review.get("limitations").unwrap()),
            ),
    ))
}

fn check_kinds(value: &Value) -> Result<(), String> {
    for field in ["type", "kind", "source_kind", "resource_kind"] {
        if let Some(raw) = value.get(field) {
            if raw.as_str() != Some("OSS_REPOSITORY") {
                return Err(format!("conflicting source kind field: {field}"));
            }
        }
    }
    Ok(())
}

fn check_record(
    record: &Record,
    alias: &str,
    revision: &str,
    license: &str,
    global: &str,
) -> Result<(), String> {
    if record.global.as_deref() != Some(global) {
        return Err("same-alias source record has conflicting imported origin".into());
    }
    let raw = json::parse_unique(&record.legacy)?;
    check_kinds(&raw)?;
    for (key, expected) in [
        ("donor_id", alias),
        ("commit_sha", revision),
        ("license", license),
    ] {
        if raw.get(key).is_some() && raw.str(key) != Some(expected) {
            return Err(format!("conflicting same-origin record {key}"));
        }
    }
    for key in ["repository_url", "resolved_remote_url", "repository"] {
        if let Some(value) = raw.get(key) {
            if value.as_str().and_then(global_donor_key).as_deref() != Some(global) {
                return Err(format!("conflicting explicit same-record origin: {key}"));
            }
        }
    }
    Ok(())
}

/// Reconciliation is an additional versioned evidence view. The v1 audit and capture,
/// original registry bytes, native IDs and lifecycle evaluator remain unchanged.
pub fn run(root: &Path) -> Result<Json, String> {
    let base = audit::run(root)?;
    let base_value = json::parse(&base.render())?;
    let declaration = declare::load(root).map_err(|e| e.to_string())?;
    let universe = Universe::load(root);
    if !universe.unreadable.is_empty() {
        return Err("universe custody unreadable; source reviews refused".into());
    }
    let mut inputs = Inputs::default();
    let ledger = inputs.read(root, REVIEWS)?;
    for d in &declaration.donors {
        for identity in census_paths(d) {
            let directory = identity
                .strip_suffix("identity.json")
                .ok_or("bad census identity path")?;
            for name in [
                "identity.json",
                "license.json",
                "files.jsonl",
                "tree.json",
                "summary.json",
            ] {
                inputs.read(root, &format!("{directory}{name}"))?;
            }
        }
    }
    inputs.read(root, "research/commerce/donor-lifecycle-review.json")?;
    let mut reviewed = BTreeMap::new();
    if let Some(raw) = ledger {
        let ledger = json::parse_unique(std::str::from_utf8(&raw).map_err(|e| e.to_string())?)?;
        closed(&ledger, &["format", "scope", "reviews"])?;
        equal(&ledger, "format", "ecdev-source-reviews-v1")?;
        equal(&ledger, "scope", SCOPE)?;
        for review in rows(&ledger, "reviews")? {
            let (alias, result) = validate_review(review, &declaration, &universe, &inputs)?;
            if reviewed.insert(alias, result).is_some() {
                return Err("duplicate reviewed source alias".into());
            }
        }
    }
    let mut donors = Vec::new();
    for donor in base_value.items_of("donors") {
        let alias = text(donor, "source_alias")?;
        let d = declaration
            .donors
            .iter()
            .find(|d| d.key == alias)
            .ok_or("audit declaration mismatch")?;
        let identity_paths = census_paths(d);
        let mut reasons = Vec::new();
        let mut observations = Vec::new();
        if !reviewed.contains_key(alias) {
            reasons.push("REVIEWED_SOURCE_CLASSIFICATION_DECISION_NOT_BOUND".to_string());
        }
        if identity_paths.is_empty() {
            reasons.push(
                "CENSUS_IDENTITY_NOT_SELECTED_IN_DECLARED_PROVENANCE; OTHER_PROVENANCE_UNREVIEWED"
                    .into(),
            );
        }
        for path in &identity_paths {
            match inputs.value(path) {
                Ok(identity) => {
                    let directory = path.strip_suffix("identity.json").unwrap();
                    let license = inputs.value(&format!("{directory}license.json"));
                    observations.push(Json::obj().with("identity_path", path)
                        .with("recorded_upstream_revision", identity.str("commit_sha").map_or(Json::Null, Json::from))
                        .with("recorded_repository_url", identity.str("repository_url").map_or(Json::Null, Json::from))
                        .with("recorded_donor_type", identity.str("donor_type").map_or(Json::Null, Json::from))
                        .with("recorded_license_label", identity.str("license").map_or(Json::Null, Json::from))
                        .with("recorded_license_paths", identity.get("license_paths").map_or(Json::Null, audit::foreign))
                        .with("recorded_license_hashes", identity.get("license_hashes").map_or(Json::Null, audit::foreign))
                        .with("original_license_receipt_status", license.as_ref().ok().and_then(|l| l.str("status")).map_or(Json::Null, Json::from))
                        .with("custody", "RETAINED_SOURCE_ASSERTIONS; REVIEW_REQUIRED; NO_CURRENT_AUTHORIZATION"));
                    if identity.str("commit_sha").is_none_or(str::is_empty) {
                        reasons.push("UPSTREAM_REVISION_NOT_RECORDED".into());
                    }
                    match license {
                        Ok(license) => {
                            if license.items_of("files").is_empty() {
                                reasons
                                    .push("NO_LICENSE_ARTIFACT_SELECTED_IN_CENSUS_RECEIPT".into());
                            }
                            if license
                                .items_of("files")
                                .iter()
                                .any(|f| f.str("text").is_none_or(str::is_empty))
                            {
                                reasons.push("LICENSE_TEXT_NOT_RETAINED_IN_SELECTED_RECEIPT; HASH_CUSTODY_MAY_EXIST_ELSEWHERE".into());
                            }
                        }
                        Err(e) => reasons.push(format!("LICENSE_RECEIPT_UNREADABLE: {e}")),
                    }
                }
                Err(e) => reasons.push(format!("IDENTITY_RECEIPT_UNREADABLE: {e}")),
            }
        }
        let global = global_donor_key(&d.origin);
        let original_candidates: Vec<_> = universe.records().filter_map(|(s, r)| {
            let raw = json::parse(&r.legacy).ok()?;
            (raw.str("donor_id") == Some(alias)).then(|| Json::obj().with("original_record_id", &r.id).with("source", &s.source)
                .with("current_imported_global", r.global.as_ref().map_or(Json::Null, Json::from))
                .with("explicit_repository_url", raw.str("repository_url").map_or(Json::Null, Json::from))
                .with("reason", if r.global.is_none() { "IMPORTER_ORIGIN_FIELDS_OMIT_REPOSITORY_URL; EXPLICIT_SOURCE_MAPPING_NEEDED" } else { "EXISTING_IMPORTED_ORIGIN; NOT_FRESH_AUTHENTICATION" }))
        }).collect();
        let review = reviewed.get(alias).cloned().unwrap_or_else(|| {
            Json::obj()
                .with("status", "UNRESOLVED")
                .with("reason_codes", reasons.clone())
        });
        donors.push(Json::obj().with("source_alias", alias).with("source_id", text(donor, "source_id")?)
            .with("origin", &d.origin).with("observed_global_candidate", global.map_or(Json::Null, Json::from))
            .with("review", review).with("unresolved_evidence_reasons", reasons)
            .with("existing_provenance", d.provenance.clone()).with("selected_identity_receipts", identity_paths)
            .with("available_source_fields", observations)
            .with("original_registry_candidates", original_candidates)
            .with("fields_required_for_source_review", vec!["original source_alias and ec1 source_id", "exact original record source/id and legacy digest", "explicit source kind/inclusion decision and evidence basis", "exact existing origin, upstream revision and inventory closure", "retained license bytes/paths/hashes and existing declared terms, without inferred licences", "all evidence file digests, component exclusions and custody limitations"])
            .with("separate_required_bindings", vec!["Current rights/copying authorization", "Canonical owner/identity and target custody", "Target adoption/knowledge/absorption and current ECG verification", "Authorized acceptance producer and native platform qualification"])
            .with("native_effective_state", text(donor, "effective_state")?).with("active_gate_member", donor.get("gate_member").map_or(Json::Null, audit::foreign))
            .with("native_stopped_by", donor.str("stopped_by").map_or(Json::Null, Json::from)));
    }
    inputs.check_stable(root)?;
    if audit::run(root)? != base {
        return Err("SOURCE_REVIEW_AUDIT_CHANGED".into());
    }
    let count = reviewed.len();
    Ok(Json::obj().with("format", FORMAT).with("binding", SCOPE).with("source_audit", base)
        .with("supplemental_inputs", inputs.report()).with("review_ledger", REVIEWS)
        .with("review_trust", "EXPLICIT_LOCAL_REVIEW_OF_BOUND_RETAINED_SOURCE; NOT_AUTHENTICATED_RIGHTS_OR_EXECUTION")
        .with("reviewed_source_oss", Json::obj().with("included", count).with("unresolved", donors.len()-count)
            .with("total", Json::Null).with("verified_extinct", Json::Null).with("status", if count==0 {"UNRESOLVED"}else{"PARTIALLY_RECONCILED"})
            .with("meaning", "Reviewed source inclusions are a partial count; global OSS total and canonical verified extinction remain UNKNOWN"))
        .with("per_donor", donors).with("lifecycle_gates_modified", false).with("canonical_registry_binding", "PENDING")
        .with("new_assimilation", 0usize).with("new_extinction", 0usize))
}
