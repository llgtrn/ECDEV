//! Legacy import: `.atlas` (and Mechatron's `research/registry`) → normalized Ynventa IR →
//! declarations + knowledge facts.
//!
//! The six repositories diverge; each dialect is read as it really is (see `dialects`). Every
//! imported status is a CLAIM with provenance, never evidence: the verifier recomputes every
//! effective state under the V1 definitions, and any legacy "absorbed", "native" or "extinct"
//! that the evidence does not support surfaces as CLAIM_EXCEEDS_EVIDENCE.

pub mod dialects;

use crate::census::{self, cargo::members};
use crate::compact::facts::Fact;
use crate::declare::{
    Capability, Declaration, Donor, Migration, Node, Package, Proof, Repository, Shim,
};
use crate::graph::id::oss_key;
use crate::repository::derive_canonical_path;
use crate::repository::files::Files;
use crate::schema::{
    DonorState, Ecosystem, ExceptionKind, ExpiryKind, FactKind, NodeKind, NodeLifecycle, ProofKind,
    ShimKind,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// A donor as read from a legacy registry, before normalization.
#[derive(Clone, Debug, Default)]
pub struct LegacyDonor {
    pub id: String,
    pub name: String,
    pub origin: String,
    pub license: String,
    /// The legacy status fields, verbatim: (field, value).
    pub status: Vec<(String, String)>,
    pub claimed: Option<DonorState>,
    pub exception: Option<(ExceptionKind, String)>,
    pub source_paths: Vec<String>,
    pub capabilities: Vec<LegacyCapability>,
    /// Package names this donor may appear under.
    pub names: Vec<String>,
    pub provenance: String,
}

#[derive(Clone, Debug, Default)]
pub struct LegacyCapability {
    pub key: String,
    pub required: bool,
    pub spec: String,
    /// A repository path of the native implementation, if the legacy record names one.
    pub native_path: Option<String>,
    /// A legacy owner name (e.g. `storage::native_log` → `storage`).
    pub owner: Option<String>,
    pub proofs: Vec<(ProofKind, String)>,
}

#[derive(Default)]
pub struct Import {
    pub declaration: Declaration,
    pub facts: Vec<Fact>,
    pub dialects: Vec<String>,
    pub warnings: Vec<String>,
}

pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// `owner/repo` of the `origin` remote, read from `.git/config`.
pub fn git_origin(root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(root.join(".git/config")).ok()?;
    let mut in_origin = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_origin = t == "[remote \"origin\"]";
        } else if in_origin {
            if let Some(url) = t
                .strip_prefix("url")
                .map(|r| r.trim_start_matches([' ', '=']).trim())
            {
                let key = oss_key(url)?;
                let parts: Vec<&str> = key.split('/').collect();
                // Proxied remotes (`host/git/owner/repo`) keep only owner/repo.
                if parts.len() >= 2 {
                    return Some(format!(
                        "{}/{}",
                        parts[parts.len() - 2],
                        parts[parts.len() - 1]
                    ));
                }
            }
        }
    }
    None
}

/// Default node kind of a directory, from its first segment.
pub fn infer_kind(dir: &str) -> NodeKind {
    let first = dir.split('/').next().unwrap_or(dir);
    match first {
        "core" | "kernel" | "genome" => NodeKind::Kernel,
        "domain" | "domains" => NodeKind::Domain,
        "adapter" | "adapters" | "mcp" | "interfaces" => NodeKind::Adapter,
        "apps" | "app" | "frontend" | "operator" | "ui" | "web" => NodeKind::Application,
        "tools" | "benchmark" | "benchmarks" | "script" | "scripts" => NodeKind::Tool,
        "tests" | "conformance" => NodeKind::Test,
        "research" | "science" => NodeKind::Research,
        "compat" => NodeKind::Compat,
        ".ynventa" => NodeKind::Ynventa,
        _ => NodeKind::Substrate,
    }
}

const CONTAINERS: &[&str] = &[
    "core",
    "domain",
    "domains",
    "adapter",
    "adapters",
    "apps",
    "tools",
    "tests",
    "runtime",
    "research",
    "science",
    "compat",
    "substrate",
];

/// Semantic key of a directory: its segments after the container, dotted.
pub fn key_of_dir(dir: &str) -> String {
    let segs: Vec<String> = dir.split('/').map(slug).filter(|s| !s.is_empty()).collect();
    if segs.len() > 1 && CONTAINERS.contains(&segs[0].as_str()) {
        segs[1..].join(".")
    } else {
        segs.join(".")
    }
}

