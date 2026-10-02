//! The real legacy dialects, as observed in the six repositories (September 2026):
//!
//! | dialect      | registry                                        | status vocabulary                          |
//! |--------------|-------------------------------------------------|--------------------------------------------|
//! | chronica     | .atlas/census/donors/inventory.toml             | PLANNED … PARTIALLY_ABSORBED, ABSORBED     |
//! | atlas-studio | .atlas/references/donor-corpus.toml             | ingestion_status + decision_status         |
//! | uc           | .atlas/provenance/donor-inventory.json          | CANDIDATE … ABSORBED, EXTINCT, REJECTED    |
//! | esellios     | .atlas/census/donors.toml + donors/*/capabilities.toml | REGISTERED … EXTINCT; dispositions  |
//! | fi-game      | .atlas/census/donors/inventory.toml + absorption.toml | census_status + absorption           |
//! | mechatron    | research/registry/donors.toml                   | DISCOVERED … NATIVE_EQUIVALENCE_PARTIAL    |
//!
//! Each maps its vocabulary onto a CLAIMED Ynventa state; none of it is evidence.

use super::{slug, LegacyCapability, LegacyDonor};
use crate::formats::{json, toml, Value};
use crate::repository::files::Files;
use crate::schema::{DonorState, ExceptionKind, NodeKind, ProofKind};
use std::collections::{BTreeMap, BTreeSet};

type Dialect = (String, Vec<LegacyDonor>, Vec<String>);

fn read_toml(files: &Files, path: &str) -> Option<Value> {
    files.read(path).and_then(|t| toml::parse(&t).ok())
}

/// `path::fn` from the legacy locator spellings (`path::fn`, `path#fn`, `path::fn (note)`).
pub fn locator(s: &str) -> Option<String> {
    let s = s.split_whitespace().next()?;
    let s = s.replacen('#', "::", 1);
    s.contains("::").then_some(s)
}

pub fn repo_toml_origin(files: &Files) -> Option<String> {
    read_toml(files, ".atlas/repo.toml").and_then(|v| v.str("repo").map(str::to_string))
}

/// Legacy architecture registries that say what kind each directory is.
pub fn kind_hints(files: &Files) -> BTreeMap<String, NodeKind> {
    let mut h = BTreeMap::new();
    // Chronica: [[crate]] root + plane.
    if let Some(v) = read_toml(files, ".atlas/census/declared.toml") {
        for c in v.items_of("crate") {
            let root = c.text("root");
            let kind = match c.text("plane").as_str() {
                "SUBSTRATE" if root == "core" => Some(NodeKind::Kernel),
                "SUBSTRATE" => Some(NodeKind::Substrate),
                "ANALYZE" => Some(NodeKind::Domain),
                "ACT" => Some(NodeKind::Adapter),
                "APP" => Some(NodeKind::Application),
                "TOOL" => Some(NodeKind::Tool),
                "TEST" => Some(NodeKind::Test),
                _ => None,
            };
            if let (Some(k), false) = (kind, root.is_empty()) {
                h.insert(root, k);
            }
        }
        // Fi-game: [[root]] path + kind.
        for r in v.items_of("root") {
            let kind = match r.text("kind").as_str() {
                "substrate" => Some(NodeKind::Kernel),
                "organ" => Some(NodeKind::Domain),
                "interface" => Some(NodeKind::Adapter),
                "app" => Some(NodeKind::Application),
                "tool" | "environment" => Some(NodeKind::Tool),
                _ => None,
            };
            if let Some(k) = kind {
                h.entry(r.text("path")).or_insert(k);
            }
        }
    }
    // Esellios: [[crate]] path.
    if let Some(v) = read_toml(files, ".atlas/architecture/crates.toml") {
        for c in v.items_of("crate") {
            let p = c.text("path");
            // A binary composes the planes: it is an application wherever it sits.
            if c.text("kind") == "bin" {
                h.insert(p, NodeKind::Application);
                continue;
            }
            let k = match p.split('/').next().unwrap_or("") {
                "core" => NodeKind::Kernel,
                "domains" => NodeKind::Domain,
                "runtime" => NodeKind::Substrate,
                "adapter" | "mcp" => NodeKind::Adapter,
                "apps" => NodeKind::Application,
                "tools" => NodeKind::Tool,
                _ => continue,
            };
            h.insert(p, k);
        }
    }
    h
}

