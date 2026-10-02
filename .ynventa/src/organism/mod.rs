//! Norl, the organism, as one shard can judge it.
//!
//! Every other shard stays an independent system; what it offers the organism is developmental
//! MATERIAL (worlds, observations, actions, experiences, curricula, evaluations) and the FEEDS /
//! TEACHES / EVALUATED_BY edges that place that material in the canonical graph. Code is never
//! exported to Norl. The organism itself — ORGAN nodes and `declared/organism.rs` — is legal
//! only in the norl shard, and its cognition never comes from the coding agent.
//!
//! This module holds the shard-local gates. What only the linked system can decide (a feed into
//! another shard, an evaluation recorded elsewhere) is decided by the linker.

use crate::declare::{Declaration, NorlRelevance, Promotion};
use crate::graph::{is_norl_node, Graph, NodeId, SYSTEM};
use crate::schema::{
    is_physical, BackendKind, Concept, EdgeKind, GrowthState, NodeKind, NodeLifecycle,
    PromotionImplementation, PromotionState, SourceMaturity, TechnologyLifecycle, NORL_SHARD,
};
use crate::{Finding, Severity};

/// Words that name the coding agent. A backend whose node, donor or weight names one of these is
/// the agent, and the agent is never a Norl capability (CLAUDE_IS_NOT_NORL).
pub const AGENT_MARKERS: &[&str] = &["claude", "anthropic"];

/// Node kinds a Norl backend may live in: shipped native code. Tools, tests, research and the
/// subsystem are never cognition.
pub const BACKEND_KINDS: &[NodeKind] = &[
    NodeKind::Kernel,
    NodeKind::Substrate,
    NodeKind::Domain,
    NodeKind::Adapter,
    NodeKind::Application,
];

/// Whether `text` names the coding agent.
pub fn names_agent(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    AGENT_MARKERS.iter().any(|m| t.contains(m))
}

fn err(findings: &mut Vec<Finding>, code: &str, subject: &str, detail: String) {
    findings.push(Finding::new(Severity::Error, code, subject, &detail));
}

/// The shard-local Norl gates: organism placement, backends, the material graph's local
/// soundness, proofs of evaluations and claims, and orphaned capabilities and technologies.
pub fn check(
    d: &Declaration,
    g: &Graph,
    technologies: &[crate::technology::TechnologyAssessment],
    findings: &mut Vec<Finding>,
) {
    let shard = d.repository.shard.as_str();
    let norl = shard == NORL_SHARD;
    let id = |k: &str| NodeId::of(SYSTEM, k);

    // 1. The organism lives only in Norl.
    if !norl {
        for n in d
            .repository
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Organ)
        {
            err(
                findings,
                "ORGANISM_OUTSIDE_NORL",
                &n.key,
                format!("an ORGAN is declared only by the norl shard, not by `{shard}`"),
            );
        }
        if !d.organism.is_empty() {
            err(
                findings,
                "ORGANISM_OUTSIDE_NORL",
                "organism",
                format!(
                    "`{shard}` declares {} organism capabilities and {} backends; organism.rs is empty outside norl (offer MATERIAL that FEEDS Norl instead)",
                    d.organism.capabilities.len(),
                    d.organism.backends.len()
                ),
            );
        }
    }

    // 2. Organism capabilities name a declared organ and backend.
    for c in &d.organism.capabilities {
        if d.node(&c.organ).is_none_or(|n| n.kind != NodeKind::Organ) {
            err(
                findings,
                "UNRESOLVED_NORL_NODE",
                &c.key,
                format!("organ `{}` is not an ORGAN declared here", c.organ),
            );
        }
        if let Some(b) = &c.backend {
            if d.organism.backend(b).is_none() {
                err(
                    findings,
                    "UNRESOLVED_NORL_NODE",
                    &c.key,
                    format!("backend `{b}` is not declared in organism.rs"),
                );
            }
        }
        if c.claimed >= GrowthState::Evaluated && c.evaluations.is_empty() {
            err(
                findings,
                "MISSING_PROOF",
                &c.key,
                format!(
                    "claims {} with no evaluation: declare the Material(EVALUATION) nodes that judge it",
                    c.claimed
                ),
            );
        }
    }

    // 3. Backends are Norl's own native code; the coding agent is never one.
    for b in &d.organism.backends {
        let node = d.node(&b.node);
        let agent = [
            Some(b.key.as_str()),
            b.donor.as_deref(),
            Some(b.weight.as_str()),
        ]
        .into_iter()
        .flatten()
        .chain(
            b.donor
                .as_deref()
                .and_then(|k| d.donor(k))
                .into_iter()
                .flat_map(|dn| {
                    [dn.name.as_str(), dn.origin.as_str()]
                        .into_iter()
                        .chain(dn.packages.iter().map(|p| p.name.as_str()))
                }),
        )
        .chain(node.into_iter().flat_map(|n| {
            [n.path.as_str(), n.name.as_str()]
                .into_iter()
                .chain(n.lineage.iter().map(String::as_str))
        }))
        .find(|t| names_agent(t));
        if let Some(t) = agent {
            err(
                findings,
                "CLAUDE_IS_NOT_NORL",
                &b.key,
                format!("`{t}` names the coding agent; the agent is never a Norl capability, its backend or its provision path"),
            );
        }
        match node {
            Some(n) if norl && BACKEND_KINDS.contains(&n.kind) && !n.path.is_empty() => {}
            _ => err(
                findings,
                "CLAUDE_IS_NOT_NORL",
                &b.key,
                format!(
                    "backend node `{}` is not a norl physical node of a shipped role ({}); cognition that norl does not hold is not Norl's",
                    b.node,
                    BACKEND_KINDS
                        .iter()
                        .map(|k| k.wire())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ),
        }
        match (&b.donor, b.kind) {
            (Some(k), _) if d.donor(k).is_none() => err(
                findings,
                "UNKNOWN_LINEAGE_DONOR",
                &b.key,
                format!("backend donor `{k}` is not a declared donor"),
            ),
            (None, BackendKind::BorrowedWeight) => err(
                findings,
                "BORROWED_WITHOUT_DONOR",
                &b.key,
                "a borrowed weight is external technology: name its registered donor".into(),
            ),
            _ => {}
        }
    }

    // 4. Material: experiences and evaluations are generated in their own shard; evaluations
    //    are bound to proofs.
    let generated = |m: NodeId| {
        g.edges.iter().any(|e| {
            e.to == m
                && e.kind == EdgeKind::Generates
                && g.nodes
                    .get(&e.from)
                    .is_some_and(|s| s.repository == shard || s.kind == NodeKind::Capability)
        })
    };
    for n in d
        .repository
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Material)
    {
        let mid = id(&n.key);
        let bound = g.nodes.get(&mid).is_some_and(|x| !x.evidence.is_empty());
        if matches!(n.concept, Concept::Experience | Concept::Evaluation) && !generated(mid) {
            err(
                findings,
                "INVALID_NORL_FEED",
                &n.key,
                format!(
                    "a Material({}) needs a GENERATES source in `{shard}`: material that nothing here produces cannot feed the organism",
                    n.concept
                ),
            );
        }
        if n.concept == Concept::Evaluation {
            if !bound {
                err(
                    findings,
                    "MISSING_PROOF",
                    &n.key,
                    "an evaluation is bound to evidence by `Edge { from: \"proof:<file>::<test>\", to: <it>, kind: EdgeKind::Verifies, .. }`".into(),
                );
            }
            let used = g.edges.iter().any(|e| {
                (e.from == mid && e.kind == EdgeKind::Feeds)
                    || (e.to == mid && e.kind == EdgeKind::EvaluatedBy)
            });
            if used && !bound {
                err(
                    findings,
                    "INVALID_NORL_FEED",
                    &n.key,
                    "an evaluation that feeds or judges Norl has no proof binding".into(),
                );
            }
        }
    }

    // 5. Technologies: a claim of PROVEN or above needs declared proofs; every technology
    //    implements something and resolves its relevance to Norl.
    for t in &d.technologies {
        let x = technologies.iter().find(|x| x.key == t.key);
        if t.claimed >= TechnologyLifecycle::Proven && t.proofs.is_empty() {
            err(
                findings,
                "MISSING_PROOF",
                &t.key,
                format!("claims {} and declares no proof", t.claimed),
            );
        }
        if x.is_some_and(|x| x.effective == TechnologyLifecycle::Superseded) {
            continue;
        }
        let tid = id(&crate::graph::technology_key(&t.key));
        let feeds = g
            .edges
            .iter()
            .any(|e| e.from == tid && e.kind == EdgeKind::Feeds);
        let why = if t.implements.is_empty() {
            Some("implements no capability".to_string())
        } else if !feeds
            && !matches!(t.norl, NorlRelevance::NotRelevant(ref r) if !r.trim().is_empty())
        {
            Some("feeds no Norl capability and declares no `norl: NorlRelevance::NotRelevant(\"<reason>\")`".to_string())
        } else {
            None
        };
        if let Some(why) = why {
            findings.push(Finding::new(
                Severity::Warning,
                "ORPHAN_TECHNOLOGY",
                &t.key,
                &why,
            ));
        }
    }

    // 6. Capabilities of a product shard that nothing places in the technology or material
    //    graph.
    for (k, providers) in orphan_capabilities(g, Some(shard)) {
        findings.push(Finding::new(
            Severity::Warning,
            "ORPHAN_CAPABILITY",
            &k,
            &format!(
                "provided by {} with no technology implementing it and no material or feed relation",
                providers.join(", ")
            ),
        ));
    }
}

