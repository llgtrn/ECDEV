//! Versioned descriptive source audit. It does not classify donors by their names, URLs,
//! declared licences or importer defaults, and does not alter any lifecycle gate.

use crate::census::Observation;
use crate::declare::Declaration;
use crate::digest::{content_digest, hex, Sha256};
use crate::formats::{json, json::Json, Value};
use crate::graph::{Graph, NodeId};
use crate::repository::files::{head_commit, Files};
use crate::schema::{DonorState, UniverseKind};
use crate::universe::{Record, Universe};
use crate::Assessment;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub const FORMAT: &str = "ecdev-source-donor-audit-v1";

fn safe_relative(path: &str) -> bool {
    !Path::new(path).is_absolute()
        && !path.contains('\\')
        && !path.split('/').any(|p| p == ".." || p.is_empty())
}

#[derive(PartialEq)]
struct Snapshot {
    head: Option<String>,
    paths: BTreeSet<String>,
    digest: String,
    entries: Json,
    gaps: Vec<String>,
    from_index: bool,
}

fn supplemental(
    root: &Path,
    path: &str,
    paths: &mut BTreeSet<String>,
    gaps: &mut Vec<String>,
    depth: usize,
) {
    if depth > 32 || paths.len() > 100_000 {
        gaps.push(format!("{path}: supplemental input budget exceeded"));
        return;
    }
    let meta = match std::fs::symlink_metadata(root.join(path)) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            paths.insert(path.into());
            return;
        }
        Err(e) => {
            gaps.push(format!("{path}: {e}"));
            return;
        }
    };
    if meta.is_dir() {
        match std::fs::read_dir(root.join(path)) {
            Ok(rd) => {
                for entry in rd {
                    match entry {
                        Ok(entry) => match entry.file_name().into_string() {
                            Ok(name) => supplemental(
                                root,
                                &format!("{path}/{name}"),
                                paths,
                                gaps,
                                depth + 1,
                            ),
                            Err(_) => gaps.push(format!("{path}: non UTF-8 supplemental path")),
                        },
                        Err(e) => gaps.push(format!("{path}: {e}")),
                    }
                }
            }
            Err(e) => gaps.push(format!("{path}: {e}")),
        }
    } else {
        paths.insert(path.into());
    }
}

impl Snapshot {
    fn read(root: &Path, files: &Files, d: &Declaration) -> Self {
        let mut paths = files.paths.clone();
        paths.insert("research/commerce/dependencies.json".into());
        let mut gaps = Vec::new();
        for dir in ["declared", "history", "evidence", "knowledge"] {
            supplemental(root, &format!(".ecdev/{dir}"), &mut paths, &mut gaps, 0);
        }
        paths.extend(d.donors.iter().flat_map(super::semantic_review_paths));
        paths.extend(d.donors.iter().flat_map(|d| {
            d.provenance
                .iter()
                .filter(|p| p.ends_with(".json") && !p.contains("://"))
                .cloned()
        }));
        let mut digest = Sha256::new();
        digest.field(b"ecdev.source-audit-inputs.v1");
        let mut entries = Vec::new();
        for path in &paths {
            digest.field(path.as_bytes());
            let safe = safe_relative(path);
            let read = if !safe {
                Err("OUTSIDE_RELATIVE_SOURCE_SCOPE".into())
            } else {
                match std::fs::symlink_metadata(root.join(path)) {
                    Ok(m) if !m.is_file() || m.file_type().is_symlink() => {
                        Err("NOT_A_REGULAR_BOUND_FILE".into())
                    }
                    Ok(m) if m.len() > 64 * 1024 * 1024 => {
                        Err("FILE_EXCEEDS_AUDIT_BINDING_BUDGET".into())
                    }
                    Ok(_) => std::fs::read(root.join(path))
                        .map(Some)
                        .map_err(|e| e.to_string()),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                    Err(e) => Err(e.to_string()),
                }
            };
            let entry = match read {
                Ok(Some(bytes)) => {
                    let hash = content_digest(&bytes);
                    digest.field(hash.as_bytes());
                    Json::obj()
                        .with("path", path)
                        .with("state", "PRESENT")
                        .with("sha256", hash)
                        .with("bytes", bytes.len())
                }
                Ok(None) => {
                    digest.field(b"MISSING");
                    Json::obj().with("path", path).with("state", "MISSING")
                }
                Err(error) => {
                    digest.field(b"UNBOUND");
                    digest.field(error.as_bytes());
                    gaps.push(format!("{path}: {error}"));
                    Json::obj()
                        .with("path", path)
                        .with("state", "UNBOUND")
                        .with("reason", error)
                }
            };
            entries.push(entry);
        }
        for path in &files.gitlinks {
            gaps.push(format!("{path}: Gitlink target bytes are not bound"));
        }
        gaps.sort();
        gaps.dedup();
        for gap in &gaps {
            digest.field(gap.as_bytes());
        }
        Self {
            head: head_commit(root),
            paths,
            digest: format!("sha256:{}", hex(&digest.finish())),
            entries: Json::from(entries),
            gaps,
            from_index: files.from_index,
        }
    }

