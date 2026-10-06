//! Runtime adoption and knowledge value are separate questions. These negative cases prove the
//! governance refuses to infer "no research value" from adoption facts — no current caller, no
//! native implementation, a different implementation, a restrictive licence — that a rejected
//! runtime keeps its knowledge, and that production completion is never knowledge completion.

mod common;

use common::*;
use ecdev_governance::declare::{Knowledge, Relevance, ResearchScope};
use ecdev_governance::schema::{DonorState, ExceptionKind, Gate, KnowledgeState};

fn check(a: &ecdev_governance::Assessment, id: &str) -> ecdev_governance::conformance::Check {
    ecdev_governance::conformance::checks(a)
        .into_iter()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("no check {id}"))
}

/// A second, non-required, not-adopted capability with the given knowledge decision.
fn with_extra(r: &Repo, key: &str, knowledge: Knowledge) {
    r.write("research/review.json", "{}\n");
    let key = key.to_string();
    r.edit(move |d| {
        let mut extra = d.donors[0].capabilities[0].clone();
        extra.key = key.clone();
        extra.required = false;
        extra.spec = "research/review.json".into();
        extra.relevance = Relevance::NotAdopted("production does not execute it".into());
        extra.knowledge = knowledge.clone();
        extra.proofs.clear();
        extra.replacement = None;
        extra.maps_to = None;
        d.donors[0].capabilities.retain(|c| c.key != key);
        d.donors[0].capabilities.push(extra);
    });
}

fn metric(a: &ecdev_governance::Assessment, name: &str) -> String {
    a.metric(name)
}

#[test]
fn no_current_caller_cannot_ground_no_research_value() {
    let r = extinct_baseline("no-caller");
    for reason in [
        "the simulator has no current caller",
        "no production caller",
        "ECDEV does not currently use embeddings",
        "ECDEV already has a simpler implementation",
        "no native implementation exists",
        "it does not contribute observed evidence",
        "the donor is GPL",
        "AGPL licence",
        "no runtime dependency exists",
    ] {
        with_extra(
            &r,
            "simulation",
            Knowledge::NoResearchValue(ResearchScope::UnrelatedDomain, reason.into()),
        );
        let a = r.assess();
        assert!(
            !check(&a, "knowledge.irrelevance_admissible").pass,
            "accepted: {reason}"
        );
    }
    // Reference-only cannot be reached on adoption grounds either.
    with_extra(
        &r,
        "simulation",
        Knowledge::ReferenceOnly("no current caller".into()),
    );
    assert!(!check(&r.assess(), "knowledge.irrelevance_admissible").pass);
    // The same capability as a study candidate is a valid state.
    with_extra(
        &r,
        "simulation",
        Knowledge::StudyCandidate("multi-run aggregation for SIMULATED scenarios".into()),
    );
    assert!(check(&r.assess(), "knowledge.irrelevance_admissible").pass);
}

#[test]
fn admissible_scope_grounds_no_research_value() {
    let r = extinct_baseline("admissible");
    with_extra(
        &r,
        "docs-site",
        Knowledge::NoResearchValue(
            ResearchScope::Frontend,
            "the donor's documentation website".into(),
        ),
    );
    let a = r.assess();
    assert!(check(&a, "knowledge.irrelevance_admissible").pass);
    // Fully resolved knowledge and every other gate: extinction stands.
    assert_eq!(a.donor("geo").unwrap().effective, DonorState::Extinct);
    assert_eq!(
        a.donor("geo").unwrap().knowledge,
        KnowledgeState::StudyComplete
    );
}

