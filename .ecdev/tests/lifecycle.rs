//! Technology lifecycle, migration waves and evidence compaction across a repository.

mod common;

use common::*;
use ecdev_governance::declare::*;
use ecdev_governance::schema::*;

/// The baseline, declaring a canonical native technology implementing `identity.digest`.
fn technology_shard(name: &str) -> Repo {
    let r = extinct_baseline(name);
    r.write("core/src/digest.rs", "pub fn digest(bytes: &[u8]) -> Digest { todo!() }\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn vectors() {}\n}\n");
    r.edit(|d| {
        d.repository.nodes[0].provides = vec!["identity.digest".into()];
        d.technologies = vec![Technology {
            key: "hash.digest".into(),
            name: "native digest".into(),
            kind: TechnologyKind::Algorithm,
            claimed: TechnologyLifecycle::Canonical,
            purpose: "content identity".into(),
            implements: vec!["identity.digest".into()],
            node: "core".into(),
            sources: vec!["core/src/digest.rs".into()],
            invariants: vec!["published vectors".into()],
            proofs: vec![proof(ProofKind::Regression, "core/src/digest.rs::vectors")],
            lineage: vec!["geo".into()],
            relations: vec![],
            claims: vec![Improvement {
                dimension: Dimension::Latency,
                baseline: "donor".into(),
                workload: "1 MiB".into(),
                statement: "faster".into(),
                evidence: vec![],
            }],
        }];
    });
    r.prove();
    r
}

#[test]
fn technology_lifecycle_is_computed_and_claims_are_checked() {
    let r = technology_shard("tech-life");
    let a = r.assess();
    let t = a
        .technologies
        .iter()
        .find(|t| t.key == "hash.digest")
        .unwrap();
    assert_eq!(
        t.effective,
        TechnologyLifecycle::Canonical,
        "{}",
        t.stopped_by
    );
    assert_eq!(a.metric("technologies_canonical"), "1");
    assert!(
        finding(&a, "IMPROVEMENT_UNPROVEN", "hash.digest").is_some(),
        "\"faster\" without evidence stays unproven"
    );
    // Proofs bind to the technology's canonical sources: unrelated edits keep them fresh...
    r.write("core/src/lib.rs", "pub fn id() -> u64 { 2 }\n");
    assert_eq!(
        r.assess().technologies[0].effective,
        TechnologyLifecycle::Canonical
    );
    // ...an edit of the sources makes them stale, and the CANONICAL claim becomes a false claim.
    r.write("core/src/digest.rs", "pub fn digest(bytes: &[u8]) -> Digest { unimplemented!() }\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn vectors() {}\n}\n");
    let a = r.assess();
    assert_eq!(a.technologies[0].effective, TechnologyLifecycle::Native);
    assert!(finding(&a, "TECHNOLOGY_CLAIM_EXCEEDS_EVIDENCE", "hash.digest").is_some());
    // A node that wraps a donor does not taint a technology whose sources stand alone...
    r.write("core/Cargo.toml", "[package]\nname = \"core-kernel\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\ngeo = \"0.33\"\n");
    assert_eq!(
        r.assess().technologies[0].effective,
        TechnologyLifecycle::Native
    );
    // ...but sources that import the donor, or reach into the wrapping node, are not native.
    for body in [
        "pub fn digest(bytes: &[u8]) -> geo::Point { todo!() }\n",
        "pub fn digest(bytes: &[u8]) -> crate::Point { todo!() }\n",
    ] {
        r.write("core/src/digest.rs", body);
        let a = r.assess();
        assert_eq!(
            a.technologies[0].effective,
            TechnologyLifecycle::Experimental,
            "{body}"
        );
        assert!(a.technologies[0].stopped_by.contains("not self-contained"));
    }
}

/// The extinct baseline with `geo` back at a legacy path, ready for a wave that moves it.
fn legacy_geo(name: &str) -> Repo {
    let r = extinct_baseline(name);
    std::fs::rename(r.path().join("substrate/geo"), r.path().join("geo")).unwrap();
    let _ = std::fs::remove_dir(r.path().join("substrate"));
    r.write(
        "Cargo.toml",
        "[workspace]\nresolver = \"2\"\nmembers = [\"core\", \"geo\", \"tests\"]\n",
    );
    r.write(
        "geo/Cargo.toml",
        &r.read("geo/Cargo.toml").replace("../../core", "../core"),
    );
    r.write(
        "tests/Cargo.toml",
        &r.read("tests/Cargo.toml")
            .replace("../substrate/geo", "../geo"),
    );
    r.edit(|d| d.repository.nodes[1].path = "geo".into());
    r
}

