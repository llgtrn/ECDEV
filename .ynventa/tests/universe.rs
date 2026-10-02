//! The donor universe: global donor identity, lossless imports of the legacy registries, the
//! repository-donor relation, raw against unique counts across the ecosystem, and legacy
//! registries that are not reconciled.

mod common;

use common::*;
use ynventa::declare::*;
use ynventa::schema::*;
use ynventa::universe::{import, Layer, SourceState};

/// A shard with no code: only declarations (and whatever registry a test writes).
fn shard(name: &str, id: &str, donors: Vec<Donor>) -> Repo {
    let r = Repo::new(name);
    r.write("README.md", "# fixture\n");
    r.store(&Declaration {
        repository: Repository {
            system: "chronica".into(),
            shard: id.into(),
            name: name.into(),
            origin: format!("llgtrn/{name}"),
            nodes: vec![node("ynventa", NodeKind::Ynventa, ".ynventa", ".ynventa")],
            edges: vec![],
        },
        donors,
        migration: Migration::default(),
        technologies: vec![],
        organism: Organism::default(),
    });
    r
}

fn declared(key: &str, origin: &str) -> Donor {
    Donor {
        key: key.into(),
        name: key.into(),
        origin: origin.into(),
        license: "MIT".into(),
        claimed: DonorState::Registered,
        exception: None,
        packages: vec![],
        source_paths: vec![],
        capabilities: vec![],
        cutover: None,
        provenance: vec!["fixture".into()],
    }
}

/// Fi-game's knowledge intake: one JSON object per line, typed by `type`, verified by
/// `verification`, cloned donors carry an `inventory`.
const FI_GAME_REGISTRY: &str = r#"{"id":"OSS-ALPHA-084","key":"github.com/hummingbot/hummingbot","name":"hummingbot","url":"https://github.com/hummingbot/hummingbot","type":"OSS_DONOR","domain":"alpha","capability":"alpha.market_making","secondary":["alpha.arbitrage"],"lifecycle":"EXTINCT","status":{"absorption":"done","census":"done","clone":"done","extinction":"done"},"inventory":"D004-hummingbot","verification":"HEAD 9af100d6822da7d2d0291a906c730ef172284ee2"}
{"id":"OSS-DATA-001","key":"github.com/serde-rs/serde","name":"serde","url":"https://github.com/serde-rs/serde","type":"OSS_DONOR","domain":"data","capability":"data.codecs","lifecycle":"REGISTERED","status":{"absorption":"pending","census":"pending","clone":"pending","extinction":"pending"},"inventory":null,"verification":"HTTP 200"}
{"id":"OSINT-INTEL-001","key":"1stheadlines.com","name":"1stHeadlines","url":"https://www.1stheadlines.com/","type":"OSINT_SOURCE","domain":"intel","capability":"intel.news_ingestion","lifecycle":"REGISTERED","status":{"not":"DONOR_TO_EXTINCT","permanence":"PERMANENT_SOURCE"},"inventory":null,"verification":"UNREACHABLE"}
{"id":"ACAD-ACCT-002","key":"doi.org/10.1080/0025570x.1985.11977191","name":"Ellerman (1985)","url":"https://doi.org/10.1080/0025570x.1985.11977191","type":"ACADEMIC_REFERENCE","domain":"acct","capability":"acct.double_entry_ledger","lifecycle":"REGISTERED","status":{"permanence":"PERMANENT_REFERENCE"},"inventory":null,"verification":"CROSSREF MATCH: The Mathematics"}
{"id":"ACAD-QUANT-141","key":"github.com/borealisai/scaleformer","name":"scaleformer","url":"https://https://github.com/BorealisAI/scaleformer","type":"ACADEMIC_REFERENCE","domain":"quant","capability":"","lifecycle":"REGISTERED","status":{"permanence":"PERMANENT_REFERENCE"},"inventory":null,"verification":"HTTP 404"}
{"id":"AUTH-DATA-001","key":"abs.gov.au","name":"ABS","url":"https://www.abs.gov.au","type":"OFFICIAL_AUTHORITY","domain":"data","capability":"data.official_statistics","lifecycle":"REGISTERED","status":{"permanence":"PERMANENT_SOURCE"},"inventory":null,"verification":"HTTP 200"}
"#;

