//! ECDEV REPOSITORY CONFORMANCE: the executable suite of ECDEV's own invariants. It measures
//! only this repository: one governance authority (`.ecdev/`), typed declarations, a closed
//! graph, lifecycle claims within evidence, capability relevance justified, graph-owned source,
//! no crate buckets, no external governance dependency, deterministic output.

use crate::compact::facts::{fold, Fact};
use crate::declare;
use crate::extinction::{gates, CapabilityVerdict, DonorFacts};
use crate::formats::json::Json;
use crate::graph::{Graph, NodeId};
use crate::metrics::DEFINITIONS;
use crate::protocol::{schema_identity, GOVERNANCE_VERSION};
use crate::schema::{FactKind, Scope};
use crate::{Assessment, Severity};

#[derive(Clone, Debug)]
pub struct Check {
    pub id: &'static str,
    pub pass: bool,
    pub detail: String,
}

impl Check {
    fn new(id: &'static str, pass: bool, detail: impl Into<String>) -> Check {
        Check {
            id,
            pass,
            detail: detail.into(),
        }
    }
}

fn no_findings(a: &Assessment, id: &'static str, codes: &[&str]) -> Check {
    let hits: Vec<String> = a
        .findings
        .iter()
        .filter(|f| f.severity == Severity::Error && codes.contains(&f.code.as_str()))
        .map(|f| format!("{} {}", f.code, f.subject))
        .collect();
    let n = hits.len();
    Check::new(
        id,
        hits.is_empty(),
        if hits.is_empty() {
            "none".to_string()
        } else {
            format!(
                "{n}: {}",
                hits.into_iter().take(5).collect::<Vec<_>>().join("; ")
            )
        },
    )
}

/// Path fragments that would make an external governance system authoritative over ECDEV.
/// Built from parts so this file does not match itself.
pub fn external_governance_tokens() -> Vec<String> {
    vec![
        [".", "ynventa"].concat(),
        ["llgtrn/", ".", "Ynventa-"].concat(),
        ["chro", "nica"].concat(),
        ["shard", "_registered"].concat(),
    ]
}

/// Tracked files that ECDEV builds, tests, ships or runs as governance: historical research
/// records are exempt (they describe the past, they govern nothing).
fn governs(f: &str) -> bool {
    !(f.starts_with("research/") || f.ends_with(".lock") || f.ends_with("package-lock.json"))
}

/// Every tracked governing file that names an external governance system, with the token.
pub fn external_governance_references(a: &Assessment) -> Vec<String> {
    let tokens = external_governance_tokens();
    let mut out = Vec::new();
    for f in a.files.paths.iter().filter(|f| governs(f)) {
        let lower_path = f.to_ascii_lowercase();
        if let Some(t) = tokens
            .iter()
            .find(|t| lower_path.contains(&t.to_ascii_lowercase()))
        {
            out.push(format!("{f} (path names {t})"));
            continue;
        }
        let Some(text) = a.files.read(f) else {
            continue;
        };
        let lower = text.to_ascii_lowercase();
        if let Some(t) = tokens
            .iter()
            .find(|t| lower.contains(&t.to_ascii_lowercase()))
        {
            out.push(format!("{f} (names {t})"));
        }
    }
    out
}

/// Capability relevance that is undecided-but-claimed or contradicts `required`: a relied-on
/// capability must be required; a NOT_RELEVANT_TO_ECDEV capability must not be required and
/// must cite an existing exclusion review as its spec.
pub fn relevance_violations(a: &Assessment) -> Vec<String> {
    let mut out = Vec::new();
    for d in &a.declaration.donors {
        for c in &d.capabilities {
            match &c.relevance {
                declare::Relevance::ReliedOn(_) if !c.required => {
                    out.push(format!("{}/{}: RELIED_ON but not required", d.key, c.key))
                }
                declare::Relevance::NotRelevant(_) if c.required => out.push(format!(
                    "{}/{}: NOT_RELEVANT_TO_ECDEV but required",
                    d.key, c.key
                )),
                declare::Relevance::NotRelevant(_) if !a.files.exists(&c.spec) => {
                    out.push(format!(
                        "{}/{}: NOT_RELEVANT_TO_ECDEV without a tracked exclusion review ({})",
                        d.key, c.key, c.spec
                    ))
                }
                r if !matches!(r, declare::Relevance::Unresolved) && !r.resolved() => {
                    out.push(format!("{}/{}: empty relevance reason", d.key, c.key))
                }
                _ => {}
            }
        }
    }
    out
}

