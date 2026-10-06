//! Governance identity. The schema identity is the digest of the canonical rendering of every
//! vocabulary, the repository shape, the extinction gates, the metric formulas, the document
//! budget and the command set. It changes exactly when ECDEV's governance semantics change, so
//! recorded evidence can name the schema it was judged under. Nothing outside ECDEV defines it.

use crate::digest::content_digest;
use crate::schema::*;
use std::path::Path;

pub const GOVERNANCE_VERSION: u32 = 1;
pub const GOVERNANCE_NAME: &str = "ecdev-governance";

/// The governance commands and their semantics.
pub const COMMANDS: &[(&str, &str)] = &[
    ("audit", "shape, document budget and lifecycle findings"),
    ("backlinks", "derived inverse edges of a node: <key>"),
    ("census", "observe foreign participation; --record appends a history batch"),
    ("compact", "fold history, knowledge and evidence; write the view; --extract-docs, --extract-legacy, --prune-docs"),
    ("conformance", "run ECDEV repository conformance against --root"),
    ("donor", "summary [--json]: the donor universe (known universe by kind, lifecycle layers, identity and verification gaps); an unreconciled legacy registry is NOT RECONCILED, never 0"),
    ("context", "agent context for the repository, with current milestones and decisions"),
    ("extinction", "extinction gates per donor (why is a donor not extinct?)"),
    ("fact", "typed knowledge facts: add | supersede <kind> <subject> <key> <value> --provenance <p>...; list [--kind <kind>] [--subject <prefix>]"),
    ("graph", "the repository graph; --json, --binary <file>"),
    ("knowledge", "view [--kind <kind>] [--text] [--out <file>] | view --document <path> [--outline] (read-only; rebuilds a stored document byte for byte); extract <document or licence text>... (exactly those)"),
    ("metrics", "the metric schema and values"),
    ("migrate", "import | map | plan | apply <wave> | universe --from <legacy registry> [--kind <KIND>] [--table <name>] [--dry-run] (lossless import into .ecdev/declared/universe/)"),
    ("protocol", "print the governance schema; --schema prints the full text"),
    ("prove", "run declared proofs and record content-bound evidence"),
    ("show", "a node, capability or technology with its edges: <key>"),
    ("status", "metrics, V1 gate, knowledge (milestones, decisions) and the most important findings"),
    ("technology", "list | search <text> | consumers <key> | materialize <key>"),
    ("verify", "every check; exit status 1 on any error finding"),
];

fn vocab<T: Copy>(
    out: &mut String,
    name: &str,
    all: &[T],
    wire: fn(T) -> &'static str,
    meaning: fn(T) -> &'static str,
) {
    for (i, v) in all.iter().enumerate() {
        out.push_str(&format!("{name} {i} {} : {}\n", wire(*v), meaning(*v)));
    }
}

