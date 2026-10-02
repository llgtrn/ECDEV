//! The `'static` types that the declaration files are written in. rustc type-checks every
//! repository's `.ynventa/declared/*.rs` against these (see `main.rs`); the run-time reader in
//! `declare` accepts exactly the same grammar. `into_model` proves both readings agree.

pub use crate::schema::{
    BackendKind, Concept, Dimension, DonorState, Ecosystem, EdgeKind, GrowthState, NodeKind,
    NodeLifecycle, PromotionImplementation, PromotionState, ProofKind, Scope, ShimKind,
    TechnologyKind, TechnologyLifecycle, WaveStatus,
};

use super::model;
use crate::schema::{ExceptionKind, ExpiryKind};

pub struct Repository {
    pub system: &'static str,
    pub shard: &'static str,
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

/// How something relates to the Norl organism: still undecided, feeding a Norl capability (its
/// key), or declared not relevant (the reason).
pub enum NorlRelevance {
    Unresolved,
    Feeds(&'static str),
    NotRelevant(&'static str),
}

pub struct Capability {
    pub key: &'static str,
    pub required: bool,
    pub spec: &'static str,
    pub replacement: Option<&'static str>,
    /// The capability or technology key (`capability/<k>`, `technology/<k>`) of the canonical
    /// graph this donor capability maps to.
    pub maps_to: Option<&'static str>,
    pub norl: NorlRelevance,
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
    LegacyImported(&'static str),
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
    pub norl: NorlRelevance,
    pub claims: &'static [Improvement],
}

/// The organism declaration (`declared/organism.rs`): empty outside the norl shard.
pub struct Organism {
    pub capabilities: &'static [OrganismCapability],
    pub backends: &'static [Backend],
    /// Capabilities proven in another repository being promoted into Norl (norl only).
    pub promotions: &'static [Promotion],
}

/// A source repository's proven capability promoted into a Norl organism capability.
pub struct Promotion {
    pub key: &'static str,
    /// The canonical shard that proves it.
    pub source_repository: &'static str,
    /// The source node's semantic key.
    pub source_node: &'static str,
    pub source_capability: Option<&'static str>,
    pub source_technology: Option<&'static str>,
    pub source_commit: &'static str,
    /// Proof locators in the source repository.
    pub source_evidence: &'static [&'static str],
    /// The organism capability key it becomes.
    pub destination_capability: &'static str,
    /// The Norl physical node implementing it, once one exists.
    pub destination_node: Option<&'static str>,
    /// FEEDS or TEACHES.
    pub relation: EdgeKind,
    pub implementation: PromotionImplementation,
    pub claimed: PromotionState,
    pub reason: &'static str,
    pub transformation: &'static str,
}

pub struct OrganismCapability {
    /// The capability key (`capability/<key>` in the graph).
    pub key: &'static str,
    /// The organ node key.
    pub organ: &'static str,
    pub claimed: GrowthState,
    /// The backend key, if any.
    pub backend: Option<&'static str>,
    /// Keys of the Material(Evaluation) nodes that judge it (in any shard).
    pub evaluations: &'static [&'static str],
}

pub struct Backend {
    pub key: &'static str,
    pub kind: BackendKind,
    /// The norl physical node implementing it.
    pub node: &'static str,
    /// The donor key when the weight is borrowed.
    pub donor: Option<&'static str>,
    /// The weight artifact identity; empty for none.
    pub weight: &'static str,
}

fn norl(n: &NorlRelevance) -> model::NorlRelevance {
    match n {
        NorlRelevance::Unresolved => model::NorlRelevance::Unresolved,
        NorlRelevance::Feeds(k) => model::NorlRelevance::Feeds(k.to_string()),
        NorlRelevance::NotRelevant(r) => model::NorlRelevance::NotRelevant(r.to_string()),
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
    organism: &Organism,
) -> model::Declaration {
    model::Declaration {
        organism: model::Organism {
            capabilities: organism
                .capabilities
                .iter()
                .map(|c| model::OrganismCapability {
                    key: c.key.into(),
                    organ: c.organ.into(),
                    claimed: c.claimed,
                    backend: c.backend.map(str::to_string),
                    evaluations: strings(c.evaluations),
                })
                .collect(),
            backends: organism
                .backends
                .iter()
                .map(|b| model::Backend {
                    key: b.key.into(),
                    kind: b.kind,
                    node: b.node.into(),
                    donor: b.donor.map(str::to_string),
                    weight: b.weight.into(),
                })
                .collect(),
            promotions: organism
                .promotions
                .iter()
                .map(|p| model::Promotion {
                    key: p.key.into(),
                    source_repository: p.source_repository.into(),
                    source_node: p.source_node.into(),
                    source_capability: p.source_capability.map(str::to_string),
                    source_technology: p.source_technology.map(str::to_string),
                    source_commit: p.source_commit.into(),
                    source_evidence: strings(p.source_evidence),
                    destination_capability: p.destination_capability.into(),
                    destination_node: p.destination_node.map(str::to_string),
                    relation: p.relation,
                    implementation: p.implementation,
                    claimed: p.claimed,
                    reason: p.reason.into(),
                    transformation: p.transformation.into(),
                })
                .collect(),
        },
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
                norl: norl(&t.norl),
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
            system: r.system.into(),
            shard: r.shard.into(),
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
                        norl: norl(&c.norl),
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
                        Expiry::LegacyImported(t) => (ExpiryKind::LegacyImported, t.into()),
                    },
                })
                .collect(),
        },
    }
}