pub fn read_all(files: &Files) -> Vec<Dialect> {
    let mut out = Vec::new();
    if let Some(d) = chronica(files) {
        out.push(d);
    }
    if let Some(d) = fi_game(files) {
        out.push(d);
    }
    if let Some(d) = atlas_studio(files) {
        out.push(d);
    }
    if let Some(d) = universal_connector(files) {
        out.push(d);
    }
    if let Some(d) = esellios(files) {
        out.push(d);
    }
    if let Some(d) = mechatron(files) {
        out.push(d);
    }
    out
}

const CHRONICA_INVENTORY: &str = ".atlas/census/donors/inventory.toml";

fn chronica(files: &Files) -> Option<Dialect> {
    let v = read_toml(files, CHRONICA_INVENTORY)?;
    if !v.text("schema").starts_with("chronica.") {
        return None;
    }
    let mut donors = Vec::new();
    for d in v.items_of("donor") {
        let id = d.text("id");
        let repo = d.text("repo");
        let lifecycle = d.text("lifecycle");
        let (claimed, exception) = match lifecycle.as_str() {
            "PLANNED" | "DISCOVERED" => (DonorState::Discovered, None),
            "CLONED" | "INVENTORIED" => (DonorState::Registered, None),
            "DEPENDENCY_CENSUSED" | "BUILD_CENSUSED" | "SOURCE_CENSUSED" | "TECHNOLOGY_MAPPED" => {
                (DonorState::Censused, None)
            }
            "ABSORPTION_PLANNED" => (DonorState::Specified, None),
            "PARTIALLY_ABSORBED" => (DonorState::NativeShadow, None),
            "ABSORBED" => (DonorState::ParityProven, None),
            "REMOVABLE" => (DonorState::Cutover, None),
            "REFERENCE_RETAINED" => (
                DonorState::Registered,
                Some((
                    ExceptionKind::Rejected,
                    "legacy REFERENCE_RETAINED: studied, never adopted".to_string(),
                )),
            ),
            _ => (DonorState::Discovered, None),
        };
        let mut caps = Vec::new();
        for c in d.strings("capabilities") {
            let future = c.contains("(future)");
            let clean = c.replace("(future)", "").trim().to_string();
            let owner = clean.split("::").next().map(str::to_string);
            caps.push(LegacyCapability {
                key: clean.replace("::", "-"),
                required: true,
                spec: String::new(),
                native_path: None,
                owner: if future { None } else { owner },
                proofs: Vec::new(),
            });
        }
        let dir = d.text("dir");
        donors.push(LegacyDonor {
            id: format!("{}-{}", id, repo.rsplit('/').next().unwrap_or(&repo)),
            name: repo.rsplit('/').next().unwrap_or(&repo).to_string(),
            origin: d.text("upstream"),
            license: d.text("license"),
            status: vec![
                ("chronica.lifecycle".into(), lifecycle.clone()),
                ("chronica.acquisition".into(), d.text("acquisition")),
                ("chronica.license_state".into(), d.text("license_state")),
            ],
            claimed: Some(claimed),
            exception,
            source_paths: if dir.is_empty() {
                Vec::new()
            } else {
                vec![format!(".atlas/temporary/donors/{dir}")]
            },
            capabilities: caps,
            names: vec![repo.rsplit('/').next().unwrap_or("").to_string()],
            provenance: format!("legacy:{CHRONICA_INVENTORY}#{id}"),
        });
    }
    Some(("chronica".into(), donors, vec![CHRONICA_INVENTORY.into()]))
}

