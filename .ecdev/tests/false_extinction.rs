//! No false green. Each test starts from a donor that is genuinely EXTINCT and introduces one
//! way a repository could still carry the donor while claiming extinction; the verifier must
//! refuse EXTINCT, name the failing gate, and report the false claim.

mod common;

use common::*;
use ecdev_governance::declare::*;
use ecdev_governance::schema::*;

fn not_extinct(r: &Repo, failing: Gate) -> ecdev_governance::Assessment {
    let a = r.assess();
    let d = a.donor("geo").unwrap();
    assert_ne!(
        d.effective,
        DonorState::Extinct,
        "must not be extinct: {:#?}",
        d.gates
    );
    assert!(
        !gate(&a, "geo", failing),
        "gate {failing} should fail: {:#?}",
        d.gates
    );
    assert!(
        finding(&a, "CLAIM_EXCEEDS_EVIDENCE", "geo").is_some(),
        "the EXTINCT claim must be reported as false"
    );
    assert_ne!(a.metric("extinction_ratio"), "1.000000");
    a
}

#[test]
fn baseline_is_genuinely_extinct() {
    let r = extinct_baseline("baseline");
    let a = r.assess();
    let d = a.donor("geo").unwrap();
    assert_eq!(d.effective, DonorState::Extinct, "{:#?}", d.gates);
    assert!(d.gates.iter().all(|g| g.pass));
    assert!(a.errors().next().is_none(), "{:#?}", a.findings);
    assert_eq!(a.metric("extinction_ratio"), "1.000000");
    assert_eq!(a.metric("proof_completion_ratio"), "1.000000");
}

#[test]
fn donor_source_deleted_but_crate_dependency_remains() {
    let r = extinct_baseline("dep-remains");
    r.write("core/Cargo.toml", "[package]\nname = \"core-kernel\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\ngeo = \"0.33\"\n");
    not_extinct(&r, Gate::RuntimeEdges);
}

#[test]
fn build_dependency_remains() {
    let r = extinct_baseline("build-dep");
    r.write("core/Cargo.toml", "[package]\nname = \"core-kernel\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[build-dependencies]\ngeo = \"0.33\"\n");
    let a = not_extinct(&r, Gate::BuildEdges);
    assert!(gate(&a, "geo", Gate::RuntimeEdges));
}

#[test]
fn wrapper_calls_donor() {
    let r = extinct_baseline("wrapper");
    r.write(
        "substrate/geo/Cargo.toml",
        "[package]\nname = \"geo-native\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\ngeo = \"0.33\"\n",
    );
    r.write(
        "substrate/geo/src/lib.rs",
        "pub fn distance(a: (f64, f64), b: (f64, f64)) -> f64 { use geo::EuclideanDistance; geo::Point::from(a).euclidean_distance(&geo::Point::from(b)) }\n#[cfg(test)]\nmod tests { #[test] fn regression_distance() {} }\n",
    );
    r.prove();
    let a = not_extinct(&r, Gate::CapabilityCoverage);
    let cap = &a.donor("geo").unwrap().capabilities[0];
    assert!(
        !cap.native && cap.native_detail.contains("WRAPPER"),
        "{}",
        cap.native_detail
    );
    assert!(!gate(&a, "geo", Gate::SourceImports));
    assert_eq!(a.analysis.node_status["geo"], NativeStatus::Wrapper);
    assert!(
        a.donor("geo").unwrap().effective < DonorState::ParityProven,
        "a wrapper never reaches PARITY_PROVEN"
    );
}

#[test]
fn wrapper_through_an_internal_dependency() {
    let r = extinct_baseline("transitive");
    r.write("Cargo.toml", "[workspace]\nresolver = \"2\"\nmembers = [\"core\", \"substrate/geo\", \"substrate/shim\", \"tests\"]\n");
    r.write("substrate/shim/Cargo.toml", "[package]\nname = \"shim\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\ngeo = \"0.33\"\n");
    r.write("substrate/shim/src/lib.rs", "pub use geo::Point;\n");
    r.write(
        "substrate/geo/Cargo.toml",
        "[package]\nname = \"geo-native\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nshim = { path = \"../shim\" }\n",
    );
    r.edit(|d| {
        d.repository.nodes.push(node(
            "shim",
            NodeKind::Substrate,
            "substrate/shim",
            "substrate/shim",
        ))
    });
    r.prove();
    let a = not_extinct(&r, Gate::CapabilityCoverage);
    let detail = &a.donor("geo").unwrap().capabilities[0].native_detail;
    assert!(detail.contains("geo -> shim"), "{detail}");
    assert_eq!(
        a.analysis.node_status["geo"],
        NativeStatus::Native,
        "geo itself imports nothing"
    );
}

