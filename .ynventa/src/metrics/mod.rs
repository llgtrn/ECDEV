//! THE metric schema. Every repository emits exactly these metrics, computed by exactly these
//! formulas; the ecosystem aggregate sums numerators and denominators and never averages ratios.
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
    MetricDef { name: "donors_rejected", formula: "donors with a legal REJECTED exception (never participated in any census)" },
    MetricDef { name: "donors_superseded", formula: "donors with a legal SUPERSEDED exception (successor carries packages and capabilities)" },
    MetricDef { name: "donors_active", formula: "donors_registered - donors_rejected - donors_superseded (vanished donors stay active)" },
    MetricDef { name: "capabilities_total", formula: "required capabilities of declared active donors" },
    MetricDef { name: "capabilities_native", formula: "of those, with an existing canonical native replacement that does not use the donor, transitively" },
    MetricDef { name: "capabilities_proven", formula: "of those native, with fresh passing parity proofs" },
    MetricDef { name: "capabilities_remaining", formula: "capabilities_total - capabilities_proven" },
    MetricDef { name: "technologies_total", formula: "declared technologies" },
    MetricDef { name: "technologies_native", formula: "technologies whose effective lifecycle >= NATIVE" },
    MetricDef { name: "technologies_proven", formula: "technologies whose effective lifecycle >= PROVEN" },
    MetricDef { name: "technologies_canonical", formula: "technologies whose effective lifecycle >= CANONICAL" },
    MetricDef { name: "technologies_adopted", formula: "technologies reused natively by another shard (decided by the system linker; 0 within one shard)" },
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
    MetricDef { name: "ynventa_protocol_conformance", formula: "passed protocol checks / protocol checks" },
    MetricDef { name: "native_capability_ratio", formula: "capabilities_native / capabilities_total; 1 when no active donor remains, 0 when active donors declare no capability" },
    MetricDef { name: "proof_completion_ratio", formula: "capabilities_proven / capabilities_total; same guards" },
    MetricDef { name: "technology_native_ratio", formula: "technologies_native / technologies_total; 1 when there are none" },
    MetricDef { name: "extinction_ratio", formula: "donors_extinct / (donors_active + distinct unregistered external packages); 1 when the denominator is 0" },
    MetricDef { name: "donors_technology_mapped", formula: "active donors with effective state >= TECHNOLOGY_MAPPED" },
    MetricDef { name: "donors_norl_resolved", formula: "active donors with effective state >= NORL_RELEVANCE_RESOLVED" },
    MetricDef { name: "organism_capabilities", formula: "organism capabilities declared by norl" },
    MetricDef { name: "organism_undefined", formula: "organism capabilities whose capability or organ is missing from the norl graph" },
    MetricDef { name: "growth_defined", formula: "organism capabilities whose effective growth state is DEFINED" },
    MetricDef { name: "growth_exposed", formula: "organism capabilities whose effective growth state is EXPOSED" },
    MetricDef { name: "growth_experienced", formula: "organism capabilities whose effective growth state is EXPERIENCED" },
    MetricDef { name: "growth_evaluated", formula: "organism capabilities whose effective growth state is EVALUATED" },
    MetricDef { name: "growth_learned", formula: "organism capabilities whose effective growth state is LEARNED" },
    MetricDef { name: "growth_native", formula: "organism capabilities whose effective growth state is NATIVE" },
    MetricDef { name: "organism_borrowed", formula: "organism capabilities whose cognition source is a BORROWED_WEIGHT backend" },
    MetricDef { name: "organism_native_backed", formula: "organism capabilities whose backend is native (DETERMINISTIC, NATIVE_TINY_WEIGHT, NATIVE_SCALED_WEIGHT)" },
    MetricDef { name: "organism_regressed", formula: "organism capabilities with an evaluation whose latest evidence FAILED" },
    MetricDef { name: "organism_with_experience", formula: "organism capabilities fed by at least one generated experience" },
    MetricDef { name: "organism_with_evaluation", formula: "organism capabilities with at least one evaluation with fresh passing evidence" },
    MetricDef { name: "materials_worlds", formula: "Material(WORLD) nodes that reach Norl (fed, observed, acted on, or used by material that does)" },
    MetricDef { name: "materials_observations", formula: "Material(OBSERVATION) nodes that reach Norl" },
    MetricDef { name: "materials_actions", formula: "Material(ACTION) nodes that reach Norl" },
    MetricDef { name: "materials_experiences", formula: "Material(EXPERIENCE) nodes that reach Norl" },
    MetricDef { name: "materials_curricula", formula: "Material(CURRICULUM) nodes that reach Norl" },
    MetricDef { name: "materials_evaluations", formula: "Material(EVALUATION) nodes that reach Norl" },
    MetricDef { name: "orphan_capabilities", formula: "capabilities provided by a product shard's physical node with no implementing technology, no GENERATES/FEEDS chain and no material relation" },
    MetricDef { name: "orphan_technologies", formula: "technologies implementing nothing, or neither feeding Norl nor declared not relevant to it" },
    MetricDef { name: "sharing_shared_implementation", formula: "technology/copy pairs: one canonical technology REUSED by another shard" },
    MetricDef { name: "sharing_independent_implementation", formula: "technology pairs implementing one capability with a direct family relation and different sources" },
    MetricDef { name: "sharing_domain_specialization", formula: "technology pairs implementing one capability where one SPECIALIZES/GENERALIZES the other" },
    MetricDef { name: "sharing_shared_concept", formula: "technology pairs implementing one capability in one declared family with no direct relation" },
    MetricDef { name: "sharing_unrelated_duplicate", formula: "technology pairs implementing one capability with no declared relation" },
    MetricDef { name: "native_independence", formula: "organism_native_backed / (organism_native_backed + organism_borrowed); undefined when both are 0" },
    MetricDef { name: "experience_coverage", formula: "organism_with_experience / organism_capabilities; undefined when there are none" },
    MetricDef { name: "evaluation_coverage", formula: "organism_with_evaluation / organism_capabilities; undefined when there are none" },
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
    MetricDef { name: "unique_global_donors", formula: "distinct global donors (after declared aliases); in the ecosystem deduplicated across repositories by the linker, never the sum of the repositories' counts" },
    MetricDef { name: "repository_donor_references", formula: "RepositoryUsesDonor relations with a global donor; the ecosystem value is their raw sum over repositories" },
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
    MetricDef { name: "ambiguous_donor_identities", formula: "groups of distinct OSS global donors that look alike (same repository name, different host or owner) and no alias joins; in the ecosystem recomputed across repositories" },
    MetricDef { name: "universe_sources_unreconciled", formula: "legacy registries of the repository with no universe file of the same bytes (NOT RECONCILED or STALE)" },
    MetricDef { name: "organism_source_proven", formula: "organism capabilities whose source maturity is PROVEN_IN_SOURCE (another repository proves what feeds or is promoted into them); never Norl's growth" },
    MetricDef { name: "promotions_total", formula: "promotions declared by norl" },
    MetricDef { name: "promotions_proven_in_source", formula: "promotions at effective state >= PROVEN_IN_SOURCE" },
    MetricDef { name: "promotions_native_implementation", formula: "promotions at effective state >= NATIVE_NORL_IMPLEMENTATION" },
    MetricDef { name: "promotions_norl_native", formula: "promotions at effective state NORL_NATIVE" },
];