/// The canonical rendering of the whole schema.
pub fn schema_text() -> String {
    let mut s = format!("governance {GOVERNANCE_NAME} {GOVERNANCE_VERSION}\n");
    vocab(
        &mut s,
        "node_kind",
        NodeKind::ALL,
        NodeKind::wire,
        NodeKind::meaning,
    );
    vocab(
        &mut s,
        "edge_kind",
        EdgeKind::ALL,
        EdgeKind::wire,
        EdgeKind::meaning,
    );
    vocab(
        &mut s,
        "concept",
        Concept::ALL,
        Concept::wire,
        Concept::meaning,
    );
    vocab(&mut s, "scope", Scope::ALL, Scope::wire, Scope::meaning);
    vocab(
        &mut s,
        "node_lifecycle",
        NodeLifecycle::ALL,
        NodeLifecycle::wire,
        NodeLifecycle::meaning,
    );
    vocab(
        &mut s,
        "donor_state",
        DonorState::ALL,
        DonorState::wire,
        DonorState::meaning,
    );
    vocab(
        &mut s,
        "exception",
        ExceptionKind::ALL,
        ExceptionKind::wire,
        ExceptionKind::meaning,
    );
    vocab(
        &mut s,
        "proof_kind",
        ProofKind::ALL,
        ProofKind::wire,
        ProofKind::meaning,
    );
    vocab(
        &mut s,
        "ecosystem",
        Ecosystem::ALL,
        Ecosystem::wire,
        Ecosystem::meaning,
    );
    vocab(&mut s, "gate", Gate::ALL, Gate::wire, Gate::meaning);
    s.push_str("cross_repo_source_dependency a Cargo path dependency leaving the repository root is an error\n");
    vocab(
        &mut s,
        "technology_sharing",
        TechnologySharing::ALL,
        TechnologySharing::wire,
        TechnologySharing::meaning,
    );
    s.push_str("capability_relevance (production adoption) UNRESOLVED | RELIED_ON(<ECDEV behaviour>) => required | NOT_ADOPTED(<reason citing the review>) => not required\n");
    s.push_str("capability_knowledge (research value, independent of adoption) UNREVIEWED | STUDY_CANDIDATE(<question>) | BENCHMARK_CANDIDATE(<question>) | ALGORITHM_CANDIDATE(<what to prove>) | REFERENCE_ONLY(<reason>) | STUDY_COMPLETE[DERIVED_NATIVE|INDEPENDENT_NATIVE|DELIBERATE_SEMANTIC_DIVERGENCE](<reason>) | NO_RESEARCH_VALUE[<scope>](<reason>); open = UNREVIEWED or a candidate\n");
    s.push_str(&format!(
        "research_scope (the only admissible grounds of NO_RESEARCH_VALUE) {}\n",
        crate::declare::ResearchScope::ALL
            .iter()
            .map(|s| s.word())
            .collect::<Vec<_>>()
            .join(" ")
    ));
    s.push_str(&format!(
        "inadmissible_irrelevance (never ground NO_RESEARCH_VALUE: they constrain adoption, not knowledge) {}\n",
        crate::declare::INADMISSIBLE_IRRELEVANCE.join(" | ")
    ));
    vocab(
        &mut s,
        "knowledge_state",
        KnowledgeState::ALL,
        KnowledgeState::wire,
        KnowledgeState::meaning,
    );
    vocab(
        &mut s,
        "universe_kind",
        UniverseKind::ALL,
        UniverseKind::wire,
        UniverseKind::meaning,
    );
    vocab(
        &mut s,
        "universe_state",
        UniverseState::ALL,
        UniverseState::wire,
        UniverseState::meaning,
    );
    s.push_str("universe_relevance UNRESOLVED | RELEVANT(<reason>) | NOT_RELEVANT(<reason>)\n");
    s.push_str("lifecycle_layers DISCOVERED REGISTERED RELEVANCE_RESOLVED SELECTED_FOR_CENSUS (universe records) then the donor ladder from CENSUSED\n");
    s.push_str(&format!(
        "universe_file {}/<legacy path with / as .>.jsonl {}: line 1 = source (path, format, digest, bytes, records, segments); one canonical record per line; the records and segments rebuild the legacy bytes\n",
        crate::universe::UNIVERSE_DIR,
        crate::universe::UNIVERSE_SCHEMA
    ));
    s.push_str(&format!(
        "universe_legacy_sources {}\n",
        crate::universe::LEGACY_SOURCES.join(",")
    ));
    s.push_str(&format!(
        "global_donor_id normalized upstream URL: scheme, userinfo, port, fragment, www., .git and trailing / dropped; host lowercased; forges ({}) host/owner/repo lowercased (GitLab: every group); elsewhere host/path[?query]; owner/repo = github.com; a name is never an identity; joined only by declared aliases with provenance; look-alikes (same repository name) are AMBIGUOUS_DONOR_IDENTITY, never merged; donor node ids keep the v1 key\n",
        crate::universe::identity::FORGES.join(",")
    ));
    vocab(&mut s, "role", Role::ALL, Role::wire, Role::meaning);
    vocab(
        &mut s,
        "shim_kind",
        ShimKind::ALL,
        ShimKind::wire,
        ShimKind::meaning,
    );
    vocab(
        &mut s,
        "expiry",
        ExpiryKind::ALL,
        ExpiryKind::wire,
        ExpiryKind::meaning,
    );
    vocab(
        &mut s,
        "wave_status",
        WaveStatus::ALL,
        WaveStatus::wire,
        WaveStatus::meaning,
    );
    vocab(
        &mut s,
        "native_status",
        NativeStatus::ALL,
        NativeStatus::wire,
        NativeStatus::meaning,
    );
    vocab(
        &mut s,
        "fact_kind",
        FactKind::ALL,
        FactKind::wire,
        FactKind::meaning,
    );
    s.push_str(&crate::repository::render_shape());
    s.push_str(&crate::metrics::render_definitions());
    s.push_str(&format!(
        "documents_allowed {}\n",
        crate::audit::ALLOWED_DOCUMENTS.join(",")
    ));
    s.push_str("documents_allowed_pattern <node path>/README.md,.github/**\n");
    s.push_str(&format!(
        "decision_records documents under a {} directory, except {} (case-insensitive); history, never stale\n",
        crate::audit::DECISION_RECORD_DIRS.join("|"),
        crate::audit::DECISION_INDEX_NAMES.join("|")
    ));
    s.push_str(&format!(
        "platform_libraries {}\n",
        crate::donors::PLATFORM_LIBRARIES.join(",")
    ));
    s.push_str(&format!(
        "toolchain_programs {}\n",
        crate::donors::TOOLCHAIN_PROGRAMS.join(",")
    ));
    s.push_str(&format!(
        "namespace {}
",
        crate::schema::NAMESPACE
    ));
    for (c, m) in COMMANDS {
        s.push_str(&format!("command {c} : {m}\n"));
    }
    s.push_str("node_id sha256(field(\"ecdev.node-id.v1\") field(namespace in ecdev|oss|external) field(semantic_key))[0..16]\n");
    s.push_str("record_format ECG 1 leb128 length-prefixed-utf8 content-addressed sha256[0..16]\n");
    s
}

pub fn schema_identity() -> String {
    content_digest(schema_text().as_bytes())
}

/// The directory of the subsystem this binary was built from.
pub fn own_subsystem_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_deterministic_and_sensitive() {
        assert_eq!(schema_identity(), schema_identity());
        assert!(schema_identity().starts_with("sha256:"));
        assert!(schema_text().contains("donor_state 10 EXTINCT"));
        assert!(schema_text().contains("namespace ecdev"));
        assert!(!schema_text().to_ascii_lowercase().contains("repo-d"));
    }
}