fn list_check(id: &'static str, v: Vec<String>, ok: &str) -> Check {
    Check::new(
        id,
        v.is_empty(),
        if v.is_empty() {
            ok.to_string()
        } else {
            format!(
                "{}: {}",
                v.len(),
                v.into_iter().take(5).collect::<Vec<_>>().join("; ")
            )
        },
    )
}

/// The ECDEV repository conformance checks. `ecdev_repository_conformance` = passed / total of
/// exactly these.
pub fn checks(a: &Assessment) -> Vec<Check> {
    let mut v = Vec::new();
    let d = &a.declaration;
    v.push(Check::new(
        "governance.single_authority",
        declare::DECLARATION_FILES
            .iter()
            .all(|f| declare::declared_dir(&a.root).join(f).is_file())
            && d.repository.id == "ecdev",
        format!(
            "{} holds the complete declaration set of repository `{}`",
            declare::DECLARED_DIR,
            d.repository.id
        ),
    ));
    v.push(list_check(
        "governance.no_external_authority",
        external_governance_references(a),
        "no governing file names an external governance system, shard registry or protocol pin",
    ));
    let fixed = declare::parse_repository(&declare::render_repository(&d.repository)).ok()
        == Some(d.repository.clone())
        && declare::parse_donors(&declare::render_donors(&d.donors)).ok() == Some(d.donors.clone())
        && declare::parse_migration(&declare::render_migration(&d.migration)).ok()
            == Some(d.migration.clone())
        && declare::parse_technologies(&declare::render_technologies(&d.technologies)).ok()
            == Some(d.technologies.clone());
    v.push(Check::new(
        "declarations.typed_roundtrip",
        fixed,
        "declarations render and re-read identically",
    ));

    v.push(no_findings(
        a,
        "graph.schema",
        &[
            "DUPLICATE_NODE_KEY",
            "RESERVED_NODE_KEY",
            "DANGLING_EDGE",
            "ILLEGAL_EDGE",
            "DEPENDENCY_CYCLE",
            "UNKNOWN_LINEAGE_DONOR",
            "IDENTITY_KIND_CONFLICT",
        ],
    ));
    let ids_ok = a.graph.nodes.values().all(|n| {
        NodeId::of(&n.namespace, &n.semantic_key) == n.id
            && matches!(n.namespace.as_str(), "ecdev" | "oss" | "external")
    });
    v.push(Check::new(
        "graph.identity",
        ids_ok,
        "every id = H(ecdev | oss | external, semantic_key)",
    ));
    let bytes = a.graph.encode();
    let rt = Graph::decode(&bytes)
        .map(|g| g.encode() == bytes && g == a.graph)
        .unwrap_or(false);
    v.push(Check::new(
        "graph.roundtrip",
        rt,
        format!(
            "{} nodes, {} edges, {} bytes",
            a.graph.nodes.len(),
            a.graph.edges.len(),
            bytes.len()
        ),
    ));

    v.push(no_findings(
        a,
        "lifecycle.claims_within_evidence",
        &[
            "CLAIM_EXCEEDS_EVIDENCE",
            "TECHNOLOGY_CLAIM_EXCEEDS_EVIDENCE",
            "NATIVE_CLAIM_WITHOUT_EVIDENCE",
            "EXTINCTION_WITHOUT_PARITY",
        ],
    ));
    v.push(no_findings(
        a,
        "lifecycle.lineage_and_proofs",
        &["DONOR_WITHOUT_LINEAGE", "MISSING_PROOF"],
    ));
    v.push(list_check(
        "lifecycle.relevance_justified",
        relevance_violations(a),
        "every resolved capability relevance agrees with `required` and cites its review",
    ));
    v.push(no_findings(
        a,
        "repository.builds_alone",
        &["CROSS_REPO_SOURCE_DEPENDENCY"],
    ));
    let structure = structure_violations(a);
    v.push(Check::new(
        "structure.compatible_root",
        structure.is_empty(),
        if structure.is_empty() {
            format!(
                "canonical roles, full {} set, no non-canonical code root, no crate buckets",
                declare::DECLARED_DIR
            )
        } else {
            format!(
                "{}: {}",
                structure.len(),
                structure.into_iter().take(5).collect::<Vec<_>>().join("; ")
            )
        },
    ));
    v.push(no_findings(
        a,
        "shape.graph_owned_source",
        &["UNOWNED_CODE"],
    ));
    v.push(no_findings(
        a,
        "lifecycle.exceptions_legal",
        &["ILLEGAL_EXCEPTION"],
    ));
    v.push(no_findings(
        a,
        "lifecycle.no_vanished_donor",
        &["DONOR_VANISHED"],
    ));
    v.push(no_findings(
        a,
        "donors.every_external_registered",
        &["UNREGISTERED_EXTERNAL", "DISCOVERED_BUT_ACTIVE"],
    ));
    v.push(no_findings(a, "state.integrity", &["CORRUPT_STATE"]));
    v.push(no_findings(
        a,
        "universe.lossless_identity",
        &["UNIVERSE_INVALID", "UNPROVEN_ALIAS"],
    ));
    v.push(no_findings(
        a,
        "migration.shims_expire",
        &[
            "SHIM_WITHOUT_VALID_EXPIRY",
            "EXPIRED_SHIM_PRESENT",
            "COMPAT_WITHOUT_EXPIRY",
        ],
    ));
    v.push(no_findings(
        a,
        "migration.waves_consistent",
        &["WAVE_UNKNOWN_NODE", "NODE_IN_TWO_WAVES", "WAVE_NOT_APPLIED"],
    ));

    let values = a.counts.values();
    let names_ok = values.len() == DEFINITIONS.len()
        && values
            .iter()
            .zip(DEFINITIONS)
            .all(|((n, _), d)| n == d.name);
    let ratios_ok = values
        .iter()
        .filter(|(_, v)| v.contains('.'))
        .all(|(_, v)| v.as_str() <= "1.000000" && v.as_str() >= "0.000000");
    let c = &a.counts;
    v.push(Check::new(
        "metrics.schema",
        names_ok
            && ratios_ok
            && c.capabilities_proven <= c.capabilities_native
            && c.capabilities_native <= c.capabilities_total,
        format!("{} metrics in schema order", values.len()),
    ));

    let manifest = std::fs::read_to_string(a.root.join(".ecdev/Cargo.toml")).unwrap_or_default();
    let zero = crate::formats::toml::parse(&manifest)
        .map(|m| {
            ["dependencies", "build-dependencies", "dev-dependencies"]
                .iter()
                .all(|s| {
                    m.get(s)
                        .and_then(|t| t.table())
                        .is_none_or(|t| t.is_empty())
                })
        })
        .unwrap_or(false);
    let self_obs = a
        .census
        .observations
        .iter()
        .filter(|o| o.file.starts_with(".ecdev/"))
        .count();
    v.push(Check::new(
        "governance.zero_dependency",
        zero && self_obs == 0,
        format!("manifest dependency-free: {zero}; foreign references from .ecdev: {self_obs}"),
    ));

    v.push(no_findings(
        a,
        "generated.untracked",
        &["GENERATED_TRACKED"],
    ));

    // Determinism: a second, independent assessment of the same universe agrees exactly.
    let again = crate::assess_core(&a.root, a.files.clone(), a.declaration.clone());
    let det = again.counts.values() == crate::count_public(a).values()
        && again.graph.encode() == bytes
        && again.findings == a.findings;
    v.push(Check::new(
        "output.deterministic",
        det,
        "metrics, graph bytes and findings are identical on re-assessment",
    ));

    v.extend(probes());
    v
}

