//! Registration from real package metadata. A donor the census discovered is REGISTERED once its
//! origin and licence are recorded; the licence is read from the package itself, never typed in:
//! for Cargo, the `license` of every version `Cargo.lock` pins, from the local registry sources
//! (`$CARGO_HOME/registry/src`); for npm, the installed `node_modules/<name>/package.json`.
//! A package whose metadata is not available locally stays unregistered, with the reason.

use crate::declare::{Donor, Package};
use crate::formats::{json, toml, Value};
use crate::repository::files::Files;
use crate::schema::DonorState;
use crate::schema::Ecosystem;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Where a licence came from, or why none could be read.
#[derive(Clone, Debug, PartialEq)]
pub enum Licence {
    Found {
        licence: String,
        /// The manifests read, as provenance.
        sources: Vec<String>,
    },
    Unavailable(String),
}

/// Package name -> versions pinned by any tracked `Cargo.lock` (registry packages only).
pub fn locked_versions(files: &Files) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for lock in files.paths.iter().filter(|p| {
        p.ends_with("Cargo.lock") && !p.starts_with(".ynventa/") && !p.starts_with(".atlas/")
    }) {
        let Some(v) = files.read(lock).and_then(|t| toml::parse(&t).ok()) else {
            continue;
        };
        for p in v.get("package").map(Value::items).unwrap_or(&[]) {
            if p.str("source").is_some_and(|s| s.starts_with("registry+")) {
                if let (Some(n), Some(ver)) = (p.str("name"), p.str("version")) {
                    out.entry(n.to_string())
                        .or_default()
                        .insert(ver.to_string());
                }
            }
        }
    }
    out
}

pub fn cargo_home() -> Option<PathBuf> {
    std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo")))
}

/// The licence of a Cargo package, from the manifests of its pinned versions.
pub fn cargo_licence(home: &Path, name: &str, versions: Option<&BTreeSet<String>>) -> Licence {
    let Some(versions) = versions.filter(|v| !v.is_empty()) else {
        return Licence::Unavailable(format!(
            "`{name}` is not pinned from a registry in Cargo.lock"
        ));
    };
    let Ok(indexes) = std::fs::read_dir(home.join("registry/src")) else {
        return Licence::Unavailable("no local Cargo registry sources (run `cargo fetch`)".into());
    };
    let indexes: Vec<PathBuf> = indexes.filter_map(Result::ok).map(|e| e.path()).collect();
    let mut licences = BTreeSet::new();
    let mut sources = Vec::new();
    for ver in versions {
        let dir = format!("{name}-{ver}");
        let Some(text) = indexes
            .iter()
            .find_map(|i| std::fs::read_to_string(i.join(&dir).join("Cargo.toml")).ok())
        else {
            return Licence::Unavailable(format!(
                "{dir} is not in the local registry (run `cargo fetch`)"
            ));
        };
        let Ok(m) = toml::parse(&text) else {
            return Licence::Unavailable(format!("{dir}/Cargo.toml does not parse"));
        };
        match m.at("package.license").and_then(Value::as_str) {
            Some(l) if !l.trim().is_empty() => {
                licences.insert(l.trim().to_string());
                sources.push(format!("registry:{dir}/Cargo.toml"));
            }
            _ => {
                return Licence::Unavailable(format!(
                    "{dir} declares no SPDX `license` (only a licence file, or none)"
                ))
            }
        }
    }
    Licence::Found {
        licence: licences.into_iter().collect::<Vec<_>>().join("; "),
        sources,
    }
}

