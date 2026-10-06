//! THE metric schema of ECDEV governance: exactly these metrics, computed by exactly these
//! formulas.
//! Ratios are exact rationals, printed with six decimals.

use crate::formats::json::Json;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ratio {
    pub num: u64,
    pub den: u64,
}

impl Ratio {
    pub fn new(num: u64, den: u64) -> Ratio {
        Ratio { num, den }
    }
    pub fn one(&self) -> bool {
        self.den > 0 && self.num == self.den
    }
    /// Six-decimal rendering, rounded half up, computed in integers.
    pub fn render(&self) -> String {
        if self.den == 0 {
            return "undefined".into();
        }
        let scaled = (self.num as u128 * 2_000_000 + self.den as u128) / (2 * self.den as u128);
        format!("{}.{:06}", scaled / 1_000_000, scaled % 1_000_000)
    }
}

/// One metric: name, formula (part of the schema identity), and value.
pub struct MetricDef {
    pub name: &'static str,
    pub formula: &'static str,
}

pub const DEFINITIONS: &[MetricDef] = &[
    MetricDef { name: "donors_discovered", formula: "declared donors + distinct unregistered external packages + vanished donors" },
    MetricDef { name: "donors_registered", formula: "|donors whose effective state >= REGISTERED ∪ donors registered in any earlier census|; never decreases" },
    MetricDef { name: "donors_censused", formula: "active donors with effective state >= CENSUSED" },
    MetricDef { name: "donors_specified", formula: "active donors with effective state >= SPECIFIED" },
    MetricDef { name: "donors_native_shadow", formula: "active donors with effective state >= NATIVE_SHADOW" },
    MetricDef { name: "donors_parity_proven", formula: "active donors with effective state >= PARITY_PROVEN" },
    MetricDef { name: "donors_cutover", formula: "active donors with effective state >= CUTOVER" },
    MetricDef { name: "donors_extinct", formula: "active donors with effective state = EXTINCT" },
    MetricDef { name: "donors_blocked", formula: "donors with a legal BLOCKED exception" },
    MetricDef { name: "donors_rejected", formula: "donors with a legal REJECT_RUNTIME exception: runtime never adopted (never participated in any census); they stay in every knowledge denominator" },
    MetricDef { name: "donors_superseded", formula: "donors with a legal SUPERSEDED exception (successor carries packages and capabilities)" },
    MetricDef { name: "donors_active", formula: "donors_registered - donors_rejected - donors_superseded (vanished donors stay active)" },
    MetricDef { name: "production_capabilities_required", formula: "PRODUCTION ONLY: required (relied-on) capabilities of declared, runtime-active donors; not a measure of what ECDEV has learned" },
    MetricDef { name: "production_capabilities_native", formula: "of those, with an existing canonical native replacement that does not use the donor, transitively" },
    MetricDef { name: "production_capabilities_proven", formula: "of those native, with fresh passing parity proofs" },
    MetricDef { name: "production_capabilities_remaining", formula: "production_capabilities_required - production_capabilities_proven; 0 means production adoption is done, never that the donor universe is studied" },
    MetricDef { name: "known_donor_capabilities", formula: "KNOWLEDGE: every declared capability of every declared donor, runtime-rejected donors included, plus capabilities_withdrawn" },
    MetricDef { name: "unreviewed_capabilities", formula: "known capabilities with no knowledge decision (UNREVIEWED)" },
    MetricDef { name: "capabilities_withdrawn", formula: "capability keys recorded by an earlier census that no declared capability carries or absorbs; still known, still open: deleting a capability never shrinks the knowledge denominator" },
    MetricDef { name: "semantic_censused_capabilities", formula: "known_donor_capabilities - unreviewed_capabilities - capabilities_withdrawn: capabilities whose knowledge was decided" },
    MetricDef { name: "study_candidates", formula: "known capabilities that are STUDY_CANDIDATE" },
    MetricDef { name: "benchmark_candidates", formula: "known capabilities that are BENCHMARK_CANDIDATE" },
    MetricDef { name: "algorithm_candidates", formula: "known capabilities that are ALGORITHM_CANDIDATE" },
    MetricDef { name: "reference_only", formula: "known capabilities that are REFERENCE_ONLY" },
    MetricDef { name: "study_complete_capabilities", formula: "known capabilities whose study is complete (derived native, independent native, or deliberate divergence)" },
    MetricDef { name: "not_relevant_after_review", formula: "known capabilities reviewed and found without research value on an admissible scope ground (frontend, release tooling, assets, vendor telemetry, hosted-service plumbing, language bindings, unrelated domain); never an adoption fact" },
    MetricDef { name: "research_questions_open", formula: "unreviewed_capabilities + capabilities_withdrawn + study_candidates + benchmark_candidates + algorithm_candidates" },
    MetricDef { name: "knowledge_coverage_ratio", formula: "(known_donor_capabilities - research_questions_open) / known_donor_capabilities; 0 when nothing is known" },
    MetricDef { name: "donors_knowledge_unreviewed", formula: "declared donors with no extracted capability (knowledge UNREVIEWED)" },
    MetricDef { name: "donors_structural_census", formula: "declared donors with some capability not yet knowledge-reviewed" },
    MetricDef { name: "donors_semantic_census", formula: "declared donors whose extracted capabilities are all decided with none open, but whose whole-source semantic review is not COMPLETE" },
    MetricDef { name: "donors_active_study", formula: "declared donors fully reviewed with open study, benchmark or algorithm candidates" },
    MetricDef { name: "donors_study_complete", formula: "declared donors whose whole-source semantic review is COMPLETE and every capability's knowledge is resolved" },
    MetricDef { name: "donors_with_open_research", formula: "donors_knowledge_unreviewed + donors_structural_census + donors_semantic_census + donors_active_study" },
    MetricDef { name: "donors_structurally_censused", formula: "declared donors with at least one extracted capability: donors_structural_census + donors_semantic_census + donors_active_study + donors_study_complete" },
    MetricDef { name: "donors_semantically_censused", formula: "declared donors whose census records a COMPLETE whole-source semantic review (every first-party source file read for meaning, not only inventoried and parsed)" },
    MetricDef { name: "donors_with_open_candidates", formula: "declared donors with at least one STUDY, BENCHMARK or ALGORITHM candidate, whatever their runtime decision" },
    MetricDef { name: "technologies_total", formula: "declared technologies" },
    MetricDef { name: "technologies_native", formula: "technologies whose effective lifecycle >= NATIVE" },
    MetricDef { name: "technologies_proven", formula: "technologies whose effective lifecycle >= PROVEN" },
    MetricDef { name: "technologies_canonical", formula: "technologies whose effective lifecycle >= CANONICAL" },
    MetricDef { name: "runtime_external_edges", formula: "observed foreign references of RUNTIME scope (manifest, process, source reference)" },
    MetricDef { name: "build_external_edges", formula: "observed foreign references of BUILD scope" },
    MetricDef { name: "linked_external_edges", formula: "observed native links of LINKED scope, and donor packages a lockfile still links only transitively (attributed to the manifest whose direct dependency pulls them in)" },
    MetricDef { name: "test_external_edges", formula: "observed foreign references of TEST scope (dev-dependencies, oracles)" },
    MetricDef { name: "external_technology_edges", formula: "distinct (owning node, foreign technology) pairs over every way foreign technology participates: manifests of any scope, imports, links, process calls, source references and held (vendored, copied) donor source" },
    MetricDef { name: "external_closure_packages", formula: "locked external packages (Cargo.lock entries with a source)" },
    MetricDef { name: "canonical_nodes_total", formula: "active physical nodes other than the repository" },
    MetricDef { name: "canonical_nodes_conformant", formula: "of those, existing at a path equal to a valid canonical path" },
    MetricDef { name: "legacy_nodes_remaining", formula: "canonical_nodes_total - canonical_nodes_conformant" },
    MetricDef { name: "unmapped_nodes", formula: "directories holding code that belongs to no declared node" },
    MetricDef { name: "documents_total", formula: "tracked Markdown documents outside held donor source" },
    MetricDef { name: "documents_over_budget", formula: "documents not permitted by the document budget" },
    MetricDef { name: "repository_shape_conformance", formula: "conformant units / units; units = root entries + active nodes + one failing unit per other shape violation (undeclared member, directory of unowned code, plane violation, research on the build path, tracked generated state, forbidden container); documents are gated by documents_over_budget" },
    MetricDef { name: "ecdev_repository_conformance", formula: "passed ECDEV repository conformance checks / ECDEV repository conformance checks" },
    MetricDef { name: "production_native_ratio", formula: "PRODUCTION ONLY: production_capabilities_native / production_capabilities_required; 1 when no active donor remains, 0 when active donors declare no capability" },
    MetricDef { name: "production_proof_ratio", formula: "PRODUCTION ONLY: production_capabilities_proven / production_capabilities_required; same guards" },
    MetricDef { name: "technology_native_ratio", formula: "technologies_native / technologies_total; 1 when there are none" },
    MetricDef { name: "extinction_ratio", formula: "donors_extinct / (donors_active + distinct unregistered external packages); 1 when the denominator is 0" },
    MetricDef { name: "donors_technology_mapped", formula: "active donors with effective state >= TECHNOLOGY_MAPPED" },
    MetricDef { name: "donors_relevance_resolved", formula: "active donors with effective state >= RELEVANCE_RESOLVED" },
    MetricDef { name: "orphan_capabilities", formula: "capabilities provided by a physical node with no implementing technology" },
    MetricDef { name: "orphan_technologies", formula: "technologies implementing no capability" },
    MetricDef { name: "sharing_shared_implementation", formula: "technology/copy pairs: one canonical technology REUSED by another node" },
    MetricDef { name: "sharing_independent_implementation", formula: "technology pairs implementing one capability with a direct family relation and different sources" },
    MetricDef { name: "sharing_domain_specialization", formula: "technology pairs implementing one capability where one SPECIALIZES/GENERALIZES the other" },
    MetricDef { name: "sharing_shared_concept", formula: "technology pairs implementing one capability in one declared family with no direct relation" },
    MetricDef { name: "sharing_unrelated_duplicate", formula: "technology pairs implementing one capability with no declared relation" },
    MetricDef { name: "donors_native_targeted", formula: "active donors with effective state >= NATIVE_TARGETED" },
    MetricDef { name: "known_universe_total", formula: "relations of the repository to what it knows: one per global donor of its universe records and declared donors, plus one per record or donor with no resolvable origin; a repository with a NOT RECONCILED legacy registry knows more than this (never read as complete)" },
    MetricDef { name: "known_oss_repositories", formula: "known relations whose primary kind is OSS_REPOSITORY (primary kind: the first, in vocabulary order, of its records' kinds; a declared donor is OSS_REPOSITORY)" },
    MetricDef { name: "known_information_sources", formula: "known relations whose primary kind is INFORMATION_SOURCE" },
    MetricDef { name: "known_official_authorities", formula: "known relations whose primary kind is OFFICIAL_AUTHORITY" },
    MetricDef { name: "known_protocol_specs", formula: "known relations whose primary kind is PROTOCOL_SPEC" },
    MetricDef { name: "known_academic_references", formula: "known relations whose primary kind is ACADEMIC_REFERENCE" },
    MetricDef { name: "known_benchmarks", formula: "known relations whose primary kind is BENCHMARK" },
    MetricDef { name: "known_datasets", formula: "known relations whose primary kind is DATASET" },
    MetricDef { name: "known_simulators", formula: "known relations whose primary kind is SIMULATOR" },
    MetricDef { name: "known_standards", formula: "known relations whose primary kind is STANDARD" },
    MetricDef { name: "known_reference_architectures", formula: "known relations whose primary kind is REFERENCE_ARCHITECTURE" },
    MetricDef { name: "known_papers", formula: "known relations whose primary kind is PAPER" },
    MetricDef { name: "registered_universe_total", formula: "known relations at layer >= REGISTERED" },
    MetricDef { name: "unique_global_donors", formula: "distinct global donors (after declared aliases)" },
    MetricDef { name: "repository_donor_references", formula: "RepositoryUsesDonor relations with a global donor" },
    MetricDef { name: "selected_for_census", formula: "known relations at layer >= SELECTED_FOR_CENSUS" },
    MetricDef { name: "censused", formula: "known relations at layer >= CENSUSED" },
    MetricDef { name: "technology_mapped", formula: "known relations at layer >= TECHNOLOGY_MAPPED" },
    MetricDef { name: "native_targeted", formula: "known relations at layer >= NATIVE_TARGETED" },
    MetricDef { name: "native_shadow", formula: "known relations at layer >= NATIVE_SHADOW" },
    MetricDef { name: "parity_proven", formula: "known relations at layer >= PARITY_PROVEN" },
    MetricDef { name: "cutover", formula: "known relations at layer >= CUTOVER" },
    MetricDef { name: "extinct", formula: "known relations at layer EXTINCT (per repository: never the extinction of a technology)" },
    MetricDef { name: "unknown_identity", formula: "known relations with no resolvable origin (no global donor identity)" },
    MetricDef { name: "unverified_origin", formula: "relations with a global donor whose origin no record verifies (reachable, remote HEAD, or fetched at a revision)" },
    MetricDef { name: "unverified_license", formula: "relations with a global donor whose licence no record verifies (read from the upstream's licence file)" },
    MetricDef { name: "unresolved_relevance", formula: "known relations whose relevance to the repository is unresolved" },
    MetricDef { name: "ambiguous_donor_identities", formula: "groups of distinct OSS global donors that look alike (same repository name, different host or owner) and no alias joins" },
    MetricDef { name: "universe_sources_unreconciled", formula: "legacy registries of the repository with no universe file of the same bytes (NOT RECONCILED or STALE)" },
];

