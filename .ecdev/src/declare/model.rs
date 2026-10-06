//! The owned, run-time form of a repository's declarations.

use crate::schema::{
    Concept, Dimension, DonorState, Ecosystem, EdgeKind, ExceptionKind, ExpiryKind, NodeKind,
    NodeLifecycle, ProofKind, Scope, ShimKind, TechnologyKind, TechnologyLifecycle, WaveStatus,
};

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Declaration {
    pub repository: Repository,
    pub donors: Vec<Donor>,
    pub migration: Migration,
    pub technologies: Vec<Technology>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Repository {
    /// The repository id (`ecdev`): names the repository node, never part of a node identity.
    pub id: String,
    pub name: String,
    /// Canonical origin, e.g. `llgtrn/ECDEV`.
    pub origin: String,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    /// Stable semantic key (`machine.spatial`); with the system it is the node's
    /// identity. Never a path, never a repository.
    pub key: String,
    pub kind: NodeKind,
    /// What the node means in the ECDEV ontology.
    pub concept: Concept,
    pub name: String,
    /// Where the node physically lives now (empty for abstract kinds and planned nodes).
    pub path: String,
    /// Where the canonical shape says it lives.
    pub canonical_path: String,
    pub lifecycle: NodeLifecycle,
    /// Capability keys this node provides to ECDEV.
    pub provides: Vec<String>,
    /// Capability keys this node needs at run time, from whichever node provides them.
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
    /// A node key, or `donor:<key>`.
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
    /// `technology/<k>`); resolved against the repository graph.
    pub maps_to: Option<String>,
    /// Production adoption (question A): does ECDEV's production rely on this behaviour? A
    /// relied-on capability is `required` and must become native; a `NotAdopted` one is never
    /// `required`. Says nothing about whether the capability is worth studying.
    pub relevance: Relevance,
    /// Knowledge value (question B), independent of adoption: what ECDEV still has to learn
    /// from this capability. A runtime-rejected donor keeps every capability here.
    pub knowledge: Knowledge,
    pub proofs: Vec<Proof>,
}

/// Production adoption of a donor capability. `ReliedOn` states which ECDEV behaviour depends
/// on it; `NotAdopted` states why production does not execute it (citing its review). Not
/// adopting a capability is never a statement about its research value: that is `Knowledge`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Relevance {
    #[default]
    Unresolved,
    ReliedOn(String),
    NotAdopted(String),
}

impl Relevance {
    pub fn resolved(&self) -> bool {
        match self {
            Relevance::Unresolved => false,
            Relevance::ReliedOn(k) | Relevance::NotAdopted(k) => !k.trim().is_empty(),
        }
    }
    pub fn not_adopted(&self) -> bool {
        matches!(self, Relevance::NotAdopted(k) if !k.trim().is_empty())
    }
    pub fn wire(&self) -> String {
        match self {
            Relevance::Unresolved => "UNRESOLVED".into(),
            Relevance::ReliedOn(k) => format!("RELIED_ON({k})"),
            Relevance::NotAdopted(r) => format!("NOT_ADOPTED({r})"),
        }
    }
}

/// The only grounds on which a capability may be declared to have no research value: it lies
/// outside the present and reasonably foreseeable commerce-engine scope. "No current caller",
/// "a different native implementation", "a restrictive licence" or "not evidence" are not
/// among them: they constrain adoption, never knowledge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResearchScope {
    /// Documentation sites, dashboards and UI shells of the donor's own product.
    Frontend,
    /// Release, CI, packaging and build plumbing of the donor's repository.
    ReleaseTooling,
    /// Logos, screenshots, sample media.
    Assets,
    /// The donor vendor's own telemetry, analytics or tracing of its users.
    VendorTelemetry,
    /// Clients, servers or hosting glue for the donor's own hosted service or cloud.
    HostedServicePlumbing,
    /// Language bindings and SDK wrappers around something ECDEV already reaches natively.
    LanguageBindings,
    /// A problem domain unrelated to commerce intelligence (e.g. a chat benchmark).
    UnrelatedDomain,
}

impl ResearchScope {
    pub const ALL: &'static [ResearchScope] = &[
        ResearchScope::Frontend,
        ResearchScope::ReleaseTooling,
        ResearchScope::Assets,
        ResearchScope::VendorTelemetry,
        ResearchScope::HostedServicePlumbing,
        ResearchScope::LanguageBindings,
        ResearchScope::UnrelatedDomain,
    ];
    pub fn word(self) -> &'static str {
        match self {
            ResearchScope::Frontend => "FRONTEND",
            ResearchScope::ReleaseTooling => "RELEASE_TOOLING",
            ResearchScope::Assets => "ASSETS",
            ResearchScope::VendorTelemetry => "VENDOR_TELEMETRY",
            ResearchScope::HostedServicePlumbing => "HOSTED_SERVICE_PLUMBING",
            ResearchScope::LanguageBindings => "LANGUAGE_BINDINGS",
            ResearchScope::UnrelatedDomain => "UNRELATED_DOMAIN",
        }
    }
    pub fn variant(self) -> &'static str {
        match self {
            ResearchScope::Frontend => "Frontend",
            ResearchScope::ReleaseTooling => "ReleaseTooling",
            ResearchScope::Assets => "Assets",
            ResearchScope::VendorTelemetry => "VendorTelemetry",
            ResearchScope::HostedServicePlumbing => "HostedServicePlumbing",
            ResearchScope::LanguageBindings => "LanguageBindings",
            ResearchScope::UnrelatedDomain => "UnrelatedDomain",
        }
    }
    pub fn from_variant(v: &str) -> Option<ResearchScope> {
        ResearchScope::ALL
            .iter()
            .copied()
            .find(|s| s.variant() == v)
    }
}

