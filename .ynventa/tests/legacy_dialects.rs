//! The importer against each real legacy dialect (formats as observed in the six repositories).
//! Every imported status is a claim; the tests check both the mapping and its re-evaluation.

mod common;

use common::*;
use ynventa::schema::*;

fn scaffold(r: &Repo, ns: &str) -> ynventa::Assessment {
    let (code, out) = r.cli(&["migrate", "scaffold", "--shard", ns]);
    assert_eq!(code, 0, "{out}");
    r.assess()
}

fn cargo_ws(r: &Repo, members: &[&str]) {
    let list = members
        .iter()
        .map(|m| format!("\"{m}\""))
        .collect::<Vec<_>>()
        .join(", ");
    r.write(
        "Cargo.toml",
        &format!("[workspace]\nresolver = \"2\"\nmembers = [{list}]\n"),
    );
    for m in members {
        let name = m.replace('/', "-");
        if !r.exists(&format!("{m}/Cargo.toml")) {
            r.write(
                &format!("{m}/Cargo.toml"),
                &format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
            );
        }
        r.write(&format!("{m}/src/lib.rs"), "pub fn f() {}\n");
    }
}

#[test]
fn atlas_studio_donor_corpus() {
    let r = Repo::new("atlas-studio");
    cargo_ws(&r, &["core", "runtime", "adapter", "apps/cli"]);
    r.write(
        ".atlas/references/donor-corpus.toml",
        r#"schema = "atlas.donor-corpus.v1"

[[donor]]
id = "ast-grep"
name = "ast-grep"
repository = "ast-grep/ast-grep"
resolved_url = "https://github.com/ast-grep/ast-grep.git"
license = [".atlas/licenses/ast-grep/LICENSE"]
technology_domains = ["structural_search", "structural_rewrite"]
ingestion_status = "EXTINCT"
storage_state = "SOURCE_DELETED"
decision_status = "REFERENCE_ONLY"
implementation_status = "NOT_ABSORBED_REFERENCE_ONLY"

[[donor]]
id = "salsa"
name = "salsa"
repository = "salsa-rs/salsa"
resolved_url = "https://github.com/salsa-rs/salsa.git"
license = [".atlas/licenses/salsa/LICENSE-MIT"]
technology_domains = ["incremental_computation"]
ingestion_status = "CLONED"
decision_status = "ABSORBED"
implementation_status = "ABSORBED_NATIVE"
"#,
    );
    r.write(
        ".atlas/temporary/donors/compiler/salsa/src/lib.rs",
        "// salsa\n",
    );
    r.write(
        ".atlas/temporary/donors/security/selinux/libselinux/x.c",
        "int x;\n",
    );
    let a = scaffold(&r, "atlas-studio");
    let d = &a.declaration;
    let ast = d.donor("ast-grep").unwrap();
    assert_eq!(ast.claimed, DonorState::Extinct);
    assert_eq!(ast.exception.as_ref().unwrap().0, ExceptionKind::Rejected);
    assert!(
        a.donor("ast-grep").unwrap().resolved(),
        "studied, never adopted, nothing left: a legal rejection"
    );
    let salsa = d.donor("salsa").unwrap();
    assert_eq!(
        salsa.source_paths,
        vec![".atlas/temporary/donors/compiler/salsa".to_string()]
    );
    assert!(
        !a.donor("salsa")
            .unwrap()
            .gates
            .iter()
            .find(|g| g.gate == Gate::ResidentSource)
            .unwrap()
            .pass
    );
    assert!(
        d.donor("unadmitted-selinux").is_some(),
        "held source without a record cannot hide"
    );
    // Node shape from the Atlas-Studio layout.
    let storage = d.node("compiler.runtime").unwrap();
    assert_eq!(
        (storage.kind, storage.canonical_path.as_str()),
        (NodeKind::Substrate, "substrate/runtime")
    );
    assert_eq!(d.node("compiler.cli").unwrap().canonical_path, "apps/cli");
}

