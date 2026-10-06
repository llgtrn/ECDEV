//! The golden fixture: one declared repository carried through the whole V1 path.
//!
//! declared graph → stable NodeId → path relocation → capability replacement → parity evidence
//! → cutover → extinction → compaction → final ECDEV repository conformance.

mod common;

use common::*;
use ecdev_governance::declare::*;
use ecdev_governance::graph::NodeId;
use ecdev_governance::schema::*;

/// A repository with a node at a legacy path, committed donor source, sprawling Markdown, and
/// two external crates.
fn legacy_repo() -> Repo {
    let r = Repo::new("golden");
    r.write("Cargo.toml", "[workspace]\nresolver = \"2\"\n# Every member is an ownership root.\nmembers = [\n    \"core\",\n    \"storage\",\n    \"tools/census\",\n    \"tests\",\n]\n");
    r.write("README.md", "# fixture\n");
    r.write(
        "ARCHITECTURE.md",
        "# Architecture\n\n- The page store is append-only.\n- Storage: durable pages and logs\n",
    );
    r.write(
        "docs/adr/0001-pages.md",
        "# Pages are native\n\nStatus: Accepted\n\n- The page store is append-only.\n",
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
        "vendor/sqlite/btree.c",
        "/* sqlite btree */\nint sqlite3BtreeOpen(void) { return 0; }\n",
    );
    r.write("vendor/d412/x.c", "int x(void) { return 1; }\n");
    let donor = |key: &str, origin: &str, license: &str, claimed: DonorState| Donor {
        key: key.into(),
        name: key.into(),
        origin: origin.into(),
        license: license.into(),
        claimed,
        exception: None,
        packages: vec![],
        source_paths: vec![],
        capabilities: vec![],
        cutover: None,
        provenance: vec!["fixture".into()],
    };
    let mut sqlite = donor(
        "d411-sqlite",
        "https://github.com/sqlite/sqlite",
        "blessing",
        DonorState::ParityProven,
    );
    sqlite.source_paths = vec!["vendor/sqlite".into()];
    let mut unknown = donor("d412-unknown", "unknown", "unknown", DonorState::Registered);
    unknown.source_paths = vec!["vendor/d412".into()];
    let package = |key: &str, name: &str| {
        let mut d = donor(
            key,
            &format!("https://crates.io/crates/{name}"),
            "",
            DonorState::Discovered,
        );
        d.packages = vec![Package {
            ecosystem: Ecosystem::Cargo,
            name: name.into(),
        }];
        d
    };
    r.store(&Declaration {
        repository: Repository {
            id: "ecdev".into(),
            name: "golden".into(),
            origin: "llgtrn/Golden".into(),
            nodes: vec![
                node("core", NodeKind::Kernel, "core", "core"),
                node(
                    "storage",
                    NodeKind::Substrate,
                    "storage",
                    "substrate/storage",
                ),
                node("census", NodeKind::Tool, "tools/census", "tools/census"),
                node("tests", NodeKind::Test, "tests", "tests"),
                node("governance", NodeKind::Governance, ".ecdev", ".ecdev"),
            ],
            edges: vec![],
        },
        donors: vec![
            package("cargo-sha2", "sha2"),
            package("cargo-toml", "toml"),
            sqlite,
            donor(
                "d900-hashes",
                "https://github.com/RustCrypto/hashes",
                "MIT OR Apache-2.0",
                DonorState::Discovered,
            ),
            unknown,
        ],
        migration: Migration::default(),
        technologies: vec![],
    });
    r
}