#[test]
fn extinction_requires_the_knowledge_to_be_learned() {
    let r = extinct_baseline("knowledge-gate");
    with_extra(
        &r,
        "embedding-clustering",
        Knowledge::BenchmarkCandidate("does HDBSCAN recover topics lexical overlap misses?".into()),
    );
    let a = r.assess();
    let geo = a.donor("geo").unwrap();
    // Every dependency gate passes, but an open benchmark question stops it at CUTOVER.
    assert_eq!(geo.effective, DonorState::Cutover);
    assert!(
        !geo.gates
            .iter()
            .find(|g| g.gate == Gate::KnowledgeResolved)
            .unwrap()
            .pass
    );
    assert_eq!(geo.knowledge, KnowledgeState::ActiveStudy);
    // Unreviewed is open too.
    with_extra(&r, "embedding-clustering", Knowledge::Unreviewed);
    let geo = r.assess();
    let geo = geo.donor("geo").unwrap();
    assert_eq!(geo.effective, DonorState::Cutover);
    assert_eq!(geo.knowledge, KnowledgeState::StructuralCensus);
}

#[test]
fn a_rejected_runtime_keeps_its_knowledge() {
    // A GPL donor ECDEV never adopted: runtime REJECT_RUNTIME, knowledge still an active study.
    let r = extinct_baseline("rejected-runtime");
    r.write("research/review.json", "{}\n");
    r.edit(|d| {
        let dn = &mut d.donors[0];
        dn.license = "GPL-3.0".into();
        dn.packages.clear();
        dn.cutover = None;
        dn.claimed = DonorState::Registered;
        dn.exception = Some((ExceptionKind::Rejected, "GPL runtime not adopted".into()));
        for c in &mut dn.capabilities {
            c.required = false;
            c.replacement = None;
            c.maps_to = None;
            c.proofs.clear();
            c.spec = "research/review.json".into();
            c.relevance = Relevance::NotAdopted("GPL runtime not adopted".into());
            c.knowledge = Knowledge::AlgorithmCandidate("clean-room distance algorithm".into());
        }
    });
    r.write(
        "core/Cargo.toml",
        "[package]\nname = \"core-kernel\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    let a = r.assess();
    let geo = a.donor("geo").unwrap();
    assert!(
        matches!(geo.exception, Some((ExceptionKind::Rejected, _))),
        "{:?}",
        geo.exception
    );
    assert_eq!(geo.knowledge, KnowledgeState::ActiveStudy);
    assert!(check(&a, "knowledge.irrelevance_admissible").pass);
    // The rejected donor stays in the knowledge denominator with its open question.
    assert_eq!(metric(&a, "known_donor_capabilities"), "1");
    assert_eq!(metric(&a, "algorithm_candidates"), "1");
    assert_eq!(metric(&a, "donors_with_open_research"), "1");
    // And its universe relation stays relevant: a runtime rejection is no knowledge verdict.
    let rel = a
        .universe
        .relations
        .iter()
        .find(|r| r.donors.iter().any(|d| d == "geo"))
        .unwrap();
    assert!(
        !matches!(
            rel.relevance,
            ecdev_governance::universe::Relevance::NotRelevant(_)
        ),
        "{:?}",
        rel.relevance
    );
}

#[test]
fn relied_on_capability_cannot_be_declared_without_research_value() {
    let r = extinct_baseline("contradiction");
    r.edit(|d| {
        d.donors[0].capabilities[0].knowledge =
            Knowledge::NoResearchValue(ResearchScope::Assets, "sample logos".into())
    });
    assert!(!check(&r.assess(), "knowledge.irrelevance_admissible").pass);
}

#[test]
fn production_completion_is_reported_apart_from_knowledge_completion() {
    let r = extinct_baseline("two-axes");
    with_extra(
        &r,
        "simulation-lifecycle",
        Knowledge::StudyCandidate("scenario state machine for SIMULATED runs".into()),
    );
    let a = r.assess();
    // Every production capability is native and proven ...
    assert_eq!(metric(&a, "production_proof_ratio"), "1.000000");
    assert_eq!(metric(&a, "production_capabilities_remaining"), "0");
    // ... while the knowledge universe is not complete.
    assert_eq!(metric(&a, "research_questions_open"), "1");
    assert_eq!(metric(&a, "knowledge_coverage_ratio"), "0.500000");
}