/// The V1 success gate.
pub const V1_GATE: &[(&str, &str)] = &[
    ("extinction_ratio", "1.000000"),
    ("runtime_external_edges", "0"),
    ("build_external_edges", "0"),
    ("linked_external_edges", "0"),
    ("external_technology_edges", "0"),
    ("production_native_ratio", "1.000000"),
    ("production_proof_ratio", "1.000000"),
    ("repository_shape_conformance", "1.000000"),
    ("documents_over_budget", "0"),
    ("ecdev_repository_conformance", "1.000000"),
];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Counts {
    pub donors_discovered: u64,
    pub donors_registered: u64,
    pub donors_censused: u64,
    pub donors_specified: u64,
    pub donors_native_shadow: u64,
    pub donors_parity_proven: u64,
    pub donors_cutover: u64,
    pub donors_extinct: u64,
    pub donors_blocked: u64,
    pub donors_rejected: u64,
    pub donors_superseded: u64,
    pub unregistered_externals: u64,
    pub capabilities_total: u64,
    pub capabilities_native: u64,
    pub capabilities_proven: u64,
    pub runtime_external_edges: u64,
    pub build_external_edges: u64,
    pub linked_external_edges: u64,
    pub test_external_edges: u64,
    pub external_technology_edges: u64,
    pub technologies_total: u64,
    pub technologies_native: u64,
    pub technologies_proven: u64,
    pub technologies_canonical: u64,
    pub unmapped_nodes: u64,
    pub external_closure_packages: u64,
    pub canonical_nodes_total: u64,
    pub canonical_nodes_conformant: u64,
    pub documents_total: u64,
    pub documents_over_budget: u64,
    pub shape_units: u64,
    pub shape_units_conformant: u64,
    pub conformance_checks: u64,
    pub conformance_checks_passed: u64,
    pub donors_technology_mapped: u64,
    pub donors_relevance_resolved: u64,
    pub orphan_capabilities: u64,
    pub orphan_technologies: u64,
    pub sharing_shared_implementation: u64,
    pub sharing_independent_implementation: u64,
    pub sharing_domain_specialization: u64,
    pub sharing_shared_concept: u64,
    pub sharing_unrelated_duplicate: u64,
    pub donors_native_targeted: u64,
    pub known_universe_total: u64,
    pub known_oss_repositories: u64,
    pub known_information_sources: u64,
    pub known_official_authorities: u64,
    pub known_protocol_specs: u64,
    pub known_academic_references: u64,
    pub known_benchmarks: u64,
    pub known_datasets: u64,
    pub known_simulators: u64,
    pub known_standards: u64,
    pub known_reference_architectures: u64,
    pub known_papers: u64,
    pub registered_universe_total: u64,
    pub unique_global_donors: u64,
    pub repository_donor_references: u64,
    pub selected_for_census: u64,
    pub censused: u64,
    pub technology_mapped: u64,
    pub native_targeted: u64,
    pub native_shadow: u64,
    pub parity_proven: u64,
    pub cutover: u64,
    pub extinct: u64,
    pub unknown_identity: u64,
    pub unverified_origin: u64,
    pub unverified_license: u64,
    pub unresolved_relevance: u64,
    pub ambiguous_donor_identities: u64,
    pub universe_sources_unreconciled: u64,
    pub known_donor_capabilities: u64,
    pub knowledge_unreviewed: u64,
    pub study_candidates: u64,
    pub benchmark_candidates: u64,
    pub algorithm_candidates: u64,
    pub reference_only: u64,
    pub study_complete_capabilities: u64,
    pub no_research_value: u64,
    pub donors_knowledge_unreviewed: u64,
    pub donors_structural_census: u64,
    pub donors_active_study: u64,
    pub donors_study_complete: u64,
    pub capabilities_withdrawn: u64,
    pub donors_with_open_candidates: u64,
    pub donors_semantic_census: u64,
    pub donors_semantically_censused: u64,
}