/// Capabilities provided by a physical node of a product shard (only `shard`'s, when given)
/// that no technology implements, that no GENERATES/FEEDS/TEACHES/EVALUATED_BY edge touches,
/// whose providers generate no material, and that are not Norl's own. Returns
/// (capability key, ["node@shard"]).
pub fn orphan_capabilities(g: &Graph, shard: Option<&str>) -> Vec<(String, Vec<String>)> {
    let product = |s: &str| {
        !s.is_empty() && s != crate::conformance::TEMPLATE_SHARD && shard.is_none_or(|x| x == s)
    };
    let material_relation = |id: NodeId| {
        g.edges.iter().any(|e| {
            (e.from == id || e.to == id)
                && matches!(
                    e.kind,
                    EdgeKind::Feeds
                        | EdgeKind::Generates
                        | EdgeKind::Teaches
                        | EdgeKind::EvaluatedBy
                )
        })
    };
    let generates_material = |id: NodeId| {
        g.edges.iter().any(|e| {
            e.from == id
                && e.kind == EdgeKind::Generates
                && g.nodes
                    .get(&e.to)
                    .is_some_and(|m| m.kind == NodeKind::Material)
        })
    };
    let mut out = Vec::new();
    for n in g
        .nodes
        .values()
        .filter(|n| n.kind == NodeKind::Capability && n.namespace == SYSTEM)
        .filter(|n| n.semantic_key.starts_with("capability/"))
    {
        let providers: Vec<&crate::graph::GNode> = g
            .edges
            .iter()
            .filter(|e| e.to == n.id && e.kind == EdgeKind::Provides)
            .filter_map(|e| g.nodes.get(&e.from))
            .filter(|p| is_physical(p.kind) && product(&p.repository))
            .collect();
        if providers.is_empty() {
            continue;
        }
        let implemented = g.edges.iter().any(|e| {
            e.to == n.id
                && e.kind == EdgeKind::Implements
                && g.nodes
                    .get(&e.from)
                    .is_some_and(|t| t.kind == NodeKind::Technology)
        });
        if implemented
            || material_relation(n.id)
            || providers.iter().any(|p| generates_material(p.id))
            || is_norl_node(g, n.id)
        {
            continue;
        }
        let mut names: Vec<String> = providers
            .iter()
            .map(|p| format!("{}@{}", p.semantic_key, p.repository))
            .collect();
        names.sort();
        out.push((
            n.semantic_key.trim_start_matches("capability/").to_string(),
            names,
        ));
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Growth: the effective state of every organism capability, computed from the graph and from
// evaluation evidence, never declared.

use crate::declare::Organism;
use crate::evidence::Verdict;
use crate::schema::NativeStatus;
use std::collections::{BTreeMap, BTreeSet};

/// Evaluation verdicts by material key: (proof locator, verdict).
pub type Evaluations = BTreeMap<String, Vec<(String, Verdict)>>;

/// The verdict of one evaluation: PASS when it has proofs and all pass; FAIL when any of its
/// freshest records failed; otherwise the first other verdict (stale, unrecorded, absent).
pub fn evaluation_verdict(ev: &Evaluations, key: &str) -> Verdict {
    match ev.get(key) {
        None => Verdict::Unrecorded,
        Some(v) if v.is_empty() => Verdict::Unrecorded,
        Some(v) if v.iter().any(|(_, x)| *x == Verdict::Fail) => Verdict::Fail,
        Some(v) if v.iter().all(|(_, x)| *x == Verdict::Pass) => Verdict::Pass,
        Some(v) => v
            .iter()
            .map(|(_, x)| *x)
            .find(|x| *x != Verdict::Pass)
            .unwrap_or(Verdict::Unrecorded),
    }
}

/// The growth of one organism capability.
#[derive(Clone, Debug, PartialEq)]
pub struct Growth {
    pub key: String,
    pub organ: String,
    pub claimed: GrowthState,
    /// The capability and its organ exist in the norl graph. When false, nothing is.
    pub defined: bool,
    pub effective: GrowthState,
    /// Why the effective state stops where it does.
    pub stopped_by: String,
    /// (backend kind, backend node).
    pub backend: Option<(BackendKind, String)>,
    /// Cognition source: a borrowed weight (reported beside the state).
    pub borrowed: bool,
    /// An evaluation's latest evidence FAILED.
    pub regressed: bool,
    /// Experiences that feed it, `<key>@<shard>`.
    pub experiences: Vec<String>,
    /// Its evaluations: (`<key>@<shard>`, verdict).
    pub evaluations: Vec<(String, Verdict)>,
    /// An unmet rung that material of another shard may still meet: judged at link.
    pub pending_link: bool,
    /// What other repositories have proven of what feeds it (promotions and feeds): reported
    /// beside its growth, never part of it. Its growth (`effective`) is Norl's maturity.
    pub source_maturity: SourceMaturity,
}

fn shard_label(g: &Graph, id: NodeId) -> String {
    g.nodes
        .get(&id)
        .map(|n| format!("{}@{}", n.semantic_key, n.repository))
        .unwrap_or_else(|| id.to_string())
}

/// Computes the growth of every organism capability of `o` in graph `g` (one shard's graph, or
/// the linked system's), with evaluation verdicts `ev`:
///
/// * DEFINED — `capability/<key>` and its ORGAN exist and the organ CONTAINS it;
/// * EXPOSED — a norl physical node PROVIDES or IMPLEMENTS it;
/// * EXPERIENCED — a Material(EXPERIENCE) FEEDS it and something of the material's own shard
///   GENERATES that material;
/// * EVALUATED — it is EVALUATED_BY a Material(EVALUATION) whose proofs all pass, fresh;
/// * LEARNED — evaluated, and its backend is NATIVE_TINY_WEIGHT or NATIVE_SCALED_WEIGHT;
/// * NATIVE — evaluated, its backend is native (DETERMINISTIC or NATIVE_*), names no donor,
///   its node is NATIVE, and every norl node providing it is NATIVE.
///
/// The rungs up to EVALUATED are cumulative; NATIVE does not require LEARNED (deterministic
/// cognition is native without weights), and NATIVE ranks above LEARNED.
pub fn growth(g: &Graph, o: &Organism, ev: &Evaluations) -> Vec<Growth> {
    let id = |k: &str| NodeId::of(SYSTEM, k);
    let mut out = Vec::new();
    for c in &o.capabilities {
        let cid = id(&crate::graph::system_capability_key(&c.key));
        let oid = id(&c.organ);
        let defined = g.nodes.contains_key(&cid)
            && g.nodes.get(&oid).is_some_and(|n| n.kind == NodeKind::Organ)
            && g.edges
                .iter()
                .any(|e| e.from == oid && e.to == cid && e.kind == EdgeKind::Contains);
        let providers: Vec<&crate::graph::GNode> = g
            .edges
            .iter()
            .filter(|e| e.to == cid && matches!(e.kind, EdgeKind::Provides | EdgeKind::Implements))
            .filter_map(|e| g.nodes.get(&e.from))
            .filter(|n| is_physical(n.kind) && n.repository == NORL_SHARD)
            .collect();
        let exposed = !providers.is_empty();
        let generated = |m: &crate::graph::GNode| {
            g.edges.iter().any(|e| {
                e.to == m.id
                    && e.kind == EdgeKind::Generates
                    && g.nodes.get(&e.from).is_some_and(|s| {
                        s.repository == m.repository || s.kind == NodeKind::Capability
                    })
            })
        };
        let mut experiences: Vec<String> = g
            .edges
            .iter()
            .filter(|e| e.to == cid && e.kind == EdgeKind::Feeds)
            .filter_map(|e| g.nodes.get(&e.from))
            .filter(|m| m.kind == NodeKind::Material && m.concept == Concept::Experience)
            .filter(|m| generated(m))
            .map(|m| shard_label(g, m.id))
            .collect();
        experiences.sort();
        experiences.dedup();
        let experienced = !experiences.is_empty();
        let mut evaluations: Vec<(String, Verdict)> = Vec::new();
        let mut foreign_evaluation = false;
        for e in c.evaluations.iter() {
            match g.nodes.get(&id(e)) {
                Some(m) if m.kind == NodeKind::Material && m.concept == Concept::Evaluation => {
                    evaluations.push((shard_label(g, m.id), evaluation_verdict(ev, e)))
                }
                Some(_) => evaluations.push((e.clone(), Verdict::Absent)),
                None => {
                    foreign_evaluation = true;
                    evaluations.push((e.clone(), Verdict::Unrecorded));
                }
            }
        }
        // Only Norl's own evaluations judge Norl. An evaluation another shard records proves
        // that shard's behaviour (Chronica recovering its log is not Norl recovering anything):
        // it is a benchmark Norl may be judged by once a norl-owned evaluation USES it, never
        // evidence that Norl has the capability (CLAUDE_CAN_DO(X) != NORL_CAN_DO(X), and neither
        // does REPO_CAN_DO(X)).
        let own = |label: &str| label.ends_with(&format!("@{NORL_SHARD}"));
        let evaluated_ok = evaluations
            .iter()
            .any(|(l, v)| own(l) && *v == Verdict::Pass);
        let regressed = evaluations
            .iter()
            .any(|(l, v)| own(l) && *v == Verdict::Fail);
        let backend = c.backend.as_deref().and_then(|b| o.backend(b));
        let native_kind = |k: BackendKind| {
            matches!(
                k,
                BackendKind::Deterministic
                    | BackendKind::NativeTinyWeight
                    | BackendKind::NativeScaledWeight
            )
        };
        let node_native = |k: &str| {
            g.nodes
                .get(&id(k))
                .is_some_and(|n| n.native_status == NativeStatus::Native)
        };
        let learned_kind = backend.is_some_and(|b| {
            matches!(
                b.kind,
                BackendKind::NativeTinyWeight | BackendKind::NativeScaledWeight
            )
        });
        let native_path = backend
            .is_some_and(|b| native_kind(b.kind) && b.donor.is_none() && node_native(&b.node))
            && providers
                .iter()
                .all(|p| p.native_status == NativeStatus::Native);
        let ladder = [
            (
                GrowthState::Exposed,
                exposed,
                "no norl physical node provides or implements it".to_string(),
            ),
            (
                GrowthState::Experienced,
                experienced,
                "no generated Material(EXPERIENCE) feeds it".to_string(),
            ),
            (
                GrowthState::Evaluated,
                evaluated_ok,
                if evaluations.is_empty() {
                    "no evaluation declared".to_string()
                } else {
                    format!(
                        "no norl-owned evaluation with fresh passing evidence: {}",
                        evaluations
                            .iter()
                            .map(|(k, v)| {
                                if own(k) {
                                    format!("{k} {}", v.wire())
                                } else {
                                    format!("{k} {} (another shard's evaluation: a benchmark, not Norl's evidence)", v.wire())
                                }
                            })
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                },
            ),
        ];
        let mut effective = GrowthState::Defined;
        let mut stopped_by = String::new();
        if !defined {
            stopped_by = format!(
                "DEFINED: `{}` is not an ORGAN of norl containing capability/{}",
                c.organ, c.key
            );
        } else {
            for (state, ok, why) in ladder {
                if ok {
                    effective = state;
                } else {
                    stopped_by = format!("{state}: {why}");
                    break;
                }
            }
        }
        if effective == GrowthState::Evaluated {
            if native_path {
                effective = GrowthState::Native;
            } else if learned_kind {
                effective = GrowthState::Learned;
                stopped_by = "NATIVE: a donor or non-native node is in its provision path".into();
            } else {
                stopped_by = match backend {
                    None => "LEARNED: no backend".into(),
                    Some(b) if b.kind == BackendKind::BorrowedWeight => {
                        "LEARNED: the backend is a borrowed weight".into()
                    }
                    Some(_) => "NATIVE: a donor or non-native node is in its provision path".into(),
                };
            }
        }
        let pending_link = (defined && effective == GrowthState::Exposed)
            || (effective == GrowthState::Experienced && foreign_evaluation);
        out.push(Growth {
            key: c.key.clone(),
            organ: c.organ.clone(),
            claimed: c.claimed,
            defined,
            effective,
            stopped_by,
            backend: backend.map(|b| (b.kind, b.node.clone())),
            borrowed: backend.is_some_and(|b| b.kind == BackendKind::BorrowedWeight),
            regressed,
            experiences,
            evaluations,
            pending_link,
            source_maturity: SourceMaturity::Unfed,
        });
    }
    out
}

/// Claims above the computed growth. Within the norl shard alone (`local`), a rung that other
/// shards' material may still meet is only a warning: it is decided when the system is linked.
pub fn claim_findings(growth: &[Growth], local: bool) -> Vec<Finding> {
    let mut out = Vec::new();
    for x in growth {
        if x.claimed <= x.effective && x.defined {
            continue;
        }
        let deferred = local && x.pending_link;
        let sev = if deferred {
            Severity::Warning
        } else {
            Severity::Error
        };
        out.push(Finding::new(
            sev,
            "CLAIM_EXCEEDS_EVIDENCE",
            &x.key,
            &format!(
                "claims {} but evidence supports {}; stopped at {}{}",
                x.claimed,
                if x.defined {
                    x.effective.wire()
                } else {
                    "nothing (undefined)"
                },
                x.stopped_by,
                if deferred {
                    "; experiences and evaluations of other shards are judged at link"
                } else {
                    ""
                }
            ),
        ));
        if x.claimed >= GrowthState::Evaluated && x.effective < GrowthState::Evaluated {
            out.push(Finding::new(
                sev,
                "NORL_CAPABILITY_WITHOUT_EVALUATION",
                &x.key,
                &format!(
                    "claims {} without an evaluation with fresh passing evidence{}",
                    x.claimed,
                    if x.regressed {
                        " (REGRESSED: an evaluation failed)"
                    } else {
                        ""
                    }
                ),
            ));
        }
        if x.claimed >= GrowthState::Learned && x.effective < x.claimed {
            out.push(Finding::new(
                sev,
                "NATIVE_CLAIM_WITHOUT_EVIDENCE",
                &x.key,
                &format!(
                    "claims {} but its cognition is {}",
                    x.claimed,
                    match &x.backend {
                        None => "undeclared (no backend)".to_string(),
                        Some((k, n)) => format!("{k} in `{n}`"),
                    }
                ),
            ));
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Promotion: a source repository's proven capability becoming Norl's. Two maturities are kept
// apart: what the source has proven (SOURCE_REPO_CAN_DO) and what Norl has grown (NORL_CAN_DO).

/// Proof verdicts by (shard, locator).
pub type ProofVerdicts = BTreeMap<(String, String), Verdict>;

/// What a report knows of the source repositories.
#[derive(Clone, Copy)]
pub struct Sources<'a> {
    /// Verdicts of every linked shard's proofs (locally: the shard's own).
    pub proofs: &'a ProofVerdicts,
    /// The linked shards; `None` within one shard, where a source's proof is judged only at link.
    pub linked: Option<&'a BTreeSet<String>>,
}

/// The effective state of one promotion.
#[derive(Clone, Debug, PartialEq)]
pub struct PromotionGrowth {
    pub key: String,
    /// `<source node>@<source repository>`.
    pub source: String,
    pub destination: String,
    pub implementation: PromotionImplementation,
    pub claimed: PromotionState,
    /// `None`: not even PROVEN_IN_SOURCE.
    pub effective: Option<PromotionState>,
    /// Why the effective state stops where it does.
    pub stopped_by: String,
    /// The source's proof: `None` within one shard (decided at link).
    pub source_proven: Option<bool>,
}

/// Edge kinds a promotion may be (developmental supply).
pub const PROMOTION_RELATIONS: &[EdgeKind] = &[EdgeKind::Feeds, EdgeKind::Teaches];

fn is_commit(s: &str) -> bool {
    s.len() >= 7 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Whether a FEEDS/TEACHES or promotion reference names a file path rather than a capability,
/// technology, material, evidence or protocol.
pub fn names_path(reference: &str) -> bool {
    let r = reference
        .strip_prefix("capability/")
        .or_else(|| reference.strip_prefix("technology/"))
        .unwrap_or(reference);
    const EXTENSIONS: &[&str] = &[
        ".rs", ".toml", ".json", ".jsonl", ".md", ".py", ".ts", ".tsx", ".js", ".yaml", ".yml",
        ".lock", ".sh", ".txt",
    ];
    r.contains('/') || r.contains('\\') || EXTENSIONS.iter().any(|e| r.ends_with(e))
}

/// The source rung of a promotion: `None` locally, otherwise whether the source shard proves
/// it with fresh passing evidence at the node, capability or technology it names.
fn source_rung(g: &Graph, p: &Promotion, sources: &Sources) -> (Option<bool>, String) {
    let Some(linked) = sources.linked else {
        return (None, "PROVEN_IN_SOURCE: decided at link".into());
    };
    let shard = p.source_repository.as_str();
    let id = |k: &str| NodeId::of(SYSTEM, k);
    let owned = |k: &str| g.nodes.get(&id(k)).is_some_and(|n| n.repository == shard);
    let why = if !linked.contains(shard) {
        format!("source repository `{shard}` is not linked")
    } else if !owned(&p.source_node) {
        format!("`{}` is not a node of {shard}", p.source_node)
    } else if let Some(t) = p
        .source_technology
        .as_deref()
        .filter(|t| !owned(&crate::graph::technology_key(t)))
    {
        format!("`{t}` is not a technology born in {shard}")
    } else if let Some(c) = p.source_capability.as_deref().filter(|c| {
        let cid = id(&crate::graph::system_capability_key(c));
        !g.edges.iter().any(|e| {
            e.to == cid
                && matches!(e.kind, EdgeKind::Provides | EdgeKind::Implements)
                && g.nodes.get(&e.from).is_some_and(|n| n.repository == shard)
        })
    }) {
        format!("{shard} neither provides nor implements capability `{c}`")
    } else if !is_commit(&p.source_commit) {
        format!("source commit `{}` is not a commit", p.source_commit)
    } else if p.source_evidence.is_empty() {
        "no source evidence is named".into()
    } else if let Some((l, v)) = p
        .source_evidence
        .iter()
        .map(|l| {
            (
                l,
                sources
                    .proofs
                    .get(&(shard.to_string(), l.clone()))
                    .copied()
                    .unwrap_or(Verdict::Absent),
            )
        })
        .find(|(_, v)| *v != Verdict::Pass)
    {
        format!("source evidence {l} is {} in {shard}", v.wire())
    } else {
        return (Some(true), String::new());
    };
    (Some(false), format!("PROVEN_IN_SOURCE: {why}"))
}

/// Computes every promotion's effective state. Within one shard the source rung is assumed met
/// (it is decided at link), so a claim the shard alone refutes is still an error there.
pub fn promotion_growth(
    g: &Graph,
    o: &Organism,
    growth: &[Growth],
    sources: &Sources,
) -> Vec<PromotionGrowth> {
    let id = |k: &str| NodeId::of(SYSTEM, k);
    let mut out = Vec::new();
    for p in &o.promotions {
        let (source_proven, source_why) = source_rung(g, p, sources);
        let gr = growth.iter().find(|x| x.key == p.destination_capability);
        let dest = p
            .destination_node
            .as_deref()
            .and_then(|k| g.nodes.get(&id(k)));
        let dest_exists = dest.is_some_and(|n| {
            n.repository == NORL_SHARD
                && is_physical(n.kind)
                && !n.path.is_empty()
                && n.lifecycle == NodeLifecycle::Active
        });
        let reuses_source = |n: &crate::graph::GNode| {
            p.source_technology.as_deref().is_some_and(|t| {
                let tid = id(&crate::graph::technology_key(t));
                g.edges
                    .iter()
                    .any(|e| e.from == n.id && e.to == tid && e.kind == EdgeKind::Reuses)
            })
        };
        let tested = dest.is_some_and(|n| {
            let prefix = format!("{}/", n.path.trim_end_matches('/'));
            sources.proofs.iter().any(|((shard, loc), v)| {
                shard == NORL_SHARD && *v == Verdict::Pass && loc.starts_with(&prefix)
            })
        });
        let at = |s: GrowthState| gr.is_some_and(|x| x.defined && x.effective >= s);
        let ladder: Vec<(PromotionState, bool, String)> = vec![
            (
                PromotionState::ProvenInSource,
                source_proven != Some(false),
                source_why,
            ),
            (
                PromotionState::NorlRelevanceResolved,
                gr.is_some_and(|x| x.defined) && !p.reason.trim().is_empty(),
                if gr.is_none_or(|x| !x.defined) {
                    format!(
                        "`{}` is not a defined organism capability of norl",
                        p.destination_capability
                    )
                } else {
                    "no reason why Norl needs it".into()
                },
            ),
            (
                PromotionState::ImportCandidate,
                crate::protocol::shard(&p.source_repository).is_some()
                    && p.source_repository != NORL_SHARD
                    && !p.source_node.trim().is_empty()
                    && is_commit(&p.source_commit)
                    && (p.source_capability.is_some() || p.source_technology.is_some())
                    && PROMOTION_RELATIONS.contains(&p.relation),
                "source provenance incomplete: a canonical source shard other than norl, its node, a commit, a capability or technology, and FEEDS or TEACHES".into(),
            ),
            (
                PromotionState::PromotionDesigned,
                !p.transformation.trim().is_empty(),
                "no transformation is stated".into(),
            ),
            (
                PromotionState::NativeNorlImplementation,
                p.implementation != PromotionImplementation::ReferenceOnly
                    && dest_exists
                    && (p.implementation != PromotionImplementation::SharedImplementation
                        || dest.is_some_and(reuses_source)),
                if p.implementation == PromotionImplementation::ReferenceOnly {
                    "a REFERENCE_ONLY promotion implements nothing in Norl".into()
                } else if !dest_exists {
                    "no active norl physical destination node".into()
                } else {
                    "a SHARED_IMPLEMENTATION's destination node does not REUSE the source technology".into()
                },
            ),
            (
                PromotionState::NorlTested,
                tested,
                "no norl-owned proof in the destination node passes, fresh".into(),
            ),
            (
                PromotionState::NorlEvaluated,
                at(GrowthState::Evaluated),
                format!(
                    "Norl's growth of `{}` is {}: no norl-owned evaluation passes",
                    p.destination_capability,
                    gr.map(|x| x.effective.wire()).unwrap_or("UNDEFINED")
                ),
            ),
            (
                PromotionState::NorlNative,
                gr.is_some_and(|x| x.defined && x.effective == GrowthState::Native),
                format!(
                    "Norl's growth of `{}` is not NATIVE",
                    p.destination_capability
                ),
            ),
        ];
        let mut effective = None;
        let mut stopped_by = String::new();
        for (state, ok, why) in ladder {
            if ok {
                effective = Some(state);
            } else {
                stopped_by = if why.starts_with(state.wire()) {
                    why
                } else {
                    format!("{state}: {why}")
                };
                break;
            }
        }
        out.push(PromotionGrowth {
            key: p.key.clone(),
            source: format!("{}@{}", p.source_node, p.source_repository),
            destination: p.destination_capability.clone(),
            implementation: p.implementation,
            claimed: p.claimed,
            effective,
            stopped_by,
            source_proven,
        });
    }
    out
}

/// Promotion claims above their computed state.
pub fn promotion_findings(promotions: &[PromotionGrowth]) -> Vec<Finding> {
    promotions
        .iter()
        .filter(|p| p.effective.is_none_or(|e| p.claimed > e))
        .map(|p| {
            Finding::new(
                Severity::Error,
                "PROMOTION_CLAIM_EXCEEDS_EVIDENCE",
                &p.key,
                &format!(
                    "claims {} but evidence supports {}; stopped at {}{}",
                    p.claimed,
                    p.effective.map(|e| e.wire()).unwrap_or("nothing"),
                    p.stopped_by,
                    if p.source_proven.is_none() {
                        " (the source's proof is judged at link)"
                    } else {
                        ""
                    }
                ),
            )
        })
        .collect()
}

/// The source maturity of one organism capability.
fn source_maturity(
    g: &Graph,
    x: &Growth,
    promotions: &[PromotionGrowth],
    sources: &Sources,
) -> SourceMaturity {
    let cid = NodeId::of(SYSTEM, &crate::graph::system_capability_key(&x.key));
    let promoted: Vec<&PromotionGrowth> = promotions
        .iter()
        .filter(|p| p.destination == x.key)
        .collect();
    let mut fed = !promoted.is_empty();
    let mut proven = promoted.iter().any(|p| p.source_proven == Some(true));
    for e in g
        .edges
        .iter()
        .filter(|e| e.to == cid && matches!(e.kind, EdgeKind::Feeds | EdgeKind::Teaches))
    {
        match g.nodes.get(&e.from) {
            Some(n) if n.repository == NORL_SHARD => {}
            Some(n) => {
                fed = true;
                proven |= !n.evidence.is_empty()
                    && n.evidence.iter().all(|l| {
                        sources.proofs.get(&(n.repository.clone(), l.clone()))
                            == Some(&Verdict::Pass)
                    });
            }
            None => fed = true,
        }
    }
    if proven {
        SourceMaturity::ProvenInSource
    } else if fed {
        SourceMaturity::Fed
    } else {
        SourceMaturity::Unfed
    }
}

/// The shard-local promotion gates: legal only in norl, well-formed, carrying provenance where
/// code crosses, and naming semantics (never file paths) wherever something feeds Norl.
pub fn check_promotions(d: &Declaration, g: &Graph, findings: &mut Vec<Finding>) {
    let shard = d.repository.shard.as_str();
    let o = &d.organism;
    if shard != NORL_SHARD && !o.promotions.is_empty() {
        err(
            findings,
            "PROMOTION_OUTSIDE_NORL",
            "organism",
            format!(
                "`{shard}` declares {} promotions; only norl promotes a capability into itself",
                o.promotions.len()
            ),
        );
    }
    let mut seen = BTreeSet::new();
    for p in &o.promotions {
        let mut invalid = Vec::new();
        if !seen.insert(p.key.as_str()) {
            invalid.push("its key is declared twice".to_string());
        }
        if crate::protocol::shard(&p.source_repository).is_none()
            || p.source_repository == NORL_SHARD
        {
            invalid.push(format!(
                "source repository `{}` is not another canonical shard",
                p.source_repository
            ));
        }
        if !PROMOTION_RELATIONS.contains(&p.relation) {
            invalid.push(format!("relation {} is not FEEDS or TEACHES", p.relation));
        }
        if !o
            .capabilities
            .iter()
            .any(|c| c.key == p.destination_capability)
        {
            invalid.push(format!(
                "`{}` is not an organism capability",
                p.destination_capability
            ));
        }
        if !invalid.is_empty() {
            err(findings, "PROMOTION_INVALID", &p.key, invalid.join("; "));
        }
        let copied = matches!(
            p.implementation,
            PromotionImplementation::CopiedAndDiverged
                | PromotionImplementation::AdaptedImplementation
        );
        if copied && p.destination_node.is_some() {
            let missing: Vec<&str> = [
                ("source_repository", p.source_repository.trim().is_empty()),
                ("source_node", p.source_node.trim().is_empty()),
                ("source_commit", !is_commit(&p.source_commit)),
                ("source_technology", p.source_technology.is_none()),
            ]
            .into_iter()
            .filter(|(_, m)| *m)
            .map(|(f, _)| f)
            .collect();
            if !missing.is_empty() {
                err(
                    findings,
                    "PROMOTION_WITHOUT_PROVENANCE",
                    &p.key,
                    format!(
                        "{} code in `{}` must carry its source provenance; missing {}",
                        p.implementation,
                        p.destination_node.as_deref().unwrap_or(""),
                        missing.join(", ")
                    ),
                );
            }
        }
        // A materialized REUSES is a shared implementation, whatever the claim.
        let reused = p
            .destination_node
            .as_deref()
            .zip(p.source_technology.as_deref())
            .is_some_and(|(n, t)| {
                let (nid, tid) = (
                    NodeId::of(SYSTEM, n),
                    NodeId::of(SYSTEM, &crate::graph::technology_key(t)),
                );
                g.edges
                    .iter()
                    .any(|e| e.from == nid && e.to == tid && e.kind == EdgeKind::Reuses)
            });
        if reused && p.implementation != PromotionImplementation::SharedImplementation {
            err(
                findings,
                "PROMOTION_IMPLEMENTATION_MISMATCH",
                &p.key,
                format!(
                    "`{}` REUSES the source technology: that is a SHARED_IMPLEMENTATION, not {}",
                    p.destination_node.as_deref().unwrap_or(""),
                    p.implementation
                ),
            );
        }
        for (field, value) in [
            ("source_node", Some(p.source_node.as_str())),
            ("source_capability", p.source_capability.as_deref()),
            ("source_technology", p.source_technology.as_deref()),
            (
                "destination_capability",
                Some(p.destination_capability.as_str()),
            ),
            ("destination_node", p.destination_node.as_deref()),
        ] {
            if let Some(v) = value.filter(|v| names_path(v)) {
                err(
                    findings,
                    "FEED_NAMES_PATH",
                    &p.key,
                    format!("{field} `{v}` is a file path; a promotion names capabilities, technologies, materials, evidence or protocols"),
                );
            }
        }
    }
    // Every FEEDS/TEACHES target of the shard names semantics, never a path.
    let feeds = d
        .repository
        .edges
        .iter()
        .filter(|e| matches!(e.kind, EdgeKind::Feeds | EdgeKind::Teaches))
        .map(|e| (e.from.clone(), e.to.clone()))
        .chain(
            d.technologies
                .iter()
                .filter_map(|t| Some((t.key.clone(), t.norl.feeds()?.to_string()))),
        )
        .chain(d.donors.iter().flat_map(|dn| {
            dn.capabilities.iter().filter_map(|c| {
                Some((format!("{}/{}", dn.key, c.key), c.norl.feeds()?.to_string()))
            })
        }));
    for (from, to) in feeds {
        if names_path(&to) {
            err(
                findings,
                "FEED_NAMES_PATH",
                &from,
                format!("feeds `{to}`, a file path: FEEDS and TEACHES name a capability or organ"),
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The organism report: what feeds Norl, from where, and how far each capability has grown.

/// One material of the graph.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialView {
    pub key: String,
    pub concept: Concept,
    pub shard: String,
    /// `<node>@<shard>` that GENERATE it.
    pub generators: Vec<String>,
    /// (edge kind, target) of its outbound developmental edges.
    pub relations: Vec<(EdgeKind, String)>,
    /// Whether it reaches the organism (a feed, a curriculum, an evaluation or an observed or
    /// acted-on world of Norl, or material such material uses).
    pub feeds_norl: bool,
}

/// One feed into the organism: source (material, technology or capability) → Norl node.
#[derive(Clone, Debug, PartialEq)]
pub struct Feed {
    pub kind: EdgeKind,
    pub source: String,
    pub source_kind: NodeKind,
    pub concept: Concept,
    pub shard: String,
    /// What generates the source (materials only), `<node>@<shard>`.
    pub generators: Vec<String>,
    pub target: String,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Report {
    /// (organ key, name).
    pub organs: Vec<(String, String)>,
    pub growth: Vec<Growth>,
    pub materials: Vec<MaterialView>,
    pub feeds: Vec<Feed>,
    pub promotions: Vec<PromotionGrowth>,
}

/// The keys a declaration names, by id: labels for endpoints that live in other shards.
pub fn referenced_keys(d: &Declaration) -> BTreeMap<NodeId, String> {
    let mut m = BTreeMap::new();
    let mut add = |k: &str| {
        m.insert(NodeId::of(SYSTEM, k), k.to_string());
    };
    for e in &d.repository.edges {
        for k in [&e.from, &e.to] {
            if !k.contains(':') {
                add(k);
            }
        }
    }
    for k in d.technologies.iter().filter_map(|t| t.norl.feeds()).chain(
        d.donors
            .iter()
            .flat_map(|dn| dn.capabilities.iter().filter_map(|c| c.norl.feeds())),
    ) {
        add(&crate::graph::system_capability_key(k));
    }
    for c in &d.organism.capabilities {
        add(&c.organ);
        for e in &c.evaluations {
            add(e);
        }
    }
    for p in &d.organism.promotions {
        add(&promotion_source_key(p));
    }
    m
}

/// The graph key a promotion's edge starts from: its source technology, else its source
/// capability, else its source node.
pub fn promotion_source_key(p: &Promotion) -> String {
    match (&p.source_technology, &p.source_capability) {
        (Some(t), _) => crate::graph::technology_key(t),
        (None, Some(c)) => crate::graph::system_capability_key(c),
        (None, None) => p.source_node.clone(),
    }
}

/// Builds the organism report from a graph (one shard's or the linked system's); `labels` names
/// endpoints the graph only references (see [`referenced_keys`]).
pub fn report(
    g: &Graph,
    o: &Organism,
    ev: &Evaluations,
    labels: &BTreeMap<NodeId, String>,
    sources: &Sources,
) -> Report {
    let label = |id: &NodeId| {
        g.nodes
            .get(id)
            .map(|n| n.semantic_key.clone())
            .or_else(|| labels.get(id).cloned())
            .unwrap_or_else(|| id.to_string())
    };
    // A FEEDS/TEACHES target counts as Norl's when it is a norl node, or lives in a shard not
    // linked here (resolved — or rejected — by the linker).
    let norl_target = |id: NodeId| is_norl_node(g, id) || !g.nodes.contains_key(&id);
    let generators = |id: NodeId| -> Vec<String> {
        let mut v: Vec<String> = g
            .edges
            .iter()
            .filter(|e| e.to == id && e.kind == EdgeKind::Generates)
            .map(|e| shard_label(g, e.from))
            .collect();
        v.sort();
        v
    };
    let mut organs: Vec<(String, String)> = g
        .nodes
        .values()
        .filter(|n| n.kind == NodeKind::Organ && n.repository == NORL_SHARD)
        .map(|n| (n.semantic_key.clone(), n.name.clone()))
        .collect();
    organs.sort();

    let mut feeds = Vec::new();
    for e in g
        .edges
        .iter()
        .filter(|e| matches!(e.kind, EdgeKind::Feeds | EdgeKind::Teaches) && norl_target(e.to))
    {
        let Some(src) = g.nodes.get(&e.from) else {
            continue;
        };
        feeds.push(Feed {
            kind: e.kind,
            source: src.semantic_key.clone(),
            source_kind: src.kind,
            concept: src.concept,
            shard: src.repository.clone(),
            generators: if src.kind == NodeKind::Material {
                generators(src.id)
            } else {
                vec![]
            },
            target: label(&e.to),
        });
    }

    // Materials that reach the organism, then what they use, to a fixpoint.
    let mut reach: BTreeSet<NodeId> = BTreeSet::new();
    for e in &g.edges {
        let from = g.nodes.get(&e.from);
        let to_material = g
            .nodes
            .get(&e.to)
            .is_some_and(|n| n.kind == NodeKind::Material);
        match e.kind {
            EdgeKind::Feeds | EdgeKind::Teaches if norl_target(e.to) => {
                reach.insert(e.from);
            }
            EdgeKind::EvaluatedBy if to_material && is_norl_node(g, e.from) => {
                reach.insert(e.to);
            }
            EdgeKind::Observes | EdgeKind::ActsOn
                if to_material
                    && from.is_some_and(|f| {
                        f.kind == NodeKind::Organ && f.repository == NORL_SHARD
                    }) =>
            {
                reach.insert(e.to);
            }
            _ => {}
        }
    }
    loop {
        let more: Vec<NodeId> = g
            .edges
            .iter()
            .filter(|e| {
                reach.contains(&e.from)
                    && matches!(
                        e.kind,
                        EdgeKind::Uses | EdgeKind::Observes | EdgeKind::ActsOn
                    )
                    && !reach.contains(&e.to)
            })
            .map(|e| e.to)
            .collect();
        if more.is_empty() {
            break;
        }
        reach.extend(more);
    }
    let materials = g
        .nodes
        .values()
        .filter(|n| n.kind == NodeKind::Material)
        .map(|m| {
            let mut relations: Vec<(EdgeKind, String)> = g
                .edges
                .iter()
                .filter(|e| e.from == m.id && !matches!(e.kind, EdgeKind::Contains))
                .map(|e| (e.kind, label(&e.to)))
                .collect();
            relations.sort();
            MaterialView {
                key: m.semantic_key.clone(),
                concept: m.concept,
                shard: m.repository.clone(),
                generators: generators(m.id),
                relations,
                feeds_norl: reach.contains(&m.id),
            }
        })
        .collect();
    let mut growth = growth(g, o, ev);
    let promotions = promotion_growth(g, o, &growth, sources);
    for x in growth.iter_mut() {
        x.source_maturity = source_maturity(g, x, &promotions, sources);
    }
    Report {
        organs,
        growth,
        materials,
        feeds,
        promotions,
    }
}

impl Report {
    /// The organism's raw counts (see `metrics`).
    pub fn count(&self, c: &mut crate::metrics::Counts) {
        let at = |s: GrowthState| {
            self.growth
                .iter()
                .filter(|x| x.defined && x.effective == s)
                .count() as u64
        };
        c.organism_capabilities = self.growth.len() as u64;
        c.organism_undefined = self.growth.iter().filter(|x| !x.defined).count() as u64;
        c.growth_defined = at(GrowthState::Defined);
        c.growth_exposed = at(GrowthState::Exposed);
        c.growth_experienced = at(GrowthState::Experienced);
        c.growth_evaluated = at(GrowthState::Evaluated);
        c.growth_learned = at(GrowthState::Learned);
        c.growth_native = at(GrowthState::Native);
        c.organism_borrowed = self.growth.iter().filter(|x| x.borrowed).count() as u64;
        c.organism_native_backed = self
            .growth
            .iter()
            .filter(|x| {
                x.backend
                    .as_ref()
                    .is_some_and(|(k, _)| *k != BackendKind::BorrowedWeight)
            })
            .count() as u64;
        c.organism_regressed = self.growth.iter().filter(|x| x.regressed).count() as u64;
        c.organism_source_proven = self
            .growth
            .iter()
            .filter(|x| x.source_maturity == SourceMaturity::ProvenInSource)
            .count() as u64;
        let promoted = |s: PromotionState| {
            self.promotions
                .iter()
                .filter(|p| p.effective.is_some_and(|e| e >= s))
                .count() as u64
        };
        c.promotions_total = self.promotions.len() as u64;
        c.promotions_proven_in_source = promoted(PromotionState::ProvenInSource);
        c.promotions_native_implementation = promoted(PromotionState::NativeNorlImplementation);
        c.promotions_norl_native = promoted(PromotionState::NorlNative);
        c.organism_with_experience = self
            .growth
            .iter()
            .filter(|x| !x.experiences.is_empty())
            .count() as u64;
        c.organism_with_evaluation = self
            .growth
            .iter()
            .filter(|x| x.evaluations.iter().any(|(_, v)| *v == Verdict::Pass))
            .count() as u64;
        let fed = |k: Concept| {
            self.materials
                .iter()
                .filter(|m| m.feeds_norl && m.concept == k)
                .count() as u64
        };
        c.materials_worlds = fed(Concept::World);
        c.materials_observations = fed(Concept::Observation);
        c.materials_actions = fed(Concept::Action);
        c.materials_experiences = fed(Concept::Experience);
        c.materials_curricula = fed(Concept::Curriculum);
        c.materials_evaluations = fed(Concept::Evaluation);
    }
}

/// Counts the technology sharing classes into `c`.
pub fn count_sharing(sharing: &[crate::technology::Sharing], c: &mut crate::metrics::Counts) {
    use crate::schema::TechnologySharing as S;
    let n = |k: S| sharing.iter().filter(|x| x.class == k).count() as u64;
    c.sharing_shared_implementation = n(S::SharedImplementation);
    c.sharing_independent_implementation = n(S::IndependentImplementation);
    c.sharing_domain_specialization = n(S::DomainSpecialization);
    c.sharing_shared_concept = n(S::SharedConcept);
    c.sharing_unrelated_duplicate = n(S::UnrelatedDuplicate);
}

// ---------------------------------------------------------------------------------------------
// The query surface.

/// The conceptual organs of the organism. Norl declares each as an ORGAN node when it grows it.
pub const CONCEPTUAL_ORGANS: &[&str] = &[
    "Cognition",
    "Memory",
    "WorldModel",
    "Planning",
    "Learning",
    "Action",
    "Perception",
    "Identity",
    "AuthorityInterface",
    "Evaluation",
    "Lifecycle",
];

pub const WHAT_NORL_IS: &str = "Norl is the native digital organism of Chronica (shard norl, llgtrn/Norl). Every other shard stays an independent system; it offers Norl developmental material — worlds, observations, actions, experiences, curricula and evaluations — placed in the canonical graph by GENERATES/FEEDS/TEACHES/EVALUATED_BY edges, never code. A capability is Norl's only with independent evidence, and the coding agent is never one.";

fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
}

/// A donor as the organism view reports it.
#[derive(Clone, Debug, PartialEq)]
pub struct DonorView {
    pub shard: String,
    pub key: String,
    pub effective: crate::schema::DonorState,
    pub runtime: bool,
}

/// A technology as the organism view reports it.
#[derive(Clone, Debug, PartialEq)]
pub struct TechnologyView {
    pub key: String,
    pub shard: String,
    /// `<donor key> (<state>)`.
    pub lineage: Vec<String>,
    pub feeds: Vec<String>,
}

/// Everything `ynventa organism` answers.
pub struct View<'a> {
    pub report: &'a Report,
    pub technologies: Vec<TechnologyView>,
    pub sharing: &'a [crate::technology::Sharing],
    pub donors: Vec<DonorView>,
    /// Where each technology key is implemented (its shard).
    pub tech_shard: BTreeMap<String, String>,
}

impl View<'_> {
    pub fn render_text(&self) -> String {
        let r = self.report;
        let mut s = format!(
            "NORL\n  {WHAT_NORL_IS}\n\nORGANS ({} declared)\n",
            r.organs.len()
        );
        for (k, name) in &r.organs {
            let caps: Vec<String> = r
                .growth
                .iter()
                .filter(|g| &g.organ == k)
                .map(|g| format!("{} {}", g.key, g.effective))
                .collect();
            s.push_str(&format!(
                "  {k:<28} {name:<20} {}\n",
                if caps.is_empty() {
                    "(no capability yet)".to_string()
                } else {
                    caps.join(", ")
                }
            ));
        }
        let missing: Vec<&str> = CONCEPTUAL_ORGANS
            .iter()
            .copied()
            .filter(|o| {
                !r.organs.iter().any(|(k, name)| {
                    norm(name) == norm(o) || norm(k.rsplit('.').next().unwrap_or(k)) == norm(o)
                })
            })
            .collect();
        s.push_str(&format!(
            "  conceptual organs not yet declared: {}\n",
            if missing.is_empty() {
                "none".to_string()
            } else {
                missing.join(", ")
            }
        ));
        s.push_str("\nCAPABILITIES (claimed -> norl maturity, the effective growth; source maturity beside it, never part of it; cognition source)\n");
        for g in &r.growth {
            s.push_str(&format!(
                "  {:<28} {:<20} {:<11} -> {:<11}{}{} source {} {}\n",
                g.key,
                g.organ,
                g.claimed.wire(),
                if g.defined {
                    g.effective.wire()
                } else {
                    "UNDEFINED"
                },
                if g.borrowed { " BORROWED" } else { "" },
                if g.regressed { " REGRESSED" } else { "" },
                g.source_maturity.wire(),
                match &g.backend {
                    None => "backend: none".to_string(),
                    Some((k, n)) => format!("backend: {k} in {n}"),
                }
            ));
            if g.effective < GrowthState::Native && !g.stopped_by.is_empty() {
                s.push_str(&format!("      next: {}\n", g.stopped_by));
            }
        }
        let list = |pred: &dyn Fn(&Growth) -> bool| -> String {
            let v: Vec<&str> = r
                .growth
                .iter()
                .filter(|g| pred(g))
                .map(|g| g.key.as_str())
                .collect();
            if v.is_empty() {
                "none".into()
            } else {
                v.join(", ")
            }
        };
        s.push_str(&format!(
            "  native: {}\n  borrowed: {}\n  missing (not yet native): {}\n",
            list(&|g| g.defined && g.effective == GrowthState::Native),
            list(&|g| g.borrowed),
            list(&|g| !g.defined || g.effective < GrowthState::Native),
        ));

        s.push_str("\nPROMOTIONS (source repository's proof -> Norl's own implementation; claimed -> effective)\n");
        if r.promotions.is_empty() {
            s.push_str("  (none)\n");
        }
        for p in &r.promotions {
            s.push_str(&format!(
                "  {:<28} {} -> {} [{}] {} -> {}{}\n",
                p.key,
                p.source,
                p.destination,
                p.implementation.wire(),
                p.claimed.wire(),
                p.effective.map(|e| e.wire()).unwrap_or("NONE"),
                match p.source_proven {
                    None => " (source proof judged at link)",
                    Some(true) => "",
                    Some(false) => " (source not proven)",
                }
            ));
            if p.effective < Some(PromotionState::NorlNative) && !p.stopped_by.is_empty() {
                s.push_str(&format!("      next: {}\n", p.stopped_by));
            }
        }

        s.push_str("\nTEACHING (feed chains: generator -> material -> Norl)\n");
        if r.feeds.is_empty() {
            s.push_str("  (no shard feeds Norl yet)\n");
        }
        for f in &r.feeds {
            s.push_str(&format!(
                "  {} <-{}- {} [{} {}] from {}{}\n",
                f.target,
                f.kind,
                f.source,
                f.source_kind,
                f.concept,
                if f.shard.is_empty() { "-" } else { &f.shard },
                if f.generators.is_empty() {
                    String::new()
                } else {
                    format!(" <- generated by {}", f.generators.join(", "))
                }
            ));
        }
        for g in r.growth.iter().filter(|g| !g.evaluations.is_empty()) {
            s.push_str(&format!(
                "  {} judged by {}\n",
                g.key,
                g.evaluations
                    .iter()
                    .map(|(k, v)| format!("{k} {}", v.wire()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        s.push_str("\nMATERIAL (by concept; * reaches Norl)\n");
        for c in crate::graph::MATERIAL_CONCEPTS {
            let ms: Vec<String> = r
                .materials
                .iter()
                .filter(|m| m.concept == *c)
                .map(|m| {
                    format!(
                        "{}{}@{}",
                        if m.feeds_norl { "*" } else { "" },
                        m.key,
                        m.shard
                    )
                })
                .collect();
            s.push_str(&format!(
                "  {:<12} {}\n",
                c.wire(),
                if ms.is_empty() {
                    "-".to_string()
                } else {
                    ms.join(", ")
                }
            ));
        }
        s.push_str("\nNUTRITION PER TEACHING SHARD (materials reaching Norl)\n");
        let mut shards: BTreeSet<&str> = BTreeSet::new();
        for m in r.materials.iter().filter(|m| m.feeds_norl) {
            shards.insert(&m.shard);
        }
        if shards.is_empty() {
            s.push_str("  (none)\n");
        }
        for sh in shards {
            let row: Vec<String> = crate::graph::MATERIAL_CONCEPTS
                .iter()
                .map(|c| {
                    format!(
                        "{} {}",
                        c.wire().to_ascii_lowercase(),
                        r.materials
                            .iter()
                            .filter(|m| m.feeds_norl && m.shard == sh && m.concept == *c)
                            .count()
                    )
                })
                .collect();
            s.push_str(&format!("  {sh:<20} {}\n", row.join(", ")));
        }
        s.push_str("\nPER ORGAN (capabilities by effective state; borrowed; regressed)\n");
        for (k, _) in &r.organs {
            let gs: Vec<&Growth> = r.growth.iter().filter(|g| &g.organ == k).collect();
            let by: Vec<String> = GrowthState::ALL
                .iter()
                .map(|st| {
                    format!(
                        "{} {}",
                        st.wire().to_ascii_lowercase(),
                        gs.iter()
                            .filter(|g| g.defined && g.effective == *st)
                            .count()
                    )
                })
                .collect();
            s.push_str(&format!(
                "  {k:<28} {}; borrowed {}; regressed {}\n",
                by.join(", "),
                gs.iter().filter(|g| g.borrowed).count(),
                gs.iter().filter(|g| g.regressed).count()
            ));
        }

        s.push_str("\nSHARED TECHNOLOGIES (where each is implemented)\n");
        if self.sharing.is_empty() {
            s.push_str("  (none)\n");
        }
        let at = |k: &str| {
            self.tech_shard.get(k).cloned().unwrap_or_else(|| {
                k.rsplit_once('@')
                    .map(|x| x.1.to_string())
                    .unwrap_or_default()
            })
        };
        for x in self.sharing {
            s.push_str(&format!(
                "  {:<26} {}@{} ~ {}{} ({})\n",
                x.class.wire(),
                x.a,
                at(&x.a),
                x.b,
                if x.b.contains('@') {
                    String::new()
                } else {
                    format!("@{}", at(&x.b))
                },
                x.capabilities.join(", ")
            ));
        }
        s.push_str("\nTECHNOLOGY LINEAGE (which donor taught each technology; what it feeds)\n");
        for t in &self.technologies {
            s.push_str(&format!(
                "  {:<32} {:<16} learned from {}; feeds {}\n",
                t.key,
                t.shard,
                if t.lineage.is_empty() {
                    "-".to_string()
                } else {
                    t.lineage.join(", ")
                },
                if t.feeds.is_empty() {
                    "-".to_string()
                } else {
                    t.feeds.join(", ")
                }
            ));
        }
        let donors = |runtime: bool, extinct: bool| -> String {
            let v: Vec<String> = self
                .donors
                .iter()
                .filter(|d| {
                    if extinct {
                        d.effective == crate::schema::DonorState::Extinct
                    } else {
                        d.runtime == runtime && d.effective != crate::schema::DonorState::Extinct
                    }
                })
                .map(|d| format!("{}:{} ({})", d.shard, d.key, d.effective))
                .collect();
            if v.is_empty() {
                "none".into()
            } else {
                format!("{}: {}", v.len(), v.join(", "))
            }
        };
        s.push_str(&format!(
            "\nDONORS\n  runtime dependencies: {}\n  extinct: {}\n",
            donors(true, false),
            donors(false, true)
        ));
        s
    }

    pub fn to_json(&self) -> crate::formats::json::Json {
        use crate::formats::json::Json;
        let r = self.report;
        Json::obj()
            .with("schema", crate::protocol::schema_identity())
            .with("norl", WHAT_NORL_IS)
            .with(
                "organs",
                Json::Array(
                    r.organs
                        .iter()
                        .map(|(k, n)| Json::obj().with("key", k).with("name", n))
                        .collect(),
                ),
            )
            .with(
                "capabilities",
                Json::Array(r.growth.iter().map(growth_json).collect()),
            )
            .with(
                "promotions",
                Json::Array(
                    r.promotions
                        .iter()
                        .map(|p| {
                            Json::obj()
                                .with("key", &p.key)
                                .with("source", &p.source)
                                .with("destination", &p.destination)
                                .with("implementation", p.implementation.wire())
                                .with("claimed", p.claimed.wire())
                                .with("effective", p.effective.map(|e| e.wire()).unwrap_or("NONE"))
                                .with(
                                    "source_proven",
                                    match p.source_proven {
                                        None => Json::Null,
                                        Some(b) => Json::Bool(b),
                                    },
                                )
                                .with("stopped_by", &p.stopped_by)
                        })
                        .collect(),
                ),
            )
            .with(
                "feeds",
                Json::Array(
                    r.feeds
                        .iter()
                        .map(|f| {
                            Json::obj()
                                .with("kind", f.kind.wire())
                                .with("source", &f.source)
                                .with("source_kind", f.source_kind.wire())
                                .with("concept", f.concept.wire())
                                .with("shard", &f.shard)
                                .with("generators", f.generators.clone())
                                .with("target", &f.target)
                        })
                        .collect(),
                ),
            )
            .with(
                "materials",
                Json::Array(
                    r.materials
                        .iter()
                        .map(|m| {
                            Json::obj()
                                .with("key", &m.key)
                                .with("concept", m.concept.wire())
                                .with("shard", &m.shard)
                                .with("generators", m.generators.clone())
                                .with("reaches_norl", m.feeds_norl)
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
                "technologies",
                Json::Array(
                    self.technologies
                        .iter()
                        .map(|t| {
                            Json::obj()
                                .with("key", &t.key)
                                .with("shard", &t.shard)
                                .with("lineage", t.lineage.clone())
                                .with("feeds", t.feeds.clone())
                        })
                        .collect(),
                ),
            )
            .with(
                "donors",
                Json::Array(
                    self.donors
                        .iter()
                        .map(|d| {
                            Json::obj()
                                .with("shard", &d.shard)
                                .with("key", &d.key)
                                .with("effective", d.effective.wire())
                                .with("runtime_dependency", d.runtime)
                        })
                        .collect(),
                ),
            )
    }
}

fn growth_json(g: &Growth) -> crate::formats::json::Json {
    use crate::formats::json::Json;
    Json::obj()
        .with("key", &g.key)
        .with("organ", &g.organ)
        .with("claimed", g.claimed.wire())
        .with("defined", g.defined)
        .with("effective", g.effective.wire())
        .with(
            "norl_maturity",
            if g.defined {
                g.effective.wire()
            } else {
                "UNDEFINED"
            },
        )
        .with("source_maturity", g.source_maturity.wire())
        .with("borrowed", g.borrowed)
        .with("regressed", g.regressed)
        .with(
            "backend",
            g.backend
                .as_ref()
                .map(|(k, n)| format!("{k} {n}"))
                .unwrap_or_default(),
        )
        .with("experiences", g.experiences.clone())
        .with(
            "evaluations",
            g.evaluations
                .iter()
                .map(|(k, v)| format!("{k} {}", v.wire()))
                .collect::<Vec<_>>(),
        )
        .with("stopped_by", &g.stopped_by)
}

/// What a change taught or provided to Norl: material, feeds and growth, old → new.
pub fn diff(old: &Report, new: &Report) -> String {
    let mut s = String::from("WHAT THE CHANGE TAUGHT OR PROVIDED TO NORL\n");
    let mat = |r: &Report| -> BTreeMap<String, (Concept, bool)> {
        r.materials
            .iter()
            .map(|m| (format!("{}@{}", m.key, m.shard), (m.concept, m.feeds_norl)))
            .collect()
    };
    let (mo, mn) = (mat(old), mat(new));
    let mut lines = Vec::new();
    for (k, (c, f)) in &mn {
        match mo.get(k) {
            None => lines.push(format!(
                "  + material {k} [{c}]{}",
                if *f { " reaches Norl" } else { "" }
            )),
            Some((_, was)) if was != f => lines.push(format!(
                "  ~ material {k} [{c}] {}",
                if *f {
                    "now reaches Norl"
                } else {
                    "no longer reaches Norl"
                }
            )),
            _ => {}
        }
    }
    for (k, (c, _)) in &mo {
        if !mn.contains_key(k) {
            lines.push(format!("  - material {k} [{c}]"));
        }
    }
    let feed = |r: &Report| -> BTreeSet<String> {
        r.feeds
            .iter()
            .map(|f| format!("{} -{}-> {} (from {})", f.source, f.kind, f.target, f.shard))
            .collect()
    };
    let (fo, fnew) = (feed(old), feed(new));
    for f in fnew.difference(&fo) {
        lines.push(format!("  + feed {f}"));
    }
    for f in fo.difference(&fnew) {
        lines.push(format!("  - feed {f}"));
    }
    let state = |r: &Report| -> BTreeMap<String, String> {
        r.growth
            .iter()
            .map(|g| {
                (
                    g.key.clone(),
                    format!(
                        "{}{}{}",
                        if g.defined {
                            g.effective.wire()
                        } else {
                            "UNDEFINED"
                        },
                        if g.borrowed { " BORROWED" } else { "" },
                        if g.regressed { " REGRESSED" } else { "" }
                    ),
                )
            })
            .collect()
    };
    let (go, gn) = (state(old), state(new));
    for (k, v) in &gn {
        match go.get(k) {
            None => lines.push(format!("  + capability {k} {v}")),
            Some(o) if o != v => lines.push(format!("  ~ capability {k} {o} -> {v}")),
            _ => {}
        }
    }
    for k in go.keys() {
        if !gn.contains_key(k) {
            lines.push(format!("  - capability {k}"));
        }
    }
    if lines.is_empty() {
        s.push_str("  nothing: no material, feed or growth state changed\n");
    } else {
        for l in lines {
            s.push_str(&l);
            s.push('\n');
        }
    }
    s
}