/// Mechatron's typed registries: a header table, records whose table name differs per file,
/// multi-line arrays, empty `donor_ref`s and licences "from memory".
const MECHATRON_UNIVERSE: &str = r#"[registry]
schema_version = 1
kind = "oss-universe"
allowed_statuses = ["DISCOVERED", "REGISTERED"]


[[repo]]
id = "U0001"
name = "rclcpp"
repository = "https://github.com/ros2/rclcpp"
license = "Apache-2.0"
license_source = "LICENSE at HEAD matches (Apache-2.0); expression from memory"
teaches = "ROS 2 node/executor/callback-group model"
subsystems = ["robotics", "runtime"]
relevant_modules = [
  "rclcpp/src/rclcpp/executors",
  "rclcpp_lifecycle",
]
donor_ref = "D001"
status = "REGISTERED"
verified = true
verified_head = "eee4b508d4357c81c0847f922617e5f498d1d4b7"

[[repo]]
id = "U0002"
name = "serde"
repository = "https://github.com/Serde-RS/serde.git"
subsystems = ["storage"]
donor_ref = ""
status = "DISCOVERED"
verified = false
"#;

const MECHATRON_DONORS: &str = r#"# Mechatron donor ledger - first wave.
[ledger]
schema_version = 1
states = ["DISCOVERED", "REGISTERED", "CLONED", "CENSUSED", "MAPPED"]

[ledger.state_definitions]
MAPPED = "Concrete algorithms identified."


[[donor]]
id = "D001"
name = "rclcpp"
repository = "https://github.com/ros2/rclcpp"
revision = "eee4b508d4357c81c0847f922617e5f498d1d4b7"
license_observed = "Apache-2.0"
mechatron_subsystems = ["runtime", "robotics"]
important_algorithms = [
  "single-threaded executor spin loop: rclcpp/src/rclcpp/executors/single_threaded_executor.cpp",
]
status = "MAPPED"
"#;

const MECHATRON_PROTOCOLS: &str = r#"[registry]
kind = "protocol-specs"

[[spec]]
id = "P001"
name = "MQTT 3.1.1"
needs = "QoS 0-2 delivery semantics"
mechatron_subsystems = ["protocol", "telemetry"]
url = "https://docs.oasis-open.org/mqtt/mqtt/v3.1.1/mqtt-v3.1.1.html"
url_check = "HTTP 200 on 2026-09-30"
status = "REGISTERED"
"#;

/// Atlas-Studio's FULL_OSS_REPLAY ledger: counts and epochs between the header and the records.
const ATLAS_REPLAY: &str = r#"# FULL_OSS_REPLAY (G151): every canonical donor replayed.

schema = "atlas.full-oss-replay.v1"
lane = "FULL_OSS_REPLAY"

[counts]
total = 2

[[epoch]]
id = "E0"
commit = "db711846e54ebba591f5c5e2c8e17502becfde9b"

[[repository]]
key = "666ghj/bettafish"
remote = "https://github.com/666ghj/BettaFish.git"
names = ["BettaFish"]
license = "GPL-2.0"
historical_pins = ["c4ca6360489b53c38d3d41d213a67b50d2fcb883"]
replay_status = "PROCESSED"
pinned_commit = "c4ca6360489b53c38d3d41d213a67b50d2fcb883"
license_status = "GPL-2.0 (root LICENSE)"
terminal_state = "REFERENCE_ONLY"

[[repository]]
key = "serde-rs/serde"
remote = "https://github.com/serde-rs/serde"
names = ["serde"]
license = "MIT OR Apache-2.0"
historical_pins = []
replay_status = "NEVER_REPLAYED"
"#;

/// Universal-Connector's inventory: one JSON document, records under `donors`, no upstream (it
/// is the declaration's).
const UC_INVENTORY: &str = "{\n  \"schema\": \"uc.donor-inventory.v1\",\n  \"donors\": [\n    {\n      \"id\": \"serde\",\n      \"name\": \"serde\",\n      \"status\": \"CANDIDATE\",\n      \"target_capabilities\": [\"codecs\"]\n    },\n    {\n      \"id\": \"orphan\",\n      \"status\": \"REJECTED\"\n    }\n  ]\n}\n";

fn import(r: &Repo, from: &str) -> (i32, String) {
    r.cli(&["migrate", "universe", "--from", from])
}