/// The licence of an npm package: from a tracked `package-lock.json` (v2/v3 lockfiles record the
/// licence of every pinned package), else from the installed `node_modules/<name>/package.json`.
pub fn npm_licence(root: &Path, files: &Files, name: &str) -> Licence {
    let key = format!("node_modules/{name}");
    for lock in files.paths.iter().filter(|p| {
        p.ends_with("package-lock.json") && !p.starts_with(".atlas/") && !p.starts_with(".ynventa/")
    }) {
        let Some(v) = files.read(lock).and_then(|t| json::parse(&t).ok()) else {
            continue;
        };
        if let Some(l) = v
            .get("packages")
            .and_then(|p| p.get(&key))
            .and_then(|p| p.str("license"))
            .filter(|l| !l.trim().is_empty())
        {
            return Licence::Found {
                licence: l.trim().to_string(),
                sources: vec![format!("npm:{lock}#{key}")],
            };
        }
    }
    let rel = format!("{key}/package.json");
    let Ok(text) = std::fs::read_to_string(root.join(&rel)) else {
        return Licence::Unavailable(format!(
            "`{name}` is in no package-lock.json and {rel} is not installed"
        ));
    };
    match json::parse(&text)
        .ok()
        .as_ref()
        .and_then(|v| v.str("license"))
    {
        Some(l) if !l.trim().is_empty() => Licence::Found {
            licence: l.trim().to_string(),
            sources: vec![format!("npm:{rel}")],
        },
        _ => Licence::Unavailable(format!("{rel} declares no `license`")),
    }
}

/// The licence of a donor: every package it names must resolve; licences of several packages
/// are joined. Programs and other ecosystems have no local metadata to read.
pub fn donor_licence(
    root: &Path,
    files: &Files,
    home: Option<&Path>,
    locked: &BTreeMap<String, BTreeSet<String>>,
    dn: &Donor,
) -> Licence {
    if dn.packages.is_empty() {
        return Licence::Unavailable(
            "no packages: a program or source donor; record its licence by hand".into(),
        );
    }
    let mut licences = BTreeSet::new();
    let mut sources = Vec::new();
    for p in &dn.packages {
        let l = match p.ecosystem {
            Ecosystem::Cargo => match home {
                Some(h) => cargo_licence(h, &p.name, locked.get(&p.name)),
                None => Licence::Unavailable("no CARGO_HOME or HOME".into()),
            },
            Ecosystem::Npm => npm_licence(root, files, &p.name),
            other => Licence::Unavailable(format!("no local metadata reader for {}", other.wire())),
        };
        match l {
            Licence::Found {
                licence,
                sources: s,
            } => {
                licences.insert(licence);
                sources.extend(s);
            }
            u => return u,
        }
    }
    Licence::Found {
        licence: licences.into_iter().collect::<Vec<_>>().join("; "),
        sources,
    }
}

/// The donor record for an external the census observed but nothing declares: DISCOVERED, with the
/// registry origin where one follows from the ecosystem and the observing files as provenance.
/// Registration stays an explicit, evidenced step.
pub fn discovered(eco: Ecosystem, name: &str, in_files: impl IntoIterator<Item = String>) -> Donor {
    let origin = match eco {
        Ecosystem::Cargo => format!("https://crates.io/crates/{name}"),
        Ecosystem::Npm => format!("https://www.npmjs.com/package/{name}"),
        Ecosystem::Python => format!("https://pypi.org/project/{name}"),
        Ecosystem::Native => String::new(),
    };
    let mut provenance: Vec<String> = in_files
        .into_iter()
        .map(|f| format!("census:{f}"))
        .collect();
    provenance.sort();
    provenance.dedup();
    Donor {
        key: format!(
            "{}-{}",
            eco.wire().to_ascii_lowercase(),
            crate::migration::atlas::slug(name)
        ),
        name: name.to_string(),
        origin,
        license: String::new(),
        // An importer cannot verify a licence: observed externals enter as DISCOVERED.
        claimed: DonorState::Discovered,
        exception: None,
        packages: vec![Package {
            ecosystem: eco,
            name: name.to_string(),
        }],
        source_paths: Vec::new(),
        capabilities: Vec::new(),
        cutover: None,
        provenance,
    }
}

/// An external program the code runs as a process, and the project that provides it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Program {
    pub program: &'static str,
    pub project: &'static str,
    pub origin: &'static str,
    pub licence: &'static str,
}

