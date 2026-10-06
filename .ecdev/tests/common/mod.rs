//! Fixture repositories built in code (no fixture files: the governance tree stays clean).
#![allow(dead_code)]

use ecdev_governance::declare::*;
use ecdev_governance::schema::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

pub struct Repo {
    pub root: PathBuf,
}

impl Repo {
    pub fn new(name: &str) -> Repo {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root =
            std::env::temp_dir().join(format!("ecdev-gov-{name}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Repo { root }
    }
    pub fn write(&self, path: &str, content: &str) {
        let p = self.root.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }
    pub fn read(&self, path: &str) -> String {
        std::fs::read_to_string(self.root.join(path)).unwrap()
    }
    pub fn exists(&self, path: &str) -> bool {
        self.root.join(path).exists()
    }
    pub fn remove(&self, path: &str) {
        let p = self.root.join(path);
        if p.is_dir() {
            std::fs::remove_dir_all(p).unwrap();
        } else {
            std::fs::remove_file(p).unwrap();
        }
    }
    pub fn assess(&self) -> ecdev_governance::Assessment {
        ecdev_governance::assess(&self.root).unwrap()
    }
    pub fn cli(&self, args: &[&str]) -> (i32, String) {
        let mut v: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        v.push("--root".into());
        v.push(self.root.display().to_string());
        ecdev_governance::cli::run(&v)
    }
    pub fn declaration(&self) -> Declaration {
        load(&self.root).unwrap()
    }
    pub fn store(&self, d: &Declaration) {
        store(&self.root, d).unwrap()
    }
    pub fn edit(&self, f: impl FnOnce(&mut Declaration)) {
        let mut d = self.declaration();
        f(&mut d);
        self.store(&d);
    }
    /// Records passing results for every declared proof, as `ecdev-gov prove` would.
    pub fn prove(&self) {
        let d = self.declaration();
        let files = ecdev_governance::repository::files::Files::scan(&self.root).unwrap();
        for (loc, pass, rec) in
            ecdev_governance::evidence::prove(&self.root, &files, &d, None, &mut Pass)
        {
            assert!(pass, "{loc}: {rec}");
        }
    }
    pub fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// A runner that reports every proof as passing (the proof bodies are not compiled here).
pub struct Pass;

impl ecdev_governance::evidence::Runner for Pass {
    fn run(
        &mut self,
        _: &Path,
        _: &ecdev_governance::repository::files::Files,
        p: &Proof,
    ) -> (bool, String) {
        (true, format!("stub run of {}", p.locator))
    }
}

pub fn node(key: &str, kind: NodeKind, path: &str, canonical: &str) -> Node {
    Node::new(key, kind, path, canonical)
}

pub fn proof(kind: ProofKind, locator: &str) -> Proof {
    Proof {
        kind,
        locator: locator.into(),
    }
}

pub fn donor(key: &str, package: &str) -> Donor {
    Donor {
        key: key.into(),
        name: package.into(),
        origin: format!("https://github.com/georust/{package}"),
        license: "MIT OR Apache-2.0".into(),
        claimed: DonorState::Extinct,
        exception: None,
        packages: vec![Package {
            ecosystem: Ecosystem::Cargo,
            name: package.into(),
        }],
        source_paths: vec![],
        capabilities: vec![Capability {
            key: "distance".into(),
            required: true,
            spec: "tests/tests/parity.rs".into(),
            replacement: Some("geo".into()),
            maps_to: Some("capability/distance".into()),
            relevance: Relevance::ReliedOn("distance between two points".into()),
            knowledge: Knowledge::Absorbed("native and proven".into()),
            proofs: vec![
                proof(
                    ProofKind::Parity,
                    "tests/tests/parity.rs::distance_matches_donor",
                ),
                proof(
                    ProofKind::Regression,
                    "substrate/geo/src/lib.rs::regression_distance",
                ),
            ],
        }],
        cutover: Some("wave-geo".into()),
        provenance: vec!["fixture".into()],
    }
}

/// A repository in which donor `geo` is genuinely EXTINCT: native replacement, fresh passing
/// parity and regression proofs, cutover, no edge of any kind to the donor.
pub fn extinct_baseline(name: &str) -> Repo {
    let r = Repo::new(name);
    r.write(
        "Cargo.toml",
        "[workspace]\nresolver = \"2\"\nmembers = [\"core\", \"substrate/geo\", \"tests\"]\n",
    );
    r.write("README.md", "# fixture\n");
    r.write(
        "core/Cargo.toml",
        "[package]\nname = \"core-kernel\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    r.write("core/src/lib.rs", "pub fn id() -> u64 { 1 }\n");
    r.write(
        "substrate/geo/Cargo.toml",
        "[package]\nname = \"geo-native\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\ncore-kernel = { path = \"../../core\" }\n",
    );
    r.write(
        "substrate/geo/src/lib.rs",
        "pub fn distance(a: (f64, f64), b: (f64, f64)) -> f64 { ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt() }\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn regression_distance() { assert_eq!(super::distance((0.0, 0.0), (3.0, 4.0)), 5.0); }\n}\n",
    );
    r.write("tests/Cargo.toml", "[package]\nname = \"verification\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dev-dependencies]\ngeo-native = { path = \"../substrate/geo\" }\n");
    r.write("tests/src/lib.rs", "");
    r.write(
        "tests/tests/parity.rs",
        "// Frozen oracle outputs (recorded from the donor, kept as data).\nconst CASES: &[((f64, f64), (f64, f64), f64)] = &[((0.0, 0.0), (3.0, 4.0), 5.0)];\n\n#[test]\nfn distance_matches_donor() { for (a, b, d) in CASES { assert_eq!(geo_native::distance(*a, *b), *d); } }\n",
    );
    r.store(&Declaration {
        technologies: vec![],
        repository: Repository {
            id: "ecdev".into(),
            name: name.into(),
            origin: "llgtrn/Fixture".into(),
            nodes: vec![
                node("core", NodeKind::Kernel, "core", "core"),
                {
                    // The native replacement was learned from the donor.
                    let mut geo =
                        node("geo", NodeKind::Substrate, "substrate/geo", "substrate/geo");
                    geo.lineage = vec!["geo".into()];
                    geo
                },
                node("tests", NodeKind::Test, "tests", "tests"),
                node("governance", NodeKind::Governance, ".ecdev", ".ecdev"),
            ],
            edges: vec![],
        },
        donors: vec![donor("geo", "geo")],
        migration: Migration::default(),
    });
    r.prove();
    r
}

pub fn finding<'a>(
    a: &'a ecdev_governance::Assessment,
    code: &str,
    subject: &str,
) -> Option<&'a ecdev_governance::Finding> {
    a.findings
        .iter()
        .find(|f| f.code == code && f.subject == subject)
}

pub fn gate(a: &ecdev_governance::Assessment, donor: &str, g: Gate) -> bool {
    a.donor(donor)
        .unwrap()
        .gates
        .iter()
        .find(|x| x.gate == g)
        .unwrap()
        .pass
}
