//! The Ynventa system linker: repository capsules → `chronica.system.ynv`.
//!
//! Like a compiler's linker, but for architecture and technology. Shards are physical; the
//! linked graph is the one logical Chronica. Invalid architecture is a link error, not a note:
//! unresolved references, required capabilities nobody provides, two shards owning one node,
//! code dependencies across shards (only REQUIRES and REUSES may cross), duplicate capability
//! providers or technologies without a declared relation, cycles, protocol mismatch.

use crate::capsule::Capsule;
use crate::compact::codec::{DecodeError, Decoder, Encoder};
use crate::declare::{NorlRelevance, Organism};
use crate::formats::json::Json;
use crate::graph::{is_norl_node, Graph, NodeId, NORL_EDGE_KINDS, SYSTEM};
use crate::metrics::Counts;
use crate::schema::{
    Concept, DonorState, EdgeKind, NodeKind, Scope, TechnologyKind, TechnologyLifecycle,
    TechnologySharing,
};
use crate::technology::{classify_sharing, family_related, Shared, Sharing};
use crate::Severity;
use std::collections::{BTreeMap, BTreeSet};

pub const SYSTEM_TAG: u8 = 7;
pub const SYSTEM_FILE: &str = "chronica.system.ynv";

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct LinkIssue {
    pub severity: Severity,
    pub code: String,
    pub subject: String,
    pub detail: String,
}

impl LinkIssue {
    fn new(severity: Severity, code: &str, subject: &str, detail: impl Into<String>) -> LinkIssue {
        LinkIssue {
            severity,
            code: code.into(),
            subject: subject.into(),
            detail: detail.into(),
        }
    }
}

/// (fingerprint, signature, [(shard, semantic key)]): one operation or type shape found in
/// several shards.
pub type SymbolGroup = (String, String, Vec<(String, String)>);

/// Edge kinds that may cross shards: run-time capability needs, materialized reuse, the
/// developmental relations of the organism, technology family relations, and lineage. A
/// DEPENDS_ON may cross only as ARCHITECTURAL (otherwise CROSS_SHARD_CODE_DEPENDENCY).
pub const CROSS_SHARD_KINDS: &[EdgeKind] = &[
    EdgeKind::Requires,
    EdgeKind::Reuses,
    EdgeKind::Feeds,
    EdgeKind::Teaches,
    EdgeKind::EvaluatedBy,
    EdgeKind::AuthorizedBy,
    EdgeKind::Observes,
    EdgeKind::ActsOn,
    EdgeKind::Uses,
    EdgeKind::Specializes,
    EdgeKind::Generalizes,
    EdgeKind::AlternativeFor,
    EdgeKind::Evolves,
    EdgeKind::ForkedFrom,
    EdgeKind::Merges,
    EdgeKind::Supersedes,
    EdgeKind::Replaces,
    EdgeKind::LearnedFrom,
    EdgeKind::DerivesFrom,
];

/// One technology of the linked system.
#[derive(Clone, Debug, PartialEq)]
pub struct SystemTechnology {
    pub key: String,
    pub name: String,
    pub kind: TechnologyKind,
    pub purpose: String,
    /// The shard that declares it: its one canonical owner (and the birthplace of its source).
    pub birthplace: String,
    /// The node holding its canonical source.
    pub node: String,
    pub effective: TechnologyLifecycle,
    pub implements: Vec<String>,
    pub source_digest: String,
    /// Shards (other than the birthplace) that reuse it natively.
    pub consumers: Vec<String>,
    /// Donors it learned from (LEARNED_FROM), as `<donor key> (<effective state>)`.
    pub lineage: Vec<String>,
    /// Proof locators bound to it.
    pub proofs: Vec<String>,
    /// Norl capabilities and organs it FEEDS.
    pub feeds: Vec<String>,
    pub norl: NorlRelevance,
    /// Still dependent on a donor: its code is not evidenced native.
    pub donor_dependent: bool,
}

/// One donor of the linked system.
#[derive(Clone, Debug, PartialEq)]
pub struct SystemDonor {
    pub shard: String,
    pub key: String,
    pub id: String,
    pub claimed: DonorState,
    pub effective: DonorState,
}

/// One global donor of the linked system: one upstream, related to every repository that knows
/// it. It has no lifecycle of its own: each relation carries its repository's (a donor extinct
/// in one shard may be studied in another).
#[derive(Clone, Debug, PartialEq)]
pub struct GlobalDonor {
    pub key: String,
    /// Other identities of the same upstream: keys joined by declared aliases, legacy record
    /// ids, and graph identities (v1 keys) that differ from the global key's.
    pub aliases: Vec<String>,
    /// One relation per repository (more only when declared aliases join several of its keys).
    pub uses: Vec<crate::universe::RepositoryUsesDonor>,
}

/// Groups every shard's relations by global donor (after declared aliases), and the groups of
/// distinct upstreams that look alike and no alias joins.
pub fn global_donors(capsules: &[Capsule]) -> (Vec<GlobalDonor>, Vec<(String, Vec<String>)>) {
    use crate::universe::identity;
    let mut aliases = identity::Aliases::default();
    for c in capsules {
        for r in &c.universe {
            let Some(g) = &r.global_donor else { continue };
            for (o, prov) in &r.aliases {
                if let Some(k) = identity::global_donor_key(o).filter(|_| !prov.trim().is_empty()) {
                    aliases.join(g, &k);
                }
            }
        }
    }
    let mut by_key: BTreeMap<String, GlobalDonor> = BTreeMap::new();
    for c in capsules {
        for r in &c.universe {
            let Some(g) = &r.global_donor else { continue };
            let key = aliases.canonical(g);
            let gd = by_key.entry(key.clone()).or_insert_with(|| GlobalDonor {
                key: key.clone(),
                aliases: vec![],
                uses: vec![],
            });
            if *g != key {
                gd.aliases.push(g.clone());
            }
            gd.aliases.extend(r.aliases.iter().map(|(a, _)| a.clone()));
            gd.aliases.extend(r.records.iter().cloned());
            let own = identity::node_id(&key).to_string();
            gd.aliases
                .extend(r.node_ids.iter().filter(|n| **n != own).cloned());
            gd.uses.push(r.clone());
        }
    }
    let mut out: Vec<GlobalDonor> = by_key.into_values().collect();
    for g in out.iter_mut() {
        g.aliases.sort();
        g.aliases.dedup();
        g.uses
            .sort_by(|a, b| (&a.repository, &a.label).cmp(&(&b.repository, &b.label)));
    }
    let ambiguous = crate::universe::ambiguous_groups(out.iter().map(|g| {
        (
            g.key.as_str(),
            g.uses
                .iter()
                .any(|r| r.kind == crate::schema::UniverseKind::OssRepository),
        )
    }));
    (out, ambiguous)
}

