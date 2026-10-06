//! ECDEV governance — the repository graph, donor lifecycle, evidence store and repository
//! conformance owned by ECDEV, kept at `.ecdev/`.
//!
//! One assessment of a repository root drives every command:
//!
//! ```text
//! .ecdev/declared/*.rs ─┐                        ┌─ graph (stable NodeIds, typed edges)
//! committed files ────────┼─► census ─► analysis ──┼─ donor lifecycle + extinction gates
//! .ecdev/evidence ──────┤                        ├─ shape + document budget
//! .ecdev/history ───────┤                        ├─ migration waves + shims
//! .ecdev/knowledge ─────┘                        └─ metrics (one schema, one formula set)
//! ```
//!
//! Zero dependencies; no network; no dependency on any other repository.

pub mod audit;
pub mod census;
pub mod cli;
pub mod compact;
pub mod conformance;
pub mod context;
pub mod declare;
pub mod digest;
pub mod donors;
pub mod evidence;
pub mod extinction;
pub mod formats;
pub mod graph;
pub mod licence;
pub mod metrics;
pub mod migration;
pub mod protocol;
pub mod repository;
pub mod schema;
pub mod technology;
pub mod universe;

pub use protocol::GOVERNANCE_VERSION;

use crate::formats::json::Json;
use crate::graph::{Graph, NodeId};
use crate::schema::{DonorState, EdgeKind, ExceptionKind, NodeKind, Scope};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

