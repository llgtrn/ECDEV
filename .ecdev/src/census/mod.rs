//! The census observes, from committed content only, every way foreign technology participates
//! in the repository: manifest dependencies (Cargo, npm, Python), source imports, native links,
//! process invocations, references to held donor source, and the locked dependency closure
//! (`Cargo.lock`, `package-lock.json`, `pnpm-lock.yaml`: a donor package locked only
//! transitively is still linked, through the direct dependency that pulls it in).
//! Observations are facts; the extinction verifier and the metrics judge them.

pub mod cargo;
pub mod js;
pub mod lock;
pub mod shell;
pub mod sources;

use crate::declare::Declaration;
use crate::repository::files::Files;
use crate::schema::{Ecosystem, Scope};
use std::collections::{BTreeMap, BTreeSet};

/// How an observation was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Via {
    Manifest,
    Import,
    Link,
    Process,
    SourceReference,
    /// A lockfile entry reached only through other external packages; `ident` is the path that
    /// pulls it in (`postgres 0.19.9 → … → sha2 0.11.0`).
    Lock,
}

impl Via {
    pub fn wire(self) -> &'static str {
        match self {
            Via::Manifest => "MANIFEST",
            Via::Import => "IMPORT",
            Via::Link => "LINK",
            Via::Process => "PROCESS",
            Via::SourceReference => "SOURCE_REFERENCE",
            Via::Lock => "LOCK",
        }
    }
}

/// One observed use of something foreign.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Observation {
    /// The file that makes the reference.
    pub file: String,
    pub ecosystem: Ecosystem,
    /// Package, library, program or donor-source path referenced.
    pub name: String,
    /// The identifier source code uses for it (Cargo renames), else the name.
    pub ident: String,
    pub scope: Scope,
    pub via: Via,
}

/// An internal workspace package.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Member {
    pub dir: String,
    pub package: String,
}

#[derive(Clone, Debug, Default)]
pub struct Census {
    pub members: Vec<Member>,
    /// Workspace-internal dependency edges: (from dir, to dir, scope).
    pub internal: BTreeSet<(String, String, Scope)>,
    /// Every foreign reference.
    pub observations: BTreeSet<Observation>,
    /// Source-level crate identifiers used by each `.rs` file (`ident::` paths, `use`, `extern crate`).
    pub imports: BTreeMap<String, BTreeSet<String>>,
    /// External packages in the locked closure (`Cargo.lock` entries with a source).
    pub closure: BTreeSet<String>,
    /// Files that could not be read or parsed.
    pub unreadable: Vec<String>,
}

impl Census {
    pub fn external(&self) -> impl Iterator<Item = &Observation> {
        self.observations.iter().filter(|o| {
            matches!(
                o.via,
                Via::Manifest | Via::Import | Via::Link | Via::Process
            )
        })
    }
}

/// Paths that are never scanned as repository technology: held donor source (judged separately
/// as resident source) and the legacy knowledge tree.
pub fn excluded_roots(d: &Declaration) -> Vec<String> {
    let mut v: Vec<String> = d
        .donors
        .iter()
        .flat_map(|dn| dn.source_paths.iter().cloned())
        .map(|p| p.trim_end_matches('/').to_string())
        .collect();
    v.sort();
    v.dedup();
    v
}

pub fn is_excluded(path: &str, excluded: &[String]) -> bool {
    excluded
        .iter()
        .any(|e| path == e || path.starts_with(&format!("{e}/")))
}

/// Scope of a reference made by `file`: tests (`tests/`, `benches/`, `__tests__/`, `*_test.rs`,
/// `*.test.*`, `*.spec.*`) are TEST, tools and CI are BUILD, else RUNTIME.
pub fn scope_of_file(file: &str) -> Scope {
    let segs: Vec<&str> = file.split('/').collect();
    let name = segs.last().copied().unwrap_or("");
    let test_file =
        name.ends_with("_test.rs") || name.contains(".test.") || name.contains(".spec.");
    if segs.contains(&"tests")
        || segs.contains(&"benches")
        || segs.contains(&"__tests__")
        || test_file
    {
        Scope::Test
    } else if name == "build.rs" || segs[0] == "tools" || segs[0] == ".github" {
        Scope::Build
    } else {
        Scope::Runtime
    }
}

pub fn run(files: &Files, d: &Declaration) -> Census {
    let excluded = excluded_roots(d);
    let mut c = Census::default();
    cargo::observe(files, &excluded, &mut c);
    sources::observe(files, d, &excluded, &mut c);
    // A donor package still in a lockfile, even only transitively, is still built and linked.
    // Only donors' packages are reported: every other transitive package is the business of
    // the direct dependency that pulls it in, not a technology of its own.
    let donor_packages: BTreeSet<(Ecosystem, String)> = d
        .donors
        .iter()
        .flat_map(|dn| dn.packages.iter())
        .map(|p| (p.ecosystem, package_key(p.ecosystem, &p.name)))
        .collect();
    for l in lock::observe(files, &excluded, &c) {
        if donor_packages.contains(&(l.ecosystem, package_key(l.ecosystem, &l.name))) {
            c.observations.insert(Observation {
                file: l.manifest.clone(),
                ecosystem: l.ecosystem,
                name: l.name.clone(),
                ident: format!("{} (in {})", l.chain.join(" → "), l.lockfile),
                scope: l.scope,
                via: Via::Lock,
            });
        }
    }
    c
}

/// A package name as donors are matched by it.
pub fn package_key(eco: Ecosystem, name: &str) -> String {
    match eco {
        Ecosystem::Python => name.to_ascii_lowercase().replace(['_', '.'], "-"),
        Ecosystem::Cargo => name.replace('_', "-"),
        _ => name.to_string(),
    }
}
