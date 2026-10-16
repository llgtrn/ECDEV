//! Package aliases from one retained repository must remain visible in locked closure.
mod common;

use common::{extinct_baseline, gate};
use ecdev_governance::declare;
use ecdev_governance::formats::{json, toml, Value};
use ecdev_governance::graph::{Graph, NodeId};
use ecdev_governance::migration::{proposal, source};
use ecdev_governance::schema::Gate;
use std::collections::BTreeSet;
use std::path::Path;

const DONOR: &str = "dependency--github-com-rust-lang-futures-rs";
const PACKAGES: [&str; 6] = [
    "futures-channel",
    "futures-core",
    "futures-io",
    "futures-sink",
    "futures-task",
    "futures-util",
];

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
}

#[test]
fn package_family_is_bound_to_lock_observations_without_replacing_source_identity() {
    let d = declare::load(root()).unwrap();
    let donor = d.donor(DONOR).unwrap();
    let capture = source::read_export_bound(
        &root().join(".ecdev/tests/fixtures/source-capture-00dd7f0-v1.json"),
        Some(proposal::CAPTURE_SHA256),
    )
    .unwrap();
    let original = capture.declaration().unwrap();
    let old = original.donor(DONOR).unwrap();
    assert_eq!(d.repository.id, original.repository.id);
    assert_eq!(donor.key, old.key);
    assert_eq!(donor.origin, old.origin);
    assert_eq!(donor.license, old.license);
    assert_eq!(donor.claimed, old.claimed);
    assert_eq!(donor.cutover, old.cutover);
    assert!(donor.cutover.is_none());
    let identity = |donor: &declare::Donor| {
        let (ns, key) = Graph::donor_id(&d.repository.id, &donor.key, &donor.origin);
        NodeId::of(&ns, &key)
    };
    assert_eq!(identity(donor), identity(old));
    assert_eq!(
        donor
            .capabilities
            .iter()
            .map(|c| &c.key)
            .collect::<Vec<_>>(),
        old.capabilities.iter().map(|c| &c.key).collect::<Vec<_>>()
    );
    let expected: BTreeSet<_> = std::iter::once("futures").chain(PACKAGES).collect();
    assert_eq!(
        donor
            .packages
            .iter()
            .map(|p| p.name.as_str())
            .collect::<BTreeSet<_>>(),
        expected
    );
    assert_eq!(donor.packages.len(), expected.len());
    let custody = json::parse_unique(
        &std::fs::read_to_string(
            root().join("research/commerce/governance/futures-package-family-custody-v1.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(custody.str("donor_alias"), Some(DONOR));
    // The custody receipt describes its immutable observed lock, not every future first-party
    // edge. Retain those exact bytes and permit only the independently tested native Web edge.
    let historical_bytes =
        std::fs::read(root().join(".ecdev/tests/fixtures/cargo-lock-338a9d92625512dc.toml"))
            .unwrap();
    assert_eq!(
        ecdev_governance::digest::content_digest(&historical_bytes),
        "sha256:338a9d92625512dcc65889b31bd2fc3684eed96f12211be025f2ba19d11087b6"
    );
    assert_eq!(
        custody.str("cargo_lock_sha256"),
        Some(&ecdev_governance::digest::content_digest(&historical_bytes)[7..])
    );
    let historical = toml::parse(std::str::from_utf8(&historical_bytes).unwrap()).unwrap();
    let lock_bytes = std::fs::read(root().join("Cargo.lock")).unwrap();
    let lock = toml::parse(std::str::from_utf8(&lock_bytes).unwrap()).unwrap();
    assert_eq!(historical.items_of("package").len(), 140);
    assert_eq!(lock.items_of("package").len(), 140);
    let Value::Table(mut expected) = historical else {
        panic!("lock table");
    };
    let Some(Value::Array(packages)) = expected.get_mut("package") else {
        panic!("lock packages");
    };
    let mut updated = 0;
    for package in packages {
        if package.str("name") == Some("ecdev-web") {
            assert_eq!(package.strings("dependencies"), ["ecdev-core", "reqwest"]);
            let Value::Table(fields) = package else {
                panic!("package table");
            };
            fields.insert(
                "dependencies".into(),
                Value::Array(
                    ["ecdev-core", "ecdev-net", "reqwest"]
                        .into_iter()
                        .map(|name| Value::Str(name.into()))
                        .collect(),
                ),
            );
            updated += 1;
        }
    }
    assert_eq!(updated, 1, "exactly one first-party package changes");
    assert_eq!(
        lock,
        Value::Table(expected),
        "every external package and lock field stays bound to the historical receipt"
    );
    assert_eq!(custody.items_of("packages").len(), PACKAGES.len());
    for name in PACKAGES {
        let observed = custody
            .items_of("packages")
            .iter()
            .find(|p| p.str("package") == Some(name))
            .unwrap();
        let locked: Vec<_> = lock
            .items_of("package")
            .iter()
            .filter(|p| p.str("name") == Some(name))
            .collect();
        assert_eq!(locked.len(), 1);
        assert_eq!(observed.get("version"), locked[0].get("version"));
        assert_eq!(observed.get("lock_checksum"), locked[0].get("checksum"));
        assert_eq!(observed.get("archive_sha256"), locked[0].get("checksum"));
        assert_eq!(
            observed.str("declared_repository"),
            Some("https://github.com/rust-lang/futures-rs")
        );
    }
}

#[test]
fn each_same_repository_package_stops_false_zero_linked_closure() {
    let packages = declare::load(root())
        .unwrap()
        .donor(DONOR)
        .unwrap()
        .packages
        .clone();
    let original = packages
        .iter()
        .find(|p| p.name == "futures")
        .unwrap()
        .clone();
    for name in PACKAGES {
        let r = extinct_baseline(&format!("futures-lock-{name}"));
        r.write(
            "core/Cargo.toml",
            "[package]\nname = \"core-kernel\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\nreqwest = \"0.12.28\"\n",
        );
        r.write(
            "Cargo.lock",
            &format!(
                "version = 4\n[[package]]\nname = \"core-kernel\"\nversion = \"0.1.0\"\ndependencies = [\"reqwest\"]\n[[package]]\nname = \"reqwest\"\nversion = \"0.12.28\"\ndependencies = [\"{name}\"]\n[[package]]\nname = \"{name}\"\nversion = \"0.3.34\"\n"
            ),
        );
        // The old declaration's monolithic alias misses the separately named package.
        r.edit(|d| d.donors[0].packages = vec![original.clone()]);
        assert!(gate(&r.assess(), "geo", Gate::LinkedEdges), "{name}");
        r.edit(|d| d.donors[0].packages = packages.clone());
        let observed = r.assess();
        assert!(!gate(&observed, "geo", Gate::LinkedEdges), "{name}");
        let failure = observed
            .donor("geo")
            .unwrap()
            .gates
            .iter()
            .find(|g| g.gate == Gate::LinkedEdges)
            .unwrap();
        assert!(failure.detail.contains(name), "{}", failure.detail);
        assert!(failure.detail.contains("reqwest"), "{}", failure.detail);
    }
}

#[test]
fn underscore_import_of_an_observed_package_is_not_native_closure() {
    let packages = declare::load(root())
        .unwrap()
        .donor(DONOR)
        .unwrap()
        .packages
        .clone();
    let r = extinct_baseline("futures-util-import");
    r.edit(|d| d.donors[0].packages = packages);
    r.write(
        "core/src/lib.rs",
        "use futures_util::StreamExt;\npub fn value() {}\n",
    );
    let observed = r.assess();
    assert!(!gate(&observed, "geo", Gate::SourceImports));
    assert_ne!(
        observed.donor("geo").unwrap().effective,
        ecdev_governance::schema::DonorState::Extinct
    );
}