#[test]
fn imports_are_lossless_and_idempotent() {
    let r = shard("fi-registry", "fi-game", vec![]);
    for (path, text) in [
        ("tools/atlas/intake/registry.jsonl", FI_GAME_REGISTRY),
        ("research/registry/universe.toml", MECHATRON_UNIVERSE),
        ("research/registry/donors.toml", MECHATRON_DONORS),
        ("research/registry/protocols.toml", MECHATRON_PROTOCOLS),
        ("tools/atlas/roadmap/FULL-OSS-REPLAY.toml", ATLAS_REPLAY),
    ] {
        r.write(path, text);
        let (code, out) = import(&r, path);
        assert_eq!(code, 0, "{out}");
        assert!(out.contains("rebuilds the registry byte for byte"), "{out}");
        assert!(out.contains("wrote"), "{out}");
        // The universe file rebuilds the legacy bytes; its digest is the registry's.
        let file = import::universe_file(path);
        let src = ynventa::universe::Source::parse(&file, &r.read(&file)).unwrap();
        assert_eq!(src.rebuild(), text, "{path}");
        assert_eq!(src.digest, ynventa::digest::content_digest(text.as_bytes()));
        // A re-import is byte-identical, and the legacy registry is never touched.
        let before = r.read(&file);
        let (_, again) = import(&r, path);
        assert!(again.contains("unchanged"), "{again}");
        assert_eq!(r.read(&file), before);
        assert_eq!(r.read(path), text);
    }
    let u = ynventa::universe::Universe::load(r.path());
    assert!(u.unreadable.is_empty(), "{:?}", u.unreadable);
    let rec = |id: &str| u.records().find(|(_, x)| x.id == id).unwrap().1.clone();

    // Fi-game: kinds from `type`, verification, the cloned donor is selected.
    let h = rec("OSS-ALPHA-084");
    assert_eq!(h.kind, UniverseKind::OssRepository);
    assert_eq!(
        h.global.as_deref(),
        Some("github.com/hummingbot/hummingbot")
    );
    assert!(h.origin_verified && h.selected);
    assert_eq!(h.effective(), UniverseState::SelectedForCensus);
    assert!(
        h.legacy_state.contains("lifecycle=EXTINCT"),
        "{}",
        h.legacy_state
    );
    assert_eq!(rec("OSINT-INTEL-001").kind, UniverseKind::InformationSource);
    assert!(!rec("OSINT-INTEL-001").origin_verified);
    assert_eq!(
        rec("OSINT-INTEL-001").global.as_deref(),
        Some("1stheadlines.com")
    );
    assert_eq!(rec("AUTH-DATA-001").kind, UniverseKind::OfficialAuthority);
    assert_eq!(rec("ACAD-ACCT-002").kind, UniverseKind::AcademicReference);
    // A malformed origin is no identity (the registry's own `key` is never guessed from).
    let bad = rec("ACAD-QUANT-141");
    assert_eq!(bad.global, None);
    assert_eq!(bad.effective(), UniverseState::Discovered);
    // Mechatron: kind from the registry header; a licence "from memory" is a claim.
    let u1 = rec("U0001");
    assert_eq!(u1.kind, UniverseKind::OssRepository);
    assert!(u1.license.is_empty() && u1.license_claim == "Apache-2.0");
    assert!(u1.selected, "donor_ref names the ledger");
    let u2 = rec("U0002");
    assert_eq!(u2.claimed, UniverseState::Discovered);
    assert!(!u2.selected, "an empty donor_ref selects nothing");
    let d1 = rec("D001");
    assert_eq!(d1.license, "Apache-2.0");
    assert_eq!(d1.revision, "eee4b508d4357c81c0847f922617e5f498d1d4b7");
    assert_eq!(rec("P001").kind, UniverseKind::ProtocolSpec);
    // Atlas: epochs and counts are kept as segments; never-replayed is undecided.
    let b = rec("666ghj/bettafish");
    assert_eq!(b.global.as_deref(), Some("github.com/666ghj/bettafish"));
    assert!(b.license == "GPL-2.0" && b.selected);
    assert_eq!(
        rec("serde-rs/serde").relevance,
        ynventa::universe::Relevance::Unresolved
    );

    // The repository's relations: one per global donor, whatever the number of records.
    let a = r.assess();
    let serde: Vec<_> = a
        .universe
        .relations
        .iter()
        .filter(|x| x.global_donor.as_deref() == Some("github.com/serde-rs/serde"))
        .collect();
    assert_eq!(serde.len(), 1);
    assert_eq!(serde[0].records.len(), 3, "{:?}", serde[0].records);
    let rclcpp = a
        .universe
        .relations
        .iter()
        .find(|x| x.global_donor.as_deref() == Some("github.com/ros2/rclcpp"))
        .unwrap();
    assert_eq!(rclcpp.records.len(), 2);
    assert!(rclcpp.license_verified && rclcpp.selected);
    assert_eq!(a.counts.unknown_identity, 1);
    assert_eq!(a.counts.known_information_sources, 1);
    assert_eq!(a.counts.universe_sources_unreconciled, 0);
}

