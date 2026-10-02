//! The owned, run-time form of a repository's declarations.

use crate::schema::{
    BackendKind, Concept, Dimension, DonorState, Ecosystem, EdgeKind, ExceptionKind, ExpiryKind,
    GrowthState, NodeKind, NodeLifecycle, PromotionImplementation, PromotionState, ProofKind,
    Scope, ShimKind, TechnologyKind, TechnologyLifecycle, WaveStatus,
};

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Declaration {
    pub repository: Repository,
    pub donors: Vec<Donor>,
    pub migration: Migration,
    pub technologies: Vec<Technology>,
    pub organism: Organism,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Repository {
    /// The logical system this repository is a shard of: always `chronica`. Every node
    /// identity is derived under it, never under the repository.
    pub system: String,
    /// This physical shard (a canonical shard id, e.g. `mechatron`): metadata, not identity.
    pub shard: String,
    pub name: String,
    /// Canonical origin, e.g. `llgtrn/Chronica`.
    pub origin: String,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    /// Stable Chronica-wide semantic key (`machine.spatial`); with the system it is the node's
    /// identity. Never a path, never a repository.
    pub key: String,
    pub kind: NodeKind,
    /// What the node means in the Chronica ontology.
    pub concept: Concept,
    pub name: String,
    /// Where the node physically lives now (empty for abstract kinds and planned nodes).
    pub path: String,
    /// Where the canonical shape says it lives.
    pub canonical_path: String,
    pub lifecycle: NodeLifecycle,
    /// Capability keys this node provides to Chronica.
    pub provides: Vec<String>,
    /// Capability keys this node needs at run time, from whichever shard provides them.
    pub requires: Vec<String>,
    /// Technology keys this node compiles in natively (materialized, never a service call).
    pub reuses: Vec<String>,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    /// Donor keys this node was learned from.
    pub lineage: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    /// A Chronica node key (in any shard: keys are global), or `donor:<key>`.
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    pub scope: Scope,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Donor {
    pub key: String,
    pub name: String,
    pub origin: String,
    pub license: String,
    /// What the repository claims. The effective state is computed; a claim above it is a
    /// false-green violation.
    pub claimed: DonorState,
    pub exception: Option<(ExceptionKind, String)>,
    /// How the donor can enter a build.
    pub packages: Vec<Package>,
    /// Where donor source was (or is) held in this repository.
    pub source_paths: Vec<String>,
    pub capabilities: Vec<Capability>,
    /// The cutover record (a commit, wave or decision key); `None` until cut over.
    pub cutover: Option<String>,
    /// Where each fact about this donor came from, including legacy locators.
    pub provenance: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Package {
    pub ecosystem: Ecosystem,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Capability {
    pub key: String,
    pub required: bool,
    /// The specification: a repository path, or an inline statement of behaviour.
    pub spec: String,
    /// Key of the native replacement node.
    pub replacement: Option<String>,
    /// The capability or technology of the canonical graph it maps to (`capability/<k>` or
    /// `technology/<k>`); resolved by the system linker.
    pub maps_to: Option<String>,
    /// Its relevance to the Norl organism.
    pub norl: NorlRelevance,
    pub proofs: Vec<Proof>,
}

/// How a donor capability or a technology relates to the Norl organism. `Feeds` names a Norl
/// capability key, resolved by the system linker; `NotRelevant` states why not.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum NorlRelevance {
    #[default]
    Unresolved,
    Feeds(String),
    NotRelevant(String),
}

impl NorlRelevance {
    pub fn resolved(&self) -> bool {
        match self {
            NorlRelevance::Unresolved => false,
            NorlRelevance::Feeds(k) | NorlRelevance::NotRelevant(k) => !k.trim().is_empty(),
        }
    }
    pub fn feeds(&self) -> Option<&str> {
        match self {
            NorlRelevance::Feeds(k) if !k.trim().is_empty() => Some(k),
            _ => None,
        }
    }
    pub fn wire(&self) -> String {
        match self {
            NorlRelevance::Unresolved => "UNRESOLVED".into(),
            NorlRelevance::Feeds(k) => format!("FEEDS({k})"),
            NorlRelevance::NotRelevant(r) => format!("NOT_RELEVANT({r})"),
        }
    }
}

/// The organism declaration: empty outside the norl shard.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Organism {
    pub capabilities: Vec<OrganismCapability>,
    pub backends: Vec<Backend>,
    /// Capabilities proven in another repository being promoted into Norl.
    pub promotions: Vec<Promotion>,
}

/// A capability a source repository has proven, promoted into a Norl organism capability. Its
/// effective state is computed; the source's proof never raises Norl's growth.
#[derive(Clone, Debug, PartialEq)]
pub struct Promotion {
    pub key: String,
    /// The canonical shard that proves it.
    pub source_repository: String,
    /// The source node's semantic key (its NodeId is Chronica's).
    pub source_node: String,
    pub source_capability: Option<String>,
    pub source_technology: Option<String>,
    /// The source commit the promotion is taken from.
    pub source_commit: String,
    /// Proof locators in the source repository (`file::test`).
    pub source_evidence: Vec<String>,
    /// The organism capability key it becomes.
    pub destination_capability: String,
    /// The Norl physical node implementing it, once one exists.
    pub destination_node: Option<String>,
    /// FEEDS or TEACHES.
    pub relation: EdgeKind,
    pub implementation: PromotionImplementation,
    pub claimed: PromotionState,
    pub reason: String,
    pub transformation: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OrganismCapability {
    /// The capability key; its graph node is `capability/<key>`.
    pub key: String,
    /// The organ node key.
    pub organ: String,
    /// The claimed growth state; the effective state is computed.
    pub claimed: GrowthState,
    pub backend: Option<String>,
    /// Keys of the Material(Evaluation) nodes that judge it, in any shard.
    pub evaluations: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Backend {
    pub key: String,
    pub kind: BackendKind,
    /// The norl physical node implementing it.
    pub node: String,
    /// The donor key when the weight is borrowed.
    pub donor: Option<String>,
    /// The weight artifact identity; empty for none.
    pub weight: String,
}

impl Organism {
    pub fn is_empty(&self) -> bool {
        self.capabilities.is_empty() && self.backends.is_empty() && self.promotions.is_empty()
    }
    pub fn backend(&self, key: &str) -> Option<&Backend> {
        self.backends.iter().find(|b| b.key == key)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Proof {
    pub kind: ProofKind,
    /// `path/to/file.rs::test_fn`
    pub locator: String,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Migration {
    pub waves: Vec<Wave>,
    pub shims: Vec<Shim>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Wave {
    pub key: String,
    pub status: WaveStatus,
    pub nodes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Shim {
    pub key: String,
    pub kind: ShimKind,
    pub path: String,
    /// The node or donor the shim serves.
    pub serves: String,
    pub expires: (ExpiryKind, String),
}

impl Node {
    /// An active node with the default concept of its kind and no relations.
    pub fn new(key: &str, kind: NodeKind, path: &str, canonical_path: &str) -> Node {
        Node {
            key: key.into(),
            kind,
            concept: crate::graph::default_concept(kind),
            name: key.into(),
            path: path.into(),
            canonical_path: canonical_path.into(),
            lifecycle: NodeLifecycle::Active,
            provides: vec![],
            requires: vec![],
            reuses: vec![],
            inputs: vec![],
            outputs: vec![],
            lineage: vec![],
        }
    }
}

impl Declaration {
    pub fn node(&self, key: &str) -> Option<&Node> {
        self.repository.nodes.iter().find(|n| n.key == key)
    }
    pub fn donor(&self, key: &str) -> Option<&Donor> {
        self.donors.iter().find(|d| d.key == key)
    }
}

impl Proof {
    /// Splits `file::function`.
    pub fn target(&self) -> (&str, &str) {
        match self.locator.rsplit_once("::") {
            Some((f, t)) => (f, t),
            None => (self.locator.as_str(), ""),
        }
    }
}

/// A canonical native technology: HOW Chronica does something. Its identity is Chronica's; the
/// declaring shard is only its birthplace.
#[derive(Clone, Debug, PartialEq)]
pub struct Technology {
    pub key: String,
    pub name: String,
    pub kind: TechnologyKind,
    /// Claimed lifecycle; the effective lifecycle is computed.
    pub claimed: TechnologyLifecycle,
    pub purpose: String,
    /// Capability keys it implements.
    pub implements: Vec<String>,
    /// The node whose tree holds its canonical source.
    pub node: String,
    /// Canonical source files (repository paths); their digests are the technology's identity
    /// for materialization.
    pub sources: Vec<String>,
    pub invariants: Vec<String>,
    pub proofs: Vec<Proof>,
    /// Donor keys it learned from.
    pub lineage: Vec<String>,
    /// Typed relations to other technologies (EVOLVES, SPECIALIZES, ALTERNATIVE_FOR, …).
    pub relations: Vec<Relation>,
    /// Its relevance to the Norl organism (a technology that feeds no Norl capability must say
    /// why it is not relevant).
    pub norl: NorlRelevance,
    pub claims: Vec<Improvement>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Relation {
    pub kind: EdgeKind,
    pub target: String,
}

/// "Better" is a claim, and a claim without fresh passing evidence is UNPROVEN.
#[derive(Clone, Debug, PartialEq)]
pub struct Improvement {
    pub dimension: Dimension,
    pub baseline: String,
    pub workload: String,
    pub statement: String,
    pub evidence: Vec<Proof>,
}

impl Declaration {
    pub fn technology(&self, key: &str) -> Option<&Technology> {
        self.technologies.iter().find(|t| t.key == key)
    }
}