fn fi_game(files: &Files) -> Option<Dialect> {
    let v = read_toml(files, CHRONICA_INVENTORY)?;
    if !v.text("schema").starts_with("fi.") {
        return None;
    }
    let absorption_file = ".atlas/census/absorption.toml";
    let absorption = read_toml(files, absorption_file);
    let mut donors = Vec::new();
    for d in v.items_of("donor") {
        let id = d.text("id");
        let absorbed = d.text("absorption");
        let census = d.text("census_status");
        let claimed = match (absorbed.as_str(), census.as_str()) {
            ("EXTINCT", _) => DonorState::Extinct,
            (_, "SEMANTIC") | (_, "STRUCTURAL") => DonorState::Censused,
            _ => DonorState::Registered,
        };
        let mut caps = Vec::new();
        if let Some(a) = &absorption {
            for c in a.items_of("capability") {
                if !c.strings("donors").contains(&id) {
                    continue;
                }
                let mut proofs: Vec<(ProofKind, String)> = c
                    .strings("tests")
                    .iter()
                    .filter_map(|t| locator(t))
                    .map(|l| (ProofKind::Regression, l))
                    .collect();
                proofs.extend(
                    c.strings("differential")
                        .iter()
                        .filter_map(|t| locator(t))
                        .map(|l| (ProofKind::Parity, l)),
                );
                caps.push(LegacyCapability {
                    key: c.text("id"),
                    required: true,
                    spec: c.text("name"),
                    native_path: c.strings("native").into_iter().next(),
                    owner: None,
                    proofs,
                });
            }
        }
        let name = id
            .split_once('-')
            .map(|(_, n)| n.to_string())
            .unwrap_or(id.clone());
        donors.push(LegacyDonor {
            id: id.clone(),
            name: name.clone(),
            origin: d.text("upstream"),
            license: d.text("license"),
            status: vec![
                ("fi.absorption".into(), absorbed),
                ("fi.census_status".into(), census),
            ],
            claimed: Some(claimed),
            exception: None,
            source_paths: vec![format!(".atlas/temporary/donors/{id}/source")],
            capabilities: caps,
            names: vec![name],
            provenance: format!("legacy:{CHRONICA_INVENTORY}#{id}"),
        });
    }
    let mut consumed = vec![CHRONICA_INVENTORY.to_string()];
    if absorption.is_some() {
        consumed.push(absorption_file.into());
    }
    Some(("fi-game".into(), donors, consumed))
}

fn atlas_studio(files: &Files) -> Option<Dialect> {
    let path = ".atlas/references/donor-corpus.toml";
    let v = read_toml(files, path)?;
    let mut donors = Vec::new();
    for d in v.items_of("donor") {
        let id = d.text("id");
        let ingestion = d.text("ingestion_status");
        let decision = d.text("decision_status");
        let claimed = match ingestion.as_str() {
            "EXTINCT" => DonorState::Extinct,
            "REMOTE_CENSUSED" => DonorState::Censused,
            _ => DonorState::Registered,
        };
        let exception = matches!(decision.as_str(), "REFERENCE_ONLY").then(|| {
            (
                ExceptionKind::Rejected,
                "legacy decision REFERENCE_ONLY: studied, never adopted".to_string(),
            )
        });
        let caps = d
            .strings("technology_domains")
            .into_iter()
            .map(|t| LegacyCapability {
                key: t,
                required: true,
                ..LegacyCapability::default()
            })
            .collect();
        let origin = if d.text("resolved_url").is_empty() {
            d.text("repository")
        } else {
            d.text("resolved_url")
        };
        let license = d
            .strings("license")
            .iter()
            .map(|l| format!("file:{l}"))
            .collect::<Vec<_>>()
            .join(" ");
        let source_paths = donor_dirs(files, &[".atlas/temporary/donors"], &id);
        donors.push(LegacyDonor {
            id: id.clone(),
            name: d.text("name"),
            origin,
            license,
            status: vec![
                ("atlas.ingestion_status".into(), ingestion),
                ("atlas.decision_status".into(), decision),
                (
                    "atlas.implementation_status".into(),
                    d.text("implementation_status"),
                ),
                ("atlas.storage_state".into(), d.text("storage_state")),
            ],
            claimed: Some(claimed),
            exception,
            source_paths,
            capabilities: caps,
            names: vec![d.text("name"), id],
            provenance: format!("legacy:{path}#{}", d.text("id")),
        });
    }
    Some(("atlas-studio".into(), donors, vec![path.into()]))
}

