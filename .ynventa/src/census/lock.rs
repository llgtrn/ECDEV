//! Lockfiles: the resolved dependency closure. A manifest names what a package asks for; a
//! lockfile records everything that is actually built and linked, transitive packages
//! included. `Cargo.lock`, `package-lock.json` (v1–v3) and `pnpm-lock.yaml` (v5–v9) are read
//! into one graph each; every external package that no workspace package depends on directly
//! is reported with the shortest path that pulls it in, from the manifest that asks for the
//! first external package on that path.

use super::{is_excluded, Census};
use crate::formats::{json, toml, Value};
use crate::repository::files::Files;
use crate::schema::{Ecosystem, Scope};
use std::collections::{BTreeMap, VecDeque};

/// A package in a lockfile reached only through other external packages.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Locked {
    pub lockfile: String,
    pub ecosystem: Ecosystem,
    pub name: String,
    pub version: String,
    /// The manifest of the workspace package at the start of the path.
    pub manifest: String,
    /// LINKED through a normal dependency, BUILD or TEST through a build or dev dependency.
    pub scope: Scope,
    /// `name version` from the direct dependency to this package.
    pub chain: Vec<String>,
}

impl Locked {
    /// The package that pulls this one in directly (its parent on the path).
    pub fn via(&self) -> &str {
        let n = self.chain.len();
        if n >= 2 {
            &self.chain[n - 2]
        } else {
            "(no workspace path)"
        }
    }
}

#[derive(Default)]
struct Graph {
    /// (name, version, internal manifest if the package is a workspace package).
    nodes: Vec<(String, String, Option<String>)>,
    /// Dependencies of each node; an internal node's edges carry the manifest scope.
    deps: Vec<Vec<(usize, Scope)>>,
}

impl Graph {
    fn add(&mut self, name: &str, version: &str, manifest: Option<String>) -> usize {
        self.nodes
            .push((name.to_string(), version.to_string(), manifest));
        self.deps.push(Vec::new());
        self.nodes.len() - 1
    }
    fn edge(&mut self, from: usize, to: usize, scope: Scope) {
        if from != to && !self.deps[from].contains(&(to, scope)) {
            self.deps[from].push((to, scope));
        }
    }

    /// Breadth-first from every workspace package, normal dependencies first, then build, then
    /// dev: each external package gets the shortest path of the strongest kind. Packages that
    /// are direct dependencies of a workspace package are skipped (their manifests say so).
    fn transitive(&self, lockfile: &str, eco: Ecosystem) -> Vec<Locked> {
        let n = self.nodes.len();
        let mut parent: Vec<Option<usize>> = vec![None; n];
        let mut scope_of: Vec<Option<Scope>> = vec![None; n];
        let internal: Vec<usize> = (0..n).filter(|i| self.nodes[*i].2.is_some()).collect();
        let class = |s: Scope| match s {
            Scope::Build => 1,
            Scope::Test => 2,
            _ => 0,
        };
        let mut direct = vec![false; n];
        for pass in 0..3 {
            let mut queue: VecDeque<usize> = VecDeque::new();
            for &i in &internal {
                for &(d, s) in &self.deps[i] {
                    if self.nodes[d].2.is_some() || class(s) != pass {
                        continue;
                    }
                    direct[d] = true;
                    if scope_of[d].is_none() {
                        scope_of[d] = Some(match pass {
                            0 => Scope::Linked,
                            1 => Scope::Build,
                            _ => Scope::Test,
                        });
                        parent[d] = Some(i);
                        queue.push_back(d);
                    }
                }
            }
            while let Some(x) = queue.pop_front() {
                for &(d, _) in &self.deps[x] {
                    if self.nodes[d].2.is_none() && scope_of[d].is_none() {
                        scope_of[d] = scope_of[x];
                        parent[d] = Some(x);
                        queue.push_back(d);
                    }
                }
            }
        }
        let label = |i: usize| {
            let (name, version, _) = &self.nodes[i];
            if version.is_empty() {
                name.clone()
            } else {
                format!("{name} {version}")
            }
        };
        let mut out = Vec::new();
        for i in 0..n {
            let (name, version, manifest) = &self.nodes[i];
            if manifest.is_some() || direct[i] {
                continue;
            }
            let mut chain = vec![label(i)];
            let mut at = i;
            let mut root = None;
            while let Some(p) = parent[at] {
                if self.nodes[p].2.is_some() {
                    root = self.nodes[p].2.clone();
                    break;
                }
                chain.push(label(p));
                at = p;
            }
            chain.reverse();
            out.push(Locked {
                lockfile: lockfile.to_string(),
                ecosystem: eco,
                name: name.clone(),
                version: version.clone(),
                manifest: root.unwrap_or_else(|| lockfile.to_string()),
                scope: scope_of[i].unwrap_or(Scope::Linked),
                chain,
            });
        }
        out
    }
}

fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

fn under(dir: &str, rel: &str) -> String {
    match (dir.is_empty(), rel.is_empty() || rel == ".") {
        (true, true) => String::new(),
        (true, false) => rel.to_string(),
        (false, true) => dir.to_string(),
        (false, false) => format!("{dir}/{rel}"),
    }
}

fn manifest_in(dir: &str, file: &str) -> String {
    if dir.is_empty() {
        file.to_string()
    } else {
        format!("{dir}/{file}")
    }
}

/// Every lockfile of the repository outside excluded roots and `node_modules`, read into the
/// packages it reaches only transitively.
pub fn observe(files: &Files, excluded: &[String], c: &Census) -> Vec<Locked> {
    let mut out = Vec::new();
    for f in &files.paths {
        if is_excluded(f, excluded) || f.split('/').any(|s| s == "node_modules") {
            continue;
        }
        let name = f.rsplit('/').next().unwrap_or(f);
        let Some(text) = (match name {
            "Cargo.lock" | "package-lock.json" | "pnpm-lock.yaml" => files.read(f),
            _ => None,
        }) else {
            continue;
        };
        let (graph, eco) = match name {
            "Cargo.lock" => (cargo(f, &text, c), Ecosystem::Cargo),
            "package-lock.json" => (npm(f, &text, files), Ecosystem::Npm),
            _ => (pnpm(f, &text), Ecosystem::Npm),
        };
        if let Some(g) = graph {
            out.extend(g.transitive(f, eco));
        }
    }
    out.sort();
    out
}

/// `Cargo.lock`: packages without a `source` are workspace (or path) packages; their edges take
/// the scope their manifest gives the dependency.
fn cargo(lockfile: &str, text: &str, c: &Census) -> Option<Graph> {
    let v = toml::parse(text).ok()?;
    let dir = dir_of(lockfile);
    let members: BTreeMap<&str, &str> = c
        .members
        .iter()
        .map(|m| (m.package.as_str(), m.dir.as_str()))
        .collect();
    let mut g = Graph::default();
    let pkgs = v.items_of("package");
    let mut by_name: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for p in pkgs {
        let name = p.text("name");
        // Workspace packages are the roots; a path package outside the workspace is foreign
        // code built in-tree, whose own dependencies no manifest census sees.
        let workspace = dir.is_empty() && !members.is_empty();
        let manifest = (p.get("source").is_none()
            && (!workspace || members.contains_key(name.as_str())))
        .then(|| match members.get(name.as_str()) {
            Some(d) if workspace => manifest_in(d, "Cargo.toml"),
            _ => lockfile.to_string(),
        });
        let i = g.add(&name, &p.text("version"), manifest);
        by_name.entry(name).or_default().push(i);
    }
    // The scope a workspace manifest gives each of its dependencies (normal before build
    // before dev when one is listed several ways).
    let mut manifest_scope: BTreeMap<(&str, String), Scope> = BTreeMap::new();
    for o in &c.observations {
        if o.ecosystem == Ecosystem::Cargo && o.via == super::Via::Manifest {
            let rank = |s: Scope| match s {
                Scope::Build => 1,
                Scope::Test => 2,
                _ => 0,
            };
            let e = manifest_scope
                .entry((o.file.as_str(), o.name.replace('_', "-")))
                .or_insert(o.scope);
            if rank(o.scope) < rank(*e) {
                *e = o.scope;
            }
        }
    }
    for (i, p) in pkgs.iter().enumerate() {
        for dep in p.strings("dependencies") {
            let mut parts = dep.split_whitespace();
            let name = parts.next().unwrap_or("");
            let version = parts.next();
            let Some(cands) = by_name.get(name) else {
                continue;
            };
            let target = cands
                .iter()
                .copied()
                .find(|j| version.is_none_or(|v| g.nodes[*j].1 == v))
                .or(cands.first().copied());
            if let Some(t) = target {
                let scope = g.nodes[i]
                    .2
                    .as_deref()
                    .and_then(|m| manifest_scope.get(&(m, name.to_string())))
                    .copied()
                    .unwrap_or(Scope::Runtime);
                g.edge(i, t, scope);
            }
        }
    }
    Some(g)
}