fn node_for(
    dir: &str,
    kind: NodeKind,
    taken: &mut BTreeSet<String>,
    domain: &str,
    shard: &str,
) -> Node {
    let mut local = if kind == NodeKind::Ynventa {
        String::new()
    } else {
        key_of_dir(dir)
    };
    if kind != NodeKind::Ynventa && (local.is_empty() || taken.contains(&qualify(domain, &local))) {
        local = dir.split('/').map(slug).collect::<Vec<_>>().join(".");
    }
    // Each shard carries its own Ynventa installation; only the template's is `ynventa`.
    let key = if kind == NodeKind::Ynventa {
        if shard == "ynventa" {
            "ynventa".to_string()
        } else {
            format!("ynventa.{shard}")
        }
    } else {
        qualify(domain, &local)
    };
    taken.insert(key.clone());
    let canonical_path = derive_canonical_path(kind, &local).unwrap_or_default();
    Node {
        key,
        kind,
        concept: crate::graph::default_concept(kind),
        name: dir.to_string(),
        path: dir.to_string(),
        canonical_path,
        lifecycle: NodeLifecycle::Active,
        provides: Vec::new(),
        requires: Vec::new(),
        reuses: Vec::new(),
        inputs: Vec::new(),
        outputs: Vec::new(),
        lineage: Vec::new(),
    }
}

/// A Chronica-wide key: the shard's semantic domain, then the local key.
pub fn qualify(domain: &str, local: &str) -> String {
    if domain.is_empty() || local.is_empty() {
        local.to_string()
    } else {
        format!("{domain}.{local}")
    }
}

const CODE: &[&str] = &[".rs", ".ts", ".tsx", ".js", ".py"];