fn universal_connector(files: &Files) -> Option<Dialect> {
    let path = ".atlas/provenance/donor-inventory.json";
    let v = files.read(path).and_then(|t| json::parse(&t).ok())?;
    let mut donors = Vec::new();
    for d in v.items_of("donors") {
        let id = d.text("id");
        let status = d.text("status");
        let (claimed, exception) = match status.as_str() {
            "CANDIDATE" => (DonorState::Discovered, None),
            "CLONED" => (DonorState::Registered, None),
            "CENSUSED" => (DonorState::Censused, None),
            "MAPPED" => (DonorState::Specified, None),
            "ABSORBING" => (DonorState::NativeShadow, None),
            "ABSORBED" => (DonorState::ParityProven, None),
            "EXTINCT" => (DonorState::Extinct, None),
            "REJECTED" => (
                DonorState::Discovered,
                Some((
                    ExceptionKind::Rejected,
                    format!("legacy REJECTED: {}", d.text("rejection_reason")),
                )),
            ),
            _ => (DonorState::Discovered, None),
        };
        let caps = d
            .strings("target_capabilities")
            .into_iter()
            .map(|t| LegacyCapability {
                key: t,
                required: true,
                ..LegacyCapability::default()
            })
            .collect();
        let snap = d.text("snapshot_path");
        donors.push(LegacyDonor {
            id: id.clone(),
            name: d.text("name"),
            origin: d.text("source_url"),
            license: d.text("license"),
            status: vec![
                ("uc.status".into(), status),
                ("uc.runtime_dependency".into(), d.text("runtime_dependency")),
            ],
            claimed: Some(claimed),
            exception,
            source_paths: if snap.is_empty() {
                Vec::new()
            } else {
                vec![snap]
            },
            capabilities: caps,
            names: vec![id.clone(), d.text("name")],
            provenance: format!("legacy:{path}#{id}"),
        });
    }
    Some(("universal-connector".into(), donors, vec![path.into()]))
}

fn esellios(files: &Files) -> Option<Dialect> {
    let path = ".atlas/census/donors.toml";
    let v = read_toml(files, path)?;
    let mut donors = Vec::new();
    let mut consumed = vec![path.to_string()];
    for d in v.items_of("donor") {
        let id = d.text("id");
        let status = d.text("status");
        let (claimed, exception) = match status.as_str() {
            "REGISTERED" | "ACQUIRED" => (DonorState::Registered, None),
            "CENSUSED" => (DonorState::Censused, None),
            "DISPOSITIONED" => (DonorState::Specified, None),
            "ABSORBING" => (DonorState::NativeShadow, None),
            "EXTINCT" => (DonorState::Extinct, None),
            "REJECTED" => (
                DonorState::Registered,
                Some((ExceptionKind::Rejected, "legacy REJECTED".to_string())),
            ),
            _ => (DonorState::Discovered, None),
        };
        let cap_file = format!(".atlas/census/donors/{id}/capabilities.toml");
        let mut caps = Vec::new();
        if let Some(cv) = read_toml(files, &cap_file) {
            consumed.push(cap_file.clone());
            for c in cv.items_of("capability") {
                let disposition = c.text("disposition");
                caps.push(LegacyCapability {
                    key: c.text("id"),
                    required: !matches!(
                        disposition.as_str(),
                        "REFERENCE" | "REJECT" | "OUT_OF_SCOPE" | "DUPLICATED" | "LICENSE_BLOCKED"
                    ),
                    spec: c.text("description"),
                    native_path: c.strings("native").into_iter().next(),
                    owner: None,
                    proofs: c
                        .strings("native_tests")
                        .iter()
                        .filter_map(|t| locator(t))
                        .map(|l| (ProofKind::Regression, l))
                        .collect(),
                });
            }
        }
        donors.push(LegacyDonor {
            id: id.clone(),
            name: id.clone(),
            origin: d.text("upstream"),
            license: d.text("license_hint"),
            status: vec![
                ("esellios.status".into(), status),
                ("esellios.disposition".into(), d.text("disposition")),
            ],
            claimed: Some(claimed),
            exception,
            source_paths: vec![format!(".atlas/temporary/donors/{id}/source")],
            capabilities: caps,
            names: vec![id.clone()],
            provenance: format!("legacy:{path}#{id}"),
        });
    }
    Some(("esellios".into(), donors, consumed))
}

