//! Agent global context: what an agent working inside one shard must know about the rest of
//! Chronica before it builds anything substantial — what exists, who physically holds it, how
//! to depend on it, which canonical technologies it should reuse instead of re-inventing, and
//! the current migration and extinction state, and the current milestones and decisions of the
//! knowledge store. Generated from the graph, never hand-written.

use crate::linker::SystemImage;
use crate::schema::{DonorState, TechnologyLifecycle, WaveStatus};
use crate::Assessment;

/// Subjects per kind shown in the knowledge section of the context.
pub const CONTEXT_KNOWLEDGE_LIMIT: usize = 40;

pub fn render(a: &Assessment, system: Option<&SystemImage>) -> String {
    let d = &a.declaration;
    let shard = &d.repository.shard;
    let mut s = format!(
        "SYSTEM\n  {} (Ynventa protocol v{}, schema {})\n\nCURRENT PHYSICAL SHARD\n  {} ({}) head {}\n\n",
        d.repository.system,
        crate::YNVENTA_PROTOCOL_VERSION,
        crate::protocol::schema_identity(),
        shard,
        d.repository.origin,
        crate::repository::files::head_commit(&a.root).unwrap_or_else(|| "-".into())
    );
    s.push_str(&format!(
        "THIS SHARD OWNS ({} nodes; identities are Chronica's: ynv://chronica/<key>)\n",
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
    s.push_str("\nREQUIRES (depend on the capability, never on another shard's paths)\n");
    if requires.is_empty() {
        s.push_str("  (none declared)\n");
    }
    for r in &requires {
        let providers = system
            .and_then(|sys| sys.capabilities.iter().find(|c| &c.key == *r))
            .map(|c| {
                c.providers
                    .iter()
                    .map(|(sh, n)| format!("{n} in {sh}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| "UNPROVIDED".into());
        s.push_str(&format!("  {r} <- {providers}\n"));
    }
    if let Some(sys) = system {
        s.push_str("\nAVAILABLE GLOBAL CAPABILITIES (physical owner)\n");
        for c in sys
            .capabilities
            .iter()
            .filter(|c| c.providers.iter().any(|(sh, _)| sh != shard))
        {
            s.push_str(&format!(
                "  {:<36} {}\n",
                c.key,
                c.providers
                    .iter()
                    .map(|(sh, n)| format!("{n}@{sh}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        s.push_str("\nCANONICAL TECHNOLOGIES (reuse natively: `ynventa technology materialize <key>`; no service calls)\n");
        for t in sys.technologies.iter().filter(|t| {
            t.effective >= TechnologyLifecycle::Canonical
                && t.effective != TechnologyLifecycle::Superseded
        }) {
            s.push_str(&format!(
                "  {:<36} {:<10} implements {} (born in {})\n",
                t.key,
                t.effective.wire(),
                t.implements.join(", "),
                t.birthplace
            ));
        }
        s.push_str("\nFORBIDDEN DUPLICATION (already provided elsewhere; declare SPECIALIZES / ALTERNATIVE_FOR if you must differ)\n");
        for c in sys
            .capabilities
            .iter()
            .filter(|c| !c.providers.is_empty() && c.providers.iter().all(|(sh, _)| sh != shard))
        {
            s.push_str(&format!("  capability {}\n", c.key));
        }
        for t in sys
            .technologies
            .iter()
            .filter(|t| t.birthplace != *shard && t.effective >= TechnologyLifecycle::Native)
        {
            s.push_str(&format!(
                "  technology {} ({})\n",
                t.key,
                t.implements.join(", ")
            ));
        }
        s.push_str(&format!(
            "\nSYSTEM LINK\n  {} ({} shards, {} errors)\n",
            if sys.pass() { "PASS" } else { "FAIL" },
            sys.shards.len(),
            sys.issues
                .iter()
                .filter(|i| i.severity == crate::Severity::Error)
                .count()
        ));
    } else {
        s.push_str("\n(no system image given: pass --system target/ynventa/chronica.system.ynv for global capabilities and technologies)\n");
    }
    s.push_str(&norl_contribution(a, system));
    let applied = d
        .migration
        .waves
        .iter()
        .filter(|w| w.status == WaveStatus::Applied)
        .count();
    s.push_str(&format!(
        "\nMIGRATION STATE\n  waves applied {applied}/{}; legacy nodes remaining {}; active shims {}; .atlas present: {}\n",
        d.migration.waves.len(),
        a.metric("legacy_nodes_remaining"),
        a.migration.shims.iter().filter(|x| x.present).count(),
        a.files.exists(".atlas")
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
    s.push_str("\nBEFORE BUILDING INFRASTRUCTURE\n  ynventa technology search <need>  ·  ynventa show <key> --system <image>  ·  never `cargo add` a donor\n");
    s
}

/// What this shard contributes to the organism: its material by concept (and how much of it
/// reaches Norl), its feeds, the Norl relevance of its technologies and donor capabilities and,
/// in the norl shard, the organism's growth.
pub fn norl_contribution(a: &Assessment, system: Option<&SystemImage>) -> String {
    use crate::declare::NorlRelevance;
    let d = &a.declaration;
    let shard = d.repository.shard.as_str();
    let sys_report = system.map(|s| s.organism_report());
    let report = sys_report.as_ref().unwrap_or(&a.organism);
    let mut s = format!(
        "\nNORL CONTRIBUTION ({})\n",
        if system.is_some() {
            "judged on the linked system"
        } else {
            "this shard alone; pass --system for the linked judgement"
        }
    );
    let mine: Vec<&crate::organism::MaterialView> = report
        .materials
        .iter()
        .filter(|m| m.shard == shard)
        .collect();
    let row: Vec<String> = crate::graph::MATERIAL_CONCEPTS
        .iter()
        .map(|c| {
            let all = mine.iter().filter(|m| m.concept == *c).count();
            let fed = mine
                .iter()
                .filter(|m| m.concept == *c && m.feeds_norl)
                .count();
            format!("{} {fed}/{all}", c.wire().to_ascii_lowercase())
        })
        .collect();
    s.push_str(&format!(
        "  material reaching Norl / declared: {}\n",
        row.join(", ")
    ));
    let feeds: Vec<String> = report
        .feeds
        .iter()
        .filter(|f| f.shard == shard)
        .map(|f| format!("{} -{}-> {}", f.source, f.kind, f.target))
        .collect();
    s.push_str(&format!(
        "  feeds: {}\n",
        if feeds.is_empty() {
            "none (offer MATERIAL that FEEDS or TEACHES a Norl capability)".to_string()
        } else {
            format!("{}: {}", feeds.len(), feeds.join("; "))
        }
    ));
    let rel = |n: &NorlRelevance| match n {
        NorlRelevance::Unresolved => 0,
        NorlRelevance::Feeds(_) => 1,
        NorlRelevance::NotRelevant(_) => 2,
    };
    let count = |v: &[usize], k: usize| v.iter().filter(|x| **x == k).count();
    let techs: Vec<usize> = d.technologies.iter().map(|t| rel(&t.norl)).collect();
    let caps: Vec<usize> = d
        .donors
        .iter()
        .flat_map(|dn| dn.capabilities.iter().filter(|c| c.required))
        .map(|c| rel(&c.norl))
        .collect();
    s.push_str(&format!(
        "  technologies: {} feed Norl, {} not relevant, {} unresolved\n  donor capabilities: {} feed Norl, {} not relevant, {} unresolved\n",
        count(&techs, 1),
        count(&techs, 2),
        count(&techs, 0),
        count(&caps, 1),
        count(&caps, 2),
        count(&caps, 0)
    ));
    if shard == crate::schema::NORL_SHARD || !d.organism.is_empty() {
        let by: Vec<String> = crate::schema::GrowthState::ALL
            .iter()
            .map(|st| {
                format!(
                    "{} {}",
                    st.wire().to_ascii_lowercase(),
                    report
                        .growth
                        .iter()
                        .filter(|g| g.defined && g.effective == *st)
                        .count()
                )
            })
            .collect();
        s.push_str(&format!(
            "  organism: {} organs, {} capabilities ({}); borrowed {}, regressed {}\n",
            report.organs.len(),
            report.growth.len(),
            by.join(", "),
            report.growth.iter().filter(|g| g.borrowed).count(),
            report.growth.iter().filter(|g| g.regressed).count()
        ));
    }
    s
}
