//! The `'static` types that the declaration files are written in. rustc type-checks every
//! repository's `.ecdev/declared/*.rs` against these (see `main.rs`); the run-time reader in
//! `declare` accepts exactly the same grammar. `into_model` proves both readings agree.

pub use crate::schema::{
    Concept, Dimension, DonorState, Ecosystem, EdgeKind, NodeKind, NodeLifecycle, ProofKind, Scope,
    ShimKind, TechnologyKind, TechnologyLifecycle, WaveStatus,
};

use super::model;
use crate::schema::{ExceptionKind, ExpiryKind};

pub struct Repository {
    pub id: &'static str,
    pub name: &'static str,
    pub origin: &'static str,
    pub nodes: &'static [Node],
    pub edges: &'static [Edge],
}

pub struct Node {
    pub key: &'static str,
    pub kind: NodeKind,
    pub concept: Concept,
    pub name: &'static str,
    pub path: &'static str,
    pub canonical_path: &'static str,
    pub lifecycle: NodeLifecycle,
    pub provides: &'static [&'static str],
    pub requires: &'static [&'static str],
    pub reuses: &'static [&'static str],
    pub inputs: &'static [&'static str],
    pub outputs: &'static [&'static str],
    pub lineage: &'static [&'static str],
}

pub struct Edge {
    pub from: &'static str,
    pub to: &'static str,
    pub kind: EdgeKind,
    pub scope: Scope,
}