fn mechatron(files: &Files) -> Option<Dialect> {
    let path = "research/registry/donors.toml";
    let v = read_toml(files, path)?;
    let mut donors = Vec::new();
    for d in v.items_of("donor") {
        let id = d.text("id");
        let name = d.text("name");
        let status = d.text("status");
        let (claimed, exception) = match status.as_str() {
            "DISCOVERED" => (DonorState::Discovered, None),
            "REGISTERED" | "CLONED" => (DonorState::Registered, None),
            "CENSUSED" | "MAPPED" => (DonorState::Censused, None),
            "PARTIALLY_EXTRACTED" => (DonorState::Specified, None),
            "NATIVE_REPLACEMENT_STARTED" | "NATIVE_EQUIVALENCE_PARTIAL" => {
                (DonorState::NativeShadow, None)
            }
            "ABSORBED" => (DonorState::ParityProven, None),
            "EXTINCTION_READY" => (DonorState::Cutover, None),
            "EXTINCT" => (DonorState::Extinct, None),
            "REFERENCE_ONLY" => (
                DonorState::Registered,
                Some((ExceptionKind::Rejected, "legacy REFERENCE_ONLY".to_string())),
            ),
            _ => (DonorState::Discovered, None),
        };
        let evidence: Vec<String> = d
            .strings("native_evidence")
            .into_iter()
            .chain(d.strings("equivalence_evidence"))
            .collect();
        let native = d.strings("native_implementation");
        let mut caps = Vec::new();
        if !evidence.is_empty() || !native.is_empty() {
            caps.push(LegacyCapability {
                key: "equivalence".into(),
                required: true,
                spec: d.text("equivalence_scope"),
                native_path: native
                    .first()
                    .and_then(|n| n.split_whitespace().next())
                    .map(|p| p.split("::").next().unwrap_or(p).to_string()),
                owner: None,
                proofs: evidence
                    .iter()
                    .filter_map(|e| locator(e))
                    .map(|l| (ProofKind::Parity, l))
                    .collect(),
            });
        }
        let clone_dir = d.text("clone_dir");
        donors.push(LegacyDonor {
            id: format!("{id}-{}", slug(&name)),
            name: name.clone(),
            origin: d.text("repository"),
            license: d.text("license_observed"),
            status: vec![
                ("mechatron.status".into(), status),
                (
                    "mechatron.dependency_class".into(),
                    d.text("dependency_class"),
                ),
            ],
            claimed: Some(claimed),
            exception,
            source_paths: if clone_dir.is_empty() {
                Vec::new()
            } else {
                vec![clone_dir]
            },
            capabilities: caps,
            names: vec![
                name.to_ascii_lowercase(),
                d.text("repository")
                    .rsplit('/')
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase(),
            ],
            provenance: format!("legacy:{path}#{id}"),
        });
    }
    Some(("mechatron".into(), donors, vec![path.into()]))
}

/// Tracked directories under the donor roots whose final segment is `id`.
fn donor_dirs(files: &Files, roots: &[&str], id: &str) -> Vec<String> {
    let mut out = BTreeSet::new();
    for r in roots {
        for p in files.under(r) {
            let rest = &p[r.len() + 1..];
            let segs: Vec<&str> = rest.split('/').collect();
            for depth in 0..segs.len().saturating_sub(1).min(2) {
                if segs[depth] == id {
                    out.insert(format!("{r}/{}", segs[..=depth].join("/")));
                }
            }
        }
    }
    out.into_iter().collect()
}

/// Donor source held in the tree that no legacy record claims: registered as its own donor so
/// that it can never hide.
pub fn unadmitted_sources(files: &Files, known: &[LegacyDonor]) -> Vec<LegacyDonor> {
    let claimed: Vec<&str> = known
        .iter()
        .flat_map(|d| d.source_paths.iter().map(String::as_str))
        .collect();
    let mut roots: BTreeMap<String, usize> = BTreeMap::new();
    for base in [
        ".atlas/temporary/donors",
        "temporary/donors",
        "vendor",
        "third_party",
    ] {
        for p in files.under(base) {
            if claimed
                .iter()
                .any(|c| p.starts_with(&format!("{c}/")) || p == c)
            {
                continue;
            }
            let rest = &p[base.len() + 1..];
            let segs: Vec<&str> = rest.split('/').collect();
            if segs.len() < 2 {
                continue; // a loose file (README) is not donor source
            }
            // A category directory holds donors; a donor directory holds files directly.
            let first_has_files = files
                .under(&format!("{base}/{}", segs[0]))
                .any(|f| f[base.len() + segs[0].len() + 2..].split('/').count() == 1);
            let root = if segs[1] == "source" || first_has_files || segs.len() < 3 {
                format!("{base}/{}", segs[0])
            } else {
                format!("{base}/{}/{}", segs[0], segs[1])
            };
            *roots.entry(root).or_default() += 1;
        }
    }
    roots
        .into_iter()
        .map(|(root, n)| {
            let name = root.rsplit('/').next().unwrap_or(&root).to_string();
            LegacyDonor {
                id: format!("unadmitted-{}", slug(&name)),
                name: name.clone(),
                origin: String::new(),
                license: String::new(),
                status: vec![("tree.unadmitted_files".into(), n.to_string())],
                claimed: Some(DonorState::Registered),
                exception: None,
                source_paths: vec![root.clone()],
                capabilities: Vec::new(),
                names: vec![name],
                provenance: format!("tree:{root}"),
            }
        })
        .collect()
}