#[test]
fn all_code_replaced_but_proof_missing() {
    let r = extinct_baseline("no-proof");
    r.remove(".ecdev/evidence");
    let a = not_extinct(&r, Gate::ParityProofs);
    let cap = &a.donor("geo").unwrap().capabilities[0];
    assert!(cap.native, "the code is native; only the proof is missing");
    assert_eq!(
        cap.parity[0].1,
        ecdev_governance::evidence::Verdict::Unrecorded
    );
    assert_eq!(a.donor("geo").unwrap().effective, DonorState::NativeShadow);
}

#[test]
fn declared_proof_that_does_not_exist() {
    let r = extinct_baseline("absent-proof");
    r.edit(|d| {
        d.donors[0].capabilities[0].proofs[0].locator = "tests/tests/parity.rs::no_such_test".into()
    });
    let a = not_extinct(&r, Gate::ParityProofs);
    assert_eq!(
        a.donor("geo").unwrap().capabilities[0].parity[0].1,
        ecdev_governance::evidence::Verdict::Absent
    );
}

#[test]
fn proof_is_stale_after_the_replacement_changes() {
    let r = extinct_baseline("stale");
    r.write("substrate/geo/src/extra.rs", "pub fn changed() {}\n");
    let a = not_extinct(&r, Gate::ParityProofs);
    assert_eq!(
        a.donor("geo").unwrap().capabilities[0].parity[0].1,
        ecdev_governance::evidence::Verdict::Stale
    );
    r.prove();
    assert_eq!(
        r.assess().donor("geo").unwrap().effective,
        DonorState::Extinct
    );
}

#[test]
fn failing_run_of_the_same_bytes_wins() {
    struct Fail;
    impl ecdev_governance::evidence::Runner for Fail {
        fn run(
            &mut self,
            _: &std::path::Path,
            _: &ecdev_governance::repository::files::Files,
            _: &Proof,
        ) -> (bool, String) {
            (false, "failed".into())
        }
    }
    let r = extinct_baseline("fail");
    let d = r.declaration();
    let files = ecdev_governance::repository::files::Files::scan(r.path()).unwrap();
    ecdev_governance::evidence::prove(r.path(), &files, &d, None, &mut Fail);
    let a = not_extinct(&r, Gate::ParityProofs);
    assert_eq!(
        a.donor("geo").unwrap().capabilities[0].parity[0].1,
        ecdev_governance::evidence::Verdict::Fail
    );
}

#[test]
fn ffi_into_the_donor() {
    let r = extinct_baseline("ffi");
    r.edit(|d| {
        d.donors[0].packages.push(Package {
            ecosystem: Ecosystem::Native,
            name: "geos".into(),
        })
    });
    r.write("core/src/lib.rs", "#[link(name = \"geos\")]\nextern \"C\" { fn GEOSversion() -> *const u8; }\npub fn id() -> u64 { 1 }\n");
    not_extinct(&r, Gate::LinkedEdges);
}

#[test]
fn donor_executed_as_a_process() {
    let r = extinct_baseline("process");
    r.edit(|d| {
        d.donors[0].packages.push(Package {
            ecosystem: Ecosystem::Native,
            name: "proj".into(),
        })
    });
    r.write("core/src/lib.rs", "pub fn id() -> u64 { std::process::Command::new(\"proj\").status().map(|_| 1).unwrap_or(0) }\n");
    not_extinct(&r, Gate::RuntimeEdges);
}

#[test]
fn live_oracle_in_tests_is_not_extinct() {
    let r = extinct_baseline("oracle");
    r.write(
        "tests/Cargo.toml",
        "[package]\nname = \"verification\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dev-dependencies]\ngeo-native = { path = \"../substrate/geo\" }\ngeo = \"0.33\"\n",
    );
    let a = not_extinct(&r, Gate::TestEdges);
    assert!(gate(&a, "geo", Gate::RuntimeEdges) && gate(&a, "geo", Gate::BuildEdges));
    assert_eq!(
        a.analysis.node_status["tests"],
        NativeStatus::Native,
        "test-only use makes no node a wrapper"
    );
}

#[test]
fn donor_source_hidden_in_research_or_fixtures() {
    for hidden in [
        "research/geo-study/src/algorithm.rs",
        "tests/fixtures/geo/src/lib.rs",
        "vendor/geo/src/lib.rs",
        ".atlas/temporary/donors/geo/src/lib.rs",
    ] {
        let r = extinct_baseline("hidden");
        let dir = hidden.rsplitn(3, '/').nth(2).unwrap().to_string();
        r.write(hidden, "// donor source\npub fn f() {}\n");
        r.edit(|d| d.donors[0].source_paths = vec![dir.clone()]);
        let a = not_extinct(&r, Gate::ResidentSource);
        assert!(
            a.donor("geo")
                .unwrap()
                .facts
                .resident
                .iter()
                .any(|f| f == hidden),
            "{hidden}"
        );
    }
}