/// The V1 success gate.
pub const V1_GATE: &[(&str, &str)] = &[
    ("extinction_ratio", "1.000000"),
    ("runtime_external_edges", "0"),
    ("build_external_edges", "0"),
    ("linked_external_edges", "0"),
    ("external_technology_edges", "0"),
    ("native_capability_ratio", "1.000000"),
    ("proof_completion_ratio", "1.000000"),
    ("repository_shape_conformance", "1.000000"),
    ("documents_over_budget", "0"),
    ("ynventa_protocol_conformance", "1.000000"),
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
    pub technologies_adopted: u64,
    pub unmapped_nodes: u64,
    pub external_closure_packages: u64,
    pub canonical_nodes_total: u64,
    pub canonical_nodes_conformant: u64,
    pub documents_total: u64,
    pub documents_over_budget: u64,
    pub shape_units: u64,
    pub shape_units_conformant: u64,
    pub protocol_checks: u64,
    pub protocol_checks_passed: u64,
    pub donors_technology_mapped: u64,
    pub donors_norl_resolved: u64,
    pub organism_capabilities: u64,
    pub organism_undefined: u64,
    pub growth_defined: u64,
    pub growth_exposed: u64,
    pub growth_experienced: u64,
    pub growth_evaluated: u64,
    pub growth_learned: u64,
    pub growth_native: u64,
    pub organism_borrowed: u64,
    pub organism_native_backed: u64,
    pub organism_regressed: u64,
    pub organism_with_experience: u64,
    pub organism_with_evaluation: u64,
    pub materials_worlds: u64,
    pub materials_observations: u64,
    pub materials_actions: u64,
    pub materials_experiences: u64,
    pub materials_curricula: u64,
    pub materials_evaluations: u64,
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
    pub organism_source_proven: u64,
    pub promotions_total: u64,
    pub promotions_proven_in_source: u64,
    pub promotions_native_implementation: u64,
    pub promotions_norl_native: u64,
}