/// A project that provides programs: (name, origin, SPDX licence).
type Project = (&'static str, &'static str, &'static str);

const COREUTILS: Project = (
    "coreutils",
    "https://www.gnu.org/software/coreutils",
    "GPL-3.0-or-later",
);
const GREP: Project = (
    "grep",
    "https://www.gnu.org/software/grep",
    "GPL-3.0-or-later",
);
const SED: Project = (
    "sed",
    "https://www.gnu.org/software/sed",
    "GPL-3.0-or-later",
);
const FINDUTILS: Project = (
    "findutils",
    "https://www.gnu.org/software/findutils",
    "GPL-3.0-or-later",
);
const DIFFUTILS: Project = (
    "diffutils",
    "https://www.gnu.org/software/diffutils",
    "GPL-3.0-or-later",
);
const MAKE: Project = (
    "make",
    "https://www.gnu.org/software/make",
    "GPL-3.0-or-later",
);
const TAR: Project = (
    "tar",
    "https://www.gnu.org/software/tar",
    "GPL-3.0-or-later",
);
const PROCPS: Project = (
    "procps-ng",
    "https://gitlab.com/procps-ng/procps",
    "GPL-2.0-or-later",
);
const UTIL_LINUX: Project = (
    "util-linux",
    "https://github.com/util-linux/util-linux",
    "GPL-2.0-or-later",
);
const NPM: Project = ("npm", "https://github.com/npm/cli", "Artistic-2.0");

const fn prog(program: &'static str, project: Project) -> Program {
    Program {
        program,
        project: project.0,
        origin: project.1,
        licence: project.2,
    }
}

/// Programs whose providing project is the same on every platform Chronica builds on (Linux with
/// the GNU userland). A program shipped by different projects on different platforms or
/// distributions (`kill`: procps-ng or util-linux; `awk`: gawk or mawk; `cc`: GCC or Clang;
/// `which`, `hostname`, `open`) is deliberately absent: its donor cannot be named without
/// choosing a platform.
pub const PROGRAMS: &[Program] = &[
    prog("basename", COREUTILS),
    prog("cat", COREUTILS),
    prog("chmod", COREUTILS),
    prog("cp", COREUTILS),
    prog("cut", COREUTILS),
    prog("date", COREUTILS),
    prog("df", COREUTILS),
    prog("dirname", COREUTILS),
    prog("du", COREUTILS),
    prog("head", COREUTILS),
    prog("ln", COREUTILS),
    prog("ls", COREUTILS),
    prog("mkdir", COREUTILS),
    prog("mkfifo", COREUTILS),
    prog("mktemp", COREUTILS),
    prog("mv", COREUTILS),
    prog("nohup", COREUTILS),
    prog("nproc", COREUTILS),
    prog("realpath", COREUTILS),
    prog("rm", COREUTILS),
    prog("seq", COREUTILS),
    prog("sha256sum", COREUTILS),
    prog("sleep", COREUTILS),
    prog("sort", COREUTILS),
    prog("stat", COREUTILS),
    prog("tail", COREUTILS),
    prog("tee", COREUTILS),
    prog("timeout", COREUTILS),
    prog("touch", COREUTILS),
    prog("tr", COREUTILS),
    prog("uniq", COREUTILS),
    prog("wc", COREUTILS),
    prog("grep", GREP),
    prog("sed", SED),
    prog("find", FINDUTILS),
    prog("xargs", FINDUTILS),
    prog("diff", DIFFUTILS),
    prog("make", MAKE),
    prog("tar", TAR),
    prog("ps", PROCPS),
    prog(
        "ip",
        (
            "iproute2",
            "https://git.kernel.org/pub/scm/network/iproute2/iproute2.git",
            "GPL-2.0-or-later",
        ),
    ),
    prog("mount", UTIL_LINUX),
    prog("nsenter", UTIL_LINUX),
    prog("unshare", UTIL_LINUX),
    prog("curl", ("curl", "https://github.com/curl/curl", "curl")),
    prog("node", ("nodejs", "https://github.com/nodejs/node", "MIT")),
    prog("npm", NPM),
    prog("npx", NPM),
    prog("pnpm", ("pnpm", "https://github.com/pnpm/pnpm", "MIT")),
    prog("git", ("git", "https://github.com/git/git", "GPL-2.0-only")),
    prog("gh", ("gh", "https://github.com/cli/cli", "MIT")),
    prog("jq", ("jq", "https://github.com/jqlang/jq", "MIT")),
    prog(
        "cmake",
        (
            "cmake",
            "https://gitlab.kitware.com/cmake/cmake",
            "BSD-3-Clause",
        ),
    ),
    prog(
        "python3",
        ("cpython", "https://github.com/python/cpython", "PSF-2.0"),
    ),
    prog(
        "openssl",
        (
            "openssl",
            "https://github.com/openssl/openssl",
            "Apache-2.0",
        ),
    ),
    prog("sqlite3", ("sqlite", "https://sqlite.org", "blessing")),
    prog(
        "docker",
        ("docker-cli", "https://github.com/docker/cli", "Apache-2.0"),
    ),
    prog(
        "podman",
        (
            "podman",
            "https://github.com/containers/podman",
            "Apache-2.0",
        ),
    ),
    prog(
        "sudo",
        ("sudo", "https://github.com/sudo-project/sudo", "ISC"),
    ),
];

pub fn program(name: &str) -> Option<&'static Program> {
    PROGRAMS.iter().find(|p| p.program == name)
}