#[test]
fn a_tampered_universe_is_not_lossless() {
    let r = shard("tamper", "mechatron", vec![]);
    r.write("research/registry/protocols.toml", MECHATRON_PROTOCOLS);
    assert_eq!(import(&r, "research/registry/protocols.toml").0, 0);
    let file = import::universe_file("research/registry/protocols.toml");
    let text = r.read(&file).replace("QoS 0-2", "QoS 0-1");
    r.write(&file, &text);
    let a = r.assess();
    let f = finding(&a, "UNIVERSE_INVALID", &file).unwrap();
    assert!(f.detail.contains("not lossless"), "{}", f.detail);
    // A record whose derived identity no longer matches its origin is refused too.
    assert_eq!(import(&r, "research/registry/protocols.toml").0, 0);
    let text = r.read(&file).replace(
        "\"global_donor\":\"docs.oasis-open.org",
        "\"global_donor\":\"github.com/x/mqtt\",\"x\":\"",
    );
    r.write(&file, &text);
    assert!(finding(&r.assess(), "UNIVERSE_INVALID", &file).is_some());
}

#[test]
fn json_inventories_join_the_declared_upstream() {
    let r = shard(
        "uc-json",
        "universal-connector",
        vec![declared("serde", "https://github.com/serde-rs/serde")],
    );
    r.write(
        "tools/atlas-check/registry/donors/inventory.json",
        UC_INVENTORY,
    );
    let (code, out) = import(&r, "tools/atlas-check/registry/donors/inventory.json");
    assert_eq!(code, 0, "{out}");
    let u = ynventa::universe::Universe::load(r.path());
    let src = &u.sources[0];
    assert_eq!(src.table, "donors");
    assert_eq!(src.rebuild(), UC_INVENTORY);
    let serde = &src.records[0];
    assert_eq!(serde.global.as_deref(), Some("github.com/serde-rs/serde"));
    assert!(serde.origin_evidence.contains("donors.rs#serde"));
    assert_eq!(serde.claimed, UniverseState::Discovered);
    // No upstream anywhere: an unknown identity, explicitly.
    assert_eq!(src.records[1].global, None);
    assert!(matches!(
        src.records[1].relevance,
        ynventa::universe::Relevance::NotRelevant(_)
    ));
    let a = r.assess();
    assert_eq!(a.counts.unknown_identity, 1);
    // The declared donor and its inventory record are one relation, selected for census.
    let rel = a
        .universe
        .relations
        .iter()
        .find(|x| x.global_donor.as_deref() == Some("github.com/serde-rs/serde"))
        .unwrap();
    assert_eq!(rel.donors, vec!["serde".to_string()]);
    assert_eq!(
        rel.lifecycle,
        Layer::Universe(UniverseState::SelectedForCensus)
    );
}

#[test]
fn unreconciled_registries_are_not_zero() {
    let r = shard("unreconciled", "fi-game", vec![]);
    r.write("tools/atlas/intake/registry.jsonl", FI_GAME_REGISTRY);
    let a = r.assess();
    assert_eq!(a.counts.universe_sources_unreconciled, 1);
    assert!(finding(
        &a,
        "UNIVERSE_SOURCE_NOT_RECONCILED",
        "tools/atlas/intake/registry.jsonl"
    )
    .is_some());
    let (_, out) = r.cli(&["donor", "summary"]);
    assert!(out.contains("not reconciled"), "{out}");
    assert!(
        out.contains("NOT_RECONCILED  tools/atlas/intake/registry.jsonl"),
        "{out}"
    );
    // Imported: reconciled, and the count is what the registry holds.
    assert_eq!(import(&r, "tools/atlas/intake/registry.jsonl").0, 0);
    let a = r.assess();
    assert_eq!(a.counts.universe_sources_unreconciled, 0);
    assert_eq!(a.counts.known_universe_total, 6);
    let (_, out) = r.cli(&["donor", "summary"]);
    assert!(!out.contains("not reconciled"), "{out}");
    // A registry changed after its import is STALE: not reconciled again.
    r.write(
        "tools/atlas/intake/registry.jsonl",
        &format!("{FI_GAME_REGISTRY}\n"),
    );
    let a = r.assess();
    assert_eq!(a.universe.sources[0].1, SourceState::Stale);
    assert_eq!(a.counts.universe_sources_unreconciled, 1);
}