    fn to_json(&self) -> Json {
        Json::obj()
            .with(
                "observed_head",
                self.head.as_ref().map_or(Json::Null, Json::from),
            )
            .with("input_manifest_sha256", &self.digest)
            .with("input_paths", self.paths.len())
            .with("path_selection_from_index", self.from_index)
            .with(
                "binding",
                "WORKTREE_BYTES_PLUS_SUPPLEMENTAL_STATE_AND_CENSUS_INPUTS; HEAD_OBSERVED_ONLY",
            )
            .with("commit_content_match", "UNKNOWN_NOT_ATTESTED")
            .with("uncommitted_inputs_status", "UNKNOWN_NOT_CHECKED; INDEX_SELECTED_WORKTREE_BYTES_MAY_DIFFER_FROM_HEAD")
            .with("stability_scope", "ENUMERATED_FILE_BYTES_ONLY; FILE_MODES_AND_FILESYSTEM_CONTROL_METADATA_NOT_ATTESTED")
            .with("stable_before_after_assessment", true)
            .with("binding_complete", self.gaps.is_empty())
            .with("binding_gaps", self.gaps.clone())
            .with("inputs", self.entries.clone())
    }
}

fn observed(rows: &[Observation]) -> Json {
    Json::from(
        rows.iter()
            .map(|o| {
                Json::obj()
                    .with("file", &o.file)
                    .with("ecosystem", o.ecosystem.wire())
                    .with("name", &o.name)
                    .with("ident", &o.ident)
                    .with("scope", o.scope.wire())
                    .with("via", o.via.wire())
            })
            .collect::<Vec<_>>(),
    )
}

pub(crate) fn foreign(value: &Value) -> Json {
    match value {
        Value::Null => Json::Null,
        Value::Bool(v) => Json::Bool(*v),
        Value::Int(v) => Json::Int(*v),
        // Foreign numeric spellings stay tagged in this descriptive projection; original
        // bytes remain bound in the legacy record and snapshot, never parsed as estimates.
        Value::Float(v) => Json::obj().with("foreign_numeric_spelling", v),
        Value::Str(v) => Json::from(v),
        Value::Array(v) => Json::from(v.iter().map(foreign).collect::<Vec<_>>()),
        Value::Table(v) => Json::Object(v.iter().map(|(k, v)| (k.clone(), foreign(v))).collect()),
    }
}

fn descriptors(value: &Value, kinds: &mut BTreeSet<UniverseKind>) -> Vec<Json> {
    let mut fields = Vec::new();
    for key in [
        "type",
        "kind",
        "source_kind",
        "resource_kind",
        "donor_type",
        "role",
        "scope",
        "external_services",
    ] {
        if let Some(raw) = value.get(key) {
            fields.push(Json::obj().with("field", key).with("value", foreign(raw)));
            if matches!(key, "type" | "kind" | "source_kind" | "resource_kind") {
                if let Some(kind) = raw.as_str().and_then(UniverseKind::from_wire) {
                    kinds.insert(kind);
                }
            }
        }
    }
    fields
}

