//! Derived source semantics for a portable capture. This is neither a canonical declaration
//! nor an effective-state assessment. Original bytes in the capture remain the lossless input.

use super::source::Capture;
use crate::compact::{codec::Decoder, facts, history};
use crate::declare::{Knowledge, Relevance};
use crate::digest::content_digest;
use crate::evidence;
use crate::formats::json::Json;
use crate::graph::{self, Graph, NodeId};
use std::collections::BTreeSet;

fn optional(value: &Option<String>) -> Json {
    value.as_ref().map_or(Json::Null, Json::from)
}

fn adoption(value: &Relevance) -> Json {
    let (variant, reason) = match value {
        Relevance::Unresolved => ("Unresolved", None),
        Relevance::ReliedOn(s) => ("ReliedOn", Some(s)),
        Relevance::NotAdopted(s) => ("NotAdopted", Some(s)),
    };
    Json::obj()
        .with("variant", variant)
        .with("reason", reason.map_or(Json::Null, Json::from))
}

fn knowledge(value: &Knowledge) -> Json {
    let variant = match value {
        Knowledge::Unreviewed => "Unreviewed",
        Knowledge::StudyCandidate(_) => "StudyCandidate",
        Knowledge::BenchmarkCandidate(_) => "BenchmarkCandidate",
        Knowledge::AlgorithmCandidate(_) => "AlgorithmCandidate",
        Knowledge::ReferenceOnly(_) => "ReferenceOnly",
        Knowledge::Absorbed(_) => "Absorbed",
        Knowledge::IndependentNative(_) => "IndependentNative",
        Knowledge::Divergent(_) => "Divergent",
        Knowledge::NoResearchValue(..) => "NoResearchValue",
    };
    let scope = match value {
        Knowledge::NoResearchValue(scope, _) => Json::from(scope.word()),
        _ => Json::Null,
    };
    Json::obj()
        .with("variant", variant)
        .with("text", value.text())
        .with("scope", scope)
}