/// The scope of an npm dependency section (as the manifest census reads it).
fn npm_scope(section: &str) -> Scope {
    if section == "devDependencies" {
        Scope::Build
    } else {
        Scope::Runtime
    }
}

/// A package awaiting resolution: (node, install path, dependency names with their scope).
type Wants = (usize, String, Vec<(String, Scope)>);

const NPM_SECTIONS: &[&str] = &[
    "dependencies",
    "optionalDependencies",
    "peerDependencies",
    "devDependencies",
];

/// `package-lock.json`: v2/v3 `packages` keyed by install path (resolved the way Node resolves
/// a `require`), v1 nested `dependencies` with `requires`.
fn npm(lockfile: &str, text: &str, files: &Files) -> Option<Graph> {
    let v = json::parse(text).ok()?;
    let dir = dir_of(lockfile);
    let mut g = Graph::default();
    let mut at: BTreeMap<String, usize> = BTreeMap::new();
    // (node, dependency names by section) to resolve once every node exists.
    let mut wants: Vec<Wants> = Vec::new();
    let pkg_name = |key: &str| {
        key.rsplit_once("node_modules/")
            .map_or(key, |(_, n)| n)
            .to_string()
    };
    let deps_of = |p: &Value, sections: &[&str]| -> Vec<(String, Scope)> {
        let mut out = Vec::new();
        for s in sections {
            if let Some(t) = p.get(s).and_then(Value::table) {
                for k in t.keys() {
                    out.push((k.clone(), npm_scope(s)));
                }
            }
        }
        out
    };
    if let Some(packages) = v.get("packages").and_then(Value::table) {
        for (key, p) in packages {
            let internal = !key.contains("node_modules/") || p.get("link").is_some();
            let manifest = internal.then(|| {
                let target = if p.get("link").is_some() {
                    p.text("resolved")
                } else {
                    key.clone()
                };
                manifest_in(&under(dir, &target), "package.json")
            });
            let name = if internal && p.get("name").is_some() {
                p.text("name")
            } else {
                pkg_name(key)
            };
            let i = g.add(&name, &p.text("version"), manifest);
            at.insert(key.clone(), i);
            wants.push((i, key.clone(), deps_of(p, NPM_SECTIONS)));
        }
    } else if let Some(deps) = v.get("dependencies").and_then(Value::table) {
        // v1: the root's direct dependencies are its sibling package.json's.
        let root_manifest = manifest_in(dir, "package.json");
        let root = g.add(
            &v.text("name"),
            &v.text("version"),
            Some(root_manifest.clone()),
        );
        at.insert(String::new(), root);
        let root_deps = files
            .read(&root_manifest)
            .and_then(|t| json::parse(&t).ok())
            .map(|m| deps_of(&m, NPM_SECTIONS))
            .unwrap_or_default();
        wants.push((root, String::new(), root_deps));
        fn walk(
            prefix: &str,
            deps: &BTreeMap<String, Value>,
            g: &mut Graph,
            at: &mut BTreeMap<String, usize>,
            wants: &mut Vec<Wants>,
        ) {
            for (name, p) in deps {
                let key = format!("{prefix}node_modules/{name}");
                let i = g.add(name, &p.text("version"), None);
                at.insert(key.clone(), i);
                let req = p
                    .get("requires")
                    .and_then(Value::table)
                    .map(|t| t.keys().map(|k| (k.clone(), Scope::Runtime)).collect())
                    .unwrap_or_default();
                wants.push((i, key.clone(), req));
                if let Some(nested) = p.get("dependencies").and_then(Value::table) {
                    walk(&format!("{key}/"), nested, g, at, wants);
                }
            }
        }
        walk("", deps, &mut g, &mut at, &mut wants);
    } else {
        return None;
    }
    for (i, key, deps) in wants {
        for (name, scope) in deps {
            // Node resolution: the nearest `node_modules/<name>` up from the requiring package.
            let mut base = key.clone();
            let found = loop {
                let cand = if base.is_empty() {
                    format!("node_modules/{name}")
                } else {
                    format!("{base}/node_modules/{name}")
                };
                if let Some(t) = at.get(&cand) {
                    break Some(*t);
                }
                if base.is_empty() {
                    break None;
                }
                base = match base.rfind("/node_modules/") {
                    Some(k) => base[..k].to_string(),
                    None => String::new(),
                };
            };
            if let Some(t) = found {
                g.edge(i, t, scope);
            }
        }
    }
    Some(g)
}