/// Imports the repository at `root` into Ynventa declarations.
pub fn import(root: &Path, files: &Files, shard: Option<&str>, origin: Option<&str>) -> Import {
    let mut imp = Import::default();
    let dir_name = root
        .canonicalize()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "repository".into());
    let origin = origin
        .map(str::to_string)
        .or_else(|| git_origin(root))
        .or_else(|| dialects::repo_toml_origin(files))
        .unwrap_or_default();
    // The shard: named, else recognised by origin, else by directory name.
    let ns = shard
        .map(str::to_string)
        .or_else(|| crate::protocol::shard_of_origin(&origin).map(|s| s.id.to_string()))
        .unwrap_or_else(|| {
            let s = slug(&dir_name);
            if s.is_empty() {
                "repository".into()
            } else {
                s
            }
        });
    let domain = crate::protocol::shard(&ns).map(|s| s.domain).unwrap_or("");

    // 1. Nodes: workspace members, then code roots outside them.
    let hint_table = dialects::kind_hints(files);
    // A hint for a root applies to every directory below it; the deepest hint wins.
    let hints = |dir: &str| -> Option<NodeKind> {
        // A benchmark is construction tooling wherever it was placed.
        if matches!(
            dir.rsplit('/').next(),
            Some("bench" | "benches" | "benchmark" | "benchmarks")
        ) {
            return Some(NodeKind::Tool);
        }
        let mut d = dir;
        loop {
            if let Some(k) = hint_table.get(d) {
                return Some(*k);
            }
            d = d.rsplit_once('/')?.0;
        }
    };
    let mut taken = BTreeSet::new();
    let mut nodes: Vec<Node> = Vec::new();
    let mems = members(files);
    for m in &mems {
        if m.dir.is_empty() || m.dir == ".ynventa" {
            continue;
        }
        let dir = m.dir.as_str();
        let kind = hints(dir).unwrap_or_else(|| infer_kind(dir));
        nodes.push(node_for(dir, kind, &mut taken, domain, &ns));
    }
    let covered = |p: &str, nodes: &[Node]| {
        nodes
            .iter()
            .any(|n| p == n.path || p.starts_with(&format!("{}/", n.path)))
    };
    let mut roots: BTreeMap<String, usize> = BTreeMap::new();
    for f in &files.paths {
        if !CODE.iter().any(|s| f.ends_with(s))
            || f.starts_with('.')
            || f.split('/')
                .any(|s| s == "node_modules" || s == "temporary")
        {
            continue;
        }
        if covered(f, &nodes) {
            continue;
        }
        let segs: Vec<&str> = f.split('/').collect();
        if segs.len() < 2 {
            continue;
        }
        // `apps/<x>/…` style containers yield one node per child; other roots one node each.
        let dir = if CONTAINERS.contains(&segs[0]) && segs.len() >= 3 {
            format!("{}/{}", segs[0], segs[1])
        } else if CONTAINERS.contains(&segs[0]) {
            continue;
        } else {
            segs[0].to_string()
        };
        *roots.entry(dir).or_default() += 1;
    }
    for dir in roots.keys() {
        if !covered(dir, &nodes) {
            let kind = hints(dir).unwrap_or_else(|| infer_kind(dir));
            nodes.push(node_for(dir, kind, &mut taken, domain, &ns));
        }
    }
    let mut yn = node_for(".ynventa", NodeKind::Ynventa, &mut taken, domain, &ns);
    yn.name = "Ynventa protocol subsystem".into();
    nodes.push(yn);
    nodes.sort_by(|a, b| a.path.cmp(&b.path));

    // 2. Donors from every legacy dialect present.
    let mut legacy: Vec<LegacyDonor> = Vec::new();
    for (name, donors, registry_files) in dialects::read_all(files) {
        imp.dialects
            .push(format!("{name} ({} records)", donors.len()));
        legacy.extend(donors);
        // A consumed registry is preserved whole, field by field, before anything relies on it.
        for f in registry_files {
            if let Some(facts) = files.read_bytes(&f).and_then(|b| extract_file(&f, &b, 1)) {
                imp.facts.extend(facts);
            }
        }
    }
    legacy.extend(dialects::unadmitted_sources(files, &legacy));

    // 3. Observed external packages: attach to a legacy donor by name, else register one.
    let bare = Declaration {
        repository: Repository {
            system: crate::schema::SYSTEM.into(),
            shard: ns.clone(),
            ..Repository::default()
        },
        technologies: Vec::new(),
        organism: Default::default(),
        donors: legacy
            .iter()
            .map(|l| Donor {
                key: l.id.clone(),
                name: l.name.clone(),
                origin: String::new(),
                license: String::new(),
                claimed: DonorState::Discovered,
                exception: None,
                packages: Vec::new(),
                source_paths: l.source_paths.clone(),
                capabilities: Vec::new(),
                cutover: None,
                provenance: Vec::new(),
            })
            .collect(),
        migration: Migration::default(),
    };
    let obs = census::run(files, &bare);
    let mut externals: BTreeMap<(Ecosystem, String), BTreeSet<String>> = BTreeMap::new();
    for o in obs.external() {
        if o.via == census::Via::Process
            && crate::donors::TOOLCHAIN_PROGRAMS.contains(&o.name.as_str())
        {
            continue;
        }
        if o.via == census::Via::Link
            && crate::donors::PLATFORM_LIBRARIES.contains(&o.name.as_str())
        {
            continue;
        }
        externals
            .entry((o.ecosystem, o.name.clone()))
            .or_default()
            .insert(o.file.clone());
    }

    let mut donors: Vec<Donor> = Vec::new();
    let mut used_keys = BTreeSet::new();
    for l in &legacy {
        let mut key = slug(&l.id);
        if key.is_empty() || !used_keys.insert(key.clone()) {
            key = format!("{}-{}", key, slug(&l.name));
            used_keys.insert(key.clone());
        }
        let names: BTreeSet<String> = l
            .names
            .iter()
            .map(|n| n.to_ascii_lowercase())
            .chain([l.name.to_ascii_lowercase()])
            .collect();
        let mut packages = Vec::new();
        externals.retain(|(eco, n), _| {
            if names.contains(&n.to_ascii_lowercase())
                || names.contains(&n.to_ascii_lowercase().replace('_', "-"))
            {
                packages.push(Package {
                    ecosystem: *eco,
                    name: n.clone(),
                });
                false
            } else {
                true
            }
        });
        let capabilities = l
            .capabilities
            .iter()
            .map(|c| {
                let replacement = c
                    .native_path
                    .as_deref()
                    .and_then(|p| {
                        nodes
                            .iter()
                            .filter(|n| {
                                n.kind != NodeKind::Ynventa
                                    && (p == n.path || p.starts_with(&format!("{}/", n.path)))
                            })
                            .max_by_key(|n| n.path.len())
                    })
                    .or_else(|| {
                        c.owner
                            .as_deref()
                            .and_then(|o| nodes.iter().find(|n| n.key == slug(o) || n.path == o))
                    })
                    .map(|n| n.key.clone());
                Capability {
                    key: slug(&c.key),
                    required: c.required,
                    spec: c.spec.clone(),
                    replacement,
                    maps_to: None,
                    norl: Default::default(),
                    proofs: c
                        .proofs
                        .iter()
                        .map(|(k, loc)| Proof {
                            kind: *k,
                            locator: loc.clone(),
                        })
                        .collect(),
                }
            })
            .fold(Vec::<Capability>::new(), |mut acc, c| {
                if !c.key.is_empty() && !acc.iter().any(|x| x.key == c.key) {
                    acc.push(c);
                }
                acc
            });
        for (field, value) in &l.status {
            imp.facts.push(Fact::new(
                FactKind::LegacyClaim,
                &key,
                field,
                value,
                &l.provenance,
                1,
            ));
        }
        donors.push(Donor {
            key,
            name: l.name.clone(),
            origin: l.origin.clone(),
            license: l.license.clone(),
            claimed: l.claimed.unwrap_or(DonorState::Registered),
            exception: l.exception.clone(),
            packages,
            source_paths: l.source_paths.clone(),
            capabilities,
            cutover: None,
            provenance: vec![l.provenance.clone()],
        });
    }
    for ((eco, name), in_files) in externals {
        donors.push(crate::migration::register::discovered(eco, &name, in_files));
    }
    donors.sort_by(|a, b| a.key.cmp(&b.key));

    // 4. The legacy tree is legal only as an expiring shim.
    let mut shims = Vec::new();
    if files.exists(".atlas") {
        shims.push(Shim {
            key: "legacy-atlas".into(),
            kind: ShimKind::LegacyInput,
            path: ".atlas".into(),
            serves: ns.clone(),
            expires: (ExpiryKind::LegacyImported, ".atlas".into()),
        });
    }

    imp.declaration = Declaration {
        technologies: Vec::new(),
        organism: Default::default(),
        repository: Repository {
            system: crate::schema::SYSTEM.into(),
            shard: ns,
            name: dir_name,
            origin,
            nodes,
            edges: Vec::new(),
        },
        donors,
        migration: Migration {
            waves: Vec::new(),
            shims,
        },
    };
    let waves = crate::migration::pathmap::plan(&imp.declaration);
    imp.declaration.migration.waves = waves;
    imp
}