impl Counts {
    /// Knowledge questions still open: unreviewed or withdrawn capabilities and every study,
    /// benchmark or algorithm candidate.
    pub fn research_questions_open(&self) -> u64 {
        self.knowledge_unreviewed
            + self.capabilities_withdrawn
            + self.study_candidates
            + self.benchmark_candidates
            + self.algorithm_candidates
    }

    pub fn add(&mut self, o: &Counts) {
        macro_rules! sum { ($($f:ident),*) => { $(self.$f += o.$f;)* } }
        sum!(
            donors_discovered,
            donors_registered,
            donors_censused,
            donors_specified,
            donors_native_shadow,
            donors_parity_proven,
            donors_cutover,
            donors_extinct,
            donors_blocked,
            donors_rejected,
            donors_superseded,
            unregistered_externals,
            capabilities_total,
            capabilities_native,
            capabilities_proven,
            runtime_external_edges,
            build_external_edges,
            linked_external_edges,
            test_external_edges,
            external_closure_packages,
            canonical_nodes_total,
            canonical_nodes_conformant,
            documents_total,
            documents_over_budget,
            shape_units,
            shape_units_conformant,
            conformance_checks,
            conformance_checks_passed,
            external_technology_edges,
            technologies_total,
            technologies_native,
            technologies_proven,
            technologies_canonical,
            unmapped_nodes,
            donors_technology_mapped,
            donors_relevance_resolved,
            orphan_capabilities,
            orphan_technologies,
            sharing_shared_implementation,
            sharing_independent_implementation,
            sharing_domain_specialization,
            sharing_shared_concept,
            sharing_unrelated_duplicate,
            donors_native_targeted,
            known_universe_total,
            known_oss_repositories,
            known_information_sources,
            known_official_authorities,
            known_protocol_specs,
            known_academic_references,
            known_benchmarks,
            known_datasets,
            known_simulators,
            known_standards,
            known_reference_architectures,
            known_papers,
            registered_universe_total,
            unique_global_donors,
            repository_donor_references,
            selected_for_census,
            censused,
            technology_mapped,
            native_targeted,
            native_shadow,
            parity_proven,
            cutover,
            extinct,
            unknown_identity,
            unverified_origin,
            unverified_license,
            unresolved_relevance,
            ambiguous_donor_identities,
            universe_sources_unreconciled,
            known_donor_capabilities,
            knowledge_unreviewed,
            study_candidates,
            benchmark_candidates,
            algorithm_candidates,
            reference_only,
            study_complete_capabilities,
            no_research_value,
            donors_knowledge_unreviewed,
            donors_structural_census,
            donors_active_study,
            donors_study_complete,
            capabilities_withdrawn,
            donors_with_open_candidates,
            donors_semantic_census,
            donors_semantically_censused
        );
    }