#[test]
fn universal_connector_inventory_json() {
    let r = Repo::new("uc");
    cargo_ws(&r, &["core", "runtime", "adapter/web"]);
    r.write(
        ".atlas/provenance/donor-inventory.json",
        r#"{"schema":"uc.donor-inventory.v1","statuses":["CANDIDATE","CLONED","CENSUSED","MAPPED","ABSORBING","ABSORBED","EXTINCT","REJECTED"],
 "donors":[
  {"id":"rust-url","name":"rust-url","license":"MIT OR Apache-2.0","source_url":"https://github.com/servo/rust-url","status":"EXTINCT",
   "target_capabilities":["WHATWG URL parsing"],"runtime_dependency":"FORBIDDEN_WITHOUT_DECISION",
   "status_history":[{"status":"ABSORBED","iteration":11},{"status":"EXTINCT","iteration":12}]},
  {"id":"hickory-dns","name":"Hickory DNS","license":"MIT OR Apache-2.0","source_url":"https://github.com/hickory-dns/hickory-dns","status":"CENSUSED",
   "target_capabilities":["DNS resolution","DNSSEC"],"snapshot_path":".atlas/temporary/donors/web/hickory-dns","runtime_dependency":"FORBIDDEN_WITHOUT_DECISION"},
  {"id":"libp2p","name":"libp2p","license":"MIT","source_url":"https://github.com/libp2p/rust-libp2p","status":"CANDIDATE","target_capabilities":[]},
  {"id":"chromium","name":"Chromium","license":"BSD-3-Clause","source_url":"https://chromium.googlesource.com/chromium/src","status":"REJECTED","rejection_reason":"chromium foundation forbidden"}
 ]}"#,
    );
    r.write(
        ".atlas/temporary/donors/web/hickory-dns/Cargo.toml",
        "[package]\nname = \"hickory\"\n",
    );
    let a = scaffold(&r, "universal-connector");
    let d = &a.declaration;
    assert_eq!(d.donor("rust-url").unwrap().claimed, DonorState::Extinct);
    assert_eq!(
        d.donor("rust-url").unwrap().capabilities[0].key,
        "whatwg-url-parsing"
    );
    // A legacy EXTINCT without native replacement evidence is re-evaluated.
    assert!(finding(&a, "CLAIM_EXCEEDS_EVIDENCE", "rust-url").is_some());
    assert_eq!(a.donor("rust-url").unwrap().effective, DonorState::Censused);
    assert_eq!(
        d.donor("hickory-dns").unwrap().claimed,
        DonorState::Censused
    );
    assert!(
        !a.donor("hickory-dns").unwrap().facts.resident.is_empty(),
        "committed snapshot is resident source"
    );
    assert_eq!(d.donor("libp2p").unwrap().claimed, DonorState::Discovered);
    assert!(a.donor("chromium").unwrap().resolved());
    assert!(d
        .node("connector.web")
        .is_some_and(|n| n.kind == NodeKind::Adapter && n.canonical_path == "adapter/web"));
}

#[test]
fn esellios_donors_and_capability_records() {
    let r = Repo::new("esellios");
    cargo_ws(&r, &["core/kernel", "domains/supply/procurement", "mcp"]);
    r.write(".atlas/architecture/crates.toml", "schema = \"esellios.crates.v1\"\n[[crate]]\nname = \"kernel\"\npath = \"core/kernel\"\n[[crate]]\nname = \"procurement\"\npath = \"domains/supply/procurement\"\n[[crate]]\nname = \"mcp\"\npath = \"mcp\"\n");
    r.write(
        ".atlas/census/donors.toml",
        "schema = \"esellios.donors.v1\"\n\n[[donor]]\nid = \"erpnext\"\nupstream = \"https://github.com/frappe/erpnext\"\nlicense_hint = \"GPL-3.0\"\nstatus = \"ACQUIRED\"\ndisposition = \"UNDECIDED\"\n",
    );
    r.write(
        ".atlas/census/donors/erpnext/capabilities.toml",
        "schema = \"esellios.donor-capabilities.v1\"\ndonor = \"erpnext\"\n\n[[capability]]\nid = \"rfq.supplier-guards\"\ndescription = \"An RFQ refuses duplicate suppliers\"\nsource_paths = [\"erpnext/buying/rfq.py\"]\nevidence_state = \"CONTRACT_VERIFIED\"\ndisposition = \"REIMPLEMENT\"\nnative = [\"domains/supply/procurement/src/\"]\nnative_tests = [\"domains/supply/procurement/src/rfq.rs#a_duplicate_supplier_is_rejected\"]\n\n[[capability]]\nid = \"print.formats\"\ndescription = \"Jinja print formats\"\nsource_paths = []\nevidence_state = \"DISCOVERED\"\ndisposition = \"REFERENCE\"\n",
    );
    let a = scaffold(&r, "esellios");
    let d = &a.declaration;
    let e = d.donor("erpnext").unwrap();
    assert_eq!(e.claimed, DonorState::Registered);
    assert_eq!(
        e.source_paths,
        vec![".atlas/temporary/donors/erpnext/source".to_string()]
    );
    let rfq = e
        .capabilities
        .iter()
        .find(|c| c.key == "rfq-supplier-guards")
        .unwrap();
    assert!(rfq.required);
    assert_eq!(
        rfq.replacement.as_deref(),
        Some("commerce.supply.procurement")
    );
    assert_eq!(
        rfq.proofs[0].locator,
        "domains/supply/procurement/src/rfq.rs::a_duplicate_supplier_is_rejected"
    );
    assert_eq!(rfq.proofs[0].kind, ProofKind::Regression);
    assert!(
        !e.capabilities
            .iter()
            .find(|c| c.key == "print-formats")
            .unwrap()
            .required
    );
    let node = d.node("commerce.supply.procurement").unwrap();
    assert_eq!(
        (node.kind, node.canonical_path.as_str()),
        (NodeKind::Domain, "domain/supply/procurement")
    );
    assert_eq!(
        d.node("commerce.kernel").unwrap().canonical_path,
        "core/kernel"
    );
    assert_eq!(d.node("commerce.mcp").unwrap().kind, NodeKind::Adapter);
}