/// `legacy_geo` with a technology inside `geo` and every declared path naming the legacy tree.
fn legacy_geo_with_technology(name: &str) -> Repo {
    let r = legacy_geo(name);
    r.write(
        "geo/src/digest.rs",
        "pub fn digest() {}\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn vectors() {}\n}\n",
    );
    r.edit(|d| {
        d.donors[0].capabilities[0].proofs[1].locator =
            "geo/src/lib.rs::regression_distance".into();
        d.donors[0].provenance = vec!["census:geo/Cargo.toml".into()];
        d.technologies = vec![Technology {
            key: "hash.digest".into(),
            name: "native digest".into(),
            kind: TechnologyKind::Algorithm,
            claimed: TechnologyLifecycle::Canonical,
            purpose: "content identity".into(),
            implements: vec!["identity.digest".into()],
            node: "geo".into(),
            sources: vec!["geo/src/digest.rs".into()],
            invariants: vec!["published vectors".into()],
            proofs: vec![proof(ProofKind::Regression, "geo/src/digest.rs::vectors")],
            lineage: vec![],
            relations: vec![],
            claims: vec![Improvement {
                dimension: Dimension::Latency,
                baseline: "donor".into(),
                workload: "1 MiB".into(),
                statement: "faster".into(),
                evidence: vec![proof(ProofKind::Regression, "geo/src/digest.rs::vectors")],
            }],
        }];
    });
    // Only evidence recorded at the legacy locators.
    r.remove(".ecdev/evidence");
    r.prove();
    r
}

fn verdicts(
    r: &Repo,
) -> std::collections::BTreeMap<(String, String), ecdev_governance::evidence::Verdict> {
    let a = r.assess();
    ecdev_governance::evidence::verdicts(&a.files, &a.declaration, &a.evidence)
}