    /// Every raw count, by field name (schema order of declaration).
    pub fn raw(&self) -> Vec<(&'static str, u64)> {
        vec![
            ("donors_discovered", self.donors_discovered),
            ("donors_registered", self.donors_registered),
            ("donors_censused", self.donors_censused),
            ("donors_specified", self.donors_specified),
            ("donors_native_shadow", self.donors_native_shadow),
            ("donors_parity_proven", self.donors_parity_proven),
            ("donors_cutover", self.donors_cutover),
            ("donors_extinct", self.donors_extinct),
            ("donors_blocked", self.donors_blocked),
            ("donors_rejected", self.donors_rejected),
            ("donors_superseded", self.donors_superseded),
            ("unregistered_externals", self.unregistered_externals),
            ("production_capabilities_required", self.capabilities_total),
            ("production_capabilities_native", self.capabilities_native),
            ("production_capabilities_proven", self.capabilities_proven),
            ("runtime_external_edges", self.runtime_external_edges),
            ("build_external_edges", self.build_external_edges),
            ("linked_external_edges", self.linked_external_edges),
            ("test_external_edges", self.test_external_edges),
            ("external_closure_packages", self.external_closure_packages),
            ("canonical_nodes_total", self.canonical_nodes_total),
            (
                "canonical_nodes_conformant",
                self.canonical_nodes_conformant,
            ),
            ("documents_total", self.documents_total),
            ("documents_over_budget", self.documents_over_budget),
            ("shape_units", self.shape_units),
            ("shape_units_conformant", self.shape_units_conformant),
            ("conformance_checks", self.conformance_checks),
            ("conformance_checks_passed", self.conformance_checks_passed),
            ("external_technology_edges", self.external_technology_edges),
            ("technologies_total", self.technologies_total),
            ("technologies_native", self.technologies_native),
            ("technologies_proven", self.technologies_proven),
            ("technologies_canonical", self.technologies_canonical),
            ("unmapped_nodes", self.unmapped_nodes),
            ("donors_technology_mapped", self.donors_technology_mapped),
            ("donors_relevance_resolved", self.donors_relevance_resolved),
            ("orphan_capabilities", self.orphan_capabilities),
            ("orphan_technologies", self.orphan_technologies),
            (
                "sharing_shared_implementation",
                self.sharing_shared_implementation,
            ),
            (
                "sharing_independent_implementation",
                self.sharing_independent_implementation,
            ),
            (
                "sharing_domain_specialization",
                self.sharing_domain_specialization,
            ),
            ("sharing_shared_concept", self.sharing_shared_concept),
            (
                "sharing_unrelated_duplicate",
                self.sharing_unrelated_duplicate,
            ),
            ("donors_native_targeted", self.donors_native_targeted),
            ("known_universe_total", self.known_universe_total),
            ("known_oss_repositories", self.known_oss_repositories),
            ("known_information_sources", self.known_information_sources),
            (
                "known_official_authorities",
                self.known_official_authorities,
            ),
            ("known_protocol_specs", self.known_protocol_specs),
            ("known_academic_references", self.known_academic_references),
            ("known_benchmarks", self.known_benchmarks),
            ("known_datasets", self.known_datasets),
            ("known_simulators", self.known_simulators),
            ("known_standards", self.known_standards),
            (
                "known_reference_architectures",
                self.known_reference_architectures,
            ),
            ("known_papers", self.known_papers),
            ("registered_universe_total", self.registered_universe_total),
            ("unique_global_donors", self.unique_global_donors),
            (
                "repository_donor_references",
                self.repository_donor_references,
            ),
            ("selected_for_census", self.selected_for_census),
            ("censused", self.censused),
            ("technology_mapped", self.technology_mapped),
            ("native_targeted", self.native_targeted),
            ("native_shadow", self.native_shadow),
            ("parity_proven", self.parity_proven),
            ("cutover", self.cutover),
            ("extinct", self.extinct),
            ("unknown_identity", self.unknown_identity),
            ("unverified_origin", self.unverified_origin),
            ("unverified_license", self.unverified_license),
            ("unresolved_relevance", self.unresolved_relevance),
            (
                "ambiguous_donor_identities",
                self.ambiguous_donor_identities,
            ),
            (
                "universe_sources_unreconciled",
                self.universe_sources_unreconciled,
            ),
            ("known_donor_capabilities", self.known_donor_capabilities),
            ("unreviewed_capabilities", self.knowledge_unreviewed),
            ("capabilities_withdrawn", self.capabilities_withdrawn),
            ("study_candidates", self.study_candidates),
            ("benchmark_candidates", self.benchmark_candidates),
            ("algorithm_candidates", self.algorithm_candidates),
            ("reference_only", self.reference_only),
            (
                "study_complete_capabilities",
                self.study_complete_capabilities,
            ),
            ("not_relevant_after_review", self.no_research_value),
            (
                "donors_knowledge_unreviewed",
                self.donors_knowledge_unreviewed,
            ),
            ("donors_structural_census", self.donors_structural_census),
            ("donors_active_study", self.donors_active_study),
            ("donors_study_complete", self.donors_study_complete),
            (
                "donors_with_open_candidates",
                self.donors_with_open_candidates,
            ),
            ("donors_semantic_census", self.donors_semantic_census),
            (
                "donors_semantically_censused",
                self.donors_semantically_censused,
            ),
        ]
    }