fn classification(records: &[&Record], additional_kinds: &BTreeSet<UniverseKind>) -> Json {
    let mut kinds = additional_kinds.clone();
    let mut evidence = Vec::new();
    for r in records {
        let legacy = json::parse(&r.legacy).ok();
        let descriptors = legacy
            .as_ref()
            .map_or_else(Vec::new, |v| descriptors(v, &mut kinds));
        evidence.push(
            Json::obj()
                .with("record_id", &r.id)
                .with("provenance", &r.provenance)
                .with("imported_kind", r.kind.wire())
                .with("imported_kind_is_reviewed_oss_decision", false)
                .with("legacy_sha256", content_digest(r.legacy.as_bytes()))
                .with("descriptors", descriptors)
                .with("legacy_record", &r.legacy),
        );
    }
    let state = if kinds.len() > 1 {
        "CONFLICTING_EXPLICIT_KIND_CLAIMS"
    } else if kinds.contains(&UniverseKind::OssRepository) {
        "EXPLICIT_OSS_KIND_CLAIM_UNVERIFIED"
    } else if kinds.is_empty() {
        "UNRESOLVED_NO_EXPLICIT_CANONICAL_KIND"
    } else {
        "EXPLICIT_OTHER_KIND_CLAIM_UNVERIFIED"
    };
    Json::obj().with("status", state).with("explicit_kind_claims", kinds.iter().map(|k| k.wire()).collect::<Vec<_>>())
        .with("evidence", evidence).with("audited_oss_inclusion", Json::Null)
        .with("blocker", "Raw imported/provenance claims only; separate source-review ledgers are not evaluated by this audit. Use donor reconcile for bound reviews and remaining evidence gaps.")
}

fn source_descriptors(a: &Assessment) -> Json {
    let path = "research/commerce/dependencies.json";
    match a.files.read(path) {
        Some(text) => match json::parse(&text) {
            Ok(value) => Json::obj()
                .with("source", path)
                .with("sha256", content_digest(text.as_bytes()))
                .with(
                    "status",
                    "PARTICIPATION_DESCRIPTORS_ONLY; NOT_A_DONOR_OR_OSS_DENOMINATOR",
                )
                .with(
                    "records",
                    value
                        .items_of("packages")
                        .iter()
                        .map(foreign)
                        .collect::<Vec<_>>(),
                ),
            Err(e) => Json::obj()
                .with("source", path)
                .with("status", "UNREADABLE")
                .with("reason", e),
        },
        None => Json::obj().with("source", path).with(
            "status",
            "MISSING_OR_UNREADABLE; NOT_PROOF_OF_NO_TOOLS_MODELS_OR_DATA",
        ),
    }
}

/// Deterministic audit of current source inputs. Re-read the complete binding to refuse a
/// report if inputs changed while assessment and projection were being computed.
pub fn run(root: &Path) -> Result<Json, String> {
    let declaration = crate::declare::load(root).map_err(|e| e.to_string())?;
    let files = Files::scan(root).map_err(|e| e.to_string())?;
    let before = Snapshot::read(root, &files, &declaration);
    let a = crate::assess(root)?;
    let universe = Universe::load(root);
    let report = project(&a, &universe, &before)?;
    let after_files = Files::scan(root).map_err(|e| e.to_string())?;
    let after_declaration = crate::declare::load(root).map_err(|e| e.to_string())?;
    if before != Snapshot::read(root, &after_files, &after_declaration)
        || files.paths != a.files.paths
    {
        return Err("AUDIT_INPUTS_CHANGED: retry against a stable source snapshot".into());
    }
    Ok(report)
}