impl Severity {
    pub fn wire(self) -> &'static str {
        match self {
            Severity::Error => "ERROR",
            Severity::Warning => "WARNING",
            Severity::Info => "INFO",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Finding {
    pub severity: Severity,
    pub code: String,
    pub subject: String,
    pub detail: String,
}

impl Finding {
    pub fn new(severity: Severity, code: &str, subject: &str, detail: &str) -> Finding {
        Finding {
            severity,
            code: code.to_string(),
            subject: subject.to_string(),
            detail: detail.to_string(),
        }
    }
    pub fn to_json(&self) -> Json {
        Json::obj()
            .with("severity", self.severity.wire())
            .with("code", &self.code)
            .with("subject", &self.subject)
            .with("detail", &self.detail)
    }
}

/// Everything known about one repository, computed once.
pub struct Assessment {
    pub root: PathBuf,
    pub files: repository::files::Files,
    pub declaration: declare::Declaration,
    pub census: census::Census,
    pub evidence: evidence::Store,
    pub history: compact::history::History,
    pub knowledge: compact::facts::Knowledge,
    pub graph: Graph,
    pub analysis: donors::Analysis,
    pub technologies: Vec<technology::TechnologyAssessment>,
    pub docs: audit::DocAudit,
    pub shape: repository::shape::ShapeReport,
    pub migration: migration::Status,
    /// The repository's universe: its relation to every global donor it knows.
    pub universe: universe::Report,
    pub findings: Vec<Finding>,
    pub counts: metrics::Counts,
}

impl Assessment {
    pub fn errors(&self) -> impl Iterator<Item = &Finding> {
        self.findings
            .iter()
            .filter(|f| f.severity == Severity::Error)
    }
    pub fn metric(&self, name: &str) -> String {
        self.counts
            .values()
            .into_iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v)
            .unwrap_or_default()
    }
    pub fn donor(&self, key: &str) -> Option<&donors::DonorAssessment> {
        self.analysis.donors.iter().find(|d| d.key == key)
    }
}

/// Assesses the repository at `root`.
pub fn assess(root: &Path) -> Result<Assessment, String> {
    let declaration = declare::load(root).map_err(|e| {
        format!(
            "{e}\n(not the ECDEV repository? {} has no readable .ecdev/declared)",
            root.display()
        )
    })?;
    let files = repository::files::Files::scan(root)
        .map_err(|e| format!("cannot scan {}: {e}", root.display()))?;
    Ok(assess_with(root, files, declaration))
}

/// Assesses with ECDEV repository conformance counted.
pub fn assess_with(
    root: &Path,
    files: repository::files::Files,
    declaration: declare::Declaration,
) -> Assessment {
    let mut a = assess_core(root, files, declaration);
    let checks = conformance::checks(&a);
    a.counts.conformance_checks = checks.len() as u64;
    a.counts.conformance_checks_passed = checks.iter().filter(|c| c.pass).count() as u64;
    a
}

/// Everything except the conformance checks (which themselves re-run this to prove determinism).
pub fn assess_core(
    root: &Path,
    files: repository::files::Files,
    declaration: declare::Declaration,
) -> Assessment {
    let census = census::run(&files, &declaration);
    let evidence = evidence::Store::load(root);
    let history = compact::history::History::load(root);
    let knowledge = compact::facts::Knowledge::load(root);
    let (mut graph, graph_issues) = Graph::from_declaration(&declaration);
    let analysis = donors::analyze(&declaration, &files, &census, &evidence, &history);
    let mut technology_findings = Vec::new();
    let technologies = technology::assess(
        &declaration,
        &files,
        &evidence,
        &analysis.node_status,
        &mut technology_findings,
    );
    let docs = audit::audit(&files, &declaration, &knowledge);
    let shape = repository::shape::check(&files, &declaration, &census);
    let migration = migration::status(&files, &declaration, &analysis);
    let universe_report = universe::assess(
        &declaration,
        &analysis,
        &universe::Universe::load(root),
        &files,
    );
    overlay(&mut graph, &declaration, &census, &analysis);
    let mut findings = Vec::new();
    for i in graph_issues {
        findings.push(Finding::new(Severity::Error, i.code, &i.subject, &i.detail));
    }
    findings.extend(analysis.findings.iter().cloned());
    findings.extend(universe_report.findings.iter().cloned());
    findings.extend(technology_findings);
    technology::check_orphans(&declaration, &graph, &technologies, &mut findings);
    // ECDEV builds alone: a Cargo path dependency that leaves the repository reaches into
    // another repository's internal crates.
    for o in census.observations.iter().filter(|o| {
        o.via == census::Via::SourceReference
            && o.ecosystem == schema::Ecosystem::Cargo
            && o.name.starts_with("../")
    }) {
        let sibling = o
            .name
            .split('/')
            .find(|s| *s != "..")
            .unwrap_or_default()
            .to_string();
        findings.push(Finding::new(
            Severity::Error,
            "CROSS_REPO_SOURCE_DEPENDENCY",
            &o.file,
            &format!(
                "`{}` is a path dependency on `{}`, outside the repository (sibling `{sibling}`): every repository builds alone; ECDEV builds alone; absorb the behaviour natively or register an external package",
                o.ident, o.name
            ),
        ));
    }
    let mut unreadable_locks = Vec::new();
    let locks = technology::Materialization::load_all(root, &mut unreadable_locks);
    technology::check_materialized(&files, &locks, &mut findings);
    for u in unreadable_locks {
        findings.push(Finding::new(
            Severity::Error,
            "CORRUPT_STATE",
            &u,
            "a materialization lock is unreadable",
        ));
    }
    findings.extend(shape.findings.iter().cloned());
    findings.extend(migration.findings.iter().cloned());
    for u in census.unreadable.iter() {
        findings.push(Finding::new(
            Severity::Warning,
            "UNREADABLE_MANIFEST",
            u,
            "could not be read or parsed",
        ));
    }
    for u in evidence
        .unreadable
        .iter()
        .chain(&history.unreadable)
        .chain(&knowledge.unreadable)
    {
        findings.push(Finding::new(
            Severity::Error,
            "CORRUPT_STATE",
            u,
            "a governance state file is unreadable or not content-addressed",
        ));
    }
    for (path, r) in &knowledge.stored {
        if let Err(e) = r {
            findings.push(Finding::new(
                Severity::Error,
                "CORRUPT_STATE",
                path,
                &format!("stored knowledge does not rebuild its text: {e}"),
            ));
        }
    }
    for doc in docs.over_budget() {
        let issues: Vec<String> = doc.issues.iter().map(|i| i.wire()).collect();
        findings.push(Finding::new(
            Severity::Warning,
            "DOCUMENT_OVER_BUDGET",
            &doc.path,
            &format!(
                "{}{}",
                issues.join("; "),
                if doc.extracted {
                    "; knowledge extracted — safe to prune"
                } else {
                    ""
                }
            ),
        ));
    }
    findings.sort();
    findings.dedup();

    let mut a = Assessment {
        root: root.to_path_buf(),
        files,
        declaration,
        census,
        evidence,
        history,
        knowledge,
        graph,
        analysis,
        technologies,
        docs,
        shape,
        migration,
        universe: universe_report,
        findings,
        counts: metrics::Counts::default(),
    };
    a.counts = count(&a);
    a
}

/// The counts of an assessment, excluding conformance checks.
pub fn count_public(a: &Assessment) -> metrics::Counts {
    count(a)
}

/// Adds census observations and computed nativeness to the declared graph.
fn overlay(g: &mut Graph, d: &declare::Declaration, c: &census::Census, a: &donors::Analysis) {
    let ns = schema::NAMESPACE.to_string();
    let repo = d.repository.id.clone();
    let index = donors::NodeIndex::new(d);
    for (key, status) in &a.node_status {
        if let Some(n) = g.nodes.get_mut(&NodeId::of(&ns, key)) {
            n.native_status = *status;
        }
    }
    let owner_id = |file: &str| -> NodeId {
        match index.owner(file) {
            Some(k) => NodeId::of(&ns, k),
            None => NodeId::of(&ns, &format!("{}/{repo}", graph::REPOSITORY_KEY)),
        }
    };
    for da in &a.donors {
        let dn = d.donor(&da.key).expect("assessed donors are declared");
        let (dns, dkey) = Graph::donor_id(&repo, &dn.key, &dn.origin);
        let did = NodeId::of(&dns, &dkey);
        for o in da
            .facts
            .runtime
            .iter()
            .chain(&da.facts.build)
            .chain(&da.facts.linked)
            .chain(&da.facts.test)
        {
            let kind = if o.via == census::Via::Process {
                EdgeKind::Calls
            } else {
                EdgeKind::DependsOn
            };
            let scope = if o.scope == Scope::Semantic {
                Scope::Runtime
            } else {
                o.scope
            };
            g.add_edge(owner_id(&o.file), did, kind, scope);
        }
    }
    for ((eco, name), obs) in &a.unregistered {
        let mut n = graph::GNode::new(
            "external",
            &format!("{}:{name}", eco.wire().to_ascii_lowercase()),
            NodeKind::External,
            name,
        );
        n.repository = format!("{eco}");
        let id = n.id;
        let _ = g.add_node(n);
        for o in obs {
            let kind = if o.via == census::Via::Process {
                EdgeKind::Calls
            } else {
                EdgeKind::DependsOn
            };
            g.add_edge(owner_id(&o.file), id, kind, o.scope);
        }
    }
    for (from, to, scope) in &c.internal {
        let f = owner_id(&format!("{from}/Cargo.toml"));
        let t = owner_id(&format!("{to}/Cargo.toml"));
        if f != t {
            g.add_edge(f, t, EdgeKind::DependsOn, *scope);
        }
    }
}

fn count(a: &Assessment) -> metrics::Counts {
    let mut c = metrics::Counts::default();
    let declared: BTreeSet<&str> = a
        .declaration
        .donors
        .iter()
        .map(|d| d.key.as_str())
        .collect();
    let mut registered: BTreeSet<String> = a.history.registered.clone();
    for d in &a.analysis.donors {
        let is_registered =
            d.effective >= DonorState::Registered || a.history.registered.contains(&d.key);
        if is_registered {
            registered.insert(d.key.clone());
        }
        match &d.exception {
            Some((ExceptionKind::Rejected, _)) if is_registered => c.donors_rejected += 1,
            Some((ExceptionKind::Superseded, _)) if is_registered => c.donors_superseded += 1,
            Some((ExceptionKind::Blocked, _)) => c.donors_blocked += 1,
            _ => {}
        }
        if !is_registered || d.resolved() {
            continue;
        }
        let at = |s: DonorState| (d.effective >= s) as u64;
        c.donors_censused += at(DonorState::Censused);
        c.donors_technology_mapped += at(DonorState::TechnologyMapped);
        c.donors_relevance_resolved += at(DonorState::RelevanceResolved);
        c.donors_specified += at(DonorState::Specified);
        c.donors_native_targeted += at(DonorState::NativeTargeted);
        c.donors_native_shadow += at(DonorState::NativeShadow);
        c.donors_parity_proven += at(DonorState::ParityProven);
        c.donors_cutover += at(DonorState::Cutover);
        c.donors_extinct += at(DonorState::Extinct);
        for cap in d.capabilities.iter().filter(|c| c.required) {
            c.capabilities_total += 1;
            c.capabilities_native += cap.native as u64;
            c.capabilities_proven += cap.proven() as u64;
        }
    }
    // Knowledge is counted over every declared donor, runtime-rejected ones included: a donor
    // whose runtime ECDEV refused can still teach it something.
    for d in &a.analysis.donors {
        match d.knowledge {
            crate::schema::KnowledgeState::Unreviewed => c.donors_knowledge_unreviewed += 1,
            crate::schema::KnowledgeState::StructuralCensus => c.donors_structural_census += 1,
            crate::schema::KnowledgeState::SemanticCensus => c.donors_semantic_census += 1,
            crate::schema::KnowledgeState::ActiveStudy => c.donors_active_study += 1,
            crate::schema::KnowledgeState::StudyComplete => c.donors_study_complete += 1,
        }
        c.donors_semantically_censused += d.facts.semantic_complete() as u64;
        if d.capabilities
            .iter()
            .any(|c| c.knowledge_open && c.knowledge_status.ends_with("_CANDIDATE"))
        {
            c.donors_with_open_candidates += 1;
        }
        for cap in &d.capabilities {
            c.known_donor_capabilities += 1;
            match cap.knowledge_status.as_str() {
                "UNREVIEWED" => c.knowledge_unreviewed += 1,
                donors::WITHDRAWN => c.capabilities_withdrawn += 1,
                "STUDY_CANDIDATE" => c.study_candidates += 1,
                "BENCHMARK_CANDIDATE" => c.benchmark_candidates += 1,
                "ALGORITHM_CANDIDATE" => c.algorithm_candidates += 1,
                "REFERENCE_ONLY" => c.reference_only += 1,
                "STUDY_COMPLETE" => c.study_complete_capabilities += 1,
                _ => c.no_research_value += 1,
            }
        }
    }
    // Capabilities of donors no longer declared at all stay known and open: removing a donor
    // never shrinks the knowledge denominator.
    for k in &a.history.capabilities {
        let donor = k.split_once('/').map_or(k.as_str(), |(d, _)| d);
        if !declared.contains(donor) {
            c.known_donor_capabilities += 1;
            c.capabilities_withdrawn += 1;
        }
    }
    // Discovered donors that participate are as unregistered as unknown externals.
    let discovered_active = a
        .analysis
        .donors
        .iter()
        .filter(|d| {
            d.effective < DonorState::Registered
                && !a.history.registered.contains(&d.key)
                && d.facts.active()
        })
        .count() as u64;
    c.donors_registered = registered.len() as u64;
    c.unregistered_externals = a.analysis.unregistered.len() as u64 + discovered_active;
    c.donors_discovered = declared.len() as u64
        + a.analysis.unregistered.len() as u64
        + a.analysis.vanished.len() as u64;

    let mut seen = BTreeSet::new();
    for o in &a.census.observations {
        if o.via == census::Via::Process && donors::TOOLCHAIN_PROGRAMS.contains(&o.name.as_str()) {
            continue;
        }
        if o.via == census::Via::Link && donors::PLATFORM_LIBRARIES.contains(&o.name.as_str()) {
            continue;
        }
        if !seen.insert((o.file.clone(), o.ident.clone(), o.scope)) {
            continue;
        }
        match o.scope {
            Scope::Runtime | Scope::Semantic | Scope::Architectural => {
                c.runtime_external_edges += 1
            }
            Scope::Build => c.build_external_edges += 1,
            Scope::Linked => c.linked_external_edges += 1,
            Scope::Test => c.test_external_edges += 1,
        }
    }
    c.external_closure_packages = a.census.closure.len() as u64;
    // External technology dependence: every (owner, foreign technology) pair, whatever the
    // coupling — including source held in the tree, which has no package edge at all.
    let index = donors::NodeIndex::new(&a.declaration);
    let owner = |f: &str| index.owner(f).unwrap_or("").to_string();
    let mut tech_edges: BTreeSet<(String, String)> = BTreeSet::new();
    for d in a.analysis.donors.iter().filter(|d| !d.resolved()) {
        let f = &d.facts;
        for o in f
            .runtime
            .iter()
            .chain(&f.build)
            .chain(&f.linked)
            .chain(&f.test)
        {
            tech_edges.insert((owner(&o.file), d.key.clone()));
        }
        for (file, _) in &f.imports {
            tech_edges.insert((owner(file), d.key.clone()));
        }
        for file in &f.resident {
            tech_edges.insert((owner(file), d.key.clone()));
        }
    }
    for ((eco, name), obs) in &a.analysis.unregistered {
        for o in obs {
            tech_edges.insert((owner(&o.file), format!("{eco}:{name}")));
        }
    }
    c.external_technology_edges = tech_edges.len() as u64;
    for t in &a.technologies {
        use schema::TechnologyLifecycle as L;
        let live = t.effective != L::Superseded;
        c.technologies_total += 1;
        c.technologies_native += (live && t.effective >= L::Native) as u64;
        c.technologies_proven += (live && t.effective >= L::Proven) as u64;
        c.technologies_canonical += (live && t.effective >= L::Canonical) as u64;
    }
    c.unmapped_nodes = a
        .shape
        .findings
        .iter()
        .filter(|f| f.code == "UNOWNED_CODE")
        .count() as u64;
    c.canonical_nodes_total = a.shape.nodes_total;
    c.canonical_nodes_conformant = a.shape.nodes_conformant;
    c.documents_total = a.docs.docs.len() as u64;
    c.documents_over_budget = a.docs.over_budget().count() as u64;
    c.shape_units = a.shape.units;
    c.shape_units_conformant = a.shape.conformant;
    a.universe.count(&mut c);
    let warned = |code: &str| a.findings.iter().filter(|f| f.code == code).count() as u64;
    c.orphan_capabilities = warned("ORPHAN_CAPABILITY");
    c.orphan_technologies = warned("ORPHAN_TECHNOLOGY");
    technology::count_sharing(&technology_sharing(a), &mut c);
    c
}

/// The sharing classification of the repository's technologies.
pub fn technology_sharing(a: &Assessment) -> Vec<technology::Sharing> {
    let repo_id = &a.declaration.repository.id;
    let shared: Vec<technology::Shared> = a
        .declaration
        .technologies
        .iter()
        .map(|t| technology::Shared {
            key: &t.key,
            birthplace: repo_id,
            implements: &t.implements,
            source_digest: a
                .technologies
                .iter()
                .find(|x| x.key == t.key)
                .map(|x| x.source_digest.as_str())
                .unwrap_or(""),
        })
        .collect();
    technology::classify_sharing(&a.graph, &shared)
}

/// The default repository root: the parent of the subsystem this binary was built from.
pub fn default_root() -> PathBuf {
    protocol::own_subsystem_dir()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}