#[test]
fn look_alikes_stay_distinct_until_an_alias_with_provenance_joins_them() {
    let registry = "{\"id\":\"A\",\"name\":\"aeron\",\"url\":\"https://github.com/real-logic/aeron\",\"type\":\"OSS_DONOR\",\"capability\":\"exec.messaging\",\"lifecycle\":\"REGISTERED\"}\n{\"id\":\"B\",\"name\":\"aeron\",\"url\":\"https://github.com/aeron-io/aeron\",\"type\":\"OSS_DONOR\",\"capability\":\"exec.messaging\",\"lifecycle\":\"REGISTERED\"}\n";
    let r = shard("alias", "fi-game", vec![]);
    r.write("tools/atlas/intake/registry.jsonl", registry);
    assert_eq!(import(&r, "tools/atlas/intake/registry.jsonl").0, 0);
    let a = r.assess();
    // Same name, different owner: ambiguous, never merged.
    let f = finding(&a, "AMBIGUOUS_DONOR_IDENTITY", "aeron").unwrap();
    assert!(
        f.detail.contains("github.com/aeron-io/aeron"),
        "{}",
        f.detail
    );
    assert_eq!(a.counts.unique_global_donors, 2);
    assert_eq!(a.counts.ambiguous_donor_identities, 1);

    // An alias without provenance is no evidence: refused, and nothing is joined.
    let file = import::universe_file("tools/atlas/intake/registry.jsonl");
    let set_alias = |provenance: &str| {
        let mut src = ynventa::universe::Source::parse(&file, &r.read(&file)).unwrap();
        src.records[0].aliases.clear();
        import::add_alias(
            &mut src,
            "A",
            "https://github.com/aeron-io/aeron",
            provenance,
        )
        .unwrap();
        r.write(&file, &src.render());
    };
    set_alias("");
    let a = r.assess();
    assert!(finding(&a, "UNPROVEN_ALIAS", "tools/atlas/intake/registry.jsonl#A").is_some());
    assert_eq!(a.counts.unique_global_donors, 2);

    // With provenance (the redirect), the two records are one upstream.
    set_alias("GitHub redirects real-logic/aeron to aeron-io/aeron (checked 2026-09-30)");
    let a = r.assess();
    assert!(finding(&a, "UNPROVEN_ALIAS", "tools/atlas/intake/registry.jsonl#A").is_none());
    assert!(finding(&a, "AMBIGUOUS_DONOR_IDENTITY", "aeron").is_none());
    assert_eq!(a.counts.unique_global_donors, 1);
    let rel = &a.universe.relations[0];
    assert_eq!(
        rel.global_donor.as_deref(),
        Some("github.com/aeron-io/aeron")
    );
    assert_eq!(rel.records.len(), 2);
    // A declared alias survives a re-import.
    assert_eq!(import(&r, "tools/atlas/intake/registry.jsonl").0, 0);
    assert_eq!(r.assess().counts.unique_global_donors, 1);
}