    /// Rebuilds counts from `raw()` output; unknown names are ignored.
    pub fn from_raw(raw: &[(String, u64)]) -> Counts {
        let mut c = Counts::default();
        for (k, v) in raw {
            match k.as_str() {
                "donors_discovered" => c.donors_discovered = *v,
                "donors_registered" => c.donors_registered = *v,
                "donors_censused" => c.donors_censused = *v,
                "donors_specified" => c.donors_specified = *v,
                "donors_native_shadow" => c.donors_native_shadow = *v,
                "donors_parity_proven" => c.donors_parity_proven = *v,
                "donors_cutover" => c.donors_cutover = *v,
                "donors_extinct" => c.donors_extinct = *v,
                "donors_blocked" => c.donors_blocked = *v,
                "donors_rejected" => c.donors_rejected = *v,
                "donors_superseded" => c.donors_superseded = *v,
                "unregistered_externals" => c.unregistered_externals = *v,
                // Pre-knowledge records named the production counts without their prefix.
                "production_capabilities_required" | "capabilities_total" => {
                    c.capabilities_total = *v
                }
                "production_capabilities_native" | "capabilities_native" => {
                    c.capabilities_native = *v
                }
                "production_capabilities_proven" | "capabilities_proven" => {
                    c.capabilities_proven = *v
                }
                "runtime_external_edges" => c.runtime_external_edges = *v,
                "build_external_edges" => c.build_external_edges = *v,
                "linked_external_edges" => c.linked_external_edges = *v,
                "test_external_edges" => c.test_external_edges = *v,
                "external_closure_packages" => c.external_closure_packages = *v,
                "canonical_nodes_total" => c.canonical_nodes_total = *v,
                "canonical_nodes_conformant" => c.canonical_nodes_conformant = *v,
                "documents_total" => c.documents_total = *v,
                "documents_over_budget" => c.documents_over_budget = *v,
                "shape_units" => c.shape_units = *v,
                "shape_units_conformant" => c.shape_units_conformant = *v,
                "conformance_checks" => c.conformance_checks = *v,
                "conformance_checks_passed" => c.conformance_checks_passed = *v,
                "external_technology_edges" => c.external_technology_edges = *v,
                "technologies_total" => c.technologies_total = *v,
                "technologies_native" => c.technologies_native = *v,
                "technologies_proven" => c.technologies_proven = *v,
                "technologies_canonical" => c.technologies_canonical = *v,
                "unmapped_nodes" => c.unmapped_nodes = *v,
                "donors_technology_mapped" => c.donors_technology_mapped = *v,
                "donors_relevance_resolved" => c.donors_relevance_resolved = *v,
                "orphan_capabilities" => c.orphan_capabilities = *v,
                "orphan_technologies" => c.orphan_technologies = *v,
                "sharing_shared_implementation" => c.sharing_shared_implementation = *v,
                "sharing_independent_implementation" => c.sharing_independent_implementation = *v,
                "sharing_domain_specialization" => c.sharing_domain_specialization = *v,
                "sharing_shared_concept" => c.sharing_shared_concept = *v,
                "sharing_unrelated_duplicate" => c.sharing_unrelated_duplicate = *v,
                "donors_native_targeted" => c.donors_native_targeted = *v,
                "known_universe_total" => c.known_universe_total = *v,
                "known_oss_repositories" => c.known_oss_repositories = *v,
                "known_information_sources" => c.known_information_sources = *v,
                "known_official_authorities" => c.known_official_authorities = *v,
                "known_protocol_specs" => c.known_protocol_specs = *v,
                "known_academic_references" => c.known_academic_references = *v,
                "known_benchmarks" => c.known_benchmarks = *v,
                "known_datasets" => c.known_datasets = *v,
                "known_simulators" => c.known_simulators = *v,
                "known_standards" => c.known_standards = *v,
                "known_reference_architectures" => c.known_reference_architectures = *v,
                "known_papers" => c.known_papers = *v,
                "registered_universe_total" => c.registered_universe_total = *v,
                "unique_global_donors" => c.unique_global_donors = *v,
                "repository_donor_references" => c.repository_donor_references = *v,
                "selected_for_census" => c.selected_for_census = *v,
                "censused" => c.censused = *v,
                "technology_mapped" => c.technology_mapped = *v,
                "native_targeted" => c.native_targeted = *v,
                "native_shadow" => c.native_shadow = *v,
                "parity_proven" => c.parity_proven = *v,
                "cutover" => c.cutover = *v,
                "extinct" => c.extinct = *v,
                "unknown_identity" => c.unknown_identity = *v,
                "unverified_origin" => c.unverified_origin = *v,
                "unverified_license" => c.unverified_license = *v,
                "unresolved_relevance" => c.unresolved_relevance = *v,
                "ambiguous_donor_identities" => c.ambiguous_donor_identities = *v,
                "universe_sources_unreconciled" => c.universe_sources_unreconciled = *v,
                "known_donor_capabilities" => c.known_donor_capabilities = *v,
                "unreviewed_capabilities" | "knowledge_unreviewed" => c.knowledge_unreviewed = *v,
                "capabilities_withdrawn" => c.capabilities_withdrawn = *v,
                "donors_with_open_candidates" => c.donors_with_open_candidates = *v,
                "donors_semantic_census" => c.donors_semantic_census = *v,
                "donors_semantically_censused" => c.donors_semantically_censused = *v,
                "study_candidates" => c.study_candidates = *v,
                "benchmark_candidates" => c.benchmark_candidates = *v,
                "algorithm_candidates" => c.algorithm_candidates = *v,
                "reference_only" => c.reference_only = *v,
                "study_complete_capabilities" => c.study_complete_capabilities = *v,
                "not_relevant_after_review" | "no_research_value" => c.no_research_value = *v,
                "donors_knowledge_unreviewed" => c.donors_knowledge_unreviewed = *v,
                "donors_structural_census" => c.donors_structural_census = *v,
                "donors_active_study" => c.donors_active_study = *v,
                "donors_study_complete" => c.donors_study_complete = *v,
                _ => {}
            }
        }
        c
    }