/// One capability of the linked system.
#[derive(Clone, Debug, PartialEq)]
pub struct SystemCapability {
    pub key: String,
    /// (shard, node key)
    pub providers: Vec<(String, String)>,
    pub requirers: Vec<(String, String)>,
    /// Technology keys implementing it.
    pub technologies: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SystemImage {
    pub schema: String,
    /// (shard, origin, head, capsule digest)
    pub shards: Vec<(String, String, String, String)>,
    pub graph: Graph,
    pub technologies: Vec<SystemTechnology>,
    pub capabilities: Vec<SystemCapability>,
    pub issues: Vec<LinkIssue>,
    pub counts: Vec<(String, u64)>,
    /// Groups of same-shaped public operations found in several shards: (fingerprint,
    /// signature, [(shard, semantic key)]). Candidates for one canonical technology.
    pub duplicate_symbols: Vec<SymbolGroup>,
    /// How technologies implementing the same capability relate (computed).
    pub sharing: Vec<Sharing>,
    pub donors: Vec<SystemDonor>,
    /// Per linked shard: its structural violations (empty when its root is compatible).
    pub structure: Vec<(String, Vec<String>)>,
    /// The organism, as the norl shard declares it.
    pub organism: Organism,
    /// Verdicts of every evaluation material of the linked shards.
    pub evaluations: crate::capsule::EvaluationRows,
    /// Every global donor, with every repository's relation to it.
    pub global_donors: Vec<GlobalDonor>,
    /// (look-alike name, distinct global keys) across the ecosystem.
    pub ambiguous_donors: Vec<(String, Vec<String>)>,
    /// (shard, legacy registry, state) of every linked shard.
    pub universe_sources: Vec<(String, String, crate::universe::SourceState)>,
    /// (shard, proof locator, verdict) of every linked shard: the source evidence of promotions.
    pub proofs: Vec<(String, String, crate::evidence::Verdict)>,
}

impl SystemImage {
    pub fn pass(&self) -> bool {
        !self.issues.iter().any(|i| i.severity == Severity::Error)
    }
    pub fn counts(&self) -> Counts {
        Counts::from_raw(&self.counts)
    }

