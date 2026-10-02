//! Norl, the organism: the grammar of its declarations, the donor ladder's mapping and Norl
//! rungs, and the migration of repositories written in the previous grammar.

mod common;

use common::*;
use ynventa::declare::*;
use ynventa::schema::*;

/// Rewrites the declarations of `r` into the previous grammar: no `maps_to`, no `norl`, no
/// `organism.rs` — what every repository held before the organism.
fn to_previous_grammar(r: &Repo) {
    for f in [
        ".ynventa/declared/donors.rs",
        ".ynventa/declared/technologies.rs",
    ] {
        let old: String = r
            .read(f)
            .lines()
            .filter(|l| {
                !l.trim_start().starts_with("maps_to:") && !l.trim_start().starts_with("norl:")
            })
            .map(|l| format!("{l}\n"))
            .collect();
        r.write(f, &old);
    }
    r.remove(".ynventa/declared/organism.rs");
}

fn ids(a: &ynventa::Assessment) -> Vec<String> {
    a.graph.nodes.keys().map(|k| k.to_string()).collect()
}

#[test]
fn the_previous_grammar_is_migrated_never_accepted_silently() {
    let r = extinct_baseline("grammar");
    let before = r.assess();
    assert_eq!(before.donor("geo").unwrap().effective, DonorState::Extinct);
    to_previous_grammar(&r);

    // The current reader refuses it and names the migration.
    let err = ynventa::assess(r.path()).err().unwrap();
    assert!(err.contains("migrate schema"), "{err}");
    let (code, out) = r.cli(&["verify"]);
    assert_eq!(code, 2, "{out}");

    // `migrate schema` rewrites into the current grammar with undecided values...
    let (code, out) = r.cli(&["migrate", "schema"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("rewrote .ynventa/declared/donors.rs"), "{out}");
    assert!(
        out.contains("rewrote .ynventa/declared/organism.rs"),
        "{out}"
    );
    let d = r.declaration();
    assert_eq!(d.organism, Organism::default());
    let cap = &d.donors[0].capabilities[0];
    assert_eq!(cap.maps_to, None);
    assert_eq!(cap.norl, NorlRelevance::Unresolved);
    assert!(r
        .read(".ynventa/declared/donors.rs")
        .contains("norl: NorlRelevance::Unresolved"));

    // ...never lowers a claim, and the claim the new ladder no longer supports is reported.
    assert_eq!(d.donors[0].claimed, DonorState::Extinct);
    assert!(out.contains("1 donor claims exceed"), "{out}");
    let a = r.assess();
    let geo = a.donor("geo").unwrap();
    assert_eq!(geo.effective, DonorState::Censused);
    assert!(
        geo.stopped_by.starts_with("TECHNOLOGY_MAPPED"),
        "{}",
        geo.stopped_by
    );
    let f = finding(&a, "CLAIM_EXCEEDS_EVIDENCE", "geo").unwrap();
    assert!(f.detail.contains("maps_to"), "{}", f.detail);
    let (code, out) = r.cli(&["verify"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("CLAIM_EXCEEDS_EVIDENCE"), "{out}");

    // The migration is idempotent and changes no node identity.
    let (_, again) = r.cli(&["migrate", "schema"]);
    assert!(again.contains("already use the current grammar"), "{again}");
    assert_eq!(ids(&a), ids(&before));
}

#[test]
fn scaffold_upgrades_the_grammar_of_existing_declarations() {
    let r = extinct_baseline("grammar-scaffold");
    to_previous_grammar(&r);
    let (code, out) = r.cli(&["migrate", "scaffold"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("organism.rs"), "{out}");
    assert!(r.exists(".ynventa/declared/organism.rs"));
    assert!(ynventa::declare::load(r.path()).is_ok());
}

#[test]
fn the_donor_ladder_has_mapping_and_norl_rungs() {
    let r = extinct_baseline("ladder");
    let state = |r: &Repo| r.assess().donor("geo").unwrap().effective;
    assert_eq!(state(&r), DonorState::Extinct);

    // TECHNOLOGY_MAPPED sits between CENSUSED and SPECIFIED.
    r.edit(|d| d.donors[0].capabilities[0].maps_to = Some("  ".into()));
    let a = r.assess();
    assert_eq!(a.donor("geo").unwrap().effective, DonorState::Censused);
    assert!(!gate(&a, "geo", Gate::TechnologyMapping));
    r.edit(|d| d.donors[0].capabilities[0].maps_to = Some("capability/distance".into()));

    // NORL_RELEVANCE_RESOLVED sits between PARITY_PROVEN and CUTOVER.
    r.edit(|d| d.donors[0].capabilities[0].norl = NorlRelevance::Unresolved);
    let a = r.assess();
    let geo = a.donor("geo").unwrap();
    assert_eq!(geo.effective, DonorState::ParityProven);
    assert!(geo.stopped_by.starts_with("NORL_RELEVANCE_RESOLVED"));
    assert!(!gate(&a, "geo", Gate::NorlRelevance));
    assert!(finding(&a, "CLAIM_EXCEEDS_EVIDENCE", "geo").is_some());
    // An empty reason resolves nothing.
    r.edit(|d| d.donors[0].capabilities[0].norl = NorlRelevance::NotRelevant(" ".into()));
    assert_eq!(state(&r), DonorState::ParityProven);
    // Feeding a Norl capability resolves it (the key is resolved by the linker).
    r.edit(|d| {
        d.donors[0].capabilities[0].norl = NorlRelevance::Feeds("perception.geometry".into())
    });
    let a = r.assess();
    assert_eq!(a.donor("geo").unwrap().effective, DonorState::Extinct);
    assert!(gate(&a, "geo", Gate::NorlRelevance) && gate(&a, "geo", Gate::TechnologyMapping));
    // The feed is an edge of the graph: donor capability -FEEDS-> capability/<key>.
    let feed = a.graph.edges.iter().any(|e| {
        e.kind == EdgeKind::Feeds
            && e.to == ynventa::graph::NodeId::of("chronica", "capability/perception.geometry")
    });
    assert!(feed);
    // NATIVE_TARGETED sits between SPECIFIED and NATIVE_SHADOW: a declared replacement that is
    // only PLANNED is targeted, not shadowing.
    r.edit(|d| {
        let geo = d
            .repository
            .nodes
            .iter_mut()
            .find(|n| n.key == "geo")
            .unwrap();
        geo.lifecycle = NodeLifecycle::Planned;
    });
    let a = r.assess();
    let geo = a.donor("geo").unwrap();
    assert_eq!(
        geo.effective,
        DonorState::NativeTargeted,
        "{}",
        geo.stopped_by
    );
    assert!(
        geo.stopped_by.starts_with("NATIVE_SHADOW"),
        "{}",
        geo.stopped_by
    );
    r.edit(|d| d.donors[0].capabilities[0].replacement = Some("nowhere".into()));
    let geo = r.assess().donor("geo").unwrap().clone();
    assert_eq!(geo.effective, DonorState::Specified);
    assert!(
        geo.stopped_by.starts_with("NATIVE_TARGETED"),
        "{}",
        geo.stopped_by
    );
    // The ladder is the protocol's.
    let words: Vec<&str> = DonorState::ALL.iter().map(|s| s.wire()).collect();
    assert_eq!(
        words,
        [
            "DISCOVERED",
            "REGISTERED",
            "CENSUSED",
            "TECHNOLOGY_MAPPED",
            "SPECIFIED",
            "NATIVE_TARGETED",
            "NATIVE_SHADOW",
            "PARITY_PROVEN",
            "NORL_RELEVANCE_RESOLVED",
            "CUTOVER",
            "EXTINCT"
        ]
    );
}

#[test]
fn materials_are_typed_by_their_concept() {
    let r = extinct_baseline("materials");
    r.edit(|d| {
        let mut world = node("sim.world", NodeKind::Material, "", "");
        world.concept = Concept::World;
        let mut bogus = node("sim.store", NodeKind::Material, "", "");
        bogus.concept = Concept::Store;
        let mut lesson = node("sim.lesson", NodeKind::Material, "", "");
        lesson.concept = Concept::Experience;
        d.repository.nodes.extend([world, bogus, lesson]);
        d.repository.edges = vec![
            Edge {
                from: "geo".into(),
                to: "sim.world".into(),
                kind: EdgeKind::Generates,
                scope: Scope::Semantic,
            },
            // A domain node is not developmental material: it cannot FEED.
            Edge {
                from: "geo".into(),
                to: "capability/perception.space".into(),
                kind: EdgeKind::Feeds,
                scope: Scope::Semantic,
            },
            // Only a curriculum TEACHES.
            Edge {
                from: "sim.lesson".into(),
                to: "capability/perception.space".into(),
                kind: EdgeKind::Teaches,
                scope: Scope::Semantic,
            },
            // A proof locator binds only an evaluation.
            Edge {
                from: "proof:tests/tests/parity.rs::distance_matches_donor".into(),
                to: "sim.world".into(),
                kind: EdgeKind::Verifies,
                scope: Scope::Semantic,
            },
        ];
    });
    let a = r.assess();
    assert!(finding(&a, "INVALID_MATERIAL_CONCEPT", "sim.store").is_some());
    let feed = |prefix: &str| {
        a.findings
            .iter()
            .any(|f| f.code == "INVALID_NORL_FEED" && f.subject.starts_with(prefix))
    };
    assert!(feed("geo -FEEDS-> "), "{:#?}", a.findings);
    assert!(feed("sim.lesson -TEACHES-> "));
    assert!(a
        .findings
        .iter()
        .any(|f| f.code == "ILLEGAL_EDGE" && f.subject.starts_with("proof:")));
    // Materials are abstract: no path, no physical shape unit.
    assert!(a
        .graph
        .node_by_key("chronica", "sim.world")
        .unwrap()
        .path
        .is_empty());
}

// ---------------------------------------------------------------------------------------------
// Fixtures: the organism and a shard that teaches it.

fn edge(from: &str, to: &str, kind: EdgeKind) -> Edge {
    Edge {
        from: from.into(),
        to: to.into(),
        kind,
        scope: Scope::Semantic,
    }
}

fn material(key: &str, concept: Concept) -> Node {
    let mut n = node(key, NodeKind::Material, "", "");
    n.concept = concept;
    n
}

/// Norl: one organ (cognition) with one capability (`cognition.plan`), a deterministic
/// backend in `domain/cognition`, and an evaluation generated and proven by its tests.
fn norl_shard(name: &str) -> Repo {
    let r = Repo::new(name);
    r.write(
        "Cargo.toml",
        "[workspace]\nresolver = \"2\"\nmembers = [\"domain/cognition\", \"tests\"]\n",
    );
    r.write("README.md", "# norl\n");
    r.write(
        "domain/cognition/Cargo.toml",
        "[package]\nname = \"cognition\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    r.write(
        "domain/cognition/src/lib.rs",
        "pub fn plan(goal: Goal) -> Plan { todo!() }\n",
    );
    r.write("tests/Cargo.toml", "[package]\nname = \"evaluations\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dev-dependencies]\ncognition = { path = \"../domain/cognition\" }\n");
    r.write("tests/src/lib.rs", "");
    r.write(
        "tests/tests/eval.rs",
        "#[test]\nfn plans_reach_the_goal() {}\n",
    );
    let mut organ = node("organ.cognition", NodeKind::Organ, "", "");
    organ.name = "Cognition".into();
    r.store(&Declaration {
        repository: Repository {
            system: "chronica".into(),
            shard: "norl".into(),
            name: name.into(),
            origin: "llgtrn/Norl".into(),
            nodes: vec![
                node(
                    "cognition",
                    NodeKind::Domain,
                    "domain/cognition",
                    "domain/cognition",
                ),
                node("norl.evaluations", NodeKind::Test, "tests", "tests"),
                node("ynventa.norl", NodeKind::Ynventa, ".ynventa", ".ynventa"),
                organ,
                material("eval.plan", Concept::Evaluation),
            ],
            edges: vec![
                edge("norl.evaluations", "eval.plan", EdgeKind::Generates),
                edge(
                    "proof:tests/tests/eval.rs::plans_reach_the_goal",
                    "eval.plan",
                    EdgeKind::Verifies,
                ),
            ],
        },
        donors: vec![],
        migration: Migration::default(),
        technologies: vec![],
        organism: Organism {
            capabilities: vec![OrganismCapability {
                key: "cognition.plan".into(),
                organ: "organ.cognition".into(),
                claimed: GrowthState::Exposed,
                backend: Some("plan.rules".into()),
                evaluations: vec!["eval.plan".into()],
            }],
            backends: vec![Backend {
                key: "plan.rules".into(),
                kind: BackendKind::Deterministic,
                node: "cognition".into(),
                donor: None,
                weight: String::new(),
            }],
            promotions: vec![],
        },
    });
    r
}

/// Mechatron offering a simulated world and the experiences generated in it, which FEED
/// Norl's planning.
fn world_shard(name: &str) -> Repo {
    let r = extinct_baseline(name);
    r.edit(|d| {
        d.repository.nodes.extend([
            material("sim.world", Concept::World),
            material("sim.episodes", Concept::Experience),
        ]);
        d.repository.edges = vec![
            edge("geo", "sim.world", EdgeKind::Generates),
            edge("geo", "sim.episodes", EdgeKind::Generates),
            edge("sim.episodes", "sim.world", EdgeKind::Uses),
            edge("sim.episodes", "capability/cognition.plan", EdgeKind::Feeds),
        ];
    });
    r
}

fn link(repos: &[&Repo]) -> ynventa::linker::SystemImage {
    let caps: Vec<ynventa::capsule::Capsule> = repos
        .iter()
        .map(|r| ynventa::capsule::Capsule::compile(&r.assess()))
        .collect();
    ynventa::linker::link(&caps)
}

fn issue<'a>(
    image: &'a ynventa::linker::SystemImage,
    code: &str,
) -> Option<&'a ynventa::linker::LinkIssue> {
    image.issues.iter().find(|i| i.code == code)
}

fn codes(a: &ynventa::Assessment) -> Vec<String> {
    a.findings
        .iter()
        .filter(|f| f.severity == ynventa::Severity::Error)
        .map(|f| format!("{} {}", f.code, f.subject))
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Local gates.

#[test]
fn the_organism_is_legal_only_in_norl() {
    let norl = norl_shard("organism-in-norl");
    let a = norl.assess();
    assert!(codes(&a).is_empty(), "{:#?}", a.findings);
    // The same declaration in any other shard is rejected, organ and organism alike.
    norl.edit(|d| {
        d.repository.shard = "mechatron".into();
        d.repository.origin = "llgtrn/Mechatron".into();
    });
    let a = norl.assess();
    assert!(finding(&a, "ORGANISM_OUTSIDE_NORL", "organ.cognition").is_some());
    assert!(finding(&a, "ORGANISM_OUTSIDE_NORL", "organism").is_some());
    // Materials are legal everywhere.
    assert!(finding(&a, "ORGANISM_OUTSIDE_NORL", "eval.plan").is_none());
    assert!(finding(
        &world_shard("materials-anywhere").assess(),
        "ORGANISM_OUTSIDE_NORL",
        "sim.world"
    )
    .is_none());
}

#[test]
fn claude_is_not_norl() {
    let r = norl_shard("not-the-agent");
    // A backend outside norl's shipped code is not Norl's cognition.
    r.edit(|d| d.organism.backends[0].node = "norl.evaluations".into());
    assert!(finding(&r.assess(), "CLAUDE_IS_NOT_NORL", "plan.rules").is_some());
    // Nor is a borrowed weight that is the coding agent.
    r.edit(|d| {
        d.organism.backends[0].node = "cognition".into();
        d.organism.backends[0].kind = BackendKind::BorrowedWeight;
        d.organism.backends[0].donor = Some("assistant".into());
        d.donors = vec![Donor {
            key: "assistant".into(),
            name: "assistant".into(),
            origin: "https://api.anthropic.com".into(),
            license: "proprietary".into(),
            claimed: DonorState::Registered,
            exception: None,
            packages: vec![],
            source_paths: vec![],
            capabilities: vec![],
            cutover: None,
            provenance: vec![],
        }];
    });
    let a = r.assess();
    let f = finding(&a, "CLAUDE_IS_NOT_NORL", "plan.rules").unwrap();
    assert!(f.detail.contains("anthropic"), "{}", f.detail);
    // A borrowed weight must name its donor.
    r.edit(|d| {
        d.organism.backends[0].donor = None;
        d.donors.clear();
    });
    let a = r.assess();
    assert!(finding(&a, "BORROWED_WITHOUT_DONOR", "plan.rules").is_some());
    assert!(finding(&a, "CLAUDE_IS_NOT_NORL", "plan.rules").is_none());
}

#[test]
fn missing_proofs_are_errors() {
    let r = norl_shard("missing-proof");
    // An evaluation without a proof binding proves nothing, and cannot judge Norl.
    r.edit(|d| d.repository.edges.retain(|e| e.kind != EdgeKind::Verifies));
    let a = r.assess();
    assert!(finding(&a, "MISSING_PROOF", "eval.plan").is_some());
    assert!(finding(&a, "INVALID_NORL_FEED", "eval.plan").is_some());
    // An organism capability claimed EVALUATED needs evaluations.
    let r = norl_shard("missing-evaluation");
    r.edit(|d| {
        d.organism.capabilities[0].claimed = GrowthState::Evaluated;
        d.organism.capabilities[0].evaluations.clear();
    });
    assert!(finding(&r.assess(), "MISSING_PROOF", "cognition.plan").is_some());
    // A technology claimed PROVEN needs proofs.
    let t = extinct_baseline("missing-tech-proof");
    t.write(
        "core/src/digest.rs",
        "pub fn digest(b: &[u8]) -> u64 { 0 }\n",
    );
    t.edit(|d| {
        d.repository.nodes[0].provides = vec!["identity.digest".into()];
        d.technologies = vec![Technology {
            key: "hash.digest".into(),
            name: "digest".into(),
            kind: TechnologyKind::Algorithm,
            claimed: TechnologyLifecycle::Proven,
            purpose: String::new(),
            implements: vec!["identity.digest".into()],
            node: "core".into(),
            sources: vec!["core/src/digest.rs".into()],
            invariants: vec![],
            proofs: vec![],
            lineage: vec![],
            relations: vec![],
            norl: NorlRelevance::NotRelevant("plumbing".into()),
            claims: vec![],
        }];
    });
    let a = t.assess();
    assert!(finding(&a, "MISSING_PROOF", "hash.digest").is_some());
    assert!(finding(&a, "TECHNOLOGY_CLAIM_EXCEEDS_EVIDENCE", "hash.digest").is_some());
    // Claiming NATIVE for code that is not native is the refinement of that false claim.
    t.edit(|d| {
        d.technologies[0].claimed = TechnologyLifecycle::Native;
        d.technologies[0].sources = vec!["core/src/missing.rs".into()];
    });
    assert!(finding(&t.assess(), "NATIVE_CLAIM_WITHOUT_EVIDENCE", "hash.digest").is_some());
}

#[test]
fn false_donor_claims_are_refined() {
    let r = extinct_baseline("refined");
    r.remove(".ynventa/evidence");
    let a = r.assess();
    assert!(finding(&a, "CLAIM_EXCEEDS_EVIDENCE", "geo").is_some());
    assert!(finding(&a, "EXTINCTION_WITHOUT_PARITY", "geo").is_some());
    assert!(finding(&a, "NATIVE_CLAIM_WITHOUT_EVIDENCE", "geo").is_none());
    r.remove("substrate/geo");
    let a = r.assess();
    assert!(finding(&a, "NATIVE_CLAIM_WITHOUT_EVIDENCE", "geo").is_some());
}

#[test]
fn native_replacements_carry_their_lineage() {
    let r = extinct_baseline("lineage");
    assert!(finding(&r.assess(), "DONOR_WITHOUT_LINEAGE", "geo").is_none());
    r.edit(|d| d.repository.nodes[1].lineage.clear());
    r.prove();
    let a = r.assess();
    let f = finding(&a, "DONOR_WITHOUT_LINEAGE", "geo").unwrap();
    assert!(f.detail.contains("geo"), "{}", f.detail);
    // A technology on the replacement that learned from the donor carries it too.
    r.write(
        "substrate/geo/src/dist.rs",
        "pub fn d(a: f64) -> f64 { a }\n",
    );
    r.edit(|d| {
        d.technologies = vec![Technology {
            key: "geometry.distance".into(),
            name: "distance".into(),
            kind: TechnologyKind::Algorithm,
            claimed: TechnologyLifecycle::Experimental,
            purpose: String::new(),
            implements: vec!["distance".into()],
            node: "geo".into(),
            sources: vec!["substrate/geo/src/dist.rs".into()],
            invariants: vec![],
            proofs: vec![],
            lineage: vec!["geo".into()],
            relations: vec![],
            norl: NorlRelevance::NotRelevant("geometry".into()),
            claims: vec![],
        }]
    });
    assert!(finding(&r.assess(), "DONOR_WITHOUT_LINEAGE", "geo").is_none());
}

#[test]
fn experiences_and_evaluations_are_generated_in_their_shard() {
    let r = world_shard("generated");
    assert!(codes(&r.assess()).is_empty(), "{:#?}", r.assess().findings);
    r.edit(|d| d.repository.edges.retain(|e| e.to != "sim.episodes"));
    let a = r.assess();
    assert!(finding(&a, "INVALID_NORL_FEED", "sim.episodes").is_some());
    // A world needs no generator locally (a feed of it does, at link).
    assert!(finding(&a, "INVALID_NORL_FEED", "sim.world").is_none());
}

#[test]
fn orphans_are_warned() {
    let r = extinct_baseline("orphans");
    r.edit(|d| d.repository.nodes[0].provides = vec!["identity.digest".into()]);
    let a = r.assess();
    let f = finding(&a, "ORPHAN_CAPABILITY", "identity.digest").unwrap();
    assert_eq!(f.severity, ynventa::Severity::Warning);
    assert!(f.detail.contains("core@mechatron"));
    // A provider that generates material places the capability in the material graph.
    r.edit(|d| {
        d.repository
            .nodes
            .push(material("core.traces", Concept::Observation));
        d.repository.edges = vec![edge("core", "core.traces", EdgeKind::Generates)];
    });
    assert!(finding(&r.assess(), "ORPHAN_CAPABILITY", "identity.digest").is_none());
    // A technology must resolve its relevance to Norl.
    r.write(
        "core/src/digest.rs",
        "pub fn digest(b: &[u8]) -> u64 { 0 }\n",
    );
    r.edit(|d| {
        d.technologies = vec![Technology {
            key: "hash.digest".into(),
            name: "digest".into(),
            kind: TechnologyKind::Algorithm,
            claimed: TechnologyLifecycle::Experimental,
            purpose: String::new(),
            implements: vec!["identity.digest".into()],
            node: "core".into(),
            sources: vec!["core/src/digest.rs".into()],
            invariants: vec![],
            proofs: vec![],
            lineage: vec![],
            relations: vec![],
            norl: NorlRelevance::Unresolved,
            claims: vec![],
        }]
    });
    assert!(finding(&r.assess(), "ORPHAN_TECHNOLOGY", "hash.digest").is_some());
    r.edit(|d| d.technologies[0].norl = NorlRelevance::NotRelevant("plumbing".into()));
    assert!(finding(&r.assess(), "ORPHAN_TECHNOLOGY", "hash.digest").is_none());
    r.edit(|d| d.technologies[0].norl = NorlRelevance::Feeds("memory.recall".into()));
    assert!(finding(&r.assess(), "ORPHAN_TECHNOLOGY", "hash.digest").is_none());
    r.edit(|d| d.technologies[0].implements.clear());
    assert!(finding(&r.assess(), "ORPHAN_TECHNOLOGY", "hash.digest").is_some());
}

// ---------------------------------------------------------------------------------------------
// The linker.

#[test]
fn feeds_resolve_into_norl_across_shards() {
    let norl = norl_shard("feed-norl");
    let world = world_shard("feed-world");
    // Without the organism, the feed names nothing.
    let image = link(&[&world]);
    let i = issue(&image, "UNRESOLVED_NORL_NODE").unwrap();
    assert_eq!(i.subject, "sim.episodes");
    // With it, the feed resolves.
    let image = link(&[&world, &norl]);
    for code in [
        "UNRESOLVED_NORL_NODE",
        "INVALID_NORL_FEED",
        "INVALID_CROSS_REPO_EDGE",
    ] {
        assert!(issue(&image, code).is_none(), "{code}: {:#?}", image.issues);
    }
    // A feed into a capability that is not the organism's does not resolve into Norl.
    world.edit(|d| {
        d.repository.nodes[0].provides = vec!["identity.digest".into()];
        d.repository.edges.push(edge(
            "sim.episodes",
            "capability/identity.digest",
            EdgeKind::Feeds,
        ));
    });
    let image = link(&[&world, &norl]);
    let i = issue(&image, "UNRESOLVED_NORL_NODE").unwrap();
    assert!(
        i.detail.contains("capability/identity.digest"),
        "{}",
        i.detail
    );
    // A donor capability that claims to feed a Norl capability must name a real one.
    let world = world_shard("feed-donor");
    world.edit(|d| {
        d.donors[0].capabilities[0].norl = NorlRelevance::Feeds("cognition.dream".into())
    });
    let image = link(&[&world, &norl]);
    assert!(
        issue(&image, "UNRESOLVED_NORL_NODE").is_some(),
        "{:#?}",
        image.issues
    );
    world
        .edit(|d| d.donors[0].capabilities[0].norl = NorlRelevance::Feeds("cognition.plan".into()));
    assert!(issue(&link(&[&world, &norl]), "UNRESOLVED_NORL_NODE").is_none());
}

#[test]
fn a_feed_whose_material_nothing_generates_does_not_resolve() {
    let norl = norl_shard("chain-norl");
    let world = world_shard("chain-world");
    world.edit(|d| {
        d.repository.edges.retain(|e| e.to != "sim.world");
        d.repository
            .edges
            .push(edge("sim.world", "organ.cognition", EdgeKind::Feeds));
    });
    let image = link(&[&world, &norl]);
    let i = image
        .issues
        .iter()
        .find(|i| i.code == "INVALID_NORL_FEED" && i.subject == "sim.world")
        .unwrap();
    assert!(i.detail.contains("GENERATES"), "{}", i.detail);
    // No flag can stand in for the chain: only GENERATES resolves it.
    world.edit(|d| {
        d.repository
            .edges
            .push(edge("geo", "sim.world", EdgeKind::Generates))
    });
    assert!(issue(&link(&[&world, &norl]), "INVALID_NORL_FEED").is_none());
}

#[test]
fn shards_relate_only_through_allowed_edges() {
    let norl = norl_shard("cross-norl");
    let world = world_shard("cross-world");
    world.edit(|d| {
        d.repository
            .edges
            .push(edge("geo", "cognition", EdgeKind::Controls));
        d.repository
            .edges
            .push(edge("geo", "eval.plan", EdgeKind::Generates));
    });
    let image = link(&[&world, &norl]);
    let bad: Vec<&str> = image
        .issues
        .iter()
        .filter(|i| i.code == "INVALID_CROSS_REPO_EDGE")
        .map(|i| i.detail.as_str())
        .collect();
    assert_eq!(bad.len(), 2, "{bad:#?}");
    assert!(bad.iter().any(|d| d.contains("CONTROLS cognition")));
    assert!(bad.iter().any(|d| d.contains("GENERATES eval.plan")));
    // Developmental relations, lineage and architectural dependencies may cross.
    world.edit(|d| {
        d.repository.edges.retain(|e| {
            e.kind == EdgeKind::Generates && e.to != "eval.plan"
                || e.kind != EdgeKind::Controls && e.kind != EdgeKind::Generates
        });
        d.repository.edges.push(Edge {
            from: "geo".into(),
            to: "cognition".into(),
            kind: EdgeKind::DependsOn,
            scope: Scope::Architectural,
        });
        d.repository
            .edges
            .push(edge("sim.episodes", "eval.plan", EdgeKind::Uses));
    });
    let image = link(&[&world, &norl]);
    assert!(
        issue(&image, "INVALID_CROSS_REPO_EDGE").is_none(),
        "{:#?}",
        image.issues
    );
    assert!(issue(&image, "CROSS_SHARD_CODE_DEPENDENCY").is_none());
}

#[test]
fn schema_forks_and_skewed_subsystems_are_told_apart() {
    let a = world_shard("fork-a");
    let b = norl_shard("fork-b");
    let mut caps = vec![
        ynventa::capsule::Capsule::compile(&a.assess()),
        ynventa::capsule::Capsule::compile(&b.assess()),
    ];
    assert!(issue(&ynventa::linker::link(&caps), "SUBSYSTEM_SKEW").is_none());
    caps[1].subsystem = "sha256:skewed".into();
    let image = ynventa::linker::link(&caps);
    let skew = issue(&image, "SUBSYSTEM_SKEW").unwrap();
    assert_eq!(skew.severity, ynventa::Severity::Warning);
    assert!(issue(&image, "SCHEMA_FORK").is_none());
    caps[1].schema = "sha256:fork".into();
    let image = ynventa::linker::link(&caps);
    assert_eq!(issue(&image, "SCHEMA_FORK").unwrap().subject, "norl");
    assert!(issue(&image, "PROTOCOL_MISMATCH").is_none());
    caps[1].protocol = 2;
    assert!(issue(&ynventa::linker::link(&caps), "PROTOCOL_MISMATCH").is_some());
}

#[test]
fn donor_mappings_resolve_in_the_canonical_graph() {
    let world = world_shard("maps");
    world.edit(|d| {
        d.donors[0].capabilities[0].maps_to = Some("technology/geometry.distance".into())
    });
    let image = link(&[&world]);
    let i = image
        .issues
        .iter()
        .find(|i| i.code == "UNRESOLVED_REFERENCE" && i.subject == "geo")
        .unwrap();
    assert!(i.detail.contains("technology/geometry.distance"));
    world.edit(|d| {
        d.repository.nodes[1].provides = vec!["distance".into()];
        d.donors[0].capabilities[0].maps_to = Some("capability/distance".into());
    });
    let image = link(&[&world]);
    assert!(!image
        .issues
        .iter()
        .any(|i| i.code == "UNRESOLVED_REFERENCE" && i.subject == "geo"));
}

#[test]
fn link_reports_the_structure_of_every_shard() {
    let norl = norl_shard("structure-norl");
    let world = world_shard("structure-world");
    let image = link(&[&world, &norl]);
    assert_eq!(image.structure.len(), 2);
    assert!(
        image.structure.iter().all(|(_, v)| v.is_empty()),
        "{:#?}",
        image.structure
    );
    // Norl is held to the same shape: a non-canonical code root is a violation.
    norl.write("brain/src/lib.rs", "pub fn think() {}\n");
    norl.remove(".ynventa/declared/organism.rs");
    norl.write(
        ".ynventa/declared/organism.rs",
        "Organism { capabilities: &[], backends: &[], promotions: &[] }\n",
    );
    let a = norl.assess();
    let check = ynventa::conformance::protocol_checks(&a, None)
        .into_iter()
        .find(|c| c.id == "structure.compatible_root")
        .unwrap();
    assert!(
        !check.pass && check.detail.contains("ILLEGAL_ROOT brain"),
        "{}",
        check.detail
    );
    let image = link(&[&world, &norl]);
    let i = issue(&image, "STRUCTURE_NONCONFORMANT").unwrap();
    assert_eq!(i.subject, "norl");
    assert!(
        image
            .render_text()
            .contains("norl                 1 violations"),
        "{}",
        image.render_text()
    );
}

#[test]
fn the_eight_shard_system_links_completely() {
    let mut repos = Vec::new();
    for s in ynventa::protocol::SHARDS {
        let r = if s.id == "norl" {
            norl_shard("eight-norl")
        } else {
            let r = Repo::new(&format!("eight-{}", s.id));
            r.write("README.md", "# shard\n");
            let ynventa_key = if s.id == "ynventa" {
                "ynventa".to_string()
            } else {
                format!("ynventa.{}", s.id)
            };
            r.store(&Declaration {
                repository: Repository {
                    system: "chronica".into(),
                    shard: s.id.into(),
                    name: s.id.into(),
                    origin: s.origin.into(),
                    nodes: vec![node(
                        &ynventa_key,
                        NodeKind::Ynventa,
                        ".ynventa",
                        ".ynventa",
                    )],
                    edges: vec![],
                },
                ..Declaration::default()
            });
            r
        };
        repos.push(r);
    }
    assert_eq!(repos.len(), 8);
    let image = link(&repos.iter().collect::<Vec<_>>());
    assert!(
        issue(&image, "INCOMPLETE_SYSTEM").is_none(),
        "{:#?}",
        image.issues
    );
    assert!(image.pass(), "{:#?}", image.issues);
    assert!(image.render_text().contains("linked shards 8/8"));
    // Seven product shards: a missing one is reported by name.
    let image = link(&repos[..7].iter().collect::<Vec<_>>());
    assert!(issue(&image, "INCOMPLETE_SYSTEM")
        .unwrap()
        .detail
        .contains("norl"));
}

// ---------------------------------------------------------------------------------------------
// Growth, metrics and the query surface.

struct Fail;

impl ynventa::evidence::Runner for Fail {
    fn run(
        &mut self,
        _: &std::path::Path,
        _: &ynventa::repository::files::Files,
        p: &Proof,
    ) -> (bool, String) {
        (false, format!("stub failing run of {}", p.locator))
    }
}

fn growth_of(report: &ynventa::organism::Report, key: &str) -> ynventa::organism::Growth {
    report.growth.iter().find(|g| g.key == key).unwrap().clone()
}

fn image_growth(image: &ynventa::linker::SystemImage) -> ynventa::organism::Growth {
    growth_of(&image.organism_report(), "cognition.plan")
}

#[test]
fn evaluations_are_proven_like_capabilities() {
    let norl = norl_shard("eval-proof");
    let a = norl.assess();
    // The binding is a proof node and an evidence locator on the material.
    let m = a.graph.node_by_key("chronica", "eval.plan").unwrap();
    assert_eq!(
        m.evidence,
        vec!["tests/tests/eval.rs::plans_reach_the_goal".to_string()]
    );
    assert_eq!(
        a.evaluations["eval.plan"][0].1,
        ynventa::evidence::Verdict::Unrecorded
    );
    // `prove` (evidence::prove) records it against what generates the evaluation and the
    // cognition it evaluates.
    norl.prove();
    let a = norl.assess();
    assert_eq!(
        a.evaluations["eval.plan"][0].1,
        ynventa::evidence::Verdict::Pass
    );
    let files = ynventa::repository::files::Files::scan(norl.path()).unwrap();
    let v = ynventa::evidence::verdicts(&files, &a.declaration, &a.evidence);
    assert_eq!(
        v[&(
            "material/eval.plan".to_string(),
            "tests/tests/eval.rs::plans_reach_the_goal".to_string()
        )],
        ynventa::evidence::Verdict::Pass
    );
    // Changing the evaluated cognition makes the evidence stale.
    norl.write(
        "domain/cognition/src/lib.rs",
        "pub fn plan(goal: Goal) -> Plan { unimplemented!() }\n",
    );
    assert_eq!(
        norl.assess().evaluations["eval.plan"][0].1,
        ynventa::evidence::Verdict::Stale
    );
}

#[test]
fn growth_is_computed_from_the_linked_graph_and_evidence() {
    let norl = norl_shard("growth-norl");
    norl.prove();
    // Alone, Norl is DEFINED and EXPOSED (its backend implements the capability), never
    // EXPERIENCED: experiences come from the shards that feed it.
    let local = growth_of(&norl.assess().organism, "cognition.plan");
    assert!(local.defined);
    assert_eq!(
        local.effective,
        GrowthState::Exposed,
        "{}",
        local.stopped_by
    );
    assert!(local.stopped_by.starts_with("EXPERIENCED"));
    // Linked with the world that feeds it, it is experienced, evaluated, and — deterministic,
    // native, no donor in its path — NATIVE.
    let world = world_shard("growth-world");
    let image = link(&[&world, &norl]);
    let g = image_growth(&image);
    assert_eq!(g.experiences, vec!["sim.episodes@mechatron".to_string()]);
    assert_eq!(g.effective, GrowthState::Native, "{}", g.stopped_by);
    assert!(!g.borrowed && !g.regressed);
    // A native tiny weight is LEARNED, and NATIVE only without a donor in its path.
    norl.edit(|d| {
        d.organism.backends[0].kind = BackendKind::NativeTinyWeight;
        d.organism.backends[0].weight = "weights/plan-v1".into();
        d.organism.backends[0].donor = Some("teacher".into());
        d.donors = vec![Donor {
            key: "teacher".into(),
            name: "teacher".into(),
            origin: "https://example.org/teacher".into(),
            license: "MIT".into(),
            claimed: DonorState::Registered,
            exception: None,
            packages: vec![],
            source_paths: vec![],
            capabilities: vec![],
            cutover: None,
            provenance: vec![],
        }];
    });
    let g = image_growth(&link(&[&world, &norl]));
    assert_eq!(g.effective, GrowthState::Learned, "{}", g.stopped_by);
    // A borrowed weight never grows past EVALUATED, and is reported BORROWED.
    norl.edit(|d| d.organism.backends[0].kind = BackendKind::BorrowedWeight);
    let g = image_growth(&link(&[&world, &norl]));
    assert_eq!(g.effective, GrowthState::Evaluated);
    assert!(g.borrowed);
    assert!(g.stopped_by.contains("borrowed"), "{}", g.stopped_by);
}

#[test]
fn claims_above_growth_are_false_and_regressions_are_reported() {
    let norl = norl_shard("claims-norl");
    let world = world_shard("claims-world");
    norl.prove();
    norl.edit(|d| d.organism.capabilities[0].claimed = GrowthState::Native);
    // Alone, the shortfall may still be met by other shards' material: a warning here...
    let a = norl.assess();
    let f = finding(&a, "CLAIM_EXCEEDS_EVIDENCE", "cognition.plan").unwrap();
    assert_eq!(f.severity, ynventa::Severity::Warning);
    assert!(f.detail.contains("judged at link"));
    // ...and decided by the linker: without the world the claim is false.
    let image = link(&[&norl]);
    let codes: Vec<(&str, ynventa::Severity)> = image
        .issues
        .iter()
        .filter(|i| i.subject == "cognition.plan")
        .map(|i| (i.code.as_str(), i.severity))
        .collect();
    for c in [
        "CLAIM_EXCEEDS_EVIDENCE",
        "NORL_CAPABILITY_WITHOUT_EVALUATION",
        "NATIVE_CLAIM_WITHOUT_EVIDENCE",
    ] {
        assert!(
            codes.contains(&(c, ynventa::Severity::Error)),
            "{c}: {codes:?}"
        );
    }
    let image = link(&[&world, &norl]);
    assert!(
        !image.issues.iter().any(|i| i.subject == "cognition.plan"),
        "{:#?}",
        image.issues
    );
    // A failing run of the same bytes is a regression: the capability falls back below
    // EVALUATED and the claim is false again.
    let files = ynventa::repository::files::Files::scan(norl.path()).unwrap();
    ynventa::evidence::prove(
        norl.path(),
        &files,
        &norl.declaration(),
        Some("eval.plan"),
        &mut Fail,
    );
    let image = link(&[&world, &norl]);
    let g = image_growth(&image);
    assert!(g.regressed);
    assert_eq!(g.effective, GrowthState::Experienced);
    let i = image
        .issues
        .iter()
        .find(|i| i.code == "NORL_CAPABILITY_WITHOUT_EVALUATION")
        .unwrap();
    assert!(i.detail.contains("REGRESSED"), "{}", i.detail);
    // A capability whose organ is missing is not even DEFINED.
    norl.edit(|d| d.organism.capabilities[0].organ = "organ.dreams".into());
    let g = growth_of(&norl.assess().organism, "cognition.plan");
    assert!(!g.defined);
    assert!(finding(&norl.assess(), "UNRESOLVED_NORL_NODE", "cognition.plan").is_some());
}

#[test]
fn nutrition_metrics_are_counted_where_they_are_decided() {
    let norl = norl_shard("nutrition-norl");
    let world = world_shard("nutrition-world");
    norl.prove();
    let get = |c: &ynventa::metrics::Counts, k: &str| {
        c.values().into_iter().find(|(n, _)| n == k).unwrap().1
    };
    // Norl alone knows its capabilities but not what feeds them.
    let local = norl.assess().counts;
    assert_eq!(get(&local, "organism_capabilities"), "1");
    assert_eq!(get(&local, "growth_exposed"), "1");
    assert_eq!(get(&local, "experience_coverage"), "0.000000");
    assert_eq!(get(&local, "native_independence"), "1.000000");
    assert_eq!(get(&local, "materials_evaluations"), "1");
    // The world shard alone knows its material, not the organism.
    let w = world.assess().counts;
    assert_eq!(get(&w, "materials_experiences"), "1");
    assert_eq!(
        get(&w, "materials_worlds"),
        "1",
        "the world its experiences happen in"
    );
    assert_eq!(get(&w, "experience_coverage"), "undefined");
    // The linked system decides.
    let image = link(&[&world, &norl]);
    let c = image.counts();
    assert_eq!(get(&c, "growth_native"), "1");
    assert_eq!(get(&c, "experience_coverage"), "1.000000");
    assert_eq!(get(&c, "evaluation_coverage"), "1.000000");
    assert_eq!(get(&c, "materials_experiences"), "1");
    assert_eq!(get(&c, "organism_borrowed"), "0");
    // `metrics --aggregate` is the linked judgement.
    let (code, out) = world.cli(&["metrics", "--aggregate", &norl.path().display().to_string()]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("growth_native 1\n"), "{out}");
    assert!(out.contains("experience_coverage 1.000000\n"), "{out}");
    let (_, json) = world.cli(&["metrics", "--json"]);
    assert!(json.contains("\"experience_coverage\": null"), "{json}");
}

#[test]
fn the_organism_answers_what_feeds_it_and_what_changed() {
    let norl = norl_shard("query-norl");
    let world = world_shard("query-world");
    norl.prove();
    let before = norl.path().join("target/before.ynv").display().to_string();
    let after = norl.path().join("target/after.ynv").display().to_string();
    let root = |r: &Repo| r.path().display().to_string();
    let (_, out) = norl.cli(&["link", &root(&norl), "--out", &before]);
    assert!(out.contains("system link"), "{out}");
    norl.cli(&["link", &root(&world), &root(&norl), "--out", &after]);
    let (code, out) = norl.cli(&["organism", "--system", &after]);
    assert_eq!(code, 0, "{out}");
    for needle in [
        "native digital organism",
        "organ.cognition",
        "conceptual organs not yet declared: Memory, WorldModel",
        "cognition.plan",
        "-> NATIVE",
        "capability/cognition.plan <-FEEDS- sim.episodes [MATERIAL EXPERIENCE] from mechatron <- generated by geo@mechatron",
        "cognition.plan judged by eval.plan@norl PASS",
        "*sim.world@mechatron",
        "mechatron            world 1, observation 0, action 0, experience 1",
        "DONORS",
        "extinct: 1: mechatron:geo (EXTINCT)",
    ] {
        assert!(out.contains(needle), "{needle}\n{out}");
    }
    let (_, json) = norl.cli(&["organism", "--system", &after, "--json"]);
    assert!(json.contains("\"effective\": \"NATIVE\""), "{json}");
    // What did the change teach Norl?
    let (_, diff) = norl.cli(&["organism", "--system", &after, "--against", &before]);
    for needle in [
        "+ material sim.episodes@mechatron [EXPERIENCE] reaches Norl",
        "+ feed sim.episodes -FEEDS-> capability/cognition.plan (from mechatron)",
        "~ capability cognition.plan EXPOSED -> NATIVE",
    ] {
        assert!(diff.contains(needle), "{needle}\n{diff}");
    }
    // Locally, every shard sees its own contribution.
    let (_, ctx) = world.cli(&["context"]);
    assert!(ctx.contains("NORL CONTRIBUTION"), "{ctx}");
    assert!(ctx.contains("experience 1/1"), "{ctx}");
    assert!(
        ctx.contains("feeds: 1: sim.episodes -FEEDS-> capability/cognition.plan"),
        "{ctx}"
    );
    let (_, ctx) = norl.cli(&["context", "--system", &after]);
    assert!(ctx.contains("organism: 1 organs, 1 capabilities"), "{ctx}");
    assert!(ctx.contains("native 1"), "{ctx}");
    // The command is part of the protocol.
    assert!(ynventa::protocol::COMMANDS
        .iter()
        .any(|(c, _)| *c == "organism"));
}

#[test]
fn technology_output_answers_owner_implementations_lineage_and_feeds() {
    let norl = norl_shard("tech-norl");
    let world = world_shard("tech-world");
    world.write("substrate/geo/src/dist.rs", "pub fn d(a: f64) -> f64 { a }\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn d_is_identity() {}\n}\n");
    world.edit(|d| {
        d.technologies = vec![Technology {
            key: "geometry.distance".into(),
            name: "planar distance".into(),
            kind: TechnologyKind::Algorithm,
            claimed: TechnologyLifecycle::Canonical,
            purpose: "distances in the simulated world".into(),
            implements: vec!["distance".into()],
            node: "geo".into(),
            sources: vec!["substrate/geo/src/dist.rs".into()],
            invariants: vec![],
            proofs: vec![proof(
                ProofKind::Regression,
                "substrate/geo/src/dist.rs::d_is_identity",
            )],
            lineage: vec!["geo".into()],
            relations: vec![],
            norl: NorlRelevance::Feeds("cognition.plan".into()),
            claims: vec![],
        }];
    });
    world.prove();
    let sys = norl.path().join("target/sys.ynv").display().to_string();
    let root = |r: &Repo| r.path().display().to_string();
    norl.cli(&["link", &root(&world), &root(&norl), "--out", &sys]);
    let (_, out) = norl.cli(&["technology", "list", "--system", &sys]);
    for needle in [
        "geometry.distance — planar distance (ALGORITHM, CANONICAL)",
        "born in mechatron (its owner) node geo",
        "lineage geo (EXTINCT)",
        "proofs substrate/geo/src/dist.rs::d_is_identity",
        "feeds Norl capability/cognition.plan",
        "donor-dependent: no",
    ] {
        assert!(out.contains(needle), "{needle}\n{out}");
    }
    let (_, json) = norl.cli(&["technology", "list", "--system", &sys, "--json"]);
    assert!(json.contains("\"born_in\": \"mechatron\""), "{json}");
    let (_, local) = world.cli(&["technology", "list"]);
    assert!(local.contains("norl FEEDS(cognition.plan)"), "{local}");
}

/// Another shard's evaluation proves that shard, never Norl: a Mechatron benchmark that
/// passes leaves Norl EXPERIENCED, not EVALUATED, until a norl-owned evaluation judges Norl.
#[test]
fn another_shards_evaluation_is_a_benchmark_not_norls_evidence() {
    let world = world_shard("bench-world");
    world.write(
        "tests/Cargo.toml",
        "[package]\nname = \"bench\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    world.write("tests/src/lib.rs", "");
    world.write(
        "tests/tests/bench.rs",
        "#[test]\nfn the_world_recovers() {}\n",
    );
    world.edit(|d| {
        d.repository
            .nodes
            .push(material("sim.benchmark", Concept::Evaluation));
        d.repository
            .edges
            .push(edge("geo", "sim.benchmark", EdgeKind::Generates));
        d.repository.edges.push(edge(
            "proof:tests/tests/bench.rs::the_world_recovers",
            "sim.benchmark",
            EdgeKind::Verifies,
        ));
    });
    world.prove();
    let norl = norl_shard("bench-norl");
    norl.edit(|d| {
        d.organism.capabilities[0].evaluations = vec!["sim.benchmark".into()];
        d.organism.capabilities[0].claimed = GrowthState::Defined;
    });
    let image = link(&[&world, &norl]);
    let g = image_growth(&image);
    assert!(
        g.evaluations
            .iter()
            .any(|(k, v)| k == "sim.benchmark@mechatron" && *v == ynventa::evidence::Verdict::Pass),
        "{:?}",
        g.evaluations
    );
    assert_eq!(g.effective, GrowthState::Experienced, "{}", g.stopped_by);
    assert!(g.stopped_by.contains("benchmark"), "{}", g.stopped_by);
}

// ---------------------------------------------------------------------------------------------
// Promotion: a source repository's proof is the first rung of a promotion into Norl, and never
// Norl's growth (SOURCE_REPO_CAN_DO(X) != NORL_CAN_DO(X)).

/// Mechatron with a proven technology: `geo.distance` in `geo`, which provides `distance`.
fn source_shard(name: &str) -> Repo {
    let r = extinct_baseline(name);
    r.edit(|d| {
        d.repository
            .nodes
            .iter_mut()
            .find(|n| n.key == "geo")
            .unwrap()
            .provides = vec!["distance".into()];
        d.technologies = vec![Technology {
            key: "geo.distance".into(),
            name: "Euclidean distance".into(),
            kind: TechnologyKind::Algorithm,
            claimed: TechnologyLifecycle::Native,
            purpose: "metric distance between points".into(),
            implements: vec!["distance".into()],
            node: "geo".into(),
            sources: vec!["substrate/geo/src/lib.rs".into()],
            invariants: vec![],
            proofs: vec![proof(
                ProofKind::Regression,
                "substrate/geo/src/lib.rs::regression_distance",
            )],
            lineage: vec![],
            relations: vec![],
            norl: NorlRelevance::NotRelevant("a geometry primitive".into()),
            claims: vec![],
        }];
    });
    r
}

fn promotion(claimed: PromotionState) -> Promotion {
    Promotion {
        key: "plan.distance".into(),
        source_repository: "mechatron".into(),
        source_node: "geo".into(),
        source_capability: Some("distance".into()),
        source_technology: Some("geo.distance".into()),
        source_commit: "0123456789abcdef0123456789abcdef01234567".into(),
        source_evidence: vec!["substrate/geo/src/lib.rs::regression_distance".into()],
        destination_capability: "cognition.plan".into(),
        destination_node: None,
        relation: EdgeKind::Feeds,
        implementation: PromotionImplementation::IndependentImplementation,
        claimed,
        reason: "planning needs a metric between places".into(),
        transformation: "re-derived as an admissible heuristic over Norl's own world model".into(),
    }
}

fn promotion_of(report: &ynventa::organism::Report) -> ynventa::organism::PromotionGrowth {
    report.promotions[0].clone()
}

#[test]
fn the_promotion_ladder_is_computed() {
    let norl = norl_shard("promote-norl");
    norl.edit(|d| d.organism.promotions = vec![promotion(PromotionState::PromotionDesigned)]);
    // Alone, the source's proof is judged at link; nothing else is missing.
    let a = norl.assess();
    let p = promotion_of(&a.organism);
    assert_eq!(p.source_proven, None);
    assert_eq!(p.effective, Some(PromotionState::PromotionDesigned));
    assert!(codes(&a).is_empty(), "{:?}", codes(&a));
    // The promotion is a feed of the source technology into the organism capability.
    let feed = a.graph.edges.iter().any(|e| {
        e.kind == EdgeKind::Feeds
            && e.from == ynventa::graph::NodeId::of("chronica", "technology/geo.distance")
            && e.to == ynventa::graph::NodeId::of("chronica", "capability/cognition.plan")
    });
    assert!(feed);

    // At link, an unproven source stops it at nothing: the claim is an error.
    let world = source_shard("promote-world");
    let image = link(&[&world, &norl]);
    let p = promotion_of(&image.organism_report());
    assert_eq!(p.source_proven, Some(false));
    assert_eq!(p.effective, None);
    assert!(
        p.stopped_by.starts_with("PROVEN_IN_SOURCE"),
        "{}",
        p.stopped_by
    );
    assert!(issue(&image, "PROMOTION_CLAIM_EXCEEDS_EVIDENCE").is_some());
    // An unlinked source shard proves nothing either.
    let image = link(&[&norl]);
    assert!(promotion_of(&image.organism_report())
        .stopped_by
        .contains("not linked"));

    // Fresh passing source evidence: PROVEN_IN_SOURCE .. PROMOTION_DESIGNED.
    world.prove();
    let image = link(&[&world, &norl]);
    let p = promotion_of(&image.organism_report());
    assert_eq!(p.source_proven, Some(true));
    assert_eq!(p.effective, Some(PromotionState::PromotionDesigned));
    assert!(issue(&image, "PROMOTION_CLAIM_EXCEEDS_EVIDENCE").is_none());
    assert!(p.stopped_by.starts_with("NATIVE_NORL_IMPLEMENTATION"));

    // A claim above it is an error, here and alone.
    norl.edit(|d| d.organism.promotions[0].claimed = PromotionState::NativeNorlImplementation);
    assert!(finding(
        &norl.assess(),
        "PROMOTION_CLAIM_EXCEEDS_EVIDENCE",
        "plan.distance"
    )
    .is_some());
    // A Norl destination node that exists: NATIVE_NORL_IMPLEMENTATION.
    norl.edit(|d| d.organism.promotions[0].destination_node = Some("cognition".into()));
    let a = norl.assess();
    assert_eq!(
        promotion_of(&a.organism).effective,
        Some(PromotionState::NativeNorlImplementation)
    );
    assert!(finding(&a, "PROMOTION_CLAIM_EXCEEDS_EVIDENCE", "plan.distance").is_none());
    // REFERENCE_ONLY implements nothing in Norl.
    norl.edit(|d| d.organism.promotions[0].implementation = PromotionImplementation::ReferenceOnly);
    let p = promotion_of(&norl.assess().organism);
    assert_eq!(p.effective, Some(PromotionState::PromotionDesigned));
    norl.edit(|d| {
        d.organism.promotions[0].implementation = PromotionImplementation::IndependentImplementation
    });

    // NORL_TESTED needs a norl-owned proof in the destination node.
    norl.write(
        "domain/cognition/src/lib.rs",
        "pub fn plan(goal: Goal) -> Plan { todo!() }\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn plans() {}\n}\n",
    );
    norl.edit(|d| {
        d.technologies = vec![Technology {
            key: "plan.heuristic".into(),
            name: "planning heuristic".into(),
            kind: TechnologyKind::Algorithm,
            claimed: TechnologyLifecycle::Native,
            purpose: "plans with a metric heuristic".into(),
            implements: vec!["cognition.plan".into()],
            node: "cognition".into(),
            sources: vec!["domain/cognition/src/lib.rs".into()],
            invariants: vec![],
            proofs: vec![proof(
                ProofKind::Regression,
                "domain/cognition/src/lib.rs::plans",
            )],
            lineage: vec![],
            relations: vec![],
            norl: NorlRelevance::Feeds("cognition.plan".into()),
            claims: vec![],
        }];
        d.organism.promotions[0].claimed = PromotionState::NorlNative;
    });
    let p = promotion_of(&norl.assess().organism);
    assert_eq!(p.effective, Some(PromotionState::NativeNorlImplementation));
    assert!(p.stopped_by.starts_with("NORL_TESTED"), "{}", p.stopped_by);
    norl.prove();
    let p = promotion_of(&norl.assess().organism);
    assert_eq!(
        p.effective,
        Some(PromotionState::NorlTested),
        "{}",
        p.stopped_by
    );
    assert!(
        p.stopped_by.starts_with("NORL_EVALUATED"),
        "{}",
        p.stopped_by
    );

    // NORL_EVALUATED and NORL_NATIVE are Norl's growth: here, experiences fed by the world and
    // Norl's own evaluation.
    let world = world_shard("promote-feeding-world");
    world.edit(|d| {
        d.repository
            .nodes
            .iter_mut()
            .find(|n| n.key == "geo")
            .unwrap()
            .provides = vec!["distance".into()];
        d.technologies = source_shard("promote-tmp").declaration().technologies;
    });
    world.prove();
    let image = link(&[&world, &norl]);
    let r = image.organism_report();
    assert_eq!(image_growth(&image).effective, GrowthState::Native);
    assert_eq!(promotion_of(&r).effective, Some(PromotionState::NorlNative));
    assert!(issue(&image, "PROMOTION_CLAIM_EXCEEDS_EVIDENCE").is_none());
    // The ladder is the protocol's.
    let words: Vec<&str> = PromotionState::ALL.iter().map(|s| s.wire()).collect();
    assert_eq!(
        words,
        [
            "PROVEN_IN_SOURCE",
            "NORL_RELEVANCE_RESOLVED",
            "IMPORT_CANDIDATE",
            "PROMOTION_DESIGNED",
            "NATIVE_NORL_IMPLEMENTATION",
            "NORL_TESTED",
            "NORL_EVALUATED",
            "NORL_NATIVE"
        ]
    );
}

#[test]
fn source_evidence_never_raises_norl_maturity() {
    let world = source_shard("maturity-world");
    world.prove();
    let norl = norl_shard("maturity-norl");
    norl.prove();
    let without = image_growth(&link(&[&world, &norl]));
    assert_eq!(without.effective, GrowthState::Exposed);
    assert_eq!(without.source_maturity, SourceMaturity::Unfed);
    norl.edit(|d| d.organism.promotions = vec![promotion(PromotionState::PromotionDesigned)]);
    let image = link(&[&world, &norl]);
    let with = image_growth(&image);
    // The source proves it, and Norl has grown exactly as much as before.
    assert_eq!(with.source_maturity, SourceMaturity::ProvenInSource);
    assert_eq!(with.effective, without.effective);
    assert_eq!(with.stopped_by, without.stopped_by);
    let c = image.counts();
    assert_eq!(c.organism_source_proven, 1);
    assert_eq!(c.growth_exposed, 1);
    assert_eq!(c.growth_evaluated + c.growth_learned + c.growth_native, 0);
    assert_eq!(c.promotions_proven_in_source, 1);
    // Both maturities are shown: in the organism view, its JSON and the link report.
    let text = image.render_text();
    assert!(
        text.contains("norl maturity") && text.contains("source maturity"),
        "{text}"
    );
    let j = ynventa::organism::View {
        report: &image.organism_report(),
        technologies: vec![],
        sharing: &[],
        donors: vec![],
        tech_shard: Default::default(),
    };
    let json = j.to_json().render();
    assert!(json.contains("\"norl_maturity\": \"EXPOSED\""), "{json}");
    assert!(
        json.contains("\"source_maturity\": \"PROVEN_IN_SOURCE\""),
        "{json}"
    );
    assert!(j.render_text().contains("PROMOTIONS"));
    // A failing source proof leaves it FED.
    world.write(
        "substrate/geo/src/lib.rs",
        "pub fn distance(a: (f64, f64), b: (f64, f64)) -> f64 { (a.0 - b.0).abs() + (a.1 - b.1).abs() }\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn regression_distance() { assert_eq!(super::distance((0.0, 0.0), (3.0, 4.0)), 5.0); }\n}\n",
    );
    let g = image_growth(&link(&[&world, &norl]));
    assert_eq!(g.source_maturity, SourceMaturity::Fed);
    assert_eq!(g.effective, without.effective);
}

#[test]
fn code_crosses_into_norl_only_with_provenance_or_as_shared_implementation() {
    let norl = norl_shard("transfer-norl");
    // Copied code without its source technology: no provenance.
    let mut p = promotion(PromotionState::ProvenInSource);
    p.implementation = PromotionImplementation::CopiedAndDiverged;
    p.destination_node = Some("cognition".into());
    p.source_technology = None;
    norl.edit(|d| d.organism.promotions = vec![p.clone()]);
    let a = norl.assess();
    let f = finding(&a, "PROMOTION_WITHOUT_PROVENANCE", "plan.distance").unwrap();
    assert!(f.detail.contains("source_technology"), "{}", f.detail);
    // With it, the provenance is complete.
    norl.edit(|d| d.organism.promotions[0].source_technology = Some("geo.distance".into()));
    assert!(finding(
        &norl.assess(),
        "PROMOTION_WITHOUT_PROVENANCE",
        "plan.distance"
    )
    .is_none());
    // A destination node that REUSES the source technology is a SHARED_IMPLEMENTATION.
    norl.edit(|d| {
        d.repository
            .nodes
            .iter_mut()
            .find(|n| n.key == "cognition")
            .unwrap()
            .reuses = vec!["geo.distance".into()];
        d.organism.promotions[0].implementation =
            PromotionImplementation::IndependentImplementation;
    });
    assert!(finding(
        &norl.assess(),
        "PROMOTION_IMPLEMENTATION_MISMATCH",
        "plan.distance"
    )
    .is_some());
    norl.edit(|d| {
        d.organism.promotions[0].implementation = PromotionImplementation::SharedImplementation
    });
    let a = norl.assess();
    assert!(finding(&a, "PROMOTION_IMPLEMENTATION_MISMATCH", "plan.distance").is_none());
    assert_eq!(
        promotion_of(&a.organism).effective,
        Some(PromotionState::NativeNorlImplementation)
    );
    // A shared implementation that does not reuse stops before the native rung.
    norl.edit(|d| {
        d.repository
            .nodes
            .iter_mut()
            .find(|n| n.key == "cognition")
            .unwrap()
            .reuses = vec![]
    });
    let p = promotion_of(&norl.assess().organism);
    assert_eq!(p.effective, Some(PromotionState::PromotionDesigned));
    assert!(p.stopped_by.contains("REUSE"), "{}", p.stopped_by);
}

#[test]
fn promotions_are_norls_and_name_semantics_not_paths() {
    // Only norl promotes.
    let world = source_shard("promotions-outside");
    world.edit(|d| d.organism.promotions = vec![promotion(PromotionState::ProvenInSource)]);
    assert!(finding(&world.assess(), "PROMOTION_OUTSIDE_NORL", "organism").is_some());
    // A malformed promotion names what is wrong.
    let norl = norl_shard("promotions-invalid");
    let mut p = promotion(PromotionState::ProvenInSource);
    p.source_repository = "norl".into();
    p.relation = EdgeKind::DependsOn;
    p.destination_capability = "memory.recall".into();
    norl.edit(|d| d.organism.promotions = vec![p]);
    let a = norl.assess();
    let f = finding(&a, "PROMOTION_INVALID", "plan.distance").unwrap();
    for part in [
        "not another canonical shard",
        "not FEEDS or TEACHES",
        "not an organism capability",
    ] {
        assert!(f.detail.contains(part), "{}", f.detail);
    }
    // Feeds and promotions name capabilities, technologies, materials or evidence; never paths.
    let mut p = promotion(PromotionState::ProvenInSource);
    p.source_technology = Some("substrate/geo/src/lib.rs".into());
    norl.edit(|d| d.organism.promotions = vec![p]);
    assert!(finding(&norl.assess(), "FEED_NAMES_PATH", "plan.distance").is_some());
    let r = extinct_baseline("feed-path");
    r.edit(|d| d.donors[0].capabilities[0].norl = NorlRelevance::Feeds("src/brain.rs".into()));
    assert!(finding(&r.assess(), "FEED_NAMES_PATH", "geo/distance").is_some());
    assert!(ynventa::organism::names_path("domain/cognition/src/lib.rs"));
    assert!(!ynventa::organism::names_path("capability/cognition.plan"));
    assert!(!ynventa::organism::names_path("cognition.plan"));
}

#[test]
fn a_path_dependency_into_a_sibling_repository_is_an_error() {
    let norl = norl_shard("sibling-norl");
    // A path dependency inside the repository is fine...
    assert!(codes(&norl.assess()).is_empty());
    // ...one into a sibling's internal crates is not: every repository builds alone.
    norl.write(
        "domain/cognition/Cargo.toml",
        "[package]\nname = \"cognition\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nchronica-core = { path = \"../../../Chronica/core\" }\n",
    );
    let a = norl.assess();
    let f = finding(
        &a,
        "CROSS_REPO_SOURCE_DEPENDENCY",
        "domain/cognition/Cargo.toml",
    )
    .unwrap();
    assert!(f.detail.contains("Chronica"), "{}", f.detail);
    let check = ynventa::conformance::protocol_checks(&a, None)
        .into_iter()
        .find(|c| c.id == "repository.builds_alone")
        .unwrap();
    assert!(!check.pass);
}

#[test]
fn an_organism_without_promotions_is_migrated_never_accepted_silently() {
    let norl = norl_shard("grammar-promotions");
    let before = norl.assess();
    // The organism as declared before promotions existed.
    let old: String = norl
        .read(".ynventa/declared/organism.rs")
        .replace("    promotions: &[],\n", "");
    assert!(!old.contains("promotions"));
    norl.write(".ynventa/declared/organism.rs", &old);
    let err = ynventa::assess(norl.path()).err().unwrap();
    assert!(err.contains("missing field `promotions`"), "{err}");
    assert!(err.contains("migrate schema"), "{err}");
    let (code, out) = norl.cli(&["migrate", "schema"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("rewrote .ynventa/declared/organism.rs"),
        "{out}"
    );
    assert!(norl
        .read(".ynventa/declared/organism.rs")
        .contains("promotions: &[]"));
    let after = norl.assess();
    // Nothing else changes: the organism, and every node identity.
    assert_eq!(after.declaration.organism, before.declaration.organism);
    assert_eq!(ids(&after), ids(&before));
    let (_, again) = norl.cli(&["migrate", "schema"]);
    assert!(again.contains("already use the current grammar"), "{again}");
}