    pub fn donors_active(&self) -> u64 {
        self.donors_registered
            .saturating_sub(self.donors_rejected + self.donors_superseded)
    }

    fn capability_ratio(&self, num: u64) -> Ratio {
        if self.capabilities_total > 0 {
            Ratio::new(num, self.capabilities_total)
        } else if self.donors_active() == 0 && self.unregistered_externals == 0 {
            Ratio::new(1, 1)
        } else {
            Ratio::new(0, 1)
        }
    }

    /// Every metric of the schema, in schema order, as (name, printed value).
    pub fn values(&self) -> Vec<(String, String)> {
        let n = |v: u64| v.to_string();
        let ext_den = self.donors_active() + self.unregistered_externals;
        let extinction = if ext_den == 0 {
            Ratio::new(1, 1)
        } else {
            Ratio::new(self.donors_extinct, ext_den)
        };
        let shape = if self.shape_units == 0 {
            Ratio::new(1, 1)
        } else {
            Ratio::new(self.shape_units_conformant, self.shape_units)
        };
        let conformance = if self.conformance_checks == 0 {
            Ratio::new(0, 1)
        } else {
            Ratio::new(self.conformance_checks_passed, self.conformance_checks)
        };
        let v: Vec<(&str, String)> = vec![
            ("donors_discovered", n(self.donors_discovered)),
            ("donors_registered", n(self.donors_registered)),
            ("donors_censused", n(self.donors_censused)),
            ("donors_specified", n(self.donors_specified)),
            ("donors_native_shadow", n(self.donors_native_shadow)),
            ("donors_parity_proven", n(self.donors_parity_proven)),
            ("donors_cutover", n(self.donors_cutover)),
            ("donors_extinct", n(self.donors_extinct)),
            ("donors_blocked", n(self.donors_blocked)),
            ("donors_rejected", n(self.donors_rejected)),
            ("donors_superseded", n(self.donors_superseded)),
            ("donors_active", n(self.donors_active())),
            (
                "production_capabilities_required",
                n(self.capabilities_total),
            ),
            (
                "production_capabilities_native",
                n(self.capabilities_native),
            ),
            (
                "production_capabilities_proven",
                n(self.capabilities_proven),
            ),
            (
                "production_capabilities_remaining",
                n(self
                    .capabilities_total
                    .saturating_sub(self.capabilities_proven)),
            ),
            ("known_donor_capabilities", n(self.known_donor_capabilities)),
            ("unreviewed_capabilities", n(self.knowledge_unreviewed)),
            ("capabilities_withdrawn", n(self.capabilities_withdrawn)),
            (
                "semantic_censused_capabilities",
                n(self
                    .known_donor_capabilities
                    .saturating_sub(self.knowledge_unreviewed + self.capabilities_withdrawn)),
            ),
            ("study_candidates", n(self.study_candidates)),
            ("benchmark_candidates", n(self.benchmark_candidates)),
            ("algorithm_candidates", n(self.algorithm_candidates)),
            ("reference_only", n(self.reference_only)),
            (
                "study_complete_capabilities",
                n(self.study_complete_capabilities),
            ),
            ("not_relevant_after_review", n(self.no_research_value)),
            ("research_questions_open", n(self.research_questions_open())),
            (
                "knowledge_coverage_ratio",
                if self.known_donor_capabilities == 0 {
                    Ratio::new(0, 1)
                } else {
                    Ratio::new(
                        self.known_donor_capabilities
                            .saturating_sub(self.research_questions_open()),
                        self.known_donor_capabilities,
                    )
                }
                .render(),
            ),
            (
                "donors_knowledge_unreviewed",
                n(self.donors_knowledge_unreviewed),
            ),
            ("donors_structural_census", n(self.donors_structural_census)),
            ("donors_semantic_census", n(self.donors_semantic_census)),
            ("donors_active_study", n(self.donors_active_study)),
            ("donors_study_complete", n(self.donors_study_complete)),
            (
                "donors_with_open_research",
                n(self.donors_knowledge_unreviewed
                    + self.donors_structural_census
                    + self.donors_semantic_census
                    + self.donors_active_study),
            ),
            (
                "donors_structurally_censused",
                n(self.donors_structural_census
                    + self.donors_semantic_census
                    + self.donors_active_study
                    + self.donors_study_complete),
            ),
            (
                "donors_semantically_censused",
                n(self.donors_semantically_censused),
            ),
            (
                "donors_with_open_candidates",
                n(self.donors_with_open_candidates),
            ),
            ("technologies_total", n(self.technologies_total)),
            ("technologies_native", n(self.technologies_native)),
            ("technologies_proven", n(self.technologies_proven)),
            ("technologies_canonical", n(self.technologies_canonical)),
            ("runtime_external_edges", n(self.runtime_external_edges)),
            ("build_external_edges", n(self.build_external_edges)),
            ("linked_external_edges", n(self.linked_external_edges)),
            ("test_external_edges", n(self.test_external_edges)),
            (
                "external_technology_edges",
                n(self.external_technology_edges),
            ),
            (
                "external_closure_packages",
                n(self.external_closure_packages),
            ),
            ("canonical_nodes_total", n(self.canonical_nodes_total)),
            (
                "canonical_nodes_conformant",
                n(self.canonical_nodes_conformant),
            ),
            (
                "legacy_nodes_remaining",
                n(self
                    .canonical_nodes_total
                    .saturating_sub(self.canonical_nodes_conformant)),
            ),
            ("unmapped_nodes", n(self.unmapped_nodes)),
            ("documents_total", n(self.documents_total)),
            ("documents_over_budget", n(self.documents_over_budget)),
            ("repository_shape_conformance", shape.render()),
            ("ecdev_repository_conformance", conformance.render()),
            (
                "production_native_ratio",
                self.capability_ratio(self.capabilities_native).render(),
            ),
            (
                "production_proof_ratio",
                self.capability_ratio(self.capabilities_proven).render(),
            ),
            (
                "technology_native_ratio",
                if self.technologies_total == 0 {
                    Ratio::new(1, 1)
                } else {
                    Ratio::new(self.technologies_native, self.technologies_total)
                }
                .render(),
            ),
            ("extinction_ratio", extinction.render()),
            ("donors_technology_mapped", n(self.donors_technology_mapped)),
            (
                "donors_relevance_resolved",
                n(self.donors_relevance_resolved),
            ),
            ("orphan_capabilities", n(self.orphan_capabilities)),
            ("orphan_technologies", n(self.orphan_technologies)),
            (
                "sharing_shared_implementation",
                n(self.sharing_shared_implementation),
            ),
            (
                "sharing_independent_implementation",
                n(self.sharing_independent_implementation),
            ),
            (
                "sharing_domain_specialization",
                n(self.sharing_domain_specialization),
            ),
            ("sharing_shared_concept", n(self.sharing_shared_concept)),
            (
                "sharing_unrelated_duplicate",
                n(self.sharing_unrelated_duplicate),
            ),
            ("donors_native_targeted", n(self.donors_native_targeted)),
            ("known_universe_total", n(self.known_universe_total)),
            ("known_oss_repositories", n(self.known_oss_repositories)),
            (
                "known_information_sources",
                n(self.known_information_sources),
            ),
            (
                "known_official_authorities",
                n(self.known_official_authorities),
            ),
            ("known_protocol_specs", n(self.known_protocol_specs)),
            (
                "known_academic_references",
                n(self.known_academic_references),
            ),
            ("known_benchmarks", n(self.known_benchmarks)),
            ("known_datasets", n(self.known_datasets)),
            ("known_simulators", n(self.known_simulators)),
            ("known_standards", n(self.known_standards)),
            (
                "known_reference_architectures",
                n(self.known_reference_architectures),
            ),
            ("known_papers", n(self.known_papers)),
            (
                "registered_universe_total",
                n(self.registered_universe_total),
            ),
            ("unique_global_donors", n(self.unique_global_donors)),
            (
                "repository_donor_references",
                n(self.repository_donor_references),
            ),
            ("selected_for_census", n(self.selected_for_census)),
            ("censused", n(self.censused)),
            ("technology_mapped", n(self.technology_mapped)),
            ("native_targeted", n(self.native_targeted)),
            ("native_shadow", n(self.native_shadow)),
            ("parity_proven", n(self.parity_proven)),
            ("cutover", n(self.cutover)),
            ("extinct", n(self.extinct)),
            ("unknown_identity", n(self.unknown_identity)),
            ("unverified_origin", n(self.unverified_origin)),
            ("unverified_license", n(self.unverified_license)),
            ("unresolved_relevance", n(self.unresolved_relevance)),
            (
                "ambiguous_donor_identities",
                n(self.ambiguous_donor_identities),
            ),
            (
                "universe_sources_unreconciled",
                n(self.universe_sources_unreconciled),
            ),
        ];
        debug_assert_eq!(v.len(), DEFINITIONS.len());
        v.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    }