fn flatten(prefix: &str, v: &crate::formats::Value, out: &mut Vec<(String, String)>) {
    use crate::formats::Value;
    match v {
        Value::Table(t) => {
            for (k, x) in t {
                flatten(
                    &if prefix.is_empty() {
                        k.clone()
                    } else {
                        format!("{prefix}.{k}")
                    },
                    x,
                    out,
                );
            }
        }
        Value::Array(a) => {
            for (i, x) in a.iter().enumerate() {
                flatten(&format!("{prefix}[{i}]"), x, out);
            }
        }
        Value::Null => out.push((prefix.to_string(), "null".into())),
        Value::Bool(b) => out.push((prefix.to_string(), b.to_string())),
        Value::Int(n) => out.push((prefix.to_string(), n.to_string())),
        Value::Float(f) | Value::Str(f) => out.push((prefix.to_string(), f.clone())),
    }
}

/// Converts one legacy knowledge file into facts without loss: its exact text as one `content`
/// fact, structured registries (TOML, JSON, JSON Lines) also field by field. The file's digest
/// (`imported`, `sha256:<hex>`) is recorded with it, and the content is proven to rebuild to it,
/// so the legacy tree's shim can expire once every file is consumed. Returns `None` for binary
/// content, which cannot be consumed and must be handled explicitly.
///
/// Before content was stored, plain text was recorded line by line (`L<n>` keys) without its
/// blank lines; those facts still load and mean what they meant, but they never rebuild a file.
pub fn extract_file(path: &str, bytes: &[u8], seq: u64) -> Option<Vec<Fact>> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut fields = Vec::new();
    let lower = path.to_ascii_lowercase();
    let parsed = if lower.ends_with(".toml") {
        crate::formats::toml::parse(text).ok()
    } else if lower.ends_with(".json") {
        crate::formats::json::parse(text).ok()
    } else {
        None
    };
    match parsed {
        Some(v) => flatten("", &v, &mut fields),
        None if lower.ends_with(".jsonl") => {
            for (i, line) in text
                .lines()
                .enumerate()
                .filter(|(_, l)| !l.trim().is_empty())
            {
                match crate::formats::json::parse(line) {
                    Ok(v) => flatten(&format!("[{i}]"), &v, &mut fields),
                    Err(_) => fields.push((format!("L{}", i + 1), line.to_string())),
                }
            }
        }
        // Plain text is its content, below.
        None => {}
    }
    fields.push(("content".to_string(), text.to_string()));
    let prov = format!("legacy:{path}");
    let mut facts: Vec<Fact> = fields
        .into_iter()
        .map(|(k, v)| Fact::new(FactKind::LegacyRecord, path, &k, &v, &prov, seq))
        .collect();
    facts.push(Fact::new(
        FactKind::LegacyRecord,
        path,
        "imported",
        &crate::digest::content_digest(bytes),
        &prov,
        seq,
    ));
    Some(facts)
}