impl Counts {
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
            protocol_checks,
            protocol_checks_passed,
            external_technology_edges,
            technologies_total,
            technologies_native,
            technologies_proven,
            technologies_canonical,
            technologies_adopted,
            unmapped_nodes,
            donors_technology_mapped,
            donors_norl_resolved,
            organism_capabilities,
            organism_undefined,
            growth_defined,
            growth_exposed,
            growth_experienced,
            growth_evaluated,
            growth_learned,
            growth_native,
            organism_borrowed,
            organism_native_backed,
            organism_regressed,
            organism_with_experience,
            organism_with_evaluation,
            materials_worlds,
            materials_observations,
            materials_actions,
            materials_experiences,
            materials_curricula,
            materials_evaluations,
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
            organism_source_proven,
            promotions_total,
            promotions_proven_in_source,
            promotions_native_implementation,
            promotions_norl_native
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
            ("capabilities_total", self.capabilities_total),
            ("capabilities_native", self.capabilities_native),
            ("capabilities_proven", self.capabilities_proven),
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
            ("protocol_checks", self.protocol_checks),
            ("protocol_checks_passed", self.protocol_checks_passed),
            ("external_technology_edges", self.external_technology_edges),
            ("technologies_total", self.technologies_total),
            ("technologies_native", self.technologies_native),
            ("technologies_proven", self.technologies_proven),
            ("technologies_canonical", self.technologies_canonical),
            ("technologies_adopted", self.technologies_adopted),
            ("unmapped_nodes", self.unmapped_nodes),
            ("donors_technology_mapped", self.donors_technology_mapped),
            ("donors_norl_resolved", self.donors_norl_resolved),
            ("organism_capabilities", self.organism_capabilities),
            ("organism_undefined", self.organism_undefined),
            ("growth_defined", self.growth_defined),
            ("growth_exposed", self.growth_exposed),
            ("growth_experienced", self.growth_experienced),
            ("growth_evaluated", self.growth_evaluated),
            ("growth_learned", self.growth_learned),
            ("growth_native", self.growth_native),
            ("organism_borrowed", self.organism_borrowed),
            ("organism_native_backed", self.organism_native_backed),
            ("organism_regressed", self.organism_regressed),
            ("organism_with_experience", self.organism_with_experience),
            ("organism_with_evaluation", self.organism_with_evaluation),
            ("materials_worlds", self.materials_worlds),
            ("materials_observations", self.materials_observations),
            ("materials_actions", self.materials_actions),
            ("materials_experiences", self.materials_experiences),
            ("materials_curricula", self.materials_curricula),
            ("materials_evaluations", self.materials_evaluations),
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
            ("organism_source_proven", self.organism_source_proven),
            ("promotions_total", self.promotions_total),
            (
                "promotions_proven_in_source",
                self.promotions_proven_in_source,
            ),
            (
                "promotions_native_implementation",
                self.promotions_native_implementation,
            ),
            ("promotions_norl_native", self.promotions_norl_native),
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
                "capabilities_total" => c.capabilities_total = *v,
                "capabilities_native" => c.capabilities_native = *v,
                "capabilities_proven" => c.capabilities_proven = *v,
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
                "protocol_checks" => c.protocol_checks = *v,
                "protocol_checks_passed" => c.protocol_checks_passed = *v,
                "external_technology_edges" => c.external_technology_edges = *v,
                "technologies_total" => c.technologies_total = *v,
                "technologies_native" => c.technologies_native = *v,
                "technologies_proven" => c.technologies_proven = *v,
                "technologies_canonical" => c.technologies_canonical = *v,
                "technologies_adopted" => c.technologies_adopted = *v,
                "unmapped_nodes" => c.unmapped_nodes = *v,
                "donors_technology_mapped" => c.donors_technology_mapped = *v,
                "donors_norl_resolved" => c.donors_norl_resolved = *v,
                "organism_capabilities" => c.organism_capabilities = *v,
                "organism_undefined" => c.organism_undefined = *v,
                "growth_defined" => c.growth_defined = *v,
                "growth_exposed" => c.growth_exposed = *v,
                "growth_experienced" => c.growth_experienced = *v,
                "growth_evaluated" => c.growth_evaluated = *v,
                "growth_learned" => c.growth_learned = *v,
                "growth_native" => c.growth_native = *v,
                "organism_borrowed" => c.organism_borrowed = *v,
                "organism_native_backed" => c.organism_native_backed = *v,
                "organism_regressed" => c.organism_regressed = *v,
                "organism_with_experience" => c.organism_with_experience = *v,
                "organism_with_evaluation" => c.organism_with_evaluation = *v,
                "materials_worlds" => c.materials_worlds = *v,
                "materials_observations" => c.materials_observations = *v,
                "materials_actions" => c.materials_actions = *v,
                "materials_experiences" => c.materials_experiences = *v,
                "materials_curricula" => c.materials_curricula = *v,
                "materials_evaluations" => c.materials_evaluations = *v,
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
                "organism_source_proven" => c.organism_source_proven = *v,
                "promotions_total" => c.promotions_total = *v,
                "promotions_proven_in_source" => c.promotions_proven_in_source = *v,
                "promotions_native_implementation" => c.promotions_native_implementation = *v,
                "promotions_norl_native" => c.promotions_norl_native = *v,
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
        let proto = if self.protocol_checks == 0 {
            Ratio::new(0, 1)
        } else {
            Ratio::new(self.protocol_checks_passed, self.protocol_checks)
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
            ("capabilities_total", n(self.capabilities_total)),
            ("capabilities_native", n(self.capabilities_native)),
            ("capabilities_proven", n(self.capabilities_proven)),
            (
                "capabilities_remaining",
                n(self
                    .capabilities_total
                    .saturating_sub(self.capabilities_proven)),
            ),
            ("technologies_total", n(self.technologies_total)),
            ("technologies_native", n(self.technologies_native)),
            ("technologies_proven", n(self.technologies_proven)),
            ("technologies_canonical", n(self.technologies_canonical)),
            ("technologies_adopted", n(self.technologies_adopted)),
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
            ("ynventa_protocol_conformance", proto.render()),
            (
                "native_capability_ratio",
                self.capability_ratio(self.capabilities_native).render(),
            ),
            (
                "proof_completion_ratio",
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
            ("donors_norl_resolved", n(self.donors_norl_resolved)),
            ("organism_capabilities", n(self.organism_capabilities)),
            ("organism_undefined", n(self.organism_undefined)),
            ("growth_defined", n(self.growth_defined)),
            ("growth_exposed", n(self.growth_exposed)),
            ("growth_experienced", n(self.growth_experienced)),
            ("growth_evaluated", n(self.growth_evaluated)),
            ("growth_learned", n(self.growth_learned)),
            ("growth_native", n(self.growth_native)),
            ("organism_borrowed", n(self.organism_borrowed)),
            ("organism_native_backed", n(self.organism_native_backed)),
            ("organism_regressed", n(self.organism_regressed)),
            ("organism_with_experience", n(self.organism_with_experience)),
            ("organism_with_evaluation", n(self.organism_with_evaluation)),
            ("materials_worlds", n(self.materials_worlds)),
            ("materials_observations", n(self.materials_observations)),
            ("materials_actions", n(self.materials_actions)),
            ("materials_experiences", n(self.materials_experiences)),
            ("materials_curricula", n(self.materials_curricula)),
            ("materials_evaluations", n(self.materials_evaluations)),
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
            (
                "native_independence",
                Ratio::new(
                    self.organism_native_backed,
                    self.organism_native_backed + self.organism_borrowed,
                )
                .render(),
            ),
            (
                "experience_coverage",
                Ratio::new(self.organism_with_experience, self.organism_capabilities).render(),
            ),
            (
                "evaluation_coverage",
                Ratio::new(self.organism_with_evaluation, self.organism_capabilities).render(),
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
            ("organism_source_proven", n(self.organism_source_proven)),
            ("promotions_total", n(self.promotions_total)),
            (
                "promotions_proven_in_source",
                n(self.promotions_proven_in_source),
            ),
            (
                "promotions_native_implementation",
                n(self.promotions_native_implementation),
            ),
            ("promotions_norl_native", n(self.promotions_norl_native)),
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
        assert_eq!(get("native_capability_ratio"), "0.000000");
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