fn project(a: &Assessment, universe: &Universe, snapshot: &Snapshot) -> Result<Json, String> {
    let mut active = a.history.registered.clone();
    for d in &a.analysis.donors {
        if d.effective >= DonorState::Registered {
            active.insert(d.key.clone());
        }
    }
    for d in a.analysis.donors.iter().filter(|d| d.resolved()) {
        active.remove(&d.key);
    }
    if active.len() as u64 != a.counts.donors_active() {
        return Err("audit denominator disagrees with existing active gate".into());
    }
    let mut states = BTreeMap::<String, usize>::new();
    let mut donors = Vec::new();
    for d in &a.declaration.donors {
        let da = a.donor(&d.key).ok_or("declared donor missing assessment")?;
        let relation = a
            .universe
            .relations
            .iter()
            .find(|r| r.donors.contains(&d.key));
        let records: Vec<_> = universe
            .records()
            .filter(|(s, r)| {
                relation.is_some_and(|u| u.records.contains(&format!("{}#{}", s.source, r.id)))
            })
            .map(|(_, r)| r)
            .collect();
        let mut extra_kinds = BTreeSet::new();
        let extra_evidence = d
            .provenance
            .iter()
            .filter(|p| p.ends_with(".json") && !p.contains("://") && safe_relative(p))
            .map(|path| match a.files.read(path) {
                Some(raw) => {
                    let parsed = json::parse(&raw);
                    let fields = parsed
                        .as_ref()
                        .map_or_else(|_| Vec::new(), |v| descriptors(v, &mut extra_kinds));
                    Json::obj()
                        .with("path", path)
                        .with("sha256", content_digest(raw.as_bytes()))
                        .with("descriptors", fields)
                        .with("source_text", raw)
                        .with("parse_error", parsed.err().map_or(Json::Null, Json::from))
                }
                None => Json::obj().with("path", path).with(
                    "status",
                    "MISSING_OR_UNREADABLE; NOT_CLASSIFICATION_EVIDENCE",
                ),
            })
            .collect::<Vec<_>>();
        let classified = classification(&records, &extra_kinds);
        if let Json::Object(fields) = &classified {
            if let Some((_, Json::Str(state))) = fields.iter().find(|(key, _)| key == "status") {
                *states.entry(state.clone()).or_default() += 1;
            }
        }
        let (ns, key) = Graph::donor_id(&a.declaration.repository.id, &d.key, &d.origin);
        let policy = crate::licence::policy(&d.license);
        let capabilities = da
            .capabilities
            .iter()
            .map(|c| {
                let declared = d.capabilities.iter().find(|p| p.key == c.key);
                let proofs = declared
                    .map(|c| {
                        c.proofs
                            .iter()
                            .map(|p| {
                                let subject = c.replacement.as_deref().unwrap_or("");
                                let (file, test) = p.target();
                                let proof_exists =
                                    crate::evidence::test_exists(&a.files, file, test);
                                let (pd, sd) = crate::evidence::current_digests(
                                    &a.files,
                                    &a.declaration,
                                    p,
                                    subject,
                                );
                                let records = a
                                    .evidence
                                    .records
                                    .get(&p.locator)
                                    .into_iter()
                                    .flatten()
                                    .map(|r| {
                                        Json::obj()
                                            .with("kind", r.kind.wire())
                                            .with("recorded_passed", r.passed)
                                            .with("proof_digest", &r.proof_digest)
                                            .with("subject", &r.subject)
                                            .with("subject_digest", &r.subject_digest)
                                            .with("command", &r.command)
                                            .with(
                                                "matches_current_bytes",
                                                proof_exists
                                                    && !pd.is_empty()
                                                    && !sd.is_empty()
                                                    && r.kind == p.kind
                                                    && r.subject == subject
                                                    && r.proof_digest == pd
                                                    && r.subject_digest == sd,
                                            )
                                    })
                                    .collect::<Vec<_>>();
                                Json::obj()
                                    .with("locator", &p.locator)
                                    .with("kind", p.kind.wire())
                                    .with("subject", subject)
                                    .with("proof_exists", proof_exists)
                                    .with("current_proof_digest", pd)
                                    .with("current_subject_digest", sd)
                                    .with("recorded_results", records)
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                Json::obj()
                    .with("key", &c.key)
                    .with("required", c.required)
                    .with("native", c.native)
                    .with("native_detail", &c.native_detail)
                    .with("targeted", c.targeted)
                    .with("replacement_exists", c.replacement_exists)
                    .with("replacement_canonical", c.replacement_canonical)
                    .with("mapped", c.mapped)
                    .with("relevance_resolved", c.relevance_resolved)
                    .with("knowledge", &c.knowledge)
                    .with("knowledge_open", c.knowledge_open)
                    .with(
                        "parity",
                        c.parity
                            .iter()
                            .map(|(p, v)| Json::obj().with("locator", p).with("verdict", v.wire()))
                            .collect::<Vec<_>>(),
                    )
                    .with(
                        "regression",
                        c.regression
                            .iter()
                            .map(|(p, v)| Json::obj().with("locator", p).with("verdict", v.wire()))
                            .collect::<Vec<_>>(),
                    )
                    .with("proof_bindings", proofs)
            })
            .collect::<Vec<_>>();
        donors.push(
            Json::obj()
                .with("source_alias", &d.key)
                .with("source_id", NodeId::of(&ns, &key).to_string())
                .with("origin", &d.origin)
                .with("provenance", d.provenance.clone())
                .with("provenance_json_evidence", extra_evidence)
                .with("classification", classified)
                .with("declared_state", d.claimed.wire())
                .with("effective_state", da.effective.wire())
                .with("effective_extinct", da.extinct())
                .with("gate_member", active.contains(&d.key))
                .with("stopped_by", &da.stopped_by)
                .with(
                    "legal_exception",
                    da.exception.as_ref().map_or(Json::Null, |(k, why)| {
                        Json::obj().with("kind", k.wire()).with("reason", why)
                    }),
                )
                .with(
                    "rights",
                    Json::obj()
                        .with("declared_license", &d.license)
                        .with("policy_basis", crate::licence::BASIS)
                        .with("declared_string_family", policy.family.word())
                        .with("declared_code_policy", policy.code_adoption.word())
                        .with(
                            "registry_origin_verified",
                            relation.is_some_and(|r| r.origin_verified),
                        )
                        .with(
                            "registry_license_verified",
                            relation.is_some_and(|r| r.license_verified),
                        )
                        .with(
                            "verification_scope",
                            "EXISTING_SOURCE_RECORD_ASSERTIONS; NOT_CURRENT_RIGHTS_AUTHORIZATION",
                        )
                        .with(
                            "copying_authorization",
                            "UNKNOWN_NOT_CERTIFIED_BY_THIS_REPORT",
                        )
                        .with(
                            "missing_evidence",
                            [
                                (!relation.is_some_and(|r| r.origin_verified))
                                    .then_some("UNIVERSE_ORIGIN_VERIFIED_FLAG_NOT_SET; REVIEW_PROVENANCE_SEPARATELY"),
                                (!relation.is_some_and(|r| r.license_verified))
                                    .then_some("UNIVERSE_OBSERVED_LICENSE_FLAG_NOT_SET; REVIEW_SEPARATE_LICENSE_RECEIPTS"),
                                Some("CURRENT_COPYING_AUTHORIZATION_NOT_ESTABLISHED_BY_REPORT"),
                                Some("REVIEWED_OSS_INCLUSION_NOT_EVALUATED_BY_RAW_AUDIT; USE_DONOR_RECONCILE"),
                            ]
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>(),
                        ),
                )
                .with(
                    "gates",
                    da.gates
                        .iter()
                        .map(|g| {
                            Json::obj()
                                .with("gate", g.gate.wire())
                                .with("pass", g.pass)
                                .with("detail", &g.detail)
                        })
                        .collect::<Vec<_>>(),
                )
                .with("capabilities", capabilities)
                .with(
                    "caller_and_closure",
                    Json::obj()
                        .with("runtime", observed(&da.facts.runtime))
                        .with("build", observed(&da.facts.build))
                        .with("linked", observed(&da.facts.linked))
                        .with("test", observed(&da.facts.test))
                        .with(
                            "imports",
                            da.facts
                                .imports
                                .iter()
                                .map(|(p, i)| Json::obj().with("file", p).with("identifier", i))
                                .collect::<Vec<_>>(),
                        )
                        .with("resident", da.facts.resident.clone())
                        .with("dependents", da.facts.dependents.clone())
                        .with(
                            "semantic_review",
                            da.facts
                                .semantic_review
                                .as_ref()
                                .map_or(Json::Null, Json::from),
                        ),
                ),
        );
    }
    let denominator = a.counts.donors_active() + a.counts.unregistered_externals;
    Ok(Json::obj().with("format", FORMAT).with("source_id", &a.declaration.repository.id).with("source_repository", &a.declaration.repository.origin)
        .with("governor_schema", crate::protocol::schema_identity()).with("snapshot", snapshot.to_json())
        .with("binding", "DESCRIPTIVE_SOURCE_AUDIT; NOT_ENROLLED; NO_PROOFS_EXECUTED; NO_RIGHTS_OR_NATIVE_OS_CERTIFICATION")
        .with("mixed_declared_cohort", Json::obj().with("total", a.declaration.donors.len())
            .with("declared_extinct", a.declaration.donors.iter().filter(|d| d.claimed==DonorState::Extinct).count())
            .with("effective_extinct", a.analysis.donors.iter().filter(|d| d.extinct()).count()))
        .with("active_extinction_gate", Json::obj().with("numerator", a.counts.donors_extinct).with("registered_active", a.counts.donors_active())
            .with("unregistered_externals", a.counts.unregistered_externals).with("denominator", denominator)
            .with("pass", a.counts.donors_extinct==denominator).with("active_keys", active.into_iter().collect::<Vec<_>>())
            .with("rule", "Historical registered union effective registered; exclude only legal REJECTED/SUPERSEDED; add all unregistered external participation"))
        .with("classification_summary", Json::Object(states.into_iter().map(|(k,v)| (k,Json::from(v))).collect()))
        .with("audited_oss", Json::obj().with("status", "UNKNOWN").with("total", Json::Null).with("verified_extinct", Json::Null)
            .with("reason", "Raw audit does not evaluate separate source-inclusion reviews; imported kind/defaults and licences alone do not establish audited eligibility. Use donor reconcile; no cohort rows removed."))
        .with("donors", donors).with("participation_descriptors", source_descriptors(a))
        .with("universe_record_evidence", universe.records().map(|(s,r)| Json::obj()
            .with("universe_file", &s.file).with("source", &s.source).with("source_digest", &s.digest)
            .with("source_table", &s.table).with("record_id", &r.id).with("provenance", &r.provenance)
            .with("global_donor", r.global.as_ref().map_or(Json::Null, Json::from))
            .with("origin", &r.origin).with("origin_verified", r.origin_verified).with("origin_evidence", &r.origin_evidence)
            .with("observed_license", &r.license).with("license_claim", &r.license_claim).with("revision", &r.revision)
            .with("kind_assignment_trace", "IMPORTED_KIND; EXPLICIT_RECORD_OR_COMMAND_OVERRIDE_OR_TABLE; ASSIGNMENT_ORIGIN_NOT_STORED; NOT_REVIEWED_OSS_DECISION")
            .with("classification", classification(&[r],&BTreeSet::new()))).collect::<Vec<_>>())
        .with("universe_sources", a.universe.sources.iter().map(|(p,s)| Json::obj().with("source",p).with("state",s.wire())).collect::<Vec<_>>())
        .with("universe_relations", a.universe.relations.iter().map(|r| Json::obj().with("label", &r.label).with("derived_primary_kind", r.kind.wire())
            .with("kind_basis", "UNIVERSE_RECORDS_PLUS_AUTOMATIC_DECLARED_DONOR_OSS_DEFAULT; NOT_REVIEWED_CLASSIFICATION")
            .with("source_aliases", r.donors.clone()).with("source_node_ids", r.node_ids.clone()).with("records",r.records.clone())).collect::<Vec<_>>())
        .with("legacy_role_definition_evidence", a.knowledge.facts.iter().filter(|f| f.subject=="research/commerce/specification.md" && matches!(f.key.as_str(),"00180"|"00186"|"00193"))
            .map(|f| Json::obj().with("source_fact_id",f.id()).with("subject", &f.subject).with("key",&f.key).with("value",&f.value).with("provenance",f.provenance.iter().cloned().collect::<Vec<_>>())).collect::<Vec<_>>())
        .with("unregistered_participation", a.analysis.unregistered.iter().map(|((eco,name),obs)| Json::obj().with("ecosystem",eco.wire()).with("name",name).with("observations",observed(obs))).collect::<Vec<_>>())
        .with("vanished_historical_donors", a.analysis.vanished.clone()).with("census_unreadable",a.census.unreadable.clone())
        .with("evidence_unreadable",a.evidence.unreadable.clone()).with("history_unreadable",a.history.unreadable.clone())
        .with("knowledge_unreadable",a.knowledge.unreadable.clone())
        .with("universe_unreadable", universe.unreadable.iter().map(|(p,e)|Json::obj().with("path",p).with("reason",e)).collect::<Vec<_>>())
        .with("findings",a.findings.iter().map(|f|f.to_json()).collect::<Vec<_>>())
        .with("hosted_scope_exemptions",Json::obj().with("platform_libraries",super::PLATFORM_LIBRARIES.to_vec()).with("toolchain_programs",super::TOOLCHAIN_PROGRAMS.to_vec()))
        .with("native_certification", "N01-N12/J1-J9/P0-P3 NOT_RUN; HOSTED SOURCE GOVERNANCE ONLY"))
}
