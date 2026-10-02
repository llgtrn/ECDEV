//! The golden fixture: one legacy repository carried through the whole V1 path.
//!
//! legacy tree → semantic graph → stable NodeId → path relocation → capability replacement →
//! parity evidence → cutover → extinction → compaction → final conformance.

mod common;

use common::*;
use ynventa::declare::*;
use ynventa::graph::NodeId;
use ynventa::schema::*;

/// A Chronica-shaped legacy repository: roots per substrate owner, `.atlas` with the real
/// inventory dialect, committed donor source, sprawling Markdown, and two external crates.
fn legacy_repo() -> Repo {
    let r = Repo::new("golden");
    r.write("Cargo.toml", "[workspace]\nresolver = \"2\"\n# Every member is an ownership root.\nmembers = [\n    \"core\",\n    \"storage\",\n    \"tools/census\",\n    \"tests\",\n]\n");
    r.write("README.md", "# Chronica fixture\n");
    r.write(
        "ARCHITECTURE.md",
        "# Architecture\n\n- The page store is append-only.\n- Storage: durable pages and logs\n",
    );
    r.write(
        "core/Cargo.toml",
        "[package]\nname = \"core-kernel\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    r.write("core/src/lib.rs", "pub fn id() -> u64 { 1 }\n");
    r.write(
        "storage/Cargo.toml",
        "[package]\nname = \"storage\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\ncore-kernel = { path = \"../core\" }\nsha2 = \"0.11\"\n",
    );
    r.write("storage/src/lib.rs", "use sha2::Digest;\npub fn put(page: &[u8]) -> Vec<u8> { sha2::Sha256::digest(page).to_vec() }\n");
    r.write(
        "tools/census/Cargo.toml",
        "[package]\nname = \"census\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\ncore-kernel = { path = \"../../core\" }\nstorage = { path = \"../../storage\" }\ntoml = \"0.8\"\n",
    );
    r.write(
        "tools/census/src/main.rs",
        "fn main() { let _v: toml::Value = toml::from_str(\"a = 1\").unwrap(); }\n",
    );
    r.write("tests/Cargo.toml", "[package]\nname = \"verification\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dev-dependencies]\nstorage = { path = \"../storage\" }\n");
    r.write("tests/src/lib.rs", "");
    r.write(
        ".atlas/repo.toml",
        "schema = \"chronica.repo.v1\"\nrepo = \"llgtrn/Chronica\"\n",
    );
    r.write(
        ".atlas/census/declared.toml",
        "schema = \"chronica.census.declared.v1\"\n\n[[crate]]\nname = \"core\"\nroot = \"core\"\nplane = \"SUBSTRATE\"\n\n[[crate]]\nname = \"storage\"\nroot = \"storage\"\nplane = \"SUBSTRATE\"\n\n[[crate]]\nname = \"census\"\nroot = \"tools/census\"\nplane = \"TOOL\"\n\n[[crate]]\nname = \"verification\"\nroot = \"tests\"\nplane = \"TEST\"\n",
    );
    r.write(
        ".atlas/census/donors/inventory.toml",
        "schema = \"chronica.donors.inventory.v1\"\n\n[[donor]]\nid = \"D411\"\nrepo = \"sqlite/sqlite\"\nupstream = \"https://github.com/sqlite/sqlite\"\ncapabilities = [\"storage::pages\"]\nlifecycle = \"ABSORBED\"\nacquisition = \"COMMITTED_SNAPSHOT\"\nlicense = \"blessing\"\nlicense_state = \"VERIFIED\"\ndir = \"D411-sqlite-sqlite\"\n\n[[donor]]\nid = \"D900\"\nrepo = \"RustCrypto/hashes\"\nupstream = \"https://github.com/RustCrypto/hashes\"\ncapabilities = []\nlifecycle = \"PLANNED\"\nacquisition = \"NOT_ACQUIRED\"\nlicense = \"MIT OR Apache-2.0\"\nlicense_state = \"NOT_ACQUIRED\"\n",
    );
    r.write(
        ".atlas/temporary/donors/D411-sqlite-sqlite/source/btree.c",
        "/* sqlite btree */\nint sqlite3BtreeOpen(void) { return 0; }\n",
    );
    r.write(
        ".atlas/temporary/donors/D412-unknown/source/x.c",
        "int x(void) { return 1; }\n",
    );
    r.write(
        ".atlas/decisions/0001-pages.md",
        "# Pages are native\n\nStatus: Accepted\n\n- The page store is append-only.\n",
    );
    r.write(
        ".atlas/census/SESSION-2026-09-30.md",
        "# Session notes\n\n- The page store is append-only.\n- census iteration 12 ran\n",
    );
    r
}

#[test]
fn legacy_repository_reaches_v1() {
    let r = legacy_repo();

    // 1. Legacy tree → semantic graph (scaffold installs the subsystem and imports .atlas).
    let (code, out) = r.cli(&["migrate", "scaffold", "--shard", "chronica"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("chronica (2 records)"), "{out}");
    assert!(r.exists(".ynventa/src/lib.rs") && r.exists(".ynventa/protocol.snapshot"));
    let a = r.assess();
    let d = &a.declaration;
    let kinds: Vec<(&str, NodeKind, &str, &str)> = d
        .repository
        .nodes
        .iter()
        .map(|n| {
            (
                n.key.as_str(),
                n.kind,
                n.path.as_str(),
                n.canonical_path.as_str(),
            )
        })
        .collect();
    assert!(
        kinds.contains(&("core", NodeKind::Kernel, "core", "core")),
        "{kinds:?}"
    );
    assert!(
        kinds.contains(&(
            "storage",
            NodeKind::Substrate,
            "storage",
            "substrate/storage"
        )),
        "{kinds:?}"
    );
    assert!(
        kinds.contains(&("census", NodeKind::Tool, "tools/census", "tools/census")),
        "{kinds:?}"
    );
    assert!(
        kinds.contains(&("tests", NodeKind::Test, "tests", "tests")),
        "{kinds:?}"
    );
    let keys: Vec<&str> = d.donors.iter().map(|x| x.key.as_str()).collect();
    assert_eq!(
        keys,
        vec![
            "cargo-sha2",
            "cargo-toml",
            "d411-sqlite",
            "d900-hashes",
            "unadmitted-d412-unknown"
        ]
    );
    let sqlite = d.donor("d411-sqlite").unwrap();
    assert_eq!(
        sqlite.claimed,
        DonorState::ParityProven,
        "legacy ABSORBED is imported as a claim"
    );
    assert_eq!(
        sqlite.capabilities[0].replacement.as_deref(),
        Some("storage")
    );
    assert_eq!(
        d.donor("d900-hashes").unwrap().claimed,
        DonorState::Discovered
    );
    // The legacy claim is re-evaluated, not trusted.
    assert!(finding(&a, "CLAIM_EXCEEDS_EVIDENCE", "d411-sqlite").is_some());
    assert_eq!(
        a.donor("d411-sqlite").unwrap().effective,
        DonorState::Censused
    );
    assert!(a
        .knowledge
        .facts
        .iter()
        .any(|f| f.kind == FactKind::LegacyClaim
            && f.subject == "d411-sqlite"
            && f.value == "ABSORBED"));
    assert!(finding(&a, "LEGACY_PLACEMENT", "storage").is_some());
    assert!(finding(&a, "ILLEGAL_ROOT_FILE", "ARCHITECTURE.md").is_some());
    // Auto-registered externals lack a verified licence: they count as unregistered, active
    // technology (in every denominator) until someone records one.
    assert_eq!(
        a.metric("donors_registered"),
        "1",
        "only sqlite has origin and licence; D900 is a candidate"
    );
    assert!(finding(&a, "DISCOVERED_BUT_ACTIVE", "cargo-sha2").is_some());
    assert!(finding(&a, "DISCOVERED_BUT_ACTIVE", "unadmitted-d412-unknown").is_some());
    assert_eq!(a.metric("extinction_ratio"), "0.000000");
    let storage_id = NodeId::of("chronica", "storage");
    assert_eq!(
        a.graph.node_by_key("chronica", "storage").unwrap().id,
        storage_id
    );
    assert!(
        a.findings.iter().all(|f| !f.code.starts_with("DUPLICATE")),
        "{:#?}",
        a.findings
    );
    let (_, map) = r.cli(&["migrate", "map"]);
    assert!(
        map.contains(&format!(
            "storage -> storage {storage_id} -> substrate/storage"
        )),
        "{map}"
    );
    assert_eq!(d.migration.waves.len(), 1);
    assert_eq!(d.migration.waves[0].key, "w01-storage");

    // 2. Physical relocation in one atomic wave; identity is unchanged.
    let (code, out) = r.cli(&["migrate", "apply", "w01-storage"]);
    assert_eq!(code, 0, "{out}");
    assert!(!r.exists("storage") && r.exists("substrate/storage/Cargo.toml"));
    assert!(r.read("Cargo.toml").contains("\"substrate/storage\","));
    assert!(r
        .read("tools/census/Cargo.toml")
        .contains("storage = { path = \"../../substrate/storage\" }"));
    assert!(r
        .read("substrate/storage/Cargo.toml")
        .contains("core-kernel = { path = \"../../core\" }"));
    assert!(r
        .read("tests/Cargo.toml")
        .contains("storage = { path = \"../substrate/storage\" }"));
    let a = r.assess();
    assert_eq!(
        a.graph.node_by_key("chronica", "storage").unwrap().id,
        storage_id
    );
    assert_eq!(
        a.graph.node_by_key("chronica", "storage").unwrap().path,
        "substrate/storage"
    );
    assert!(finding(&a, "LEGACY_PLACEMENT", "storage").is_none());
    assert!(a
        .census
        .members
        .iter()
        .any(|m| m.dir == "substrate/storage"));
    assert!(
        a.census
            .internal
            .iter()
            .any(|(f, t, _)| f == "tools/census" && t == "substrate/storage"),
        "path dependencies still resolve"
    );
    assert_eq!(a.declaration.migration.waves[0].status, WaveStatus::Applied);

    // 3. Capabilities specified and replaced natively (the donors still run: NATIVE_SHADOW).
    r.write("core/src/lib.rs", "pub fn id() -> u64 { 1 }\npub fn sha256(_b: &[u8]) -> [u8; 32] { [0; 32] }\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn sha256_empty() {}\n}\n");
    r.write(
        "tests/tests/sha_parity.rs",
        "#[test]\nfn sha256_matches_nist_vectors() {}\n",
    );
    r.write("tests/tests/pages_parity.rs", "// Pages written natively are read back identically to the frozen sqlite oracle output.\n#[test]\nfn pages_match_sqlite_format() {}\n");
    r.write("substrate/storage/src/pages.rs", "pub struct Page;\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn pages_roundtrip() {}\n}\n");
    r.write("tools/census/src/manifest.rs", "pub fn read(_s: &str) {}\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn reads_like_toml() {}\n    #[test]\n    fn reads_members() {}\n}\n");
    let cap = |key: &str, spec: &str, node: &str, parity: &str, regression: &str| Capability {
        key: key.into(),
        required: true,
        spec: spec.into(),
        replacement: Some(node.into()),
        maps_to: Some(format!("capability/{key}")),
        norl: NorlRelevance::NotRelevant("plumbing; no developmental material".into()),
        proofs: vec![
            proof(ProofKind::Parity, parity),
            proof(ProofKind::Regression, regression),
        ],
    };
    r.edit(|d| {
        let s = d
            .donors
            .iter_mut()
            .find(|x| x.key == "d411-sqlite")
            .unwrap();
        s.capabilities = vec![cap(
            "storage-pages",
            "tests/tests/pages_parity.rs",
            "storage",
            "tests/tests/pages_parity.rs::pages_match_sqlite_format",
            "substrate/storage/src/pages.rs::pages_roundtrip",
        )];
        let h = d.donors.iter_mut().find(|x| x.key == "cargo-sha2").unwrap();
        h.license = "MIT OR Apache-2.0".into();
        h.claimed = DonorState::Registered;
        h.capabilities = vec![cap(
            "sha256",
            "FIPS 180-4 SHA-256 of a byte string",
            "core",
            "tests/tests/sha_parity.rs::sha256_matches_nist_vectors",
            "core/src/lib.rs::sha256_empty",
        )];
        let t = d.donors.iter_mut().find(|x| x.key == "cargo-toml").unwrap();
        t.license = "MIT OR Apache-2.0".into();
        t.claimed = DonorState::Registered;
        t.capabilities = vec![cap(
            "manifest-read",
            "read Cargo manifests: tables, arrays, strings",
            "census",
            "tools/census/src/manifest.rs::reads_like_toml",
            "tools/census/src/manifest.rs::reads_members",
        )];
        let u = d
            .donors
            .iter_mut()
            .find(|x| x.key == "unadmitted-d412-unknown")
            .unwrap();
        u.origin = "unknown".into();
        u.license = "unknown".into();
        u.claimed = DonorState::Registered;
        // Each replacement records the donor it was learned from.
        for (node, donor) in [
            ("storage", "d411-sqlite"),
            ("core", "cargo-sha2"),
            ("census", "cargo-toml"),
        ] {
            let n = d
                .repository
                .nodes
                .iter_mut()
                .find(|n| n.key == node)
                .unwrap();
            n.lineage.push(donor.into());
        }
    });
    let a = r.assess();
    for k in ["d411-sqlite", "cargo-sha2", "cargo-toml"] {
        assert_eq!(
            a.donor(k).unwrap().effective,
            DonorState::NativeShadow,
            "{k}: {}",
            a.donor(k).unwrap().stopped_by
        );
    }

    // 4. Parity evidence: proven while the donors still run beside the replacements; the
    //    capabilities' Norl relevance is resolved, so the donors stop just before CUTOVER.
    r.prove();
    let a = r.assess();
    for k in ["d411-sqlite", "cargo-sha2"] {
        assert_eq!(
            a.donor(k).unwrap().effective,
            DonorState::NorlRelevanceResolved,
            "{k}: {}",
            a.donor(k).unwrap().stopped_by
        );
    }
    // `census` still links `toml` while claiming to replace it: a wrapper, never PARITY_PROVEN.
    let toml = a.donor("cargo-toml").unwrap();
    assert_eq!(toml.effective, DonorState::NativeShadow);
    assert!(
        toml.capabilities[0].native_detail.contains("WRAPPER"),
        "{}",
        toml.capabilities[0].native_detail
    );
    assert!(
        !gate(&a, "cargo-sha2", Gate::RuntimeEdges),
        "sha2 still runs"
    );
    assert!(
        !gate(&a, "d411-sqlite", Gate::ResidentSource),
        "sqlite source is still committed"
    );

    // 5. Cutover: the donors leave the build and the tree; unused checkouts are rejected.
    r.write(
        "substrate/storage/Cargo.toml",
        &r.read("substrate/storage/Cargo.toml")
            .replace("sha2 = \"0.11\"\n", ""),
    );
    r.write(
        "substrate/storage/src/lib.rs",
        "pub mod pages;\npub fn put(page: &[u8]) -> [u8; 32] { core_kernel::sha256(page) }\n",
    );
    r.write(
        "tools/census/Cargo.toml",
        &r.read("tools/census/Cargo.toml")
            .replace("toml = \"0.8\"\n", ""),
    );
    r.write(
        "tools/census/src/main.rs",
        "mod manifest;\nfn main() { manifest::read(\"a = 1\"); }\n",
    );
    r.remove(".atlas/temporary/donors/D411-sqlite-sqlite");
    r.remove(".atlas/temporary/donors/D412-unknown");
    r.edit(|d| {
        for x in d.donors.iter_mut() {
            match x.key.as_str() {
                "unadmitted-d412-unknown" => {
                    x.exception = Some((
                        ExceptionKind::Rejected,
                        "unadmitted checkout, never built or imported".into(),
                    ))
                }
                "d900-hashes" => {}
                _ => {
                    x.cutover = Some("wave-native-1".into());
                    x.claimed = DonorState::Extinct;
                }
            }
        }
    });
    // Changed replacement trees make the recorded proofs stale: no extinction on old evidence.
    let a = r.assess();
    // (sha2's replacement, `core`, did not change: its evidence stays fresh.)
    assert_ne!(
        a.donor("d411-sqlite").unwrap().effective,
        DonorState::Extinct
    );
    assert!(a.donor("d411-sqlite").unwrap().capabilities[0]
        .parity
        .iter()
        .any(|(_, v)| *v == ynventa::evidence::Verdict::Stale));
    assert_eq!(
        a.donor("cargo-sha2").unwrap().effective,
        DonorState::Extinct
    );
    r.prove();
    let a = r.assess();
    for k in ["d411-sqlite", "cargo-sha2", "cargo-toml"] {
        let x = a.donor(k).unwrap();
        assert_eq!(
            x.effective,
            DonorState::Extinct,
            "{k}: {} {:#?}",
            x.stopped_by,
            x.gates
        );
    }
    assert!(a.donor("unadmitted-d412-unknown").unwrap().resolved());
    assert_eq!(a.metric("extinction_ratio"), "1.000000");
    assert_eq!(a.metric("runtime_external_edges"), "0");
    assert_eq!(a.metric("native_capability_ratio"), "1.000000");
    assert_eq!(a.metric("proof_completion_ratio"), "1.000000");
    assert_eq!(r.cli(&["census", "--record"]).0, 0);

    // 6. Compaction: documents and legacy registries become facts; then the legacy tree must go.
    let (code, out) = r.cli(&["compact", "--extract-docs", "--extract-legacy"]);
    assert_eq!(code, 0, "{out}");
    let a = r.assess();
    let st = a
        .knowledge
        .facts
        .iter()
        .find(|f| f.kind == FactKind::Statement && f.value.contains("append-only"))
        .unwrap();
    assert!(
        st.provenance.len() >= 3,
        "one fact from three documents: {:?}",
        st.provenance
    );
    assert!(a
        .knowledge
        .facts
        .iter()
        .any(|f| f.kind == FactKind::Decision && f.key == "status" && f.value == "Accepted"));
    assert!(a
        .knowledge
        .facts
        .iter()
        .any(|f| f.kind == FactKind::LegacyRecord
            && f.subject == ".atlas/census/donors/inventory.toml"
            && f.value == "ABSORBED"));
    assert!(a.docs.over_budget().all(|d| d.extracted));
    assert!(
        finding(&a, "EXPIRED_SHIM_PRESENT", "legacy-atlas").is_some(),
        "a consumed legacy tree may not linger: {:#?}",
        a.findings
    );
    let (code, out) = r.cli(&["compact", "--prune-docs"]);
    assert_eq!(code, 0, "{out}");
    assert!(!r.exists("ARCHITECTURE.md") && !r.exists(".atlas/decisions/0001-pages.md"));
    r.remove(".atlas");
    r.edit(|d| d.migration.shims.clear());

    // 7. Generated views are views: deleting them changes no authoritative knowledge.
    let before = r.assess();
    let view = r.read("target/ynventa/VIEW.md");
    r.remove("target");
    let after = r.assess();
    assert_eq!(before.counts.values(), after.counts.values());
    assert_eq!(before.knowledge.facts, after.knowledge.facts);
    assert_eq!(r.cli(&["compact"]).0, 0);
    let regenerated = r.read("target/ynventa/VIEW.md");
    assert_eq!(
        view.lines().filter(|l| l.contains("Knowledge")).count(),
        regenerated
            .lines()
            .filter(|l| l.contains("Knowledge"))
            .count()
    );
    assert_eq!(r.cli(&["compact"]).0, 0);
    assert_eq!(
        r.read("target/ynventa/VIEW.md"),
        regenerated,
        "views are deterministic"
    );

    // 8. Final conformance.
    let a = r.assess();
    assert!(a.errors().next().is_none(), "{:#?}", a.findings);
    let (code, out) = r.cli(&["conformance"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("V1 gate PASS"), "{out}");
    let (code, out) = r.cli(&["verify"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        a.graph.node_by_key("chronica", "storage").unwrap().id,
        storage_id,
        "the id survived the whole migration"
    );
}