#[test]
fn tool_naming_the_donor_checkout_is_an_edge() {
    // An oracle script that reads and executes the untracked checkout, in POSIX and Windows
    // spelling; neither imports a package, so only the source-path reference reveals it.
    for (file, text) in [
        (
            "tools/oracle.py",
            "DONOR='research/checkouts/geo'\nexec(open(DONOR+'/x.py').read())\n",
        ),
        (
            "tools/build.cmd",
            "set SRC=research\\checkouts\\geo\\src\ncl %SRC%\\a.cpp\n",
        ),
    ] {
        let r = extinct_baseline("checkout");
        r.write(file, text);
        r.edit(|d| d.donors[0].source_paths = vec!["research/checkouts/geo".into()]);
        let a = not_extinct(&r, Gate::BuildEdges);
        assert!(
            a.donor("geo")
                .unwrap()
                .facts
                .build
                .iter()
                .any(|o| o.file == file),
            "{file}"
        );
    }
}

#[test]
fn deletion_is_not_extinction() {
    // The directory is gone and the claim says EXTINCT, but nothing was ever decomposed,
    // replaced or proven.
    let r = extinct_baseline("deleted");
    r.edit(|d| {
        d.donors[0].capabilities.clear();
        d.donors[0].cutover = None;
    });
    let a = not_extinct(&r, Gate::CapabilityCoverage);
    assert_eq!(a.donor("geo").unwrap().effective, DonorState::Registered);
}

#[test]
fn unregistering_a_difficult_donor_cannot_raise_the_ratio() {
    let r = extinct_baseline("vanish");
    r.edit(|d| {
        let mut hard = donor("hard", "hard");
        hard.capabilities.clear();
        hard.claimed = DonorState::Registered;
        d.donors.push(hard);
    });
    let before = r.assess();
    assert_eq!(before.metric("extinction_ratio"), "0.500000");
    assert_eq!(r.cli(&["census", "--record"]).0, 0);
    r.edit(|d| d.donors.retain(|x| x.key != "hard"));
    let after = r.assess();
    assert!(finding(&after, "DONOR_VANISHED", "hard").is_some());
    assert_eq!(after.metric("donors_registered"), "2");
    assert_eq!(
        after.metric("extinction_ratio"),
        "0.500000",
        "the denominator never shrinks"
    );
}

#[test]
fn rejecting_or_superseding_an_active_donor_is_illegal() {
    let r = extinct_baseline("reject");
    r.write("core/Cargo.toml", "[package]\nname = \"core-kernel\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\ngeo = \"0.33\"\n");
    r.edit(|d| d.donors[0].exception = Some((ExceptionKind::Rejected, "never needed".into())));
    let a = r.assess();
    assert!(finding(&a, "ILLEGAL_EXCEPTION", "geo").is_some());
    assert_eq!(a.metric("donors_rejected"), "0");
    assert_eq!(a.metric("donors_active"), "1");

    r.edit(|d| {
        let mut succ = donor("successor", "other");
        succ.capabilities.clear();
        succ.claimed = DonorState::Registered;
        d.donors.push(succ);
        d.donors[0].exception = Some((ExceptionKind::Superseded, "successor".into()));
    });
    let a = r.assess();
    assert!(finding(&a, "ILLEGAL_EXCEPTION", "geo")
        .unwrap()
        .detail
        .contains("packages"));
    assert_eq!(a.metric("donors_superseded"), "0");
}

#[test]
fn donor_fallback_shim_blocks_extinction() {
    let r = extinct_baseline("fallback");
    r.edit(|d| {
        d.migration.shims.push(Shim {
            key: "geo-fallback".into(),
            kind: ShimKind::DonorFallback,
            path: String::new(),
            serves: "geo".into(),
            expires: (ExpiryKind::DonorExtinct, "geo".into()),
        })
    });
    not_extinct(&r, Gate::RollbackIndependent);
}

#[test]
fn replacement_must_be_a_canonical_native_role() {
    let r = extinct_baseline("compat-replacement");
    r.edit(|d| {
        d.repository.nodes[1].kind = NodeKind::Compat;
        d.repository.nodes[1].canonical_path = "compat/geo".into();
    });
    let a = not_extinct(&r, Gate::CanonicalReplacement);
    assert!(finding(&a, "COMPAT_WITHOUT_EXPIRY", "geo").is_some());
}

#[test]
fn unregistered_external_joins_the_denominator() {
    let r = extinct_baseline("unregistered");
    r.write("core/Cargo.toml", "[package]\nname = \"core-kernel\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nserde = \"1\"\n");
    let a = r.assess();
    assert!(finding(&a, "UNREGISTERED_EXTERNAL", "CARGO:serde").is_some());
    assert_eq!(a.metric("extinction_ratio"), "0.500000");
    assert_eq!(a.metric("runtime_external_edges"), "1");
    assert_eq!(a.analysis.node_status["core"], NativeStatus::Dependent);
}