#[test]
fn one_upstream_is_one_global_donor_with_a_relation_per_repository() {
    // The same upstream, written three ways in three repositories.
    let fi = shard(
        "uses-fi",
        "fi-game",
        vec![declared("serde", "https://github.com/serde-rs/serde.git")],
    );
    let chronica = shard(
        "uses-chronica",
        "chronica",
        vec![
            declared("cargo-serde", "git@github.com:Serde-RS/serde"),
            declared("coreutils", "https://www.gnu.org/software/coreutils"),
        ],
    );
    let norl = shard(
        "uses-norl",
        "norl",
        vec![declared("serde-study", "serde-rs/serde")],
    );
    // Fi-game also knows a paper and a second record of the same upstream.
    fi.write("tools/atlas/intake/registry.jsonl", FI_GAME_REGISTRY);
    assert_eq!(import(&fi, "tools/atlas/intake/registry.jsonl").0, 0);

    // In each graph the repository USES its donor; the donor node keeps its v1 identity.
    for (r, key) in [
        (&fi, "serde"),
        (&chronica, "cargo-serde"),
        (&norl, "serde-study"),
    ] {
        let a = r.assess();
        let donor = ynventa::graph::NodeId::of("oss", "github.com/serde-rs/serde");
        let repo = ynventa::graph::NodeId::of(
            "chronica",
            &format!("repository/{}", a.declaration.repository.shard),
        );
        assert!(
            a.graph
                .edges
                .iter()
                .any(|e| e.from == repo && e.to == donor && e.kind == EdgeKind::Uses),
            "{key}"
        );
    }
    let caps: Vec<_> = [&fi, &chronica, &norl]
        .iter()
        .map(|r| ynventa::capsule::Capsule::compile(&r.assess()))
        .collect();
    // Capsules round-trip their relations.
    for c in &caps {
        assert_eq!(
            ynventa::capsule::Capsule::decode(&c.encode())
                .unwrap()
                .universe,
            c.universe
        );
    }
    let image = ynventa::linker::link(&caps);
    let serde: Vec<_> = image
        .global_donors
        .iter()
        .filter(|g| g.key == "github.com/serde-rs/serde")
        .collect();
    assert_eq!(serde.len(), 1, "one global donor");
    let repos: Vec<&str> = serde[0]
        .uses
        .iter()
        .map(|u| u.repository.as_str())
        .collect();
    assert_eq!(
        repos,
        vec!["chronica", "fi-game", "norl"],
        "three relations"
    );
    // In the graph too: one donor node, used by three repositories.
    let donor = ynventa::graph::NodeId::of("oss", "github.com/serde-rs/serde");
    assert_eq!(
        image
            .graph
            .edges
            .iter()
            .filter(|e| e.to == donor && e.kind == EdgeKind::Uses)
            .count(),
        3
    );
    // The www. origin: the global key drops it, the node keeps its v1 id as an alias.
    let gnu = image
        .global_donors
        .iter()
        .find(|g| g.key == "gnu.org/software/coreutils")
        .unwrap();
    let v1 = ynventa::graph::NodeId::of("oss", "www.gnu.org/software/coreutils").to_string();
    assert!(gnu.aliases.contains(&v1), "{:?}", gnu.aliases);

    // Raw references are summed; unique global donors are deduplicated, never summed.
    let c = image.counts();
    let per_repo: u64 = caps
        .iter()
        .map(|c| c.counts().repository_donor_references)
        .sum();
    assert_eq!(c.repository_donor_references, per_repo);
    let summed_unique: u64 = caps.iter().map(|c| c.counts().unique_global_donors).sum();
    assert_eq!(c.unique_global_donors, image.global_donors.len() as u64);
    assert_eq!(
        summed_unique - c.unique_global_donors,
        2,
        "serde counted once, not three times"
    );
    // The summary says so.
    let (code, out) = fi.cli(&[
        "donor",
        "summary",
        "--ecosystem",
        &fi.path().display().to_string(),
        &chronica.path().display().to_string(),
        &norl.path().display().to_string(),
    ]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains(&format!("raw cross-repo references:      {per_repo}")),
        "{out}"
    );
    assert!(
        out.contains(&format!(
            "deduplicated unique global donors: {}",
            c.unique_global_donors
        )),
        "{out}"
    );
    assert!(
        out.contains("shared by two or more repositories: 1"),
        "{out}"
    );
}

#[test]
fn relations_take_the_donor_ladder_from_censused_up() {
    let r = extinct_baseline("layers");
    let a = r.assess();
    let rel = &a.universe.relations[0];
    assert_eq!(rel.lifecycle, Layer::Donor(DonorState::Extinct));
    assert!(rel.selected);
    assert_eq!(a.counts.extinct, 1);
    assert_eq!(a.counts.native_targeted, 1);
    assert_eq!(a.counts.selected_for_census, 1);
    // The layers read as one ladder.
    let words: Vec<&str> = ynventa::universe::layers()
        .iter()
        .map(|l| l.wire())
        .collect();
    assert_eq!(
        words,
        [
            "DISCOVERED",
            "REGISTERED",
            "RELEVANCE_RESOLVED",
            "SELECTED_FOR_CENSUS",
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