pub fn inspect(capture: &Capture) -> Result<Json, String> {
    capture.validate()?;
    let declaration = capture.declaration()?;
    let repo = &declaration.repository.id;
    let mut declared = BTreeSet::new();
    let mut absorbed = BTreeSet::new();
    let donors = declaration
        .donors
        .iter()
        .map(|donor| {
            let (namespace, key) = Graph::donor_id(repo, &donor.key, &donor.origin);
            let capabilities = donor
                .capabilities
                .iter()
                .map(|cap| {
                    declared.insert(format!("{}/{key}", donor.key, key = cap.key));
                    for old in &cap.absorbs {
                        absorbed.insert(format!("{}/{old}", donor.key));
                    }
                    Json::obj()
                        .with("key", &cap.key)
                        .with(
                            "source_id",
                            NodeId::of(
                                graph::NAMESPACE,
                                &graph::capability_key(repo, &donor.key, &cap.key),
                            )
                            .to_string(),
                        )
                        .with("required", cap.required)
                        .with("spec", &cap.spec)
                        .with("replacement", optional(&cap.replacement))
                        .with("maps_to", optional(&cap.maps_to))
                        .with("adoption", adoption(&cap.relevance))
                        .with("knowledge", knowledge(&cap.knowledge))
                        .with("absorbs", cap.absorbs.clone())
                        .with(
                            "proofs",
                            cap.proofs
                                .iter()
                                .map(|p| {
                                    Json::obj()
                                        .with("kind", p.kind.wire())
                                        .with("locator", &p.locator)
                                })
                                .collect::<Vec<_>>(),
                        )
                })
                .collect::<Vec<_>>();
            Json::obj()
                .with("source_alias", &donor.key)
                .with("source_id", NodeId::of(&namespace, &key).to_string())
                .with("source_namespace", namespace)
                .with("source_semantic_key", key)
                .with(
                    "global_origin_key",
                    crate::universe::identity::global_donor_key(&donor.origin)
                        .map_or(Json::Null, Json::from),
                )
                .with("name", &donor.name)
                .with("origin", &donor.origin)
                .with("license", &donor.license)
                .with("claimed", donor.claimed.wire())
                .with("canonical_effective_state", Json::Null)
                .with(
                    "exception",
                    donor
                        .exception
                        .as_ref()
                        .map_or(Json::Null, |(kind, reason)| {
                            Json::obj().with("kind", kind.wire()).with("reason", reason)
                        }),
                )
                .with(
                    "packages",
                    donor
                        .packages
                        .iter()
                        .map(|p| {
                            Json::obj()
                                .with("ecosystem", p.ecosystem.wire())
                                .with("name", &p.name)
                        })
                        .collect::<Vec<_>>(),
                )
                .with("source_paths", donor.source_paths.clone())
                .with("provenance", donor.provenance.clone())
                .with("cutover", optional(&donor.cutover))
                .with("capabilities", capabilities)
        })
        .collect::<Vec<_>>();
    let mut history_donors = BTreeSet::new();
    let mut history_active = BTreeSet::new();
    let mut history_capabilities = BTreeSet::new();
    let mut capability_floor_recorded = false;
    let mut source_facts = Vec::new();
    let mut records = Vec::new();
    let (mut active, mut retired) = (0usize, 0usize);
    for (path, bytes) in &capture.files {
        if path.starts_with(".ecdev/history/") {
            let batch = history::Batch::decode(bytes).map_err(|e| e.to_string())?;
            capability_floor_recorded |= Decoder::tag_of(bytes) == Some(history::HISTORY_TAG);
            history_donors.extend(batch.registered);
            history_active.extend(batch.active);
            history_capabilities.extend(batch.capabilities);
        } else if path.starts_with(".ecdev/knowledge/") {
            for fact in facts::decode(bytes).map_err(|e| e.to_string())? {
                source_facts.push(
                    Json::obj()
                        .with("source_record", path)
                        .with("source_id", fact.id())
                        .with("kind", fact.kind.wire())
                        .with("subject", fact.subject)
                        .with("key", fact.key)
                        .with("value", fact.value)
                        .with(
                            "provenance",
                            fact.provenance.into_iter().collect::<Vec<_>>(),
                        )
                        // A decimal string avoids narrowing u64 sequence values in the JSON view.
                        .with("sequence_decimal", fact.seq.to_string())
                        .with("superseded", fact.superseded),
                );
            }
        } else if path.starts_with(".ecdev/evidence/") {
            if Decoder::tag_of(bytes) == Some(evidence::RETIRED_TAG) {
                retired += evidence::decode_retired(bytes)
                    .map_err(|e| e.to_string())?
                    .len();
            } else {
                active += 1;
            }
        }
        records.push(
            Json::obj()
                .with("path", path)
                .with("sha256", &content_digest(bytes)[7..])
                .with("bytes", bytes.len()),
        );
    }
    let known = capability_floor_recorded.then(|| {
        declared
            .iter()
            .chain(history_capabilities.difference(&absorbed))
            .cloned()
            .collect::<BTreeSet<_>>()
    });
    Ok(Json::obj()
        .with("format", "ecdev-source-inspection-v1")
        .with(
            "binding",
            "DERIVED_SOURCE_VIEW; NOT_ENROLLED; NOT_REVERIFIED; NO_CANONICAL_IDENTITY_MAPPING",
        )
        .with(
            "capture_sha256",
            &content_digest(capture.render()?.as_bytes())[7..],
        )
        .with("source_repository", declaration.repository.origin)
        .with("source_id", repo)
        .with("observed_head", &capture.observed_head)
        .with(
            "commit_membership",
            "NOT_EXTERNALLY_VERIFIED; V1_ENVELOPE_CONTAINS_NO_GIT_MEMBERSHIP_PROOF",
        )
        .with("canonical_enrollment", "BLOCKED")
        .with("canonical_identity_mapping", Json::Null)
        .with("norl_relevance_mapping", Json::Null)
        .with(
            "recorded_schema_claim",
            "NOT_REVERIFIED; DOES_NOT_SELECT_READER_AUTHORITY",
        )
        .with("source_graph", capture.source_graph()?)
        .with("donors", donors)
        .with(
            "history",
            Json::obj()
                .with(
                    "registered_donors",
                    history_donors.into_iter().collect::<Vec<_>>(),
                )
                .with(
                    "active_donors",
                    history_active.into_iter().collect::<Vec<_>>(),
                )
                .with("capability_floor_recorded", capability_floor_recorded)
                .with(
                    "recorded_capabilities",
                    history_capabilities.into_iter().collect::<Vec<_>>(),
                )
                .with(
                    "declared_capabilities",
                    declared.into_iter().collect::<Vec<_>>(),
                )
                .with(
                    "absorbed_same_donor_keys",
                    absorbed.into_iter().collect::<Vec<_>>(),
                )
                .with(
                    "known_capabilities",
                    known.map_or(Json::Null, |keys| {
                        Json::from(keys.into_iter().collect::<Vec<_>>())
                    }),
                )
                .with(
                    "completeness",
                    "SUPPLIED_RECORDS_ONLY; REPOSITORY_HISTORY_NOT_ATTESTED",
                ),
        )
        .with("facts", source_facts)
        .with("active_evidence_records", active)
        .with("retired_evidence_records", retired)
        .with("records", records))
}