    /// The V1 gate rows: (metric, required, actual, pass).
    pub fn v1_gate(&self) -> Vec<(String, String, String, bool)> {
        let values = self.values();
        V1_GATE
            .iter()
            .map(|(k, want)| {
                let got = values
                    .iter()
                    .find(|(n, _)| n == k)
                    .map(|(_, v)| v.clone())
                    .unwrap_or_default();
                (k.to_string(), want.to_string(), got.clone(), got == *want)
            })
            .collect()
    }

    pub fn to_json(&self) -> Json {
        let mut o = Json::obj().with("schema", crate::protocol::schema_identity());
        for (k, v) in self.values() {
            // A ratio without a denominator is not a number.
            o = o.with(
                &k,
                if v == "undefined" {
                    Json::Null
                } else {
                    Json::Number(v)
                },
            );
        }
        o
    }
}

pub fn render_definitions() -> String {
    let mut s = String::new();
    for d in DEFINITIONS {
        s.push_str(&format!("metric {} = {}\n", d.name, d.formula));
    }
    for (k, v) in V1_GATE {
        s.push_str(&format!("v1_gate {k} == {v}\n"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratios_render_exactly() {
        assert_eq!(Ratio::new(1, 3).render(), "0.333333");
        assert_eq!(Ratio::new(2, 3).render(), "0.666667");
        assert_eq!(Ratio::new(3, 3).render(), "1.000000");
        assert_eq!(Ratio::new(0, 7).render(), "0.000000");
    }

    #[test]
    fn production_completion_is_not_knowledge_completion() {
        // Every production capability proven, yet open research remains (including on a
        // runtime-rejected donor): production reads 100 %, knowledge must not.
        let c = Counts {
            donors_registered: 2,
            donors_rejected: 1,
            capabilities_total: 3,
            capabilities_native: 3,
            capabilities_proven: 3,
            known_donor_capabilities: 10,
            study_candidates: 2,
            benchmark_candidates: 1,
            study_complete_capabilities: 5,
            reference_only: 2,
            donors_active_study: 1,
            donors_study_complete: 1,
            ..Counts::default()
        };
        let v = c.values();
        let get = |k: &str| v.iter().find(|(n, _)| n == k).unwrap().1.clone();
        assert_eq!(get("production_proof_ratio"), "1.000000");
        assert_eq!(get("production_capabilities_remaining"), "0");
        assert_eq!(get("research_questions_open"), "3");
        assert_eq!(get("knowledge_coverage_ratio"), "0.700000");
        assert_eq!(get("donors_with_open_research"), "1");
        // The legacy name of a production count is still read from older records.
        let back = Counts::from_raw(&[("capabilities_total".into(), 3)]);
        assert_eq!(back.capabilities_total, 3);
    }

    #[test]
    fn denominator_cannot_be_gamed() {
        // Registered donors are counted forever; an active donor without capabilities is 0 %.
        let c = Counts {
            donors_registered: 2,
            donors_extinct: 1,
            ..Counts::default()
        };
        let v = c.values();
        let get = |k: &str| v.iter().find(|(n, _)| n == k).unwrap().1.clone();
        assert_eq!(get("extinction_ratio"), "0.500000");
        assert_eq!(get("production_native_ratio"), "0.000000");
        // Unregistered externals join the denominator.
        let c = Counts {
            unregistered_externals: 1,
            ..Counts::default()
        };
        assert_eq!(
            c.values()
                .iter()
                .find(|(n, _)| n == "extinction_ratio")
                .unwrap()
                .1,
            "0.000000"
        );
        // Nothing foreign at all: vacuously complete.
        let c = Counts::default();
        assert_eq!(
            c.values()
                .iter()
                .find(|(n, _)| n == "extinction_ratio")
                .unwrap()
                .1,
            "1.000000"
        );
    }

    #[test]
    fn aggregation_sums_then_divides() {
        let a = Counts {
            donors_registered: 1,
            donors_extinct: 1,
            ..Counts::default()
        };
        let b = Counts {
            donors_registered: 3,
            donors_extinct: 0,
            ..Counts::default()
        };
        let mut t = a.clone();
        t.add(&b);
        // Not the mean of 1.0 and 0.0.
        assert_eq!(
            t.values()
                .iter()
                .find(|(n, _)| n == "extinction_ratio")
                .unwrap()
                .1,
            "0.250000"
        );
    }
}