#[test]
fn a_wave_rewrites_declared_paths_and_old_evidence_stays_history() {
    use ecdev_governance::evidence::Verdict;
    let r = legacy_geo_with_technology("relocators");
    let old = (
        "geo".to_string(),
        "geo/src/lib.rs::regression_distance".to_string(),
    );
    assert_eq!(verdicts(&r).get(&old), Some(&Verdict::Pass));
    let (_, plan) = r.cli(&["migrate", "plan", "--write"]);
    assert!(plan.contains("w01-geo PLANNED: geo"), "{plan}");
    let (code, out) = r.cli(&["migrate", "apply", "w01-geo"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("declared donor `geo` proof: geo/src/lib.rs::regression_distance -> substrate/geo/src/lib.rs::regression_distance"),
        "{out}"
    );
    assert!(out.contains("ecdev-gov prove hash.digest"), "{out}");

    let d = r.declaration();
    let c = &d.donors[0].capabilities[0];
    assert_eq!(
        c.proofs[1].locator,
        "substrate/geo/src/lib.rs::regression_distance"
    );
    assert_eq!(
        c.proofs[0].locator, "tests/tests/parity.rs::distance_matches_donor",
        "a locator outside the moved tree is untouched"
    );
    let t = &d.technologies[0];
    assert_eq!(t.sources, vec!["substrate/geo/src/digest.rs".to_string()]);
    assert_eq!(t.proofs[0].locator, "substrate/geo/src/digest.rs::vectors");
    assert_eq!(
        t.claims[0].evidence[0].locator,
        "substrate/geo/src/digest.rs::vectors"
    );
    assert_eq!(
        d.donors[0].provenance,
        vec!["census:geo/Cargo.toml".to_string()],
        "provenance keeps the paths of its time"
    );

    // Evidence recorded under the old locators stays as it was and proves nothing new.
    let store = ecdev_governance::evidence::Store::load(r.path());
    assert!(store.records.contains_key(&old.1));
    let v = verdicts(&r);
    assert!(!v.contains_key(&old));
    let new = (
        "geo".to_string(),
        "substrate/geo/src/lib.rs::regression_distance".to_string(),
    );
    assert_eq!(v.get(&new), Some(&Verdict::Unrecorded));
    // Re-proving is how it is freshened.
    r.prove();
    assert_eq!(verdicts(&r).get(&new), Some(&Verdict::Pass));
}

#[test]
fn compact_retires_evidence_of_undeclared_locators_into_history() {
    use ecdev_governance::evidence::Store;
    let r = legacy_geo_with_technology("retire");
    r.cli(&["migrate", "plan", "--write"]);
    let (code, out) = r.cli(&["migrate", "apply", "w01-geo"]);
    assert_eq!(code, 0, "{out}");
    r.prove();
    let before = verdicts(&r);
    let store = Store::load(r.path());
    let mut old: Vec<ecdev_governance::evidence::Record> = store
        .records
        .iter()
        .filter(|(l, _)| l.starts_with("geo/"))
        .flat_map(|(_, v)| v.clone())
        .collect();
    old.sort();
    assert_eq!(old.len(), 2, "{old:?}");

    let (code, out) = r.cli(&["compact"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("evidence: retired 2 records of undeclared proofs"),
        "{out}"
    );
    assert_eq!(verdicts(&r), before, "no current verdict changes");
    let after = Store::load(r.path());
    assert!(after.unreadable.is_empty(), "{:?}", after.unreadable);
    assert!(!after.records.keys().any(|l| l.starts_with("geo/")));
    let mut kept: Vec<ecdev_governance::evidence::Record> =
        after.retired.iter().map(|x| x.record.clone()).collect();
    kept.sort();
    assert_eq!(kept, old, "history keeps every retired record whole");
    for x in &after.retired {
        assert_eq!(x.seq, 1);
        assert_eq!(
            x.address,
            ecdev_governance::evidence::address(&x.record.encode())
        );
        assert!(!r.exists(&format!(".ecdev/evidence/{}", x.address)));
    }
    assert_eq!(after.retired_files.len(), 1);
    assert!(ecdev_governance::assess(r.path())
        .unwrap()
        .findings
        .iter()
        .all(|f| f.code != "CORRUPT_STATE"));

    // Compacting again retires nothing and keeps the same history.
    let (_, out) = r.cli(&["compact"]);
    assert!(out.contains("evidence: retired 0 records"), "{out}");
    let again = Store::load(r.path());
    assert_eq!(again.retired, after.retired);
    assert_eq!(again.retired_files, after.retired_files);
    assert_eq!(verdicts(&r), before);
}

/// Records of one locator, as (subject, passed), sorted.
fn records_of(r: &Repo, locator: &str) -> Vec<(String, bool)> {
    let store = ecdev_governance::evidence::Store::load(r.path());
    let mut v: Vec<(String, bool)> = store
        .records
        .get(locator)
        .into_iter()
        .flatten()
        .map(|x| (x.subject.clone(), x.passed))
        .collect();
    v.sort();
    v
}

#[test]
fn compact_keeps_evidence_of_every_subject_sharing_a_locator() {
    use ecdev_governance::evidence::Verdict;
    const SHARED: &str = "tests/tests/parity.rs::distance_matches_donor";
    let r = extinct_baseline("shared-locator");
    // A second donor whose capability is replaced by another node, proven by the same test.
    r.edit(|d| {
        let mut second = donor("proj", "proj");
        second.capabilities[0].replacement = Some("core".into());
        second.capabilities[0].proofs.truncate(1);
        assert_eq!(second.capabilities[0].proofs[0].locator, SHARED);
        d.donors.push(second);
    });
    r.prove();
    // The proof file changes: every record of the shared locator goes stale; re-proving records
    // fresh ones for both subjects beside the stale ones.
    r.write(
        "tests/tests/parity.rs",
        &format!(
            "{}// one more frozen case to come\n",
            r.read("tests/tests/parity.rs")
        ),
    );
    r.prove();
    let before = verdicts(&r);
    assert_eq!(
        before.get(&("geo".to_string(), SHARED.to_string())),
        Some(&Verdict::Pass)
    );
    assert_eq!(
        before.get(&("proj".to_string(), SHARED.to_string())),
        Some(&Verdict::Pass)
    );
    let proven_before = r.assess().counts.values();
    assert_eq!(
        records_of(&r, SHARED).len(),
        4,
        "{:?}",
        records_of(&r, SHARED)
    );

    let (code, out) = r.cli(&["compact"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("evidence: pruned 2 stale records"), "{out}");
    assert!(out.contains("evidence: retired 0 records"), "{out}");
    assert_eq!(
        records_of(&r, SHARED),
        vec![("core".to_string(), true), ("geo".to_string(), true)],
        "each subject keeps its fresh passing record"
    );
    assert_eq!(verdicts(&r), before, "no verdict changes");
    assert_eq!(r.assess().counts.values(), proven_before);
    // Compacting again changes nothing.
    let (_, out) = r.cli(&["compact"]);
    assert!(out.contains("evidence: pruned 0 stale records"), "{out}");
    assert_eq!(verdicts(&r), before);

    // Once no declared proof judges `core` with that locator, its record is history; the other
    // subject's record of the same locator stays evidence.
    r.edit(|d| d.donors.retain(|x| x.key != "proj"));
    let before = verdicts(&r);
    let (_, out) = r.cli(&["compact"]);
    assert!(out.contains("evidence: retired 1 records"), "{out}");
    assert_eq!(records_of(&r, SHARED), vec![("geo".to_string(), true)]);
    assert_eq!(verdicts(&r), before);
}

#[test]
fn planning_names_misplaced_nodes_whose_waves_are_pending() {
    let r = legacy_geo("pending-plan");
    let (_, plan) = r.cli(&["migrate", "plan", "--write"]);
    assert!(plan.contains("w01-geo PLANNED: geo"), "{plan}");
    let (code, plan) = r.cli(&["migrate", "plan"]);
    assert_eq!(code, 0, "{plan}");
    assert!(
        !plan.contains("every node is at its canonical path"),
        "{plan}"
    );
    assert!(
        plan.contains("1 node is not at its canonical path"),
        "{plan}"
    );
    assert!(
        plan.contains("pending w01-geo PLANNED: geo (geo -> substrate/geo)"),
        "{plan}"
    );
    let (code, out) = r.cli(&["migrate", "apply", "w01-geo"]);
    assert_eq!(code, 0, "{out}");
    let (_, plan) = r.cli(&["migrate", "plan"]);
    assert_eq!(
        plan,
        "nothing to plan: every node is at its canonical path\n"
    );
}