#[test]
fn declared_repository_reaches_v1() {
    let r = legacy_repo();

    // 1. Declared graph: claims are re-evaluated against evidence, never trusted.
    let a = r.assess();
    let d = &a.declaration;
    assert!(finding(&a, "CLAIM_EXCEEDS_EVIDENCE", "d411-sqlite").is_some());
    assert!(a.donor("d411-sqlite").unwrap().effective < DonorState::Censused);
    assert!(finding(&a, "LEGACY_PLACEMENT", "storage").is_some());
    assert!(finding(&a, "ILLEGAL_ROOT_FILE", "ARCHITECTURE.md").is_some());
    assert!(finding(&a, "DISCOVERED_BUT_ACTIVE", "cargo-sha2").is_some());
    assert_eq!(a.metric("extinction_ratio"), "0.000000");
    let storage_id = NodeId::of("ecdev", "storage");
    assert_eq!(
        a.graph.node_by_key("ecdev", "storage").unwrap().id,
        storage_id
    );
    let (_, map) = r.cli(&["migrate", "map"]);
    assert!(
        map.contains(&format!(
            "storage -> storage {storage_id} -> substrate/storage"
        )),
        "{map}"
    );
    assert!(d.migration.waves.is_empty());
    let (code, plan) = r.cli(&["migrate", "plan", "--write"]);
    assert_eq!(code, 0, "{plan}");
    assert!(plan.contains("w01-storage"), "{plan}");
    // The sqlite claim is reclaimed down to its evidence; the claim stays a fact.
    assert_eq!(r.cli(&["migrate", "reclaim"]).0, 0);
    let a = r.assess();
    assert!(finding(&a, "CLAIM_EXCEEDS_EVIDENCE", "d411-sqlite").is_none());
    assert!(a
        .knowledge
        .facts
        .iter()
        .any(|f| f.kind == FactKind::LegacyClaim
            && f.subject == "d411-sqlite"
            && f.value == "PARITY_PROVEN"));

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
        a.graph.node_by_key("ecdev", "storage").unwrap().id,
        storage_id
    );
    assert_eq!(
        a.graph.node_by_key("ecdev", "storage").unwrap().path,
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
        relevance: Relevance::ReliedOn(format!("ECDEV calls {key} on its production path")),
        knowledge: Knowledge::Absorbed("native and proven".into()),
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

    // 4. Parity evidence: proven while the donors still run beside the replacements; every
    //    capability is relied on by ECDEV, so the donors stop just before CUTOVER.
    r.prove();
    let a = r.assess();
    for k in ["d411-sqlite", "cargo-sha2"] {
        assert_eq!(
            a.donor(k).unwrap().effective,
            DonorState::RelevanceResolved,
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
    r.remove("vendor");
    r.edit(|d| {
        for x in d.donors.iter_mut() {
            match x.key.as_str() {
                "d412-unknown" => {
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
        .any(|(_, v)| *v == ecdev_governance::evidence::Verdict::Stale));
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
    assert!(a.donor("d412-unknown").unwrap().resolved());
    assert_eq!(a.metric("extinction_ratio"), "1.000000");
    assert_eq!(a.metric("runtime_external_edges"), "0");
    assert_eq!(a.metric("production_native_ratio"), "1.000000");
    assert_eq!(a.metric("production_proof_ratio"), "1.000000");
    assert_eq!(r.cli(&["census", "--record"]).0, 0);

    // 6. Compaction: documents become facts; then they may go.
    let (code, out) = r.cli(&["compact", "--extract-docs"]);
    assert_eq!(code, 0, "{out}");
    let a = r.assess();
    let st = a
        .knowledge
        .facts
        .iter()
        .find(|f| f.kind == FactKind::Statement && f.value.contains("append-only"))
        .unwrap();
    assert!(
        st.provenance.len() >= 2,
        "one fact from two documents: {:?}",
        st.provenance
    );
    assert!(a
        .knowledge
        .facts
        .iter()
        .any(|f| f.kind == FactKind::Decision && f.key == "status" && f.value == "Accepted"));
    assert!(a.docs.over_budget().all(|d| d.extracted));
    let (code, out) = r.cli(&["compact", "--prune-docs"]);
    assert_eq!(code, 0, "{out}");
    assert!(!r.exists("ARCHITECTURE.md") && !r.exists("docs/adr/0001-pages.md"));

    // 7. Generated views are views: deleting them changes no authoritative knowledge.
    let before = r.assess();
    let view = r.read("target/ecdev-governance/VIEW.md");
    r.remove("target");
    let after = r.assess();
    assert_eq!(before.counts.values(), after.counts.values());
    assert_eq!(before.knowledge.facts, after.knowledge.facts);
    assert_eq!(r.cli(&["compact"]).0, 0);
    let regenerated = r.read("target/ecdev-governance/VIEW.md");
    assert_eq!(
        view.lines().filter(|l| l.contains("Knowledge")).count(),
        regenerated
            .lines()
            .filter(|l| l.contains("Knowledge"))
            .count()
    );
    assert_eq!(r.cli(&["compact"]).0, 0);
    assert_eq!(
        r.read("target/ecdev-governance/VIEW.md"),
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
        a.graph.node_by_key("ecdev", "storage").unwrap().id,
        storage_id,
        "the id survived the whole migration"
    );
}