/// In-memory probes of the verifier's semantics.
pub fn probes() -> Vec<Check> {
    let mut v = Vec::new();
    // Identity survives a move.
    let before = NodeId::of("probe", "storage");
    let mut n = crate::graph::GNode::new(
        "probe",
        "storage",
        crate::schema::NodeKind::Substrate,
        "storage",
    );
    n.path = "storage".into();
    let moved = {
        let mut m = n.clone();
        m.path = "substrate/storage".into();
        m
    };
    v.push(Check::new(
        "probe.identity_survives_moves",
        n.id == before && moved.id == before,
        before.to_string(),
    ));

    // A donor with one surviving build edge is not extinct, whatever else holds.
    let cap = CapabilityVerdict {
        key: "c".into(),
        required: true,
        specified: true,
        targeted: true,
        replacement_exists: true,
        replacement_canonical: true,
        mapped: true,
        relevance_resolved: true,
        native: true,
        native_detail: String::new(),
        parity: vec![("t::p".into(), crate::evidence::Verdict::Pass)],
        regression: vec![("t::r".into(), crate::evidence::Verdict::Pass)],
    };
    let clean = gates(&DonorFacts::default(), std::slice::from_ref(&cap), true)
        .iter()
        .all(|g| g.pass);
    let mut dirty = DonorFacts::default();
    dirty.push(crate::census::Observation {
        file: "x/Cargo.toml".into(),
        ecosystem: crate::schema::Ecosystem::Cargo,
        name: "d".into(),
        ident: "d".into(),
        scope: Scope::Build,
        via: crate::census::Via::Manifest,
    });
    let still = gates(&dirty, &[cap], true).iter().all(|g| g.pass);
    v.push(Check::new(
        "probe.no_false_extinction",
        clean && !still,
        "one BUILD edge blocks EXTINCT",
    ));

    // Compaction is order-independent and supersedes.
    let f1 = Fact::new(FactKind::Definition, "", "t", "old", "p1", 1);
    let f2 = Fact::new(FactKind::Definition, "", "t", "new", "p2", 2);
    let x = fold(vec![f1.clone(), f2.clone()]);
    let y = fold(vec![f2, f1]);
    v.push(Check::new(
        "probe.compaction",
        x == y && x.len() == 1 && x[0].value == "new" && x[0].superseded == ["old"],
        "fold is order-independent; newer supersedes",
    ));

    // Path rewriting keeps workspaces resolvable.
    let moves = vec![("storage".to_string(), "substrate/storage".to_string())];
    let out = crate::migration::pathmap::rewrite_paths(
        "core = { path = \"../core\" }",
        "storage",
        "substrate/storage",
        &moves,
    );
    v.push(Check::new(
        "probe.migration_pathmap",
        out.contains("\"../../core\""),
        out,
    ));
    v
}