/// What ECDEV still has to learn from a donor capability (question B). Open states keep a
/// research question; resolved states say how the knowledge was settled.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Knowledge {
    /// Not yet semantically reviewed: open.
    #[default]
    Unreviewed,
    /// Potentially useful to future ECDEV; the string is the open study question.
    StudyCandidate(String),
    /// An alternative approach worth empirical comparison with ECDEV's; the benchmark question.
    BenchmarkCandidate(String),
    /// A specific algorithm worth independent implementation and proof; what to prove.
    AlgorithmCandidate(String),
    /// Useful architecture or reference, deliberately not a native target.
    ReferenceOnly(String),
    /// Study complete: the behaviour is native, derived from this donor and proven.
    Absorbed(String),
    /// Study complete: ECDEV has an independent native equivalent.
    IndependentNative(String),
    /// Study complete: ECDEV deliberately differs (the donor's behaviour would violate
    /// ECDEV's evidence principles).
    Divergent(String),
    /// Outside present and reasonably foreseeable scope, on an admissible ground only.
    NoResearchValue(ResearchScope, String),
}

/// Phrases that justify not adopting a capability but never its research irrelevance.
pub const INADMISSIBLE_IRRELEVANCE: &[&str] = &[
    "no current caller",
    "no production caller",
    "no caller",
    "not currently",
    "does not currently",
    "doesn't currently",
    "currently uses",
    "already has",
    "already implements",
    "different implementation",
    "simpler implementation",
    "no runtime dependency",
    "not evidence",
    "observed evidence",
    "gpl",
    "agpl",
    "licen",
    "non-commercial",
    "noncommercial",
    "not used",
    "never called",
    "no native",
    "no embeddings",
    "no simulator",
];

impl Knowledge {
    /// Still has an open research question (unreviewed or a candidate).
    pub fn open(&self) -> bool {
        matches!(
            self,
            Knowledge::Unreviewed
                | Knowledge::StudyCandidate(_)
                | Knowledge::BenchmarkCandidate(_)
                | Knowledge::AlgorithmCandidate(_)
        )
    }
    pub fn text(&self) -> &str {
        match self {
            Knowledge::Unreviewed => "",
            Knowledge::StudyCandidate(t)
            | Knowledge::BenchmarkCandidate(t)
            | Knowledge::AlgorithmCandidate(t)
            | Knowledge::ReferenceOnly(t)
            | Knowledge::Absorbed(t)
            | Knowledge::IndependentNative(t)
            | Knowledge::Divergent(t)
            | Knowledge::NoResearchValue(_, t) => t,
        }
    }
    /// The status word reported by metrics.
    pub fn status(&self) -> &'static str {
        match self {
            Knowledge::Unreviewed => "UNREVIEWED",
            Knowledge::StudyCandidate(_) => "STUDY_CANDIDATE",
            Knowledge::BenchmarkCandidate(_) => "BENCHMARK_CANDIDATE",
            Knowledge::AlgorithmCandidate(_) => "ALGORITHM_CANDIDATE",
            Knowledge::ReferenceOnly(_) => "REFERENCE_ONLY",
            Knowledge::Absorbed(_) | Knowledge::IndependentNative(_) | Knowledge::Divergent(_) => {
                "STUDY_COMPLETE"
            }
            Knowledge::NoResearchValue(..) => "NO_RESEARCH_VALUE",
        }
    }
    /// How a completed study relates to ECDEV's implementation.
    pub fn relation(&self) -> Option<&'static str> {
        match self {
            Knowledge::Absorbed(_) => Some("DERIVED_NATIVE"),
            Knowledge::IndependentNative(_) => Some("INDEPENDENT_NATIVE"),
            Knowledge::Divergent(_) => Some("DELIBERATE_SEMANTIC_DIVERGENCE"),
            _ => None,
        }
    }
    pub fn wire(&self) -> String {
        match (self, self.relation()) {
            (Knowledge::Unreviewed, _) => "UNREVIEWED".into(),
            (Knowledge::NoResearchValue(s, t), _) => {
                format!("NO_RESEARCH_VALUE[{}]({t})", s.word())
            }
            (k, Some(r)) => format!("STUDY_COMPLETE[{r}]({})", k.text()),
            (k, None) => format!("{}({})", k.status(), k.text()),
        }
    }
    /// Why this knowledge decision is not admissible, if it is not.
    pub fn inadmissible(&self) -> Option<String> {
        if !matches!(self, Knowledge::Unreviewed) && self.text().trim().is_empty() {
            return Some("a knowledge decision must state its question or reason".into());
        }
        if let Knowledge::NoResearchValue(_, t) = self {
            let lower = t.to_lowercase();
            if let Some(p) = INADMISSIBLE_IRRELEVANCE.iter().find(|p| lower.contains(*p)) {
                return Some(format!(
                    "\"{p}\" constrains adoption, not research value; use NotAdopted for the runtime decision and a study, benchmark, algorithm or reference status for the knowledge"
                ));
            }
        }
        None
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

/// A canonical native technology: HOW ECDEV does something.
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
