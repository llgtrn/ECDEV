//! Chronica as one system: capsules, the linker, the technology graph and agent context.

mod common;

use common::*;
use ynventa::capsule::Capsule;
use ynventa::declare::*;
use ynventa::linker::{link, SystemImage};
use ynventa::schema::*;

/// A second shard (Esellios) with its own keys, a node that needs `identity.digest`.
fn commerce_shard(name: &str) -> Repo {
    let r = Repo::new(name);
    r.write(
        "Cargo.toml",
        "[workspace]\nresolver = \"2\"\nmembers = [\"domain/order\"]\n",
    );
    r.write("README.md", "# commerce\n");
    r.write(
        "domain/order/Cargo.toml",
        "[package]\nname = \"order\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    r.write("domain/order/src/lib.rs", "pub fn place_order(cart: Cart, buyer: BuyerId) -> Result<OrderId, OrderError> { todo!() }\n");
    let mut order = node(
        "commerce.order",
        NodeKind::Domain,
        "domain/order",
        "domain/order",
    );
    order.requires = vec!["identity.digest".into()];
    order.provides = vec!["commerce.order".into()];
    r.store(&Declaration {
        repository: Repository {
            system: "chronica".into(),
            shard: "esellios".into(),
            name: name.into(),
            origin: "llgtrn/Esellios".into(),
            nodes: vec![
                order,
                node(
                    "ynventa.esellios",
                    NodeKind::Ynventa,
                    ".ynventa",
                    ".ynventa",
                ),
            ],
            edges: vec![],
        },
        donors: vec![],
        migration: Migration::default(),
        technologies: vec![],
        organism: Organism::default(),
    });
    r
}

/// The Mechatron baseline, declaring a canonical native technology implementing `identity.digest`.
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
            norl: NorlRelevance::NotRelevant("fixture technology".into()),
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

fn link_repos(repos: &[&Repo]) -> SystemImage {
    let caps: Vec<Capsule> = repos
        .iter()
        .map(|r| Capsule::compile(&r.assess()))
        .collect();
    link(&caps)
}

fn has(image: &SystemImage, code: &str) -> bool {
    image.issues.iter().any(|i| i.code == code)
}

#[test]
fn required_capabilities_must_resolve_across_shards() {
    let commerce = commerce_shard("req-commerce");
    let image = link_repos(&[&commerce]);
    let unresolved = image
        .issues
        .iter()
        .find(|i| i.code == "UNPROVIDED_CAPABILITY")
        .unwrap();
    assert_eq!(unresolved.subject, "identity.digest");
    assert!(!image.pass());
    let machine = technology_shard("req-machine");
    let image = link_repos(&[&commerce, &machine]);
    assert!(!has(&image, "UNPROVIDED_CAPABILITY"), "{:#?}", image.issues);
    let cap = image
        .capabilities
        .iter()
        .find(|c| c.key == "identity.digest")
        .unwrap();
    assert_eq!(
        cap.providers,
        vec![("mechatron".to_string(), "core".to_string())]
    );
    assert_eq!(
        cap.requirers,
        vec![("esellios".to_string(), "commerce.order".to_string())]
    );
    assert_eq!(cap.technologies, vec!["hash.digest".to_string()]);
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

#[test]
fn technology_is_reused_by_materialization_not_by_service() {
    let machine = technology_shard("reuse-machine");
    let commerce = commerce_shard("reuse-commerce");
    let (code, out) = commerce.cli(&[
        "technology",
        "materialize",
        "hash.digest",
        "--from",
        &machine.path().display().to_string(),
        "--node",
        "commerce.order",
        "--into",
        "domain/order/src/tech",
    ]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        commerce.read("domain/order/src/tech/digest.rs"),
        machine.read("core/src/digest.rs")
    );
    assert_eq!(
        commerce
            .declaration()
            .node("commerce.order")
            .unwrap()
            .reuses,
        vec!["hash.digest".to_string()]
    );
    // Adoption is a system fact: decided by linking.
    let image = link_repos(&[&machine, &commerce]);
    let t = image
        .technologies
        .iter()
        .find(|t| t.key == "hash.digest")
        .unwrap();
    assert_eq!(t.effective, TechnologyLifecycle::Adopted);
    assert_eq!(t.birthplace, "mechatron");
    assert_eq!(t.consumers, vec!["esellios".to_string()]);
    assert_eq!(image.counts().technologies_adopted, 1);
    // No runtime coupling was created: no cross-shard code dependency.
    assert!(!has(&image, "CROSS_SHARD_CODE_DEPENDENCY"));
    // A local edit of materialized source is a silent fork.
    commerce.write(
        "domain/order/src/tech/digest.rs",
        "pub fn digest(bytes: &[u8]) -> Digest { unimplemented!() }\n",
    );
    assert!(finding(&commerce.assess(), "MATERIALIZED_FORK", "hash.digest").is_some());
    // The canonical source evolves at its birthplace; re-materializing supersedes the old lock.
    machine.write(
        "core/src/digest.rs",
        "pub fn digest(bytes: &[u8]) -> Digest { todo!(\"v2\") }\n",
    );
    let (code, out) = commerce.cli(&[
        "technology",
        "materialize",
        "hash.digest",
        "--from",
        &machine.path().display().to_string(),
        "--node",
        "commerce.order",
        "--into",
        "domain/order/src/tech",
    ]);
    assert_eq!(code, 0, "{out}");
    let a = commerce.assess();
    assert!(finding(&a, "MATERIALIZED_FORK", "hash.digest").is_none());
    let locks = std::fs::read_dir(commerce.path().join(".ynventa/materialized"))
        .unwrap()
        .count();
    assert_eq!(locks, 1);
}

#[test]
fn duplicated_technology_without_relation_fails_linking() {
    let machine = technology_shard("dup-machine");
    let commerce = commerce_shard("dup-commerce");
    commerce.write(
        "domain/order/src/digest.rs",
        "pub fn digest(bytes: &[u8]) -> Digest { todo!() }\n",
    );
    commerce.edit(|d| {
        d.repository.nodes[0]
            .provides
            .push("identity.digest".into());
        d.technologies = vec![Technology {
            key: "hash.order-digest".into(),
            name: "another digest".into(),
            kind: TechnologyKind::Algorithm,
            claimed: TechnologyLifecycle::Experimental,
            purpose: String::new(),
            implements: vec!["identity.digest".into()],
            node: "commerce.order".into(),
            sources: vec!["domain/order/src/digest.rs".into()],
            invariants: vec![],
            proofs: vec![],
            lineage: vec![],
            relations: vec![],
            norl: NorlRelevance::NotRelevant("fixture technology".into()),
            claims: vec![],
        }];
    });
    let image = link_repos(&[&machine, &commerce]);
    assert!(has(&image, "DUPLICATE_TECHNOLOGY"), "{:#?}", image.issues);
    assert!(has(&image, "DUPLICATE_CAPABILITY"));
    // The same operation shape in both shards is reported from YIR.
    assert!(
        image
            .duplicate_symbols
            .iter()
            .any(|(_, sig, m)| sig.starts_with("digest(") && m.len() == 2),
        "{:#?}",
        image.duplicate_symbols
    );
    // Declaring the relation makes the specialization legitimate.
    commerce.edit(|d| {
        d.technologies[0].relations = vec![Relation {
            kind: EdgeKind::Specializes,
            target: "hash.digest".into(),
        }];
        d.repository.edges = vec![Edge {
            from: "commerce.order".into(),
            to: "core".into(),
            kind: EdgeKind::Specializes,
            scope: Scope::Semantic,
        }];
    });
    let image = link_repos(&[&machine, &commerce]);
    assert!(
        !has(&image, "DUPLICATE_TECHNOLOGY") && !has(&image, "DUPLICATE_CAPABILITY"),
        "{:#?}",
        image.issues
    );
    // A third shard's alternative for the same canonical technology joins its declared family:
    // no pairwise relation to every sibling is needed...
    let third = commerce_shard("dup-third");
    third.write(
        "domain/order/src/digest.rs",
        "pub fn digest_alt(bytes: &[u8]) -> Digest { todo!() }\n",
    );
    third.edit(|d| {
        d.repository.shard = "fi-game".into();
        d.repository.origin = "llgtrn/Fi-game".into();
        d.repository.nodes[0].key = "finance.digest".into();
        d.repository.nodes[0].provides = vec![];
        d.repository.nodes[0].requires = vec![];
        d.repository.nodes[1].key = "ynventa.fi-game".into();
        d.technologies = vec![Technology {
            key: "hash.alt-digest".into(),
            name: "alternative digest".into(),
            kind: TechnologyKind::Algorithm,
            claimed: TechnologyLifecycle::Experimental,
            purpose: String::new(),
            implements: vec!["identity.digest".into()],
            node: "finance.digest".into(),
            sources: vec!["domain/order/src/digest.rs".into()],
            invariants: vec![],
            proofs: vec![],
            lineage: vec![],
            relations: vec![Relation {
                kind: EdgeKind::AlternativeFor,
                target: "hash.digest".into(),
            }],
            norl: NorlRelevance::NotRelevant("fixture technology".into()),
            claims: vec![],
        }];
    });
    let image = link_repos(&[&machine, &commerce, &third]);
    assert!(!has(&image, "DUPLICATE_TECHNOLOGY"), "{:#?}", image.issues);
    // ...but without its relation it is an independent duplicate of both.
    third.edit(|d| d.technologies[0].relations.clear());
    let image = link_repos(&[&machine, &commerce, &third]);
    let dups = image
        .issues
        .iter()
        .filter(|i| i.code == "DUPLICATE_TECHNOLOGY")
        .count();
    assert_eq!(dups, 2, "{:#?}", image.issues);
}

#[test]
fn shards_couple_only_through_capabilities_and_technologies() {
    let machine = technology_shard("couple-machine");
    let commerce = commerce_shard("couple-commerce");
    commerce.edit(|d| {
        d.repository.edges = vec![Edge {
            from: "commerce.order".into(),
            to: "core".into(),
            kind: EdgeKind::DependsOn,
            scope: Scope::Build,
        }]
    });
    let image = link_repos(&[&machine, &commerce]);
    let e = image
        .issues
        .iter()
        .find(|i| i.code == "CROSS_SHARD_CODE_DEPENDENCY")
        .unwrap();
    assert!(e.detail.contains("esellios") && e.detail.contains("mechatron"));
    // A reference to a node no shard declares is a link error.
    commerce.edit(|d| {
        d.repository.edges = vec![Edge {
            from: "commerce.order".into(),
            to: "payment.unknown".into(),
            kind: EdgeKind::DependsOn,
            scope: Scope::Architectural,
        }]
    });
    let image = link_repos(&[&machine, &commerce]);
    assert!(has(&image, "UNRESOLVED_REFERENCE"), "{:#?}", image.issues);
}

#[test]
fn capsules_are_deterministic_and_round_trip() {
    let r = technology_shard("capsule");
    let a = Capsule::compile(&r.assess());
    let b = Capsule::compile(&r.assess());
    assert_eq!(a.encode(), b.encode());
    assert_eq!(Capsule::decode(&a.encode()).unwrap(), a);
    assert_eq!(a.provides(), vec!["identity.digest".to_string()]);
    assert!(a.owns().contains(&"technology/hash.digest".to_string()));
    assert!(a
        .symbols
        .iter()
        .any(|s| s.name == "digest" && s.output == "Digest"));
    let (code, out) = r.cli(&["capsule"]);
    assert_eq!(code, 0, "{out}");
    assert!(r.exists("target/ynventa/mechatron.ynv"));
}

#[test]
fn agent_context_knows_what_exists_elsewhere() {
    let machine = technology_shard("ctx-machine");
    let commerce = commerce_shard("ctx-commerce");
    let sys = commerce
        .path()
        .join("target/chronica.system.ynv")
        .display()
        .to_string();
    let (_, out) = commerce.cli(&[
        "link",
        &machine.path().display().to_string(),
        &commerce.path().display().to_string(),
        "--out",
        &sys,
    ]);
    assert!(out.contains("system link"), "{out}");
    let (code, ctx) = commerce.cli(&["context", "--system", &sys]);
    assert_eq!(code, 0);
    assert!(ctx.contains("CURRENT PHYSICAL SHARD\n  esellios"), "{ctx}");
    assert!(
        ctx.contains("identity.digest <- core in mechatron"),
        "{ctx}"
    );
    assert!(ctx.contains("hash.digest"), "{ctx}");
    assert!(ctx.contains("FORBIDDEN DUPLICATION"), "{ctx}");
    let (_, search) = commerce.cli(&["technology", "search", "digest", "--system", &sys]);
    assert!(
        search.contains("hash.digest") && search.contains("born in mechatron"),
        "{search}"
    );
    let (_, show) = commerce.cli(&["show", "identity.digest", "--system", &sys]);
    assert!(
        show.contains("<- PROVIDES")
            && show.contains("<- REQUIRES")
            && show.contains("<- IMPLEMENTS"),
        "{show}"
    );
    let (_, back) = commerce.cli(&["backlinks", "ynv://chronica/core", "--system", &sys]);
    assert!(back.contains("physical owner mechatron"), "{back}");
}

#[test]
fn a_refresh_that_invalidates_evidence_names_what_to_re_prove() {
    let birthplace = technology_shard("stale-birth");
    let consumer = extinct_baseline("stale-consumer");
    assert_eq!(
        consumer.assess().analysis.donors[0].effective,
        DonorState::Extinct
    );
    let from = birthplace.path().display().to_string();
    let materialize = |r: &Repo| {
        r.cli(&[
            "technology",
            "materialize",
            "hash.digest",
            "--from",
            &from,
            "--node",
            "geo",
            "--into",
            "substrate/geo/src/tech",
        ])
    };
    // New sources inside the donor's replacement node change what its proofs were judged on.
    let (code, out) = materialize(&consumer);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("evidence made stale by this refresh"), "{out}");
    assert!(out.contains("`ynventa prove geo`"), "{out}");
    assert_ne!(
        consumer.assess().analysis.donors[0].effective,
        DonorState::Extinct
    );
    // Re-proving restores the state; refreshing identical sources then invalidates nothing.
    consumer.prove();
    assert_eq!(
        consumer.assess().analysis.donors[0].effective,
        DonorState::Extinct
    );
    let (code, out) = materialize(&consumer);
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("evidence made stale"), "{out}");
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

fn locks(r: &Repo) -> Vec<ynventa::technology::Materialization> {
    let mut unreadable = Vec::new();
    let v = ynventa::technology::Materialization::load_all(r.path(), &mut unreadable);
    assert!(unreadable.is_empty(), "{unreadable:?}");
    v
}

#[test]
fn a_wave_moves_materialization_locks_with_their_node() {
    let birthplace = technology_shard("relock-birth");
    let consumer = legacy_geo("relock-consumer");
    let (code, out) = consumer.cli(&[
        "technology",
        "materialize",
        "hash.digest",
        "--from",
        &birthplace.path().display().to_string(),
        "--node",
        "geo",
        "--into",
        "geo/src/tech",
    ]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(locks(&consumer)[0].files[0].0, "geo/src/tech/digest.rs");
    let (_, plan) = consumer.cli(&["migrate", "plan", "--write"]);
    assert!(plan.contains("w01-geo PLANNED: geo"), "{plan}");
    let (code, out) = consumer.cli(&["migrate", "apply", "w01-geo"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("relocated materialization lock of hash.digest"),
        "{out}"
    );
    // The lock now names the moved copy: it verifies, and a fork is still detected.
    let l = locks(&consumer);
    assert_eq!(l.len(), 1, "the old lock is gone");
    assert_eq!(l[0].files[0].0, "substrate/geo/src/tech/digest.rs");
    assert_eq!(l[0].node, "geo");
    let a = consumer.assess();
    assert!(finding(&a, "MATERIALIZED_MISSING", "hash.digest").is_none());
    assert!(finding(&a, "MATERIALIZED_FORK", "hash.digest").is_none());
    consumer.write("substrate/geo/src/tech/digest.rs", "pub fn digest() {}\n");
    assert!(finding(&consumer.assess(), "MATERIALIZED_FORK", "hash.digest").is_some());
}

#[test]
fn re_materializing_for_a_node_supersedes_its_earlier_lock_elsewhere() {
    let birthplace = technology_shard("resupersede-birth");
    let consumer = extinct_baseline("resupersede-consumer");
    let from = birthplace.path().display().to_string();
    let materialize = |into: &str| {
        consumer.cli(&[
            "technology",
            "materialize",
            "hash.digest",
            "--from",
            &from,
            "--node",
            "geo",
            "--into",
            into,
        ])
    };
    let (code, out) = materialize("substrate/geo/src/tech");
    assert_eq!(code, 0, "{out}");
    // The copy moves within the node (or the node moved by hand): the node re-materializes.
    consumer.remove("substrate/geo/src/tech");
    let (code, out) = materialize("substrate/geo/src/vendor");
    assert_eq!(code, 0, "{out}");
    let l = locks(&consumer);
    assert_eq!(l.len(), 1, "{l:?}");
    assert_eq!(l[0].files[0].0, "substrate/geo/src/vendor/digest.rs");
    assert!(finding(&consumer.assess(), "MATERIALIZED_MISSING", "hash.digest").is_none());
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
            norl: NorlRelevance::NotRelevant("fixture technology".into()),
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
    r.remove(".ynventa/evidence");
    r.prove();
    r
}

fn verdicts(r: &Repo) -> std::collections::BTreeMap<(String, String), ynventa::evidence::Verdict> {
    let a = r.assess();
    ynventa::evidence::verdicts(&a.files, &a.declaration, &a.evidence)
}

#[test]
fn a_wave_rewrites_declared_paths_and_old_evidence_stays_history() {
    use ynventa::evidence::Verdict;
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
    assert!(out.contains("ynventa prove hash.digest"), "{out}");

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
    let store = ynventa::evidence::Store::load(r.path());
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
    use ynventa::evidence::Store;
    let r = legacy_geo_with_technology("retire");
    r.cli(&["migrate", "plan", "--write"]);
    let (code, out) = r.cli(&["migrate", "apply", "w01-geo"]);
    assert_eq!(code, 0, "{out}");
    r.prove();
    let before = verdicts(&r);
    let store = Store::load(r.path());
    let mut old: Vec<ynventa::evidence::Record> = store
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
    let mut kept: Vec<ynventa::evidence::Record> =
        after.retired.iter().map(|x| x.record.clone()).collect();
    kept.sort();
    assert_eq!(kept, old, "history keeps every retired record whole");
    for x in &after.retired {
        assert_eq!(x.seq, 1);
        assert_eq!(x.address, ynventa::evidence::address(&x.record.encode()));
        assert!(!r.exists(&format!(".ynventa/evidence/{}", x.address)));
    }
    assert_eq!(after.retired_files.len(), 1);
    assert!(ynventa::assess(r.path())
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
    let store = ynventa::evidence::Store::load(r.path());
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
    use ynventa::evidence::Verdict;
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
