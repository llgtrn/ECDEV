//! Protocol identity. The schema identity is the digest of the canonical rendering of every
//! vocabulary, the repository shape, the extinction gates, the metric formulas, the document
//! budget and the command set. Two repositories speak the same protocol iff their snapshots
//! carry the same version and schema identity; their subsystems are byte-identical iff the
//! subsystem digests agree.

use crate::digest::{content_digest, hex, Sha256};
use crate::schema::*;
use std::path::Path;

pub const YNVENTA_PROTOCOL_VERSION: u32 = 1;
pub const PROTOCOL_NAME: &str = "ynventa";
pub const SNAPSHOT_FILE: &str = ".ynventa/protocol.snapshot";

/// A physical shard of Chronica. The set is closed: these are the only repositories of the
/// ecosystem. `domain` prefixes the semantic keys the importer proposes for a shard's legacy
/// nodes (keys are Chronica-wide; the prefix is semantic, never the repository name).
pub struct Shard {
    pub id: &'static str,
    pub origin: &'static str,
    pub domain: &'static str,
}

pub const SHARDS: &[Shard] = &[
    Shard {
        id: "ynventa",
        origin: "llgtrn/.Ynventa-",
        domain: "",
    },
    Shard {
        id: "chronica",
        origin: "llgtrn/Chronica",
        domain: "",
    },
    Shard {
        id: "atlas-studio",
        origin: "llgtrn/Atlas-Studio",
        domain: "compiler",
    },
    Shard {
        id: "universal-connector",
        origin: "llgtrn/Universal-Connector",
        domain: "connector",
    },
    Shard {
        id: "esellios",
        origin: "llgtrn/Esellios",
        domain: "commerce",
    },
    Shard {
        id: "fi-game",
        origin: "llgtrn/Fi-game",
        domain: "finance",
    },
    Shard {
        id: "mechatron",
        origin: "llgtrn/Mechatron",
        domain: "machine",
    },
    Shard {
        id: "norl",
        origin: "llgtrn/Norl",
        domain: "norl",
    },
];

pub fn shard(id: &str) -> Option<&'static Shard> {
    SHARDS.iter().find(|s| s.id == id)
}

/// The canonical shard of an origin (`llgtrn/Chronica`, any case).
pub fn shard_of_origin(origin: &str) -> Option<&'static Shard> {
    SHARDS
        .iter()
        .find(|s| s.origin.eq_ignore_ascii_case(origin))
}

/// Every repository exposes exactly these commands with these semantics.
pub const COMMANDS: &[(&str, &str)] = &[
    ("audit", "shape, document budget and lifecycle findings"),
    ("backlinks", "derived inverse edges of a node: <key> [--system <image>]"),
    ("capsule", "compile this shard into <shard>.ynv: [--out <file>]"),
    ("census", "observe foreign participation; --record appends a history batch"),
    ("compact", "fold history, knowledge and evidence; write the capsule and view; --extract-docs, --extract-legacy, --prune-docs"),
    ("conformance", "run the conformance suite against --root; --reference compares snapshots"),
    ("donor", "summary [--ecosystem <roots or capsules>...] [--json]: the donor universe per repository (known universe by kind, lifecycle layers, identity and verification gaps); with --ecosystem, per-repository rows, raw cross-repo references against deduplicated unique global donors, and the ambiguity count; an unreconciled legacy registry is NOT RECONCILED, never 0"),
    ("context", "agent global context for this shard, with current milestones and decisions: [--system <image>]"),
    ("extinction", "extinction gates per donor (why is a donor not extinct?)"),
    ("fact", "typed knowledge facts: add | supersede <kind> <subject> <key> <value> --provenance <p>...; list [--kind <kind>] [--subject <prefix>]"),
    ("graph", "the shard graph; --json, --binary <file>, --merge <root>..."),
    ("knowledge", "view [--kind <kind>] [--text] [--out <file>] | view --document <path> [--outline] (read-only; rebuilds a stored document byte for byte); extract <document or licence text>... (exactly those)"),
    ("link", "link capsules or shard roots into chronica.system.ynv: <capsule|root>... [--out <file>] [--json]"),
    ("metrics", "the metric schema; --aggregate <root>... for the ecosystem"),
    ("migrate", "import | map | plan | apply <wave> | scaffold | schema (rewrite declarations into the current grammar) | universe --from <legacy registry> [--kind <KIND>] [--table <name>] [--dry-run] (lossless import into .ynventa/declared/universe/)"),
    ("organism", "the Norl organism: organs, growth states, feed chains, materials, shared technologies, donors: [--system <image>] [--against <older image>] [--json]"),
    ("protocol", "print the protocol snapshot; --write, --check, --schema"),
    ("prove", "run declared proofs and record content-bound evidence"),
    ("show", "a node, capability or technology with its edges: <key> [--system <image>]"),
    ("status", "metrics, V1 gate, knowledge (milestones, decisions) and the most important findings"),
    ("technology", "list | search <text> | consumers <key> | materialize <key> [--system <image>]"),
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
    let mut s = format!("protocol {PROTOCOL_NAME} {YNVENTA_PROTOCOL_VERSION}\n");
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
    vocab(
        &mut s,
        "growth_state",
        GrowthState::ALL,
        GrowthState::wire,
        GrowthState::meaning,
    );
    vocab(
        &mut s,
        "promotion_implementation",
        PromotionImplementation::ALL,
        PromotionImplementation::wire,
        PromotionImplementation::meaning,
    );
    vocab(
        &mut s,
        "promotion_state",
        PromotionState::ALL,
        PromotionState::wire,
        PromotionState::meaning,
    );
    s.push_str("promotion_rules legal only in norl; relation FEEDS or TEACHES from the source technology, else capability, else node; PROVEN_IN_SOURCE decided at link from the source shard's fresh passing proofs; COPIED_AND_DIVERGED and ADAPTED_IMPLEMENTATION with a destination node carry repository, NodeId, commit and technology; a destination node that REUSES the source technology is SHARED_IMPLEMENTATION; no reference names a file path; source maturity never raises Norl's growth\n");
    s.push_str("cross_repo_source_dependency a Cargo path dependency leaving the repository root is an error in every shard\n");
    vocab(
        &mut s,
        "source_maturity",
        SourceMaturity::ALL,
        SourceMaturity::wire,
        SourceMaturity::meaning,
    );
    vocab(
        &mut s,
        "backend_kind",
        BackendKind::ALL,
        BackendKind::wire,
        BackendKind::meaning,
    );
    vocab(
        &mut s,
        "technology_sharing",
        TechnologySharing::ALL,
        TechnologySharing::wire,
        TechnologySharing::meaning,
    );
    s.push_str("norl_relevance UNRESOLVED | FEEDS(<norl capability>) | NOT_RELEVANT(<reason>)\n");
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
    for sh in SHARDS {
        s.push_str(&format!(
            "shard {} origin={} domain={}\n",
            sh.id, sh.origin, sh.domain
        ));
    }
    s.push_str(&format!("system {}\n", crate::schema::SYSTEM));
    for (c, m) in COMMANDS {
        s.push_str(&format!("command {c} : {m}\n"));
    }
    s.push_str("node_id sha256(field(\"ynventa.node-id.v1\") field(namespace in chronica|oss|external) field(semantic_key))[0..16]\n");
    s.push_str("record_format YNV 1 leb128 length-prefixed-utf8 content-addressed sha256[0..16]\n");
    s
}