/// The project every package of a DISCOVERED program donor comes from, when they agree.
pub fn program_project(dn: &Donor) -> Option<&'static Program> {
    let mut found: Option<&'static Program> = None;
    for p in &dn.packages {
        let prog = (p.ecosystem == Ecosystem::Native)
            .then(|| program(&p.name))
            .flatten()?;
        match found {
            Some(f) if f.project != prog.project => return None,
            _ => found = Some(prog),
        }
    }
    found
}

/// Collapses DISCOVERED program donors into one donor per providing project (a donor is the
/// external project; its programs are what it provides), registered with the project's origin
/// and licence. Returns (old key, new key) for every donor registered this way; the keys are equal
/// when the donor already carried its project's key.
pub fn consolidate_programs(donors: &mut Vec<Donor>) -> Vec<(String, String)> {
    let mut moves = Vec::new();
    let mut i = 0;
    while i < donors.len() {
        let dn = &donors[i];
        let Some(prog) = (dn.claimed <= DonorState::Discovered && dn.capabilities.is_empty())
            .then(|| program_project(dn))
            .flatten()
        else {
            i += 1;
            continue;
        };
        let key = format!("native-{}", prog.project);
        if dn.key == key {
            let dn = &mut donors[i];
            dn.origin = prog.origin.into();
            dn.license = prog.licence.into();
            dn.claimed = DonorState::Registered;
            moves.push((key.clone(), key));
            i += 1;
            continue;
        }
        let old = donors.remove(i);
        moves.push((old.key.clone(), key.clone()));
        match donors.iter_mut().find(|d| d.key == key) {
            Some(target) => {
                target.packages.extend(old.packages);
                target.provenance.extend(old.provenance);
            }
            None => donors.push(Donor {
                key: key.clone(),
                name: prog.project.into(),
                origin: prog.origin.into(),
                license: prog.licence.into(),
                claimed: DonorState::Registered,
                exception: None,
                packages: old.packages,
                source_paths: Vec::new(),
                capabilities: Vec::new(),
                cutover: None,
                provenance: old.provenance,
            }),
        }
        // Positions shifted; restart the scan (the list is small).
        i = 0;
    }
    for d in donors.iter_mut() {
        d.packages
            .sort_by(|a, b| (a.ecosystem, &a.name).cmp(&(b.ecosystem, &b.name)));
        d.packages.dedup();
        d.provenance.sort();
        d.provenance.dedup();
    }
    donors.sort_by(|a, b| a.key.cmp(&b.key));
    moves
}
