//! Manifest observation: Cargo workspaces and manifests, `Cargo.lock`, npm `package.json` and
//! Python requirement files.

use super::{is_excluded, Census, Member, Observation, Via};
use crate::formats::{json, toml, Value};
use crate::repository::files::Files;
use crate::schema::{Ecosystem, Scope};
use std::collections::BTreeMap;

/// Directory of a manifest path (`a/b/Cargo.toml` → `a/b`, `Cargo.toml` → ``).
fn dir_of(path: &str) -> String {
    match path.rsplit_once('/') {
        Some((d, _)) => d.to_string(),
        None => String::new(),
    }
}

/// Lexically normalises `base/rel` (handles `.` and `..`).
pub fn join(base: &str, rel: &str) -> String {
    let mut parts: Vec<&str> = if base.is_empty() {
        Vec::new()
    } else {
        base.split('/').collect()
    };
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    parts.push("..");
                }
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

fn glob_match(pattern: &str, path: &str) -> bool {
    let ps: Vec<&str> = pattern.split('/').collect();
    let xs: Vec<&str> = path.split('/').collect();
    ps.len() == xs.len()
        && ps.iter().zip(&xs).all(|(p, x)| {
            if *p == "*" {
                true
            } else if let Some((a, b)) = p.split_once('*') {
                x.starts_with(a) && x.ends_with(b) && x.len() >= a.len() + b.len()
            } else {
                p == x
            }
        })
}

/// The workspace member directories of the root manifest (plus `.ecdev`).
pub fn members(files: &Files) -> Vec<Member> {
    let mut out = Vec::new();
    let mut push = |dir: &str, files: &Files| {
        let manifest = if dir.is_empty() {
            "Cargo.toml".to_string()
        } else {
            format!("{dir}/Cargo.toml")
        };
        if let Some(v) = files.read(&manifest).and_then(|t| toml::parse(&t).ok()) {
            if let Some(name) = v.get("package").and_then(|p| p.str("name")) {
                out.push(Member {
                    dir: dir.to_string(),
                    package: name.to_string(),
                });
            }
        }
    };
    if let Some(root) = files.read("Cargo.toml").and_then(|t| toml::parse(&t).ok()) {
        if let Some(ws) = root.get("workspace") {
            let exclude = ws.strings("exclude");
            let manifests: Vec<String> = files
                .with_suffix("Cargo.toml")
                .filter(|p| *p == "Cargo.toml" || p.ends_with("/Cargo.toml"))
                .cloned()
                .collect();
            for pattern in ws.strings("members") {
                let pattern = pattern.trim_end_matches('/').to_string();
                if pattern.contains('*') {
                    for m in &manifests {
                        let d = dir_of(m);
                        if glob_match(&pattern, &d) && !exclude.contains(&d) {
                            push(&d, files);
                        }
                    }
                } else if !exclude.contains(&pattern) {
                    push(&pattern, files);
                }
            }
        }
        if root.get("package").is_some() {
            push("", files);
        }
    }
    if files.paths.contains(".ecdev/Cargo.toml") {
        push(".ecdev", files);
    }
    out.sort();
    out.dedup();
    out
}

const SECTIONS: &[(&str, Scope)] = &[
    ("dependencies", Scope::Runtime),
    ("build-dependencies", Scope::Build),
    ("dev-dependencies", Scope::Test),
];