pub fn schema_identity() -> String {
    content_digest(schema_text().as_bytes())
}

/// Files of the canonical subsystem (relative to `.ynventa/`): everything except repository
/// state, build output and the snapshot itself.
pub fn subsystem_files(ynventa_dir: &Path) -> Vec<String> {
    fn walk(base: &Path, rel: &str, out: &mut Vec<String>) {
        let Ok(rd) = std::fs::read_dir(base.join(rel)) else {
            return;
        };
        let mut entries: Vec<_> = rd.filter_map(Result::ok).collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            if rel.is_empty()
                && matches!(
                    name.as_str(),
                    "target"
                        | "declared"
                        | "evidence"
                        | "history"
                        | "knowledge"
                        | "materialized"
                        | "protocol.snapshot"
                        | "Cargo.lock"
                )
            {
                continue;
            }
            match e.file_type() {
                Ok(t) if t.is_dir() => walk(base, &path, out),
                Ok(t) if t.is_file() => out.push(path),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(ynventa_dir, "", &mut out);
    out
}

/// Digest of the subsystem at `ynventa_dir`.
pub fn subsystem_digest(ynventa_dir: &Path) -> String {
    let mut h = Sha256::new();
    for f in subsystem_files(ynventa_dir) {
        h.field(f.as_bytes());
        h.field(&std::fs::read(ynventa_dir.join(&f)).unwrap_or_default());
    }
    format!("sha256:{}", hex(&h.finish()))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub protocol: String,
    pub version: u32,
    pub schema: String,
    pub subsystem: String,
    pub commands: String,
}

impl Snapshot {
    /// The snapshot of the running verifier, over the subsystem directory given.
    pub fn current(ynventa_dir: &Path) -> Snapshot {
        Snapshot {
            protocol: PROTOCOL_NAME.into(),
            version: YNVENTA_PROTOCOL_VERSION,
            schema: schema_identity(),
            subsystem: subsystem_digest(ynventa_dir),
            commands: COMMANDS
                .iter()
                .map(|(c, _)| *c)
                .collect::<Vec<_>>()
                .join(" "),
        }
    }

    pub fn render(&self) -> String {
        format!(
            "# Ynventa protocol snapshot. Generated by `ynventa protocol --write`; compared by `ynventa conformance`.\nprotocol {}\nversion {}\nschema {}\nsubsystem {}\ncommands {}\n",
            self.protocol, self.version, self.schema, self.subsystem, self.commands
        )
    }

    pub fn parse(text: &str) -> Option<Snapshot> {
        let get = |k: &str| {
            text.lines()
                .find_map(|l| l.strip_prefix(k).and_then(|r| r.strip_prefix(' ')))
                .map(str::to_string)
        };
        Some(Snapshot {
            protocol: get("protocol")?,
            version: get("version")?.parse().ok()?,
            schema: get("schema")?,
            subsystem: get("subsystem")?,
            commands: get("commands")?,
        })
    }

    pub fn load(root: &Path) -> Option<Snapshot> {
        Snapshot::parse(&std::fs::read_to_string(root.join(SNAPSHOT_FILE)).ok()?)
    }
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
        assert!(schema_text().contains("shard norl origin=llgtrn/Norl domain=norl"));
        let s = Snapshot::current(own_subsystem_dir());
        assert_eq!(Snapshot::parse(&s.render()), Some(s));
    }
}