pub enum Exception {
    None,
    Blocked(&'static str),
    Rejected(&'static str),
    Superseded(&'static str),
}

pub struct Package {
    pub ecosystem: Ecosystem,
    pub name: &'static str,
}

pub struct Proof {
    pub kind: ProofKind,
    pub locator: &'static str,
}

/// Production adoption of a donor capability: undecided, relied on (which behaviour), or not
/// adopted (why production does not execute it). Not a statement about research value.
pub enum Relevance {
    Unresolved,
    ReliedOn(&'static str),
    NotAdopted(&'static str),
}

pub use super::model::ResearchScope;

/// What ECDEV still has to learn from a donor capability (see `model::Knowledge`).
pub enum Knowledge {
    Unreviewed,
    StudyCandidate(&'static str),
    BenchmarkCandidate(&'static str),
    AlgorithmCandidate(&'static str),
    ReferenceOnly(&'static str),
    Absorbed(&'static str),
    IndependentNative(&'static str),
    Divergent(&'static str),
    NoResearchValue(ResearchScope, &'static str),
}

pub struct Capability {
    pub key: &'static str,
    pub required: bool,
    pub spec: &'static str,
    pub replacement: Option<&'static str>,
    /// The capability or technology key (`capability/<k>`, `technology/<k>`) of the canonical
    /// graph this donor capability maps to.
    pub maps_to: Option<&'static str>,
    pub relevance: Relevance,
    pub knowledge: Knowledge,
    pub proofs: &'static [Proof],
}

pub struct Donor {
    pub key: &'static str,
    pub name: &'static str,
    pub origin: &'static str,
    pub license: &'static str,
    pub claimed: DonorState,
    pub exception: Exception,
    pub packages: &'static [Package],
    pub source_paths: &'static [&'static str],
    pub capabilities: &'static [Capability],
    pub cutover: Option<&'static str>,
    pub provenance: &'static [&'static str],
}

pub enum Expiry {
    WaveApplied(&'static str),
    DonorExtinct(&'static str),
    NodeCanonical(&'static str),
}

pub struct Wave {
    pub key: &'static str,
    pub status: WaveStatus,
    pub nodes: &'static [&'static str],
}

pub struct Shim {
    pub key: &'static str,
    pub kind: ShimKind,
    pub path: &'static str,
    pub serves: &'static str,
    pub expires: Expiry,
}

pub struct Migration {
    pub waves: &'static [Wave],
    pub shims: &'static [Shim],
}

pub struct Relation {
    pub kind: EdgeKind,
    pub target: &'static str,
}

pub struct Improvement {
    pub dimension: Dimension,
    pub baseline: &'static str,
    pub workload: &'static str,
    pub statement: &'static str,
    pub evidence: &'static [Proof],
}

pub struct Technology {
    pub key: &'static str,
    pub name: &'static str,
    pub kind: TechnologyKind,
    pub claimed: TechnologyLifecycle,
    pub purpose: &'static str,
    pub implements: &'static [&'static str],
    pub node: &'static str,
    pub sources: &'static [&'static str],
    pub invariants: &'static [&'static str],
    pub proofs: &'static [Proof],
    pub lineage: &'static [&'static str],
    pub relations: &'static [Relation],
    pub claims: &'static [Improvement],
}

fn relevance(n: &Relevance) -> model::Relevance {
    match n {
        Relevance::Unresolved => model::Relevance::Unresolved,
        Relevance::ReliedOn(k) => model::Relevance::ReliedOn(k.to_string()),
        Relevance::NotAdopted(r) => model::Relevance::NotAdopted(r.to_string()),
    }
}

fn knowledge(n: &Knowledge) -> model::Knowledge {
    use model::Knowledge as K;
    match n {
        Knowledge::Unreviewed => K::Unreviewed,
        Knowledge::StudyCandidate(t) => K::StudyCandidate(t.to_string()),
        Knowledge::BenchmarkCandidate(t) => K::BenchmarkCandidate(t.to_string()),
        Knowledge::AlgorithmCandidate(t) => K::AlgorithmCandidate(t.to_string()),
        Knowledge::ReferenceOnly(t) => K::ReferenceOnly(t.to_string()),
        Knowledge::Absorbed(t) => K::Absorbed(t.to_string()),
        Knowledge::IndependentNative(t) => K::IndependentNative(t.to_string()),
        Knowledge::Divergent(t) => K::Divergent(t.to_string()),
        Knowledge::NoResearchValue(s, t) => K::NoResearchValue(*s, t.to_string()),
    }
}

fn strings(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// Converts compiled declarations into the run-time model.
fn proofs(p: &[Proof]) -> Vec<model::Proof> {
    p.iter()
        .map(|p| model::Proof {
            kind: p.kind,
            locator: p.locator.into(),
        })
        .collect()
}

pub fn into_model(
    r: &Repository,
    donors: &[Donor],
    m: &Migration,
    technologies: &[Technology],
) -> model::Declaration {
    model::Declaration {
        technologies: technologies
            .iter()
            .map(|t| model::Technology {
                key: t.key.into(),
                name: t.name.into(),
                kind: t.kind,
                claimed: t.claimed,
                purpose: t.purpose.into(),
                implements: strings(t.implements),
                node: t.node.into(),
                sources: strings(t.sources),
                invariants: strings(t.invariants),
                proofs: proofs(t.proofs),
                lineage: strings(t.lineage),
                relations: t
                    .relations
                    .iter()
                    .map(|x| model::Relation {
                        kind: x.kind,
                        target: x.target.into(),
                    })
                    .collect(),
                claims: t
                    .claims
                    .iter()
                    .map(|c| model::Improvement {
                        dimension: c.dimension,
                        baseline: c.baseline.into(),
                        workload: c.workload.into(),
                        statement: c.statement.into(),
                        evidence: proofs(c.evidence),
                    })
                    .collect(),
            })
            .collect(),
        repository: model::Repository {
            id: r.id.into(),
            name: r.name.into(),
            origin: r.origin.into(),
            nodes: r
                .nodes
                .iter()
                .map(|n| model::Node {
                    key: n.key.into(),
                    kind: n.kind,
                    concept: n.concept,
                    name: n.name.into(),
                    path: n.path.into(),
                    canonical_path: n.canonical_path.into(),
                    lifecycle: n.lifecycle,
                    provides: strings(n.provides),
                    requires: strings(n.requires),
                    reuses: strings(n.reuses),
                    inputs: strings(n.inputs),
                    outputs: strings(n.outputs),
                    lineage: strings(n.lineage),
                })
                .collect(),
            edges: r
                .edges
                .iter()
                .map(|e| model::Edge {
                    from: e.from.into(),
                    to: e.to.into(),
                    kind: e.kind,
                    scope: e.scope,
                })
                .collect(),
        },
        donors: donors
            .iter()
            .map(|d| model::Donor {
                key: d.key.into(),
                name: d.name.into(),
                origin: d.origin.into(),
                license: d.license.into(),
                claimed: d.claimed,
                exception: match d.exception {
                    Exception::None => None,
                    Exception::Blocked(s) => Some((ExceptionKind::Blocked, s.into())),
                    Exception::Rejected(s) => Some((ExceptionKind::Rejected, s.into())),
                    Exception::Superseded(s) => Some((ExceptionKind::Superseded, s.into())),
                },
                packages: d
                    .packages
                    .iter()
                    .map(|p| model::Package {
                        ecosystem: p.ecosystem,
                        name: p.name.into(),
                    })
                    .collect(),
                source_paths: strings(d.source_paths),
                capabilities: d
                    .capabilities
                    .iter()
                    .map(|c| model::Capability {
                        key: c.key.into(),
                        required: c.required,
                        spec: c.spec.into(),
                        replacement: c.replacement.map(str::to_string),
                        maps_to: c.maps_to.map(str::to_string),
                        relevance: relevance(&c.relevance),
                        knowledge: knowledge(&c.knowledge),
                        proofs: c
                            .proofs
                            .iter()
                            .map(|p| model::Proof {
                                kind: p.kind,
                                locator: p.locator.into(),
                            })
                            .collect(),
                    })
                    .collect(),
                cutover: d.cutover.map(str::to_string),
                provenance: strings(d.provenance),
            })
            .collect(),
        migration: model::Migration {
            waves: m
                .waves
                .iter()
                .map(|w| model::Wave {
                    key: w.key.into(),
                    status: w.status,
                    nodes: strings(w.nodes),
                })
                .collect(),
            shims: m
                .shims
                .iter()
                .map(|s| model::Shim {
                    key: s.key.into(),
                    kind: s.kind,
                    path: s.path.into(),
                    serves: s.serves.into(),
                    expires: match s.expires {
                        Expiry::WaveApplied(t) => (ExpiryKind::WaveApplied, t.into()),
                        Expiry::DonorExtinct(t) => (ExpiryKind::DonorExtinct, t.into()),
                        Expiry::NodeCanonical(t) => (ExpiryKind::NodeCanonical, t.into()),
                    },
                })
                .collect(),
        },
    }
}