/// A YAML mapping line as (key, rest): quotes around the key dropped, `rest` empty for a key
/// that opens a nested mapping.
fn yaml_key(t: &str) -> (&str, &str) {
    let t = t.trim();
    if let Some(q) = t.chars().next().filter(|c| *c == '\'' || *c == '"') {
        if let Some(end) = t[1..].find(q) {
            let rest = t[end + 2..].trim_start_matches(':').trim();
            return (&t[1..end + 1], rest);
        }
    }
    match t.split_once(": ") {
        Some((k, r)) => (k.trim(), r.trim()),
        None => (t.trim_end_matches(':').trim(), ""),
    }
}

/// A pnpm package key (`/name/1.0.0`, `/@s/n@1.0.0(peer@2)`, `name@1.0.0`) or dependency
/// version as (name, version).
fn pnpm_id(key: &str) -> (String, String) {
    let k = yaml_key(key).0.trim_start_matches('/');
    let k = k.split('(').next().unwrap_or(k);
    // v5: `name/version_peers` (the last segment is a version).
    if let Some((n, v)) = k.rsplit_once('/') {
        if v.starts_with(|c: char| c.is_ascii_digit()) {
            return (n.to_string(), v.split('_').next().unwrap_or(v).to_string());
        }
    }
    match k[1.min(k.len())..].rfind('@') {
        Some(at) => (k[..at + 1].to_string(), k[at + 2..].to_string()),
        None => (k.to_string(), String::new()),
    }
}

/// `pnpm-lock.yaml`, read by indentation: `importers` (or v5's top-level dependency lists)
/// are the workspace packages; `packages` and `snapshots` entries carry the dependencies.
fn pnpm(lockfile: &str, text: &str) -> Option<Graph> {
    let dir = dir_of(lockfile);
    let mut g = Graph::default();
    let mut ids: BTreeMap<(String, String), usize> = BTreeMap::new();
    let mut by_name: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    // (from, dependency name, dependency version, scope)
    let mut edges: Vec<(usize, String, String, Scope)> = Vec::new();
    let mut importers: BTreeMap<String, usize> = BTreeMap::new();
    let mut section = String::new();
    let mut current: Option<usize> = None;
    let mut dep_section: Option<Scope> = None;
    let mut pending: Option<(usize, String, Scope)> = None; // an importer entry awaiting `version:`
    let indent = |l: &str| l.len() - l.trim_start().len();
    let mut node = |g: &mut Graph, name: &str, version: &str| -> usize {
        *ids.entry((name.to_string(), version.to_string()))
            .or_insert_with(|| {
                let i = g.add(name, version, None);
                by_name.entry(name.to_string()).or_default().push(i);
                i
            })
    };
    let mut importer = |g: &mut Graph, key: &str| -> usize {
        let rel = yaml_key(key).0;
        *importers
            .entry(rel.to_string())
            .or_insert_with(|| g.add(rel, "", Some(manifest_in(&under(dir, rel), "package.json"))))
    };
    let dep_scope = |s: &str| match s {
        "dependencies" | "optionalDependencies" | "peerDependencies" => Some(Scope::Runtime),
        "devDependencies" => Some(Scope::Build),
        _ => None,
    };
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let ind = indent(line);
        let (key, _) = yaml_key(t);
        if ind == 0 {
            section = key.to_string();
            current = None;
            dep_section = None;
            pending = None;
            // v5 single project: top-level dependency lists belong to the root importer.
            if let Some(s) = dep_scope(key) {
                current = Some(importer(&mut g, "."));
                dep_section = Some(s);
                section = "importer".into();
            }
            continue;
        }
        match section.as_str() {
            "importers" if ind == 2 => {
                current = Some(importer(&mut g, t));
                dep_section = None;
            }
            "importers" | "importer" => {
                let base = if section == "importer" { 0 } else { 2 };
                if ind == base + 2 && section == "importers" {
                    dep_section = dep_scope(key);
                    continue;
                }
                let (Some(from), Some(scope)) = (current, dep_section) else {
                    continue;
                };
                if ind == base + 4 || (section == "importer" && ind == 2) {
                    let (name, value) = yaml_key(t);
                    let name = name.to_string();
                    if value.is_empty() {
                        pending = Some((from, name, scope));
                    } else {
                        edges.push((from, name, value.to_string(), scope));
                    }
                } else if let Some(v) = t.strip_prefix("version:") {
                    if let Some((from, name, scope)) = pending.take() {
                        edges.push((from, name, v.trim().to_string(), scope));
                    }
                }
            }
            "packages" | "snapshots" if ind == 2 => {
                let (name, version) = pnpm_id(t);
                current = Some(node(&mut g, &name, &version));
                dep_section = None;
            }
            "packages" | "snapshots" if ind == 4 => {
                dep_section = dep_scope(key).filter(|_| key != "devDependencies");
            }
            "packages" | "snapshots" if ind == 6 => {
                if let (Some(from), Some(scope)) = (current, dep_section) {
                    let (name, value) = yaml_key(t);
                    edges.push((from, name.to_string(), value.to_string(), scope));
                }
            }
            _ => {}
        }
    }
    if importers.is_empty() && ids.is_empty() {
        return None;
    }
    for (from, name, value, scope) in edges {
        let value = value.trim_matches(['\'', '"']);
        if value.starts_with("link:")
            || value.starts_with("workspace:")
            || value.starts_with("file:")
        {
            continue;
        }
        let version = value
            .split('(')
            .next()
            .unwrap_or(value)
            .split('_')
            .next()
            .unwrap_or(value)
            .to_string();
        // A version may itself be an id (`/name/1.0.0`, `npm:x@1`): read it as one.
        let target = ids
            .get(&(name.clone(), version.clone()))
            .copied()
            .or_else(|| {
                let (n, v) = pnpm_id(&version);
                ids.get(&(n, v))
                    .copied()
                    .or_else(|| by_name.get(&name).and_then(|c| c.first().copied()))
            });
        if let Some(t) = target {
            g.edge(from, t, scope);
        }
    }
    Some(g)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::census::{Member, Observation, Via};

    fn census(members: &[(&str, &str)], manifest: &[(&str, &str, Scope)]) -> Census {
        let mut c = Census::default();
        for (dir, package) in members {
            c.members.push(Member {
                dir: dir.to_string(),
                package: package.to_string(),
            });
        }
        for (file, name, scope) in manifest {
            c.observations.insert(Observation {
                file: file.to_string(),
                ecosystem: Ecosystem::Cargo,
                name: name.to_string(),
                ident: name.replace('-', "_"),
                scope: *scope,
                via: Via::Manifest,
            });
        }
        c
    }

    const CARGO_LOCK: &str = r#"