/// Structural compatibility of the repository root: the full `.ecdev/declared/` set, only
/// canonical roles and legal files at the root, no non-canonical top-level code root, no
/// forbidden container (`crate/`, `crates/`, ...). Reuses the shape audit's findings; returns the
/// violations (empty when compatible).
pub fn structure_violations(a: &Assessment) -> Vec<String> {
    let mut v: Vec<String> = declare::DECLARATION_FILES
        .iter()
        .filter(|f| !declare::declared_dir(&a.root).join(f).is_file())
        .map(|f| format!("MISSING_DECLARATION {}/{f}", declare::DECLARED_DIR))
        .collect();
    for f in &a.shape.findings {
        let structural = matches!(
            f.code.as_str(),
            "ILLEGAL_ROOT" | "ILLEGAL_ROOT_FILE" | "FORBIDDEN_CONTAINER" | "LEGACY_ROOT"
        );
        if structural {
            v.push(format!("{} {}", f.code, f.subject));
        }
    }
    v.sort();
    v.dedup();
    v
}

/// What the governance test suite requires of the ECDEV repository: every repository
/// conformance check passes. The V1 gate is ECDEV's target, not its current state, and is not
/// asserted. Returns the failures; empty when the repository conforms.
pub fn self_conformance(a: &Assessment) -> Vec<String> {
    checks(a)
        .into_iter()
        .filter(|c| !c.pass)
        .map(|c| format!("check {}: {}", c.id, c.detail))
        .collect()
}

pub struct Suite {
    pub checks: Vec<Check>,
}

pub fn run(a: &Assessment) -> Suite {
    Suite { checks: checks(a) }
}

pub fn render_json(a: &Assessment, suite: &Suite) -> Json {
    let checks: Vec<Json> = suite
        .checks
        .iter()
        .map(|c| {
            Json::obj()
                .with("id", c.id)
                .with("pass", c.pass)
                .with("detail", &c.detail)
        })
        .collect();
    let gate: Vec<Json> = a
        .counts
        .v1_gate()
        .into_iter()
        .map(|(k, want, got, pass)| {
            Json::obj()
                .with("metric", k)
                .with("required", want)
                .with("actual", got)
                .with("pass", pass)
        })
        .collect();
    Json::obj()
        .with("schema", schema_identity())
        .with("suite", "ECDEV REPOSITORY CONFORMANCE")
        .with("governance_version", GOVERNANCE_VERSION as u64)
        .with("repository", &a.declaration.repository.id)
        .with("origin", &a.declaration.repository.origin)
        .with("checks", Json::Array(checks))
        .with("metrics", a.counts.to_json())
        .with("v1_gate", Json::Array(gate))
        .with(
            "findings",
            Json::Array(a.findings.iter().map(|f| f.to_json()).collect()),
        )
}