pub fn observe(files: &Files, excluded: &[String], c: &mut Census) {
    c.members = members(files);
    let by_dir: BTreeMap<&str, &str> = c
        .members
        .iter()
        .map(|m| (m.dir.as_str(), m.package.as_str()))
        .collect();
    let root = files.read("Cargo.toml").and_then(|t| toml::parse(&t).ok());
    let ws_deps = root
        .as_ref()
        .and_then(|r| r.at("workspace.dependencies"))
        .cloned();

    for m in c.members.clone() {
        let manifest = if m.dir.is_empty() {
            "Cargo.toml".to_string()
        } else {
            format!("{}/Cargo.toml", m.dir)
        };
        let Some(v) = files.read(&manifest).and_then(|t| toml::parse(&t).ok()) else {
            c.unreadable.push(manifest);
            continue;
        };
        let mut tables: Vec<(&Value, Scope)> = Vec::new();
        for (section, scope) in SECTIONS {
            if let Some(t) = v.get(section) {
                tables.push((t, *scope));
            }
            if let Some(Value::Table(targets)) = v.get("target") {
                for t in targets.values() {
                    if let Some(t) = t.get(section) {
                        tables.push((t, *scope));
                    }
                }
            }
        }
        for (table, scope) in tables {
            let Some(deps) = table.table() else { continue };
            for (ident, spec) in deps {
                // Resolve workspace inheritance.
                let resolved = if spec.get("workspace") == Some(&Value::Bool(true)) {
                    ws_deps
                        .as_ref()
                        .and_then(|w| w.get(ident))
                        .cloned()
                        .map(|w| (w, ""))
                } else {
                    Some((spec.clone(), m.dir.as_str()))
                };
                let Some((spec, base)) = resolved else {
                    c.unreadable.push(format!("{manifest}#{ident}"));
                    continue;
                };
                let package = spec.str("package").unwrap_or(ident).to_string();
                if let Some(p) = spec.str("path") {
                    let target = join(base, p);
                    if by_dir.contains_key(target.as_str()) {
                        c.internal.insert((m.dir.clone(), target, scope));
                        continue;
                    }
                    // A path dependency outside the workspace: foreign code built in-tree.
                    c.observations.insert(Observation {
                        file: manifest.clone(),
                        ecosystem: Ecosystem::Cargo,
                        name: package.clone(),
                        ident: ident.replace('-', "_"),
                        scope,
                        via: Via::Manifest,
                    });
                    c.observations.insert(Observation {
                        file: manifest.clone(),
                        ecosystem: Ecosystem::Cargo,
                        name: target,
                        ident: ident.replace('-', "_"),
                        scope,
                        via: Via::SourceReference,
                    });
                    continue;
                }
                c.observations.insert(Observation {
                    file: manifest.clone(),
                    ecosystem: Ecosystem::Cargo,
                    name: package,
                    ident: ident.replace('-', "_"),
                    scope,
                    via: Via::Manifest,
                });
            }
        }
    }

    // Locked closure.
    for lock in ["Cargo.lock", ".ecdev/Cargo.lock"] {
        if let Some(v) = files.read(lock).and_then(|t| toml::parse(&t).ok()) {
            for p in v.items_of("package") {
                if p.get("source").is_some() {
                    c.closure
                        .insert(format!("{}@{}", p.text("name"), p.text("version")));
                }
            }
        }
    }

    // npm manifests.
    for pj in files.with_suffix("package.json") {
        if !(pj == "package.json" || pj.ends_with("/package.json")) || is_excluded(pj, excluded) {
            continue;
        }
        if pj
            .split('/')
            .any(|s| s == "node_modules" || s == "research")
        {
            continue;
        }
        let Some(v) = files.read(pj).and_then(|t| json::parse(&t).ok()) else {
            c.unreadable.push(pj.clone());
            continue;
        };
        for (section, scope) in [
            ("dependencies", Scope::Runtime),
            ("peerDependencies", Scope::Runtime),
            ("optionalDependencies", Scope::Runtime),
            ("devDependencies", Scope::Build),
        ] {
            if let Some(Value::Table(deps)) = v.get(section) {
                for (name, spec) in deps {
                    // Workspace-internal packages are not foreign.
                    if spec.as_str().is_some_and(|s| {
                        s.starts_with("workspace:")
                            || s.starts_with("file:")
                            || s.starts_with("link:")
                    }) {
                        continue;
                    }
                    c.observations.insert(Observation {
                        file: pj.clone(),
                        ecosystem: Ecosystem::Npm,
                        name: name.clone(),
                        ident: name.clone(),
                        scope,
                        via: Via::Manifest,
                    });
                }
            }
        }
    }

    // Python requirements outside research (research is never on a build path).
    for f in files.paths.iter() {
        let name = f.rsplit('/').next().unwrap_or(f);
        let is_req = name.starts_with("requirements") && name.ends_with(".txt");
        if !(is_req || name == "pyproject.toml") || is_excluded(f, excluded) {
            continue;
        }
        if f.split('/').next() == Some("research") {
            continue;
        }
        let Some(text) = files.read(f) else { continue };
        let reqs: Vec<String> = if is_req {
            text.lines()
                .map(|l| l.split('#').next().unwrap_or("").trim().to_string())
                .filter(|l| !l.is_empty() && !l.starts_with('-'))
                .collect()
        } else {
            toml::parse(&text)
                .ok()
                .map(|v| {
                    v.get("project")
                        .map(|p| p.strings("dependencies"))
                        .unwrap_or_default()
                })
                .unwrap_or_default()
        };
        for r in reqs {
            let pkg: String = r
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
                .collect();
            if pkg.is_empty() {
                continue;
            }
            c.observations.insert(Observation {
                file: f.clone(),
                ecosystem: Ecosystem::Python,
                name: pkg.to_ascii_lowercase(),
                ident: pkg.to_ascii_lowercase().replace('-', "_"),
                scope: super::scope_of_file(f),
                via: Via::Manifest,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_join_and_globs() {
        assert_eq!(join("storage", "../core"), "core");
        assert_eq!(join("a/b", "../../c/d"), "c/d");
        assert_eq!(join("", "./x"), "x");
        assert!(glob_match("domain/*", "domain/ai"));
        assert!(!glob_match("domain/*", "domain/ai/x"));
        assert!(glob_match("adapter/web-*", "adapter/web-http"));
    }
}
