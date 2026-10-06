//! Agent context: what an agent working inside ECDEV must know before it builds anything
//! substantial — which nodes exist, what they provide, which canonical technologies to reuse
//! instead of re-inventing, the current migration and extinction state, donor capability
//! relevance, and the current milestones and decisions of the knowledge store. Generated from
//! the graph, never hand-written.

use crate::declare::Relevance;
use crate::schema::{DonorState, TechnologyLifecycle, WaveStatus};
use crate::Assessment;

/// Subjects per kind shown in the knowledge section of the context.
pub const CONTEXT_KNOWLEDGE_LIMIT: usize = 40;

pub fn render(a: &Assessment) -> String {
    let d = &a.declaration;
    let mut s = format!(
        "REPOSITORY\n  {} ({}) head {}; ECDEV governance v{}, schema {}\n\n",
        d.repository.name,
        d.repository.origin,
        crate::repository::files::head_commit(&a.root).unwrap_or_else(|| "-".into()),
        crate::GOVERNANCE_VERSION,
        crate::protocol::schema_identity(),
    );
    s.push_str(&format!(
        "NODES ({} nodes; identities: ecdev://graph/ecdev/<key>)\n",
        d.repository.nodes.len()
    ));
    for n in &d.repository.nodes {
        s.push_str(&format!(
            "  {:<36} {:<12} {}{}\n",
            n.key,
            n.kind.wire(),
            n.path,
            if n.path != n.canonical_path {
                format!(" -> {}", n.canonical_path)
            } else {
                String::new()
            }
        ));
    }
    let provides: Vec<&String> = d
        .repository
        .nodes
        .iter()
        .flat_map(|n| &n.provides)
        .collect();
    let requires: Vec<&String> = d
        .repository
        .nodes
        .iter()
        .flat_map(|n| &n.requires)
        .filter(|c| !provides.contains(c))
        .collect();
    s.push_str(&format!(
        "\nPROVIDES\n  {}\n",
        if provides.is_empty() {
            "(none declared)".into()
        } else {
            provides
                .iter()
                .map(|x| x.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        }
    ));
    s.push_str("\nREQUIRES (capabilities no node provides)\n");
    if requires.is_empty() {
        s.push_str("  (none)\n");
    }
    for r in &requires {
        s.push_str(&format!("  {r} <- UNPROVIDED\n"));
    }
    s.push_str("\nCANONICAL TECHNOLOGIES (reuse natively; never re-implement)\n");
    for t in a.technologies.iter().filter(|t| {
        t.effective >= TechnologyLifecycle::Canonical
            && t.effective != TechnologyLifecycle::Superseded
    }) {
        s.push_str(&format!("  {:<36} {}\n", t.key, t.effective.wire()));
    }
    s.push_str(&relevance_summary(a));
    let applied = d
        .migration
        .waves
        .iter()
        .filter(|w| w.status == WaveStatus::Applied)
        .count();
    s.push_str(&format!(
        "\nMIGRATION STATE\n  waves applied {applied}/{}; legacy nodes remaining {}; active shims {}\n",
        d.migration.waves.len(),
        a.metric("legacy_nodes_remaining"),
        a.migration.shims.iter().filter(|x| x.present).count(),
    ));
    let extinct = a
        .analysis
        .donors
        .iter()
        .filter(|x| x.effective == DonorState::Extinct)
        .count();
    s.push_str(&format!(
        "\nEXTINCTION STATE\n  donors extinct {extinct} / registered {}; external technology edges {}; extinction ratio {}\n",
        a.metric("donors_registered"),
        a.metric("external_technology_edges"),
        a.metric("extinction_ratio")
    ));
    s.push('\n');
    s.push_str(&crate::compact::view::summary(
        &a.knowledge,
        CONTEXT_KNOWLEDGE_LIMIT,
    ));
    s.push_str("\nBEFORE BUILDING INFRASTRUCTURE\n  ecdev-gov technology search <need>  ·  ecdev-gov show <key>  ·  never `cargo add` a donor\n");
    s
}

/// How ECDEV relates to every declared donor capability: relied on (must become native) or
/// justified as not relevant, and what remains unresolved.
pub fn relevance_summary(a: &Assessment) -> String {
    let caps: Vec<&crate::declare::Capability> = a
        .declaration
        .donors
        .iter()
        .flat_map(|dn| dn.capabilities.iter())
        .collect();
    let n = |f: &dyn Fn(&crate::declare::Capability) -> bool| caps.iter().filter(|c| f(c)).count();
    format!(
        "\nDONOR CAPABILITY ADOPTION (production)\n  relied on {}, not adopted {}, unresolved {}\nDONOR CAPABILITY KNOWLEDGE (research, includes runtime-rejected donors)\n  open {} (unreviewed {}, study {}, benchmark {}, algorithm {}), reference only {}, study complete {}, no research value {}\n",
        n(&|c| matches!(c.relevance, Relevance::ReliedOn(_))),
        n(&|c| matches!(c.relevance, Relevance::NotAdopted(_))),
        n(&|c| matches!(c.relevance, Relevance::Unresolved)),
        n(&|c| c.knowledge.open()),
        n(&|c| c.knowledge.status() == "UNREVIEWED"),
        n(&|c| c.knowledge.status() == "STUDY_CANDIDATE"),
        n(&|c| c.knowledge.status() == "BENCHMARK_CANDIDATE"),
        n(&|c| c.knowledge.status() == "ALGORITHM_CANDIDATE"),
        n(&|c| c.knowledge.status() == "REFERENCE_ONLY"),
        n(&|c| c.knowledge.status() == "STUDY_COMPLETE"),
        n(&|c| c.knowledge.status() == "NO_RESEARCH_VALUE"),
    )
}