#[test]
fn platform_libraries_are_not_foreign() {
    let r = extinct_baseline("platform");
    r.write(
        "core/src/lib.rs",
        "#[cfg(windows)]\n#[link(name = \"kernel32\")]\nextern \"system\" { fn GetLastError() -> u32; }\npub fn id() -> u64 { 1 }\n",
    );
    let a = r.assess();
    assert_eq!(a.metric("linked_external_edges"), "0");
    assert!(a.analysis.unregistered.is_empty());
    assert_eq!(a.donor("geo").unwrap().effective, DonorState::Extinct);
}

/// A `Cargo.lock` in which `core-kernel` pulls in `geo` only through other crates.
const TRANSITIVE_LOCK: &str = "version = 4\n\n[[package]]\nname = \"core-kernel\"\nversion = \"0.1.0\"\ndependencies = [\"spatial-db\"]\n\n[[package]]\nname = \"geo-native\"\nversion = \"0.1.0\"\ndependencies = [\"core-kernel\"]\n\n[[package]]\nname = \"verification\"\nversion = \"0.1.0\"\ndependencies = [\"geo-native\"]\n\n[[package]]\nname = \"spatial-db\"\nversion = \"0.6.12\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\ndependencies = [\"spatial-protocol\"]\n\n[[package]]\nname = \"spatial-protocol\"\nversion = \"0.6.12\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\ndependencies = [\"geo 0.33.0\"]\n\n[[package]]\nname = \"geo\"\nversion = \"0.33.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n";

#[test]
fn donor_crate_remains_only_in_the_lockfile() {
    let r = extinct_baseline("lock-transitive");
    r.write("Cargo.lock", TRANSITIVE_LOCK);
    let a = not_extinct(&r, Gate::LinkedEdges);
    let detail = &a
        .donor("geo")
        .unwrap()
        .gates
        .iter()
        .find(|g| g.gate == Gate::LinkedEdges)
        .unwrap()
        .detail;
    assert!(
        detail.contains("linked via spatial-protocol 0.6.12")
            && detail.contains(
                "core/Cargo.toml → spatial-db 0.6.12 → spatial-protocol 0.6.12 → geo 0.33.0"
            ),
        "the gate names the path that pulls the donor in: {detail}"
    );
    assert_eq!(a.metric("linked_external_edges"), "1");
    assert_eq!(a.metric("external_technology_edges"), "1");
    // The crates in between are the direct dependency's business: no finding of their own.
    assert!(
        a.findings.iter().all(|f| f.code != "UNREGISTERED_EXTERNAL"),
        "{:#?}",
        a.findings
    );
}

#[test]
fn a_lockfile_without_the_donor_keeps_it_extinct() {
    let r = extinct_baseline("lock-clean");
    r.write(
        "Cargo.lock",
        &TRANSITIVE_LOCK.replace("dependencies = [\"geo 0.33.0\"]\n", "")
            .replace("[[package]]\nname = \"geo\"\nversion = \"0.33.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n", ""),
    );
    let a = r.assess();
    let d = a.donor("geo").unwrap();
    assert_eq!(d.effective, DonorState::Extinct, "{:#?}", d.gates);
    assert_eq!(a.metric("linked_external_edges"), "0");
}

#[test]
fn donor_package_remains_only_in_an_npm_lockfile() {
    let r = extinct_baseline("lock-npm");
    r.edit(|d| {
        d.donors[0].packages.push(Package {
            ecosystem: Ecosystem::Npm,
            name: "geo".into(),
        })
    });
    r.write(
        "apps/web/package.json",
        "{\"name\":\"web\",\"devDependencies\":{\"vite\":\"^6\"}}",
    );
    r.write("apps/web/package-lock.json", "{\"name\":\"web\",\"lockfileVersion\":3,\"packages\":{\"\":{\"name\":\"web\",\"devDependencies\":{\"vite\":\"^6\"}},\"node_modules/vite\":{\"version\":\"6.0.0\",\"dev\":true,\"dependencies\":{\"geo\":\"^1\"}},\"node_modules/geo\":{\"version\":\"1.0.0\",\"dev\":true}}}");
    let a = r.assess();
    let d = a.donor("geo").unwrap();
    assert_ne!(d.effective, DonorState::Extinct, "{:#?}", d.gates);
    assert!(
        !gate(&a, "geo", Gate::BuildEdges),
        "a dev dependency's closure is build participation"
    );
    assert!(gate(&a, "geo", Gate::LinkedEdges));
}