#[test]
fn fi_game_inventory_and_absorption_ledger() {
    let r = Repo::new("fi-game");
    cargo_ws(&r, &["genome", "body/book"]);
    r.write(".atlas/census/declared.toml", "schema = \"fi.census.declared.v1\"\n[[root]]\npath = \"genome\"\nkind = \"substrate\"\n[[root]]\npath = \"body\"\nkind = \"organ\"\n");
    r.write(
        ".atlas/census/donors/inventory.toml",
        "schema = \"fi.atlas.donors.v1\"\nintake_root = \".atlas/temporary/donors\"\n\n[[donor]]\nid = \"D001-ccxt\"\nupstream = \"https://github.com/ccxt/ccxt\"\nlicense = \"MIT\"\ncensus_status = \"SEMANTIC\"\nabsorption = \"EXTINCT\"\nextinct_at = \"2026-09-29\"\n",
    );
    r.write(
        ".atlas/census/absorption.toml",
        "schema = \"fi.atlas.absorption.v1\"\n\n[[capability]]\nid = \"genome.fixed_point\"\nname = \"exact decimal fixed point\"\nstate = \"ABSORBED\"\ndonors = [\"D001-ccxt\"]\nnative = [\"genome/src/num.rs\"]\ntests = [\"genome/src/num.rs::parse_exact_and_canonical_format\"]\ndifferential = [\"genome/tests/differential_precise.rs::fixed_point_agrees_with_ccxt_precise\"]\n",
    );
    r.write(
        "genome/src/num.rs",
        "#[test]\nfn parse_exact_and_canonical_format() {}\n",
    );
    r.write(
        "genome/tests/differential_precise.rs",
        "#[test]\nfn fixed_point_agrees_with_ccxt_precise() {}\n",
    );
    let a = scaffold(&r, "fi-game");
    let d = &a.declaration;
    assert_eq!(d.node("finance.genome").unwrap().kind, NodeKind::Kernel);
    assert_eq!(
        d.node("finance.body.book").unwrap().canonical_path,
        "domain/body/book"
    );
    let c = d.donor("d001-ccxt").unwrap();
    assert_eq!(c.claimed, DonorState::Extinct);
    let cap = &c.capabilities[0];
    assert_eq!(cap.replacement.as_deref(), Some("finance.genome"));
    assert!(cap.proofs.iter().any(|p| p.kind == ProofKind::Parity
        && p.locator
            .ends_with("::fixed_point_agrees_with_ccxt_precise")));
    // Imported proofs are not evidence until they are run against the current bytes.
    let x = a.donor("d001-ccxt").unwrap();
    assert!(x.capabilities[0]
        .parity
        .iter()
        .all(|(_, v)| *v == ynventa::evidence::Verdict::Unrecorded));
    assert!(finding(&a, "CLAIM_EXCEEDS_EVIDENCE", "d001-ccxt").is_some());
    // Declared legacy placement: every organ root becomes a wave.
    assert!(d
        .migration
        .waves
        .iter()
        .any(|w| w.nodes.contains(&"finance.genome".to_string())));
}