version = 4

[[package]]
name = "app"
version = "0.1.0"
dependencies = ["postgres", "criterion"]

[[package]]
name = "postgres"
version = "0.19.9"
source = "registry+https://github.com/rust-lang/crates.io-index"
dependencies = ["tokio-postgres"]

[[package]]
name = "tokio-postgres"
version = "0.7.13"
source = "registry+https://github.com/rust-lang/crates.io-index"
dependencies = ["postgres-protocol"]

[[package]]
name = "postgres-protocol"
version = "0.6.12"
source = "registry+https://github.com/rust-lang/crates.io-index"
dependencies = ["sha2 0.11.0"]

[[package]]
name = "sha2"
version = "0.11.0"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "sha2"
version = "0.10.8"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "criterion"
version = "0.5.1"
source = "registry+https://github.com/rust-lang/crates.io-index"
dependencies = ["plotters"]

[[package]]
name = "plotters"
version = "0.3.7"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#;

    #[test]
    fn cargo_lock_paths_name_who_pulls_a_package_in() {
        let c = census(
            &[("app", "app")],
            &[
                ("app/Cargo.toml", "postgres", Scope::Runtime),
                ("app/Cargo.toml", "criterion", Scope::Test),
            ],
        );
        let g = cargo("Cargo.lock", CARGO_LOCK, &c).unwrap();
        let t = g.transitive("Cargo.lock", Ecosystem::Cargo);
        let get = |n: &str, v: &str| t.iter().find(|l| l.name == n && l.version == v);
        let sha = get("sha2", "0.11.0").unwrap();
        assert_eq!(sha.scope, Scope::Linked);
        assert_eq!(sha.manifest, "app/Cargo.toml");
        assert_eq!(
            sha.chain,
            [
                "postgres 0.19.9",
                "tokio-postgres 0.7.13",
                "postgres-protocol 0.6.12",
                "sha2 0.11.0"
            ]
        );
        assert_eq!(sha.via(), "postgres-protocol 0.6.12");
        // A dev dependency's closure is test participation; an orphan entry still counts.
        assert_eq!(get("plotters", "0.3.7").unwrap().scope, Scope::Test);
        let orphan = get("sha2", "0.10.8").unwrap();
        assert_eq!(
            (orphan.scope, orphan.manifest.as_str()),
            (Scope::Linked, "Cargo.lock")
        );
        // Direct dependencies are the manifests' business.
        assert!(get("postgres", "0.19.9").is_none() && get("criterion", "0.5.1").is_none());
    }

    #[test]
    fn npm_and_pnpm_lockfiles_are_read() {
        let files = Files {
            root: std::path::PathBuf::from("/nonexistent"),
            paths: Default::default(),
            gitlinks: Default::default(),
            from_index: false,
        };
        let lock = r#"{"name":"web","lockfileVersion":3,"packages":{
            "":{"name":"web","dependencies":{"a":"^1"},"devDependencies":{"t":"^2"}},
            "node_modules/a":{"version":"1.0.0","dependencies":{"b":"^1"}},
            "node_modules/a/node_modules/b":{"version":"1.1.0","dependencies":{"c":"^1"}},
            "node_modules/b":{"version":"2.0.0"},
            "node_modules/c":{"version":"3.0.0"},
            "node_modules/t":{"version":"2.0.0","dev":true,"dependencies":{"u":"^1"}},
            "node_modules/u":{"version":"1.0.0","dev":true}}}"#;
        let g = npm("apps/web/package-lock.json", lock, &files).unwrap();
        let t = g.transitive("apps/web/package-lock.json", Ecosystem::Npm);
        let get = |n: &str| t.iter().find(|l| l.name == n).unwrap();
        assert_eq!(
            get("b").version,
            "1.1.0",
            "nested node_modules resolve first"
        );
        assert_eq!(get("c").chain, ["a 1.0.0", "b 1.1.0", "c 3.0.0"]);
        assert_eq!(get("c").manifest, "apps/web/package.json");
        assert_eq!(get("c").scope, Scope::Linked);
        assert_eq!(get("u").scope, Scope::Build);

        let pnpm_lock = "lockfileVersion: '9.0'\n\nimporters:\n\n  .:\n    dependencies:\n      a:\n        specifier: ^1\n        version: 1.0.0(react@18.2.0)\n    devDependencies:\n      t:\n        specifier: ^2\n        version: 2.0.0\n\npackages:\n\n  a@1.0.0:\n    resolution: {integrity: sha512-x}\n\n  '@s/b@1.1.0':\n    resolution: {integrity: sha512-y}\n\n  t@2.0.0:\n    resolution: {integrity: sha512-z}\n\nsnapshots:\n\n  a@1.0.0(react@18.2.0):\n    dependencies:\n      '@s/b': 1.1.0\n\n  '@s/b@1.1.0': {}\n\n  t@2.0.0: {}\n";
        let g = pnpm("pnpm-lock.yaml", pnpm_lock).unwrap();
        let t = g.transitive("pnpm-lock.yaml", Ecosystem::Npm);
        assert_eq!(t.len(), 1, "{t:?}");
        assert_eq!(t[0].name, "@s/b");
        assert_eq!(t[0].chain, ["a 1.0.0", "@s/b 1.1.0"]);
        assert_eq!(t[0].manifest, "package.json");
        let v5 = "lockfileVersion: 5.4\n\nspecifiers:\n  a: ^1\n\ndependencies:\n  a: 1.0.0\n\npackages:\n\n  /a/1.0.0:\n    dependencies:\n      b: 2.0.0\n    dev: false\n\n  /b/2.0.0:\n    dev: false\n";
        let g = pnpm("x/pnpm-lock.yaml", v5).unwrap();
        let t = g.transitive("x/pnpm-lock.yaml", Ecosystem::Npm);
        assert_eq!(t.len(), 1, "{t:?}");
        assert_eq!((t[0].name.as_str(), t[0].via()), ("b", "a 1.0.0"));
        assert_eq!(t[0].manifest, "x/package.json");
        assert_eq!(pnpm_id("/@s/n@1.2.3(p@1)"), ("@s/n".into(), "1.2.3".into()));
        assert_eq!(pnpm_id("/@s/n/1.2.3_p@1"), ("@s/n".into(), "1.2.3".into()));
    }
}