    /// The organism report of the linked system.
    pub fn organism_report(&self) -> crate::organism::Report {
        let ev: crate::organism::Evaluations = self.evaluations.iter().cloned().collect();
        let verdicts: crate::organism::ProofVerdicts = self
            .proofs
            .iter()
            .map(|(s, l, v)| ((s.clone(), l.clone()), *v))
            .collect();
        let linked: BTreeSet<String> = self.shards.iter().map(|x| x.0.clone()).collect();
        crate::organism::report(
            &self.graph,
            &self.organism,
            &ev,
            &BTreeMap::new(),
            &crate::organism::Sources {
                proofs: &verdicts,
                linked: Some(&linked),
            },
        )
    }
}

const COMMON_NAMES: &[&str] = &[
    "new", "default", "from", "into", "fmt", "main", "clone", "len", "is_empty", "get", "set",
    "run", "build", "parse", "open", "close", "read", "write", "with", "name", "value", "key",
    "id", "apply", "check", "render", "load", "store", "encode", "decode", "hash", "eq", "cmp",
];

/// Links capsules into the Chronica system image.
pub fn link(capsules: &[Capsule]) -> SystemImage {
    let mut issues = Vec::new();
    let schema = crate::protocol::schema_identity();
    let mut shards = Vec::new();
    let mut seen_shards = BTreeSet::new();
    for c in capsules {
        if c.protocol != crate::YNVENTA_PROTOCOL_VERSION {
            issues.push(LinkIssue::new(
                Severity::Error,
                "PROTOCOL_MISMATCH",
                &c.shard,
                format!(
                    "capsule protocol v{}; linker v{}",
                    c.protocol,
                    crate::YNVENTA_PROTOCOL_VERSION
                ),
            ));
        }
        if c.schema != schema {
            issues.push(LinkIssue::new(
                Severity::Error,
                "SCHEMA_FORK",
                &c.shard,
                format!(
                    "capsule schema {}; linker schema {schema}: the shard speaks another vocabulary (upgrade with `ynventa migrate scaffold`)",
                    c.schema
                ),
            ));
        }
        if c.system != SYSTEM {
            issues.push(LinkIssue::new(
                Severity::Error,
                "FOREIGN_SYSTEM",
                &c.shard,
                format!("belongs to `{}`, not `{SYSTEM}`", c.system),
            ));
        }
        if crate::protocol::shard(&c.shard).is_none() {
            issues.push(LinkIssue::new(
                Severity::Error,
                "UNKNOWN_SHARD",
                &c.shard,
                "not one of the canonical shards of Chronica",
            ));
        }
        if !seen_shards.insert(c.shard.clone()) {
            issues.push(LinkIssue::new(
                Severity::Error,
                "DUPLICATE_SHARD",
                &c.shard,
                "linked twice",
            ));
        }
        let digest = crate::digest::content_digest(&c.encode());
        shards.push((c.shard.clone(), c.origin.clone(), c.head.clone(), digest));
    }
    shards.sort();
    // Same schema, different subsystem bytes: a skewed copy (byte-identity is the rule).
    let mut by_subsystem: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for c in capsules.iter().filter(|c| c.schema == schema) {
        by_subsystem.entry(&c.subsystem).or_default().push(&c.shard);
    }
    if by_subsystem.len() > 1 {
        for (digest, members) in &by_subsystem {
            issues.push(LinkIssue::new(
                Severity::Warning,
                "SUBSYSTEM_SKEW",
                &members.join(","),
                format!(
                    "subsystem {digest} differs from the other shards' ({} distinct copies under one schema); re-install the canonical subsystem",
                    by_subsystem.len()
                ),
            ));
        }
    }
    let missing: Vec<&str> = crate::protocol::SHARDS
        .iter()
        .map(|s| s.id)
        .filter(|s| *s != "ynventa" && !seen_shards.contains(*s))
        .collect();
    if !missing.is_empty() {
        issues.push(LinkIssue::new(
            Severity::Warning,
            "INCOMPLETE_SYSTEM",
            "chronica",
            format!("shards not linked: {}", missing.join(", ")),
        ));
    }

    // Ownership: a Chronica node lives in exactly one shard.
    let mut owner: BTreeMap<NodeId, String> = BTreeMap::new();
    for c in capsules {
        for n in c.graph.nodes.values() {
            let owned = n.namespace == SYSTEM
                && n.repository == c.shard
                && (crate::schema::is_physical(n.kind) || n.kind == NodeKind::Technology);
            if !owned || n.kind == NodeKind::Repository {
                continue;
            }
            if let Some(prev) = owner.insert(n.id, c.shard.clone()) {
                if prev != c.shard {
                    issues.push(LinkIssue::new(Severity::Error, "OWNERSHIP_COLLISION", &n.semantic_key, format!("owned by both `{prev}` and `{}`; one semantic identity has one physical owner", c.shard)));
                }
            }
        }
    }

    let mut graph = Graph::default();
    for c in capsules {
        for i in graph.merge(&c.graph) {
            issues.push(LinkIssue::new(
                Severity::Error,
                i.code,
                &i.subject,
                i.detail,
            ));
        }
    }
    let label = |g: &Graph, id: &NodeId| {
        g.nodes
            .get(id)
            .map(|n| n.semantic_key.clone())
            .unwrap_or_else(|| id.to_string())
    };
    let shard_of = |g: &Graph, id: &NodeId| {
        g.nodes
            .get(id)
            .map(|n| n.repository.clone())
            .unwrap_or_default()
    };

    // References.
    let unresolved: Vec<&crate::graph::GEdge> = graph
        .edges
        .iter()
        .filter(|e| !graph.nodes.contains_key(&e.from) || !graph.nodes.contains_key(&e.to))
        .collect();
    for e in &unresolved {
        let (known, missing) = if graph.nodes.contains_key(&e.from) {
            (e.from, e.to)
        } else {
            (e.to, e.from)
        };
        let norl = NORL_EDGE_KINDS.contains(&e.kind);
        issues.push(LinkIssue::new(
            Severity::Error,
            if norl {
                "UNRESOLVED_NORL_NODE"
            } else {
                "UNRESOLVED_REFERENCE"
            },
            &label(&graph, &known),
            format!(
                "{} {} references {missing}, which no linked shard declares",
                shard_of(&graph, &known),
                e.kind
            ),
        ));
    }
    for i in graph.validate(&BTreeSet::new()) {
        if i.code != "DANGLING_EDGE" {
            issues.push(LinkIssue::new(
                Severity::Error,
                i.code,
                &i.subject,
                i.detail,
            ));
        }
    }

    // Only REQUIRES (run-time capability) and REUSES (materialized technology) may cross shards.
    for e in &graph.edges {
        if e.kind == EdgeKind::DependsOn && !matches!(e.scope, Scope::Architectural) {
            let (a, b) = (shard_of(&graph, &e.from), shard_of(&graph, &e.to));
            let both_chronica = graph
                .nodes
                .get(&e.from)
                .is_some_and(|n| n.namespace == SYSTEM)
                && graph
                    .nodes
                    .get(&e.to)
                    .is_some_and(|n| n.namespace == SYSTEM);
            if both_chronica && !a.is_empty() && !b.is_empty() && a != b {
                issues.push(LinkIssue::new(
                    Severity::Error,
                    "CROSS_SHARD_CODE_DEPENDENCY",
                    &label(&graph, &e.from),
                    format!("{a} depends ({}) on {} in {b}; shards couple only through REQUIRES or REUSES", e.scope, label(&graph, &e.to)),
                ));
            }
        }
    }

    // Every other edge that crosses shards must be of a kind that may.
    let canonical_shard = |s: &str| crate::protocol::shard(s).is_some();
    for e in &graph.edges {
        let (Some(f), Some(t)) = (graph.nodes.get(&e.from), graph.nodes.get(&e.to)) else {
            continue;
        };
        let crosses = f.namespace == SYSTEM
            && t.namespace == SYSTEM
            && canonical_shard(&f.repository)
            && canonical_shard(&t.repository)
            && f.repository != t.repository;
        if !crosses || e.kind == EdgeKind::DependsOn {
            continue;
        }
        if !CROSS_SHARD_KINDS.contains(&e.kind) {
            issues.push(LinkIssue::new(
                Severity::Error,
                "INVALID_CROSS_REPO_EDGE",
                &f.semantic_key,
                format!(
                    "{} {} {} in {}; shards relate only through {} (DEPENDS_ON only ARCHITECTURAL)",
                    f.repository,
                    e.kind,
                    t.semantic_key,
                    t.repository,
                    CROSS_SHARD_KINDS
                        .iter()
                        .map(|k| k.wire())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ));
        }
    }

    // The organism: every feed lands in Norl, every feeding material is produced in its shard,
    // every evaluation that feeds or judges is bound to proofs.
    for e in &graph.edges {
        let (Some(f), Some(t)) = (graph.nodes.get(&e.from), graph.nodes.get(&e.to)) else {
            continue;
        };
        if matches!(e.kind, EdgeKind::Feeds | EdgeKind::Teaches) {
            if !is_norl_node(&graph, e.to) {
                issues.push(LinkIssue::new(
                    Severity::Error,
                    "UNRESOLVED_NORL_NODE",
                    &f.semantic_key,
                    format!(
                        "{} {} `{}`, which is not an organ or organism capability of norl",
                        f.repository, e.kind, t.semantic_key
                    ),
                ));
            }
            if f.kind == NodeKind::Material {
                let generated = graph.edges.iter().any(|g| {
                    g.to == f.id
                        && g.kind == EdgeKind::Generates
                        && graph.nodes.get(&g.from).is_some_and(|s| {
                            s.repository == f.repository || s.kind == NodeKind::Capability
                        })
                });
                if !generated {
                    issues.push(LinkIssue::new(
                        Severity::Error,
                        "INVALID_NORL_FEED",
                        &f.semantic_key,
                        format!(
                            "Material({}) of {} {} `{}` but nothing in {} GENERATES it: the feed chain does not resolve",
                            f.concept, f.repository, e.kind, t.semantic_key, f.repository
                        ),
                    ));
                }
            }
        }
        let judged = (e.kind == EdgeKind::EvaluatedBy && t.kind == NodeKind::Material)
            .then_some(t)
            .or((e.kind == EdgeKind::Feeds && f.kind == NodeKind::Material).then_some(f));
        if let Some(m) = judged.filter(|m| m.concept == Concept::Evaluation) {
            if m.evidence.is_empty() {
                issues.push(LinkIssue::new(
                    Severity::Error,
                    "INVALID_NORL_FEED",
                    &m.semantic_key,
                    format!(
                        "evaluation of {} {} with no proof binding",
                        m.repository,
                        if e.kind == EdgeKind::Feeds {
                            "feeds Norl"
                        } else {
                            "judges Norl"
                        }
                    ),
                ));
            }
        }
    }

    // Donor capabilities map to a capability or technology of the canonical graph.
    let resolves = |k: &str| {
        [
            k.to_string(),
            crate::graph::system_capability_key(k),
            crate::graph::technology_key(k),
        ]
        .iter()
        .any(|x| graph.node_by_key(SYSTEM, x).is_some())
    };
    for c in capsules {
        for d in &c.donors {
            for (cap, k) in &d.maps {
                if !k.trim().is_empty() && !resolves(k) {
                    issues.push(LinkIssue::new(
                        Severity::Error,
                        "UNRESOLVED_REFERENCE",
                        &d.key,
                        format!(
                            "{} donor capability `{cap}` maps_to `{k}`, which no linked shard declares",
                            c.shard
                        ),
                    ));
                }
            }
        }
    }

    // Capabilities.
    // Related: connected through any chain of declared lineage/alternative relations, so every
    // alternative of one canonical technology belongs to one declared family.
    let related = family_related;
    let mut caps: BTreeMap<String, SystemCapability> = BTreeMap::new();
    for n in graph
        .nodes
        .values()
        .filter(|n| n.kind == NodeKind::Capability && n.namespace == SYSTEM)
    {
        let Some(key) = n.semantic_key.strip_prefix("capability/") else {
            continue;
        };
        let mut sc = SystemCapability {
            key: key.to_string(),
            providers: vec![],
            requirers: vec![],
            technologies: vec![],
        };
        for e in graph.backlinks(n.id) {
            let Some(src) = graph.nodes.get(&e.from) else {
                continue;
            };
            match (e.kind, src.kind) {
                (EdgeKind::Provides, _) => sc
                    .providers
                    .push((src.repository.clone(), src.semantic_key.clone())),
                (EdgeKind::Requires, _) => sc
                    .requirers
                    .push((src.repository.clone(), src.semantic_key.clone())),
                (EdgeKind::Implements, NodeKind::Technology) => sc.technologies.push(
                    src.semantic_key
                        .trim_start_matches("technology/")
                        .to_string(),
                ),
                _ => {}
            }
        }
        sc.providers.sort();
        sc.requirers.sort();
        sc.technologies.sort();
        if !sc.requirers.is_empty() && sc.providers.is_empty() {
            issues.push(LinkIssue::new(
                Severity::Error,
                "UNPROVIDED_CAPABILITY",
                key,
                format!(
                    "required by {} but provided by no shard",
                    sc.requirers
                        .iter()
                        .map(|(s, n)| format!("{n} ({s})"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ));
        }
        let provider_shards: BTreeSet<&String> = sc.providers.iter().map(|(s, _)| s).collect();
        if provider_shards.len() > 1 {
            let ids: Vec<NodeId> = sc
                .providers
                .iter()
                .map(|(_, k)| NodeId::of(SYSTEM, k))
                .collect();
            let unrelated = ids
                .iter()
                .enumerate()
                .any(|(i, a)| ids.iter().skip(i + 1).any(|b| !related(&graph, *a, *b)));
            if unrelated {
                issues.push(LinkIssue::new(
                    Severity::Error,
                    "DUPLICATE_CAPABILITY",
                    key,
                    format!("provided independently by {}; declare SPECIALIZES / ALTERNATIVE_FOR or converge on one provider", sc.providers.iter().map(|(s, n)| format!("{n} ({s})")).collect::<Vec<_>>().join(", ")),
                ));
            }
        }
        caps.insert(key.to_string(), sc);
    }

    // Technologies: owner, adoption, duplication, lineage, proofs, feeds.
    let mut techs: Vec<SystemTechnology> = Vec::new();
    for c in capsules {
        for t in &c.technologies {
            let tid = NodeId::of(SYSTEM, &crate::graph::technology_key(&t.key));
            let lineage = t
                .lineage
                .iter()
                .map(|k| match c.donors.iter().find(|d| &d.key == k) {
                    Some(d) => format!("{k} ({})", d.effective),
                    None => k.clone(),
                })
                .collect();
            let mut feeds: Vec<String> = graph
                .edges
                .iter()
                .filter(|e| e.from == tid && e.kind == EdgeKind::Feeds)
                .map(|e| label(&graph, &e.to))
                .collect();
            feeds.sort();
            feeds.dedup();
            let mut consumers: Vec<String> = graph
                .backlinks(tid)
                .iter()
                .filter(|e| e.kind == EdgeKind::Reuses)
                .map(|e| shard_of(&graph, &e.from))
                .filter(|s| !s.is_empty() && *s != c.shard)
                .collect();
            consumers.sort();
            consumers.dedup();
            let mut effective = t.effective;
            if !consumers.is_empty() && effective == TechnologyLifecycle::Canonical {
                effective = TechnologyLifecycle::Adopted;
            }
            techs.push(SystemTechnology {
                key: t.key.clone(),
                name: t.name.clone(),
                kind: t.kind,
                purpose: t.purpose.clone(),
                birthplace: c.shard.clone(),
                node: t.node.clone(),
                effective,
                implements: t.implements.clone(),
                source_digest: t.source_digest.clone(),
                consumers,
                lineage,
                proofs: graph
                    .nodes
                    .get(&tid)
                    .map(|n| n.evidence.clone())
                    .unwrap_or_default(),
                feeds,
                norl: t.norl.clone(),
                donor_dependent: t.effective < TechnologyLifecycle::Native,
            });
        }
    }
    techs.sort_by(|a, b| a.key.cmp(&b.key));
    for e in graph.edges.iter().filter(|e| e.kind == EdgeKind::Reuses) {
        let Some(t) = graph.nodes.get(&e.to) else {
            continue;
        };
        let key = t.semantic_key.trim_start_matches("technology/");
        if let Some(st) = techs.iter().find(|x| x.key == key) {
            if st.effective < TechnologyLifecycle::Canonical {
                issues.push(LinkIssue::new(
                    Severity::Warning,
                    "REUSE_OF_NON_CANONICAL",
                    key,
                    format!(
                        "{} reuses it while it is {}",
                        label(&graph, &e.from),
                        st.effective
                    ),
                ));
            }
        }
    }
    for (i, a) in techs.iter().enumerate() {
        for b in techs.iter().skip(i + 1) {
            if a.birthplace != b.birthplace && a.implements.iter().any(|x| b.implements.contains(x))
            {
                let ida = NodeId::of(SYSTEM, &crate::graph::technology_key(&a.key));
                let idb = NodeId::of(SYSTEM, &crate::graph::technology_key(&b.key));
                if !related(&graph, ida, idb) {
                    issues.push(LinkIssue::new(
                        Severity::Error,
                        "DUPLICATE_TECHNOLOGY",
                        &a.key,
                        format!("`{}` ({}) and `{}` ({}) implement the same capability with no declared relation", a.key, a.birthplace, b.key, b.birthplace),
                    ));
                }
            }
        }
    }

    let sharing = classify_sharing(
        &graph,
        &techs
            .iter()
            .map(|t| Shared {
                key: &t.key,
                birthplace: &t.birthplace,
                implements: &t.implements,
                source_digest: &t.source_digest,
            })
            .collect::<Vec<_>>(),
    );

    // Orphans: capabilities and technologies nothing places in the technology or material graph.
    for (k, providers) in crate::organism::orphan_capabilities(&graph, None) {
        issues.push(LinkIssue::new(
            Severity::Warning,
            "ORPHAN_CAPABILITY",
            &k,
            format!(
                "provided by {} with no technology implementing it and no material or feed relation",
                providers.join(", ")
            ),
        ));
    }
    for t in techs
        .iter()
        .filter(|t| t.effective != TechnologyLifecycle::Superseded)
    {
        let not_relevant = matches!(&t.norl, NorlRelevance::NotRelevant(r) if !r.trim().is_empty());
        if t.implements.is_empty() || (t.feeds.is_empty() && !not_relevant) {
            issues.push(LinkIssue::new(
                Severity::Warning,
                "ORPHAN_TECHNOLOGY",
                &t.key,
                if t.implements.is_empty() {
                    "implements no capability".to_string()
                } else {
                    format!(
                        "{}: feeds no Norl capability and is not declared irrelevant to Norl",
                        t.birthplace
                    )
                },
            ));
        }
    }

    // Structural conformance of every linked shard's root.
    let mut structure: Vec<(String, Vec<String>)> = capsules
        .iter()
        .map(|c| (c.shard.clone(), c.structure.clone()))
        .collect();
    structure.sort();
    for (shard, v) in structure.iter().filter(|(_, v)| !v.is_empty()) {
        issues.push(LinkIssue::new(
            Severity::Warning,
            "STRUCTURE_NONCONFORMANT",
            shard,
            format!(
                "{} structural violations: {}",
                v.len(),
                v.iter().take(3).cloned().collect::<Vec<_>>().join("; ")
            ),
        ));
    }
    let mut donors: Vec<SystemDonor> = capsules
        .iter()
        .flat_map(|c| {
            c.donors.iter().map(|d| SystemDonor {
                shard: c.shard.clone(),
                key: d.key.clone(),
                id: d.id.clone(),
                claimed: d.claimed,
                effective: d.effective,
            })
        })
        .collect();
    donors.sort_by(|a, b| (&a.shard, &a.key).cmp(&(&b.shard, &b.key)));
    let mut organism = Organism::default();
    for c in capsules {
        organism
            .capabilities
            .extend(c.organism.capabilities.iter().cloned());
        organism
            .backends
            .extend(c.organism.backends.iter().cloned());
        organism
            .promotions
            .extend(c.organism.promotions.iter().cloned());
    }
    let mut proofs: Vec<(String, String, crate::evidence::Verdict)> = capsules
        .iter()
        .flat_map(|c| {
            c.proofs
                .iter()
                .map(|(l, v)| (c.shard.clone(), l.clone(), *v))
        })
        .collect();
    proofs.sort();

    // Growth of the organism, judged on the whole system: claims above it are errors here.
    let mut evaluations: crate::capsule::EvaluationRows = capsules
        .iter()
        .flat_map(|c| c.evaluations.iter().cloned())
        .collect();
    evaluations.sort();
    let ev: crate::organism::Evaluations = evaluations.iter().cloned().collect();
    let verdicts: crate::organism::ProofVerdicts = proofs
        .iter()
        .map(|(s, l, v)| ((s.clone(), l.clone()), *v))
        .collect();
    let linked: BTreeSet<String> = capsules.iter().map(|c| c.shard.clone()).collect();
    let organism_report = crate::organism::report(
        &graph,
        &organism,
        &ev,
        &BTreeMap::new(),
        &crate::organism::Sources {
            proofs: &verdicts,
            linked: Some(&linked),
        },
    );
    for f in crate::organism::claim_findings(&organism_report.growth, false)
        .into_iter()
        .chain(crate::organism::promotion_findings(
            &organism_report.promotions,
        ))
    {
        issues.push(LinkIssue::new(f.severity, &f.code, &f.subject, f.detail));
    }

    // YIR: same-shaped public operations in several shards are technology candidates.
    let mut groups: BTreeMap<String, (String, BTreeSet<(String, String)>)> = BTreeMap::new();
    for c in capsules {
        for s in &c.symbols {
            let meaningful = match s.kind {
                crate::ir::SymbolKind::Operation => {
                    !s.inputs.is_empty()
                        && s.name.len() >= 5
                        && !COMMON_NAMES.contains(&s.name.as_str())
                }
                crate::ir::SymbolKind::Type | crate::ir::SymbolKind::Interface => s.name.len() >= 6,
                _ => false,
            };
            if meaningful {
                let g = groups
                    .entry(s.fingerprint())
                    .or_insert_with(|| (s.signature(), BTreeSet::new()));
                g.1.insert((c.shard.clone(), s.semantic_key()));
            }
        }
    }
    let mut duplicate_symbols: Vec<SymbolGroup> = groups
        .into_iter()
        .filter(|(_, (_, m))| m.iter().map(|(s, _)| s).collect::<BTreeSet<_>>().len() > 1)
        .map(|(f, (sig, m))| (f, sig, m.into_iter().collect()))
        .collect();
    duplicate_symbols.sort_by(|a, b| b.2.len().cmp(&a.2.len()).then(a.0.cmp(&b.0)));

    // The donor universe: one global donor per upstream, every repository's relation to it.
    let (global_donors, ambiguous_donors) = global_donors(capsules);
    for (name, keys) in &ambiguous_donors {
        let repos: BTreeSet<&str> = global_donors
            .iter()
            .filter(|g| keys.contains(&g.key))
            .flat_map(|g| g.uses.iter().map(|r| r.repository.as_str()))
            .collect();
        issues.push(LinkIssue::new(
            Severity::Warning,
            "AMBIGUOUS_DONOR_IDENTITY",
            name,
            format!(
                "{} distinct upstreams look alike ({}; known to {}); never merged without an alias with provenance",
                keys.len(),
                keys.join(", "),
                repos.into_iter().collect::<Vec<_>>().join(", ")
            ),
        ));
    }
    let mut universe_sources: Vec<(String, String, crate::universe::SourceState)> = capsules
        .iter()
        .flat_map(|c| {
            c.universe_sources
                .iter()
                .map(|(p, st)| (c.shard.clone(), p.clone(), *st))
        })
        .collect();
    universe_sources.sort();

    let mut total = Counts::default();
    for c in capsules {
        total.add(&c.counts());
    }
    // Unique global donors are deduplicated across repositories, never summed.
    total.unique_global_donors = global_donors.len() as u64;
    total.ambiguous_donor_identities = ambiguous_donors.len() as u64;
    total.technologies_adopted = techs
        .iter()
        .filter(|t| t.effective == TechnologyLifecycle::Adopted)
        .count() as u64;
    // System-wide values replace the shards' local views of the organism, the orphans and the
    // sharing pairs (which only the linked graph decides).
    organism_report.count(&mut total);
    let warned = |code: &str| issues.iter().filter(|i| i.code == code).count() as u64;
    total.orphan_capabilities = warned("ORPHAN_CAPABILITY");
    total.orphan_technologies = warned("ORPHAN_TECHNOLOGY");
    crate::organism::count_sharing(&sharing, &mut total);
    issues.sort();
    issues.dedup();
    SystemImage {
        schema,
        shards,
        graph,
        technologies: techs,
        capabilities: caps.into_values().collect(),
        issues,
        counts: total
            .raw()
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
        duplicate_symbols,
        sharing,
        donors,
        structure,
        organism,
        evaluations,
        global_donors,
        ambiguous_donors,
        universe_sources,
        proofs,
    }
}

impl SystemImage {
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new(SYSTEM_TAG);
        e.str(&self.schema);
        e.u64(self.shards.len() as u64);
        for (a, b, c, d) in &self.shards {
            e.str(a).str(b).str(c).str(d);
        }
        e.bytes(&self.graph.encode());
        e.u64(self.technologies.len() as u64);
        for t in &self.technologies {
            e.str(&t.key)
                .str(&t.name)
                .u8(t.kind.rank())
                .str(&t.purpose)
                .str(&t.birthplace)
                .str(&t.node)
                .u8(t.effective.rank())
                .strs(&t.implements)
                .str(&t.source_digest)
                .strs(&t.consumers)
                .strs(&t.lineage)
                .strs(&t.proofs)
                .strs(&t.feeds)
                .bool(t.donor_dependent);
            crate::capsule::encode_norl(&mut e, &t.norl);
        }
        e.u64(self.capabilities.len() as u64);
        let pairs = |e: &mut Encoder, v: &[(String, String)]| {
            e.u64(v.len() as u64);
            for (a, b) in v {
                e.str(a).str(b);
            }
        };
        for c in &self.capabilities {
            e.str(&c.key);
            pairs(&mut e, &c.providers);
            pairs(&mut e, &c.requirers);
            e.strs(&c.technologies);
        }
        e.u64(self.issues.len() as u64);
        for i in &self.issues {
            e.u8(i.severity as u8)
                .str(&i.code)
                .str(&i.subject)
                .str(&i.detail);
        }
        e.u64(self.counts.len() as u64);
        for (k, v) in &self.counts {
            e.str(k).u64(*v);
        }
        e.u64(self.duplicate_symbols.len() as u64);
        for (f, sig, m) in &self.duplicate_symbols {
            e.str(f).str(sig);
            pairs(&mut e, m);
        }
        e.u64(self.sharing.len() as u64);
        for x in &self.sharing {
            e.str(x.class.wire())
                .str(&x.a)
                .str(&x.b)
                .strs(&x.capabilities);
        }
        e.u64(self.donors.len() as u64);
        for x in &self.donors {
            e.str(&x.shard)
                .str(&x.key)
                .str(&x.id)
                .str(x.claimed.wire())
                .str(x.effective.wire());
        }
        e.u64(self.structure.len() as u64);
        for (shard, v) in &self.structure {
            e.str(shard).strs(v);
        }
        crate::capsule::encode_organism(&mut e, &self.organism);
        crate::capsule::encode_evaluations(&mut e, &self.evaluations);
        e.u64(self.global_donors.len() as u64);
        for g in &self.global_donors {
            e.str(&g.key).strs(&g.aliases);
            crate::capsule::encode_relations(&mut e, &g.uses);
        }
        e.u64(self.ambiguous_donors.len() as u64);
        for (n, ks) in &self.ambiguous_donors {
            e.str(n).strs(ks);
        }
        e.u64(self.universe_sources.len() as u64);
        for (sh, p, st) in &self.universe_sources {
            e.str(sh).str(p).str(st.wire());
        }
        e.u64(self.proofs.len() as u64);
        for (sh, l, v) in &self.proofs {
            e.str(sh).str(l).str(v.wire());
        }
        e.finish()
    }

    pub fn decode(b: &[u8]) -> Result<SystemImage, DecodeError> {
        let mut d = Decoder::open(b, SYSTEM_TAG)?;
        let schema = d.str()?;
        let mut shards = Vec::new();
        for _ in 0..d.u64()? {
            shards.push((d.str()?, d.str()?, d.str()?, d.str()?));
        }
        let graph = Graph::decode(d.bytes()?)?;
        let mut technologies = Vec::new();
        for _ in 0..d.u64()? {
            technologies.push(SystemTechnology {
                key: d.str()?,
                name: d.str()?,
                kind: d.word(TechnologyKind::ALL)?,
                purpose: d.str()?,
                birthplace: d.str()?,
                node: d.str()?,
                effective: d.word(TechnologyLifecycle::ALL)?,
                implements: d.strs()?,
                source_digest: d.str()?,
                consumers: d.strs()?,
                lineage: d.strs()?,
                proofs: d.strs()?,
                feeds: d.strs()?,
                donor_dependent: d.bool()?,
                norl: crate::capsule::decode_norl(&mut d)?,
            });
        }
        fn pairs(d: &mut Decoder) -> Result<Vec<(String, String)>, DecodeError> {
            let n = d.u64()?;
            (0..n).map(|_| Ok((d.str()?, d.str()?))).collect()
        }
        let mut capabilities = Vec::new();
        for _ in 0..d.u64()? {
            capabilities.push(SystemCapability {
                key: d.str()?,
                providers: pairs(&mut d)?,
                requirers: pairs(&mut d)?,
                technologies: d.strs()?,
            });
        }
        let mut issues = Vec::new();
        for _ in 0..d.u64()? {
            let sev = match d.u8()? {
                0 => Severity::Error,
                1 => Severity::Warning,
                _ => Severity::Info,
            };
            issues.push(LinkIssue {
                severity: sev,
                code: d.str()?,
                subject: d.str()?,
                detail: d.str()?,
            });
        }
        let mut counts = Vec::new();
        for _ in 0..d.u64()? {
            counts.push((d.str()?, d.u64()?));
        }
        let mut duplicate_symbols = Vec::new();
        for _ in 0..d.u64()? {
            duplicate_symbols.push((d.str()?, d.str()?, pairs(&mut d)?));
        }
        let word = |d: &mut Decoder| -> Result<String, DecodeError> { d.str() };
        let mut sharing = Vec::new();
        for _ in 0..d.u64()? {
            let w = word(&mut d)?;
            sharing.push(Sharing {
                class: TechnologySharing::from_wire(&w)
                    .ok_or_else(|| DecodeError(format!("`{w}` is not a sharing class")))?,
                a: d.str()?,
                b: d.str()?,
                capabilities: d.strs()?,
            });
        }
        let state = |d: &mut Decoder| -> Result<DonorState, DecodeError> {
            let w = d.str()?;
            DonorState::from_wire(&w)
                .ok_or_else(|| DecodeError(format!("`{w}` is not a donor state")))
        };
        let mut donors = Vec::new();
        for _ in 0..d.u64()? {
            donors.push(SystemDonor {
                shard: d.str()?,
                key: d.str()?,
                id: d.str()?,
                claimed: state(&mut d)?,
                effective: state(&mut d)?,
            });
        }
        let mut structure = Vec::new();
        for _ in 0..d.u64()? {
            structure.push((d.str()?, d.strs()?));
        }
        let organism = crate::capsule::decode_organism(&mut d)?;
        let evaluations = crate::capsule::decode_evaluations(&mut d)?;
        let mut global_donors = Vec::new();
        for _ in 0..d.u64()? {
            global_donors.push(GlobalDonor {
                key: d.str()?,
                aliases: d.strs()?,
                uses: crate::capsule::decode_relations(&mut d)?,
            });
        }
        let mut ambiguous_donors = Vec::new();
        for _ in 0..d.u64()? {
            ambiguous_donors.push((d.str()?, d.strs()?));
        }
        let mut universe_sources = Vec::new();
        for _ in 0..d.u64()? {
            let (sh, p, w) = (d.str()?, d.str()?, d.str()?);
            universe_sources.push((
                sh,
                p,
                crate::universe::SourceState::from_wire(&w)
                    .ok_or_else(|| DecodeError(format!("`{w}` is not a source state")))?,
            ));
        }
        let mut proofs = Vec::new();
        for _ in 0..d.u64()? {
            let (sh, l, w) = (d.str()?, d.str()?, d.str()?);
            let v = crate::evidence::Verdict::ALL
                .iter()
                .copied()
                .find(|v| v.wire() == w)
                .ok_or_else(|| DecodeError(format!("`{w}` is not a verdict")))?;
            proofs.push((sh, l, v));
        }
        d.end()?;
        Ok(SystemImage {
            schema,
            shards,
            graph,
            technologies,
            capabilities,
            issues,
            counts,
            duplicate_symbols,
            sharing,
            donors,
            structure,
            organism,
            evaluations,
            global_donors,
            ambiguous_donors,
            universe_sources,
            proofs,
        })
    }

    /// The ecosystem report.
    pub fn render_text(&self) -> String {
        let c = self.counts();
        let v = c.values();
        let get = |k: &str| {
            v.iter()
                .find(|(n, _)| n == k)
                .map(|(_, x)| x.clone())
                .unwrap_or_default()
        };
        let errors = self
            .issues
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .count();
        let mut s = format!(
            "Chronica global system: linked shards {}/{} ({})\nsystem link: {}\nglobal nodes: {}\nglobal edges: {}\nglobal canonical technologies: {}\ncross-shard technology consumers: {}\nduplicate capabilities: {}\nduplicate operation/type candidates: {}\nunresolved graph references: {}\ntotal donors: {}\nextinct donors: {}\nglobal extinction ratio: {}\nexternal technology edges: {}\nlink errors: {errors}, warnings: {}\n",
            self.shards.len(),
            crate::protocol::SHARDS.len(),
            self.shards.iter().map(|x| x.0.as_str()).collect::<Vec<_>>().join(" "),
            if self.pass() { "PASS" } else { "FAIL" },
            self.graph.nodes.len(),
            self.graph.edges.len(),
            self.technologies.iter().filter(|t| t.effective >= TechnologyLifecycle::Canonical && t.effective != TechnologyLifecycle::Superseded).count(),
            self.technologies.iter().map(|t| t.consumers.len()).sum::<usize>(),
            self.issues.iter().filter(|i| i.code == "DUPLICATE_CAPABILITY").count(),
            self.duplicate_symbols.len(),
            self.issues.iter().filter(|i| i.code == "UNRESOLVED_REFERENCE").count(),
            get("donors_registered"),
            get("donors_extinct"),
            get("extinction_ratio"),
            get("external_technology_edges"),
            self.issues.iter().filter(|i| i.severity == Severity::Warning).count(),
        );
        let org = self.organism_report();
        let at = |g: crate::schema::GrowthState| {
            org.growth
                .iter()
                .filter(|x| x.defined && x.effective == g)
                .count()
        };
        let src = |m: crate::schema::SourceMaturity| {
            org.growth.iter().filter(|x| x.source_maturity == m).count()
        };
        s.push_str(&format!(
            "organism: {} capabilities; norl maturity {}; source maturity {} (beside, never part of, Norl's growth); promotions {}\n",
            org.growth.len(),
            crate::schema::GrowthState::ALL
                .iter()
                .map(|g| format!("{} {}", g.wire(), at(*g)))
                .collect::<Vec<_>>()
                .join(", "),
            crate::schema::SourceMaturity::ALL
                .iter()
                .map(|m| format!("{} {}", m.wire(), src(*m)))
                .collect::<Vec<_>>()
                .join(", "),
            org.promotions.len()
        ));
        s.push_str(&format!(
            "donor universe: {} raw cross-repo references, {} unique global donors (deduplicated), {} ambiguous look-alike groups; legacy registries not reconciled: {}\n",
            get("repository_donor_references"),
            self.global_donors.len(),
            self.ambiguous_donors.len(),
            self.universe_sources
                .iter()
                .filter(|x| x.2 != crate::universe::SourceState::Reconciled)
                .count()
        ));
        s.push_str("technology sharing (pairs implementing one capability):\n");
        for class in TechnologySharing::ALL {
            s.push_str(&format!(
                "  {:<28} {}\n",
                class.wire(),
                self.sharing.iter().filter(|x| x.class == *class).count()
            ));
        }
        s.push_str("shard structure:\n");
        for (shard, v) in &self.structure {
            s.push_str(&format!(
                "  {shard:<20} {}\n",
                if v.is_empty() {
                    "CONFORMANT".to_string()
                } else {
                    format!("{} violations: {}", v.len(), v.join("; "))
                }
            ));
        }
        let mut by_code: BTreeMap<(&str, &str), usize> = BTreeMap::new();
        for i in &self.issues {
            *by_code
                .entry((i.severity.wire(), i.code.as_str()))
                .or_default() += 1;
        }
        for ((sev, code), n) in by_code {
            s.push_str(&format!("  {sev:<7} {code:<28} {n}\n"));
            for i in self.issues.iter().filter(|i| i.code == code).take(3) {
                s.push_str(&format!("      {}: {}\n", i.subject, i.detail));
            }
        }
        s
    }

    /// Everything the system knows about one technology: what it is and who owns it, its
    /// implementations by shard (and how they relate), consumers, lineage, proofs, what it feeds
    /// in Norl and whether it still depends on a donor.
    pub fn technology_json(&self, t: &SystemTechnology) -> Json {
        Json::obj()
            .with("key", &t.key)
            .with("name", &t.name)
            .with("kind", t.kind.wire())
            .with("purpose", &t.purpose)
            .with("born_in", &t.birthplace)
            .with("node", &t.node)
            .with("effective", t.effective.wire())
            .with("implements", t.implements.clone())
            .with(
                "implementations",
                Json::Array(
                    self.implementations(&t.key)
                        .into_iter()
                        .map(|(class, other, shard)| {
                            Json::obj()
                                .with("class", class.wire())
                                .with("with", other)
                                .with("shard", shard)
                        })
                        .collect(),
                ),
            )
            .with("consumers", t.consumers.clone())
            .with("lineage", t.lineage.clone())
            .with("proofs", t.proofs.clone())
            .with("feeds", t.feeds.clone())
            .with("norl", t.norl.wire())
            .with("donor_dependent", t.donor_dependent)
    }

    /// The other implementations related to technology `key`: (class, other technology key or
    /// `<node>@<shard>` copy, shard).
    pub fn implementations(&self, key: &str) -> Vec<(TechnologySharing, String, String)> {
        let shard_of = |k: &str| {
            self.technologies
                .iter()
                .find(|t| t.key == k)
                .map(|t| t.birthplace.clone())
                .unwrap_or_else(|| {
                    k.rsplit_once('@')
                        .map(|(_, s)| s.to_string())
                        .unwrap_or_default()
                })
        };
        self.sharing
            .iter()
            .filter_map(|x| {
                let other = if x.a == key {
                    &x.b
                } else if x.b == key {
                    &x.a
                } else {
                    return None;
                };
                Some((x.class, other.clone(), shard_of(other)))
            })
            .collect()
    }

    /// The multi-line text answer for one technology.
    pub fn technology_text(&self, t: &SystemTechnology) -> String {
        let or_dash = |v: &[String]| {
            if v.is_empty() {
                "-".to_string()
            } else {
                v.join(", ")
            }
        };
        let impls: Vec<String> = self
            .implementations(&t.key)
            .into_iter()
            .map(|(c, o, sh)| format!("{o} in {sh} [{}]", c.wire()))
            .collect();
        format!(
            "{} — {} ({}, {})\n  born in {} (its owner) node {}; purpose: {}\n  implements {}\n  implementations {}\n  consumers {}\n  lineage {}\n  proofs {}\n  feeds Norl {}\n  norl {}; donor-dependent: {}\n",
            t.key,
            t.name,
            t.kind.wire(),
            t.effective.wire(),
            t.birthplace,
            t.node,
            if t.purpose.is_empty() { "-" } else { &t.purpose },
            or_dash(&t.implements),
            or_dash(&impls),
            or_dash(&t.consumers),
            or_dash(&t.lineage),
            or_dash(&t.proofs),
            or_dash(&t.feeds),
            t.norl.wire(),
            if t.donor_dependent { "yes" } else { "no" }
        )
    }

    pub fn to_json(&self) -> Json {
        Json::obj()
            .with("schema", &self.schema)
            .with("pass", self.pass())
            .with(
                "shards",
                Json::Array(
                    self.shards
                        .iter()
                        .map(|(a, b, c, d)| {
                            Json::obj()
                                .with("shard", a)
                                .with("origin", b)
                                .with("head", c)
                                .with("capsule", d)
                        })
                        .collect(),
                ),
            )
            .with("metrics", self.counts().to_json())
            .with(
                "technologies",
                Json::Array(
                    self.technologies
                        .iter()
                        .map(|t| self.technology_json(t))
                        .collect(),
                ),
            )
            .with(
                "capabilities",
                Json::Array(
                    self.capabilities
                        .iter()
                        .map(|c| {
                            Json::obj()
                                .with("key", &c.key)
                                .with(
                                    "providers",
                                    c.providers
                                        .iter()
                                        .map(|(s, n)| format!("{n}@{s}"))
                                        .collect::<Vec<_>>(),
                                )
                                .with(
                                    "requirers",
                                    c.requirers
                                        .iter()
                                        .map(|(s, n)| format!("{n}@{s}"))
                                        .collect::<Vec<_>>(),
                                )
                                .with("technologies", c.technologies.clone())
                        })
                        .collect(),
                ),
            )
            .with(
                "issues",
                Json::Array(
                    self.issues
                        .iter()
                        .map(|i| {
                            Json::obj()
                                .with("severity", i.severity.wire())
                                .with("code", &i.code)
                                .with("subject", &i.subject)
                                .with("detail", &i.detail)
                        })
                        .collect(),
                ),
            )
            .with(
                "sharing",
                Json::Array(
                    self.sharing
                        .iter()
                        .map(|x| {
                            Json::obj()
                                .with("class", x.class.wire())
                                .with("a", &x.a)
                                .with("b", &x.b)
                                .with("capabilities", x.capabilities.clone())
                        })
                        .collect(),
                ),
            )
            .with(
                "structure",
                Json::Array(
                    self.structure
                        .iter()
                        .map(|(shard, v)| {
                            Json::obj()
                                .with("shard", shard)
                                .with("conformant", v.is_empty())
                                .with("violations", v.clone())
                        })
                        .collect(),
                ),
            )
            .with(
                "duplicate_symbols",
                Json::Array(
                    self.duplicate_symbols
                        .iter()
                        .map(|(f, sig, m)| {
                            Json::obj()
                                .with("fingerprint", f)
                                .with("signature", sig)
                                .with(
                                    "members",
                                    m.iter()
                                        .map(|(s, k)| format!("{k}@{s}"))
                                        .collect::<Vec<_>>(),
                                )
                        })
                        .collect(),
                ),
            )
    }
}