#[test]
fn mechatron_research_registry_and_live_oracles() {
    let r = Repo::new("mechatron");
    cargo_ws(&r, &["entity", "spatial"]);
    r.write(
        "spatial/Cargo.toml",
        "[package]\nname = \"spatial\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dev-dependencies]\ngeo = \"0.33.1\"\n",
    );
    r.write("spatial/tests/proj_differential.rs", "#[test]\nfn transverse_mercator_agrees_with_proj() { let _ = std::env::var(\"MECHATRON_PROJ_BIN\"); }\n");
    r.write(
        "tools/oracles/proj.sh",
        "#!/bin/sh\ncd temporary/donors/D014-OSGeo-PROJ && cmake .\n",
    );
    r.write(
        "ARCHITECTURE.md",
        "# Architecture\n\n- entity is the root of every crate graph.\n",
    );
    r.write("ROADMAP.md", "# Roadmap\n");
    r.write(
        "research/registry/donors.toml",
        r#"[ledger]
schema_version = 1
states = ["DISCOVERED", "REGISTERED", "MAPPED", "NATIVE_EQUIVALENCE_PARTIAL", "EXTINCT"]

[[donor]]
id = "D014"
name = "PROJ"
repository = "https://github.com/OSGeo/PROJ"
license_observed = "MIT"
status = "NATIVE_EQUIVALENCE_PARTIAL"
clone_dir = "temporary/donors/D014-OSGeo-PROJ"
dependency_class = "reference-only (oracle)"
native_implementation = ["spatial/src/tmerc.rs"]
native_evidence = ["spatial/tests/proj_differential.rs::transverse_mercator_agrees_with_proj (4,840 points)"]
equivalence_scope = "Transverse Mercator forward and inverse"

[[donor]]
id = "D027"
name = "geo"
repository = "https://github.com/georust/geo"
license_observed = "MIT OR Apache-2.0"
status = "NATIVE_EQUIVALENCE_PARTIAL"
clone_dir = "temporary/donors/D027-georust-geo"
dependency_class = "temporary donor"
"#,
    );
    let a = scaffold(&r, "mechatron");
    let d = &a.declaration;
    assert!(d.migration.shims.is_empty(), "no .atlas, no legacy shim");
    let proj = d.donor("d014-proj").unwrap();
    assert_eq!(proj.claimed, DonorState::NativeShadow);
    assert_eq!(
        proj.capabilities[0].replacement.as_deref(),
        Some("machine.spatial")
    );
    assert_eq!(
        proj.capabilities[0].proofs[0].locator,
        "spatial/tests/proj_differential.rs::transverse_mercator_agrees_with_proj"
    );
    // The oracle build script executes the donor's source: a BUILD edge, never extinct.
    assert!(
        !a.donor("d014-proj").unwrap().facts.build.is_empty(),
        "{:#?}",
        a.donor("d014-proj").unwrap().facts
    );
    // `geo` is both a registered donor and a crates.io dev-dependency: a live test oracle.
    let geo = d.donor("d027-geo").unwrap();
    assert!(geo
        .packages
        .iter()
        .any(|p| p.ecosystem == Ecosystem::Cargo && p.name == "geo"));
    assert!(
        !a.donor("d027-geo")
            .unwrap()
            .gates
            .iter()
            .find(|g| g.gate == Gate::TestEdges)
            .unwrap()
            .pass
    );
    assert!(finding(&a, "ILLEGAL_ROOT_FILE", "ROADMAP.md").is_some());
    assert_eq!(
        d.node("machine.spatial").unwrap().canonical_path,
        "substrate/spatial"
    );
}

#[test]
fn chronica_reference_retained_is_a_rejection() {
    let r = Repo::new("chronica-ref");
    cargo_ws(&r, &["core"]);
    r.write(
        ".atlas/census/donors/inventory.toml",
        "schema = \"chronica.donors.inventory.v1\"\n\n[[donor]]\nid = \"D100\"\nrepo = \"apache/age\"\nupstream = \"https://github.com/apache/age\"\ncapabilities = []\nlifecycle = \"REFERENCE_RETAINED\"\nlicense = \"Apache-2.0\"\n",
    );
    let a = scaffold(&r, "chronica");
    assert!(a.donor("d100-age").unwrap().resolved());
    assert_eq!(a.metric("donors_rejected"), "1");
    assert_eq!(
        a.metric("extinction_ratio"),
        "1.000000",
        "a legal rejection leaves the denominator"
    );
}
