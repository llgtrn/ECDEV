//! Executable shape validation: legal roots, canonical node placement, declared workspace
//! members, owned code, the plane dependency order, research isolation, and untracked
//! generated state.
//!
//! `repository_shape_conformance` = conformant units / units, where the units are the physical
//! shape only: one per root entry, one per active node, and one failing unit per violation not
//! tied to a root entry or a node (an undeclared workspace member, a directory of unowned code, a
//! plane violation, research on the build path, tracked generated state, a forbidden container).
//! Documents are not shape units: the document budget is measured by `documents_over_budget`,
//! which is its own V1 gate row.

use super::files::Files;
use super::{check_canonical_path, role_of_path, role_spec, root_file_allowed, ROLES};
use crate::census::{is_excluded, Census};
use crate::declare::Declaration;
use crate::donors::NodeIndex;
use crate::schema::{is_physical, EdgeKind, NodeKind, NodeLifecycle, Role, Scope, ShimKind};
use crate::{Finding, Severity};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub struct ShapeReport {
    pub findings: Vec<Finding>,
    pub units: u64,
    pub conformant: u64,
    pub nodes_total: u64,
    pub nodes_conformant: u64,
}

impl ShapeReport {
    fn unit(&mut self, ok: bool) {
        self.units += 1;
        if ok {
            self.conformant += 1;
        }
    }
}

const CODE_SUFFIXES: &[&str] = &[
    ".rs", ".ts", ".tsx", ".js", ".py", ".c", ".cc", ".cpp", ".go",
];

pub fn check(files: &Files, d: &Declaration, census: &Census) -> ShapeReport {
    let mut r = ShapeReport::default();
    let excluded = crate::census::excluded_roots(d);
    let legacy_shim = d
        .migration
        .shims
        .iter()
        .any(|s| s.kind == ShimKind::LegacyInput && s.path == ".atlas");
    let active: Vec<_> = d
        .repository
        .nodes
        .iter()
        .filter(|n| {
            is_physical(n.kind)
                && n.kind != NodeKind::Repository
                && n.lifecycle != NodeLifecycle::Retired
        })
        .collect();

    // 1. Root entries.
    for e in files.root_entries() {
        if !files.is_dir(&e) {
            let ok = root_file_allowed(&e);
            if !ok {
                r.findings.push(Finding::new(
                    Severity::Error,
                    "ILLEGAL_ROOT_FILE",
                    &e,
                    "not a legal root file; move its content into the graph or a node",
                ));
            }
            r.unit(ok);
            continue;
        }
        match ROLES.iter().find(|s| s.dir == e) {
            Some(spec) if spec.role == Role::Legacy => {
                r.findings.push(if legacy_shim {
                    Finding::new(
                        Severity::Warning,
                        "LEGACY_ROOT",
                        &e,
                        "legacy knowledge root, legal only while its LEGACY_INPUT shim is active",
                    )
                } else {
                    Finding::new(
                        Severity::Error,
                        "ILLEGAL_ROOT",
                        &e,
                        "legacy knowledge root without a LEGACY_INPUT shim",
                    )
                });
                r.unit(false);
            }
            Some(_) => r.unit(true),
            None => {
                let tracked = active
                    .iter()
                    .any(|n| n.path == e || n.path.starts_with(&format!("{e}/")));
                r.findings.push(if tracked {
                    Finding::new(Severity::Warning, "LEGACY_ROOT", &e, "a non-canonical root whose nodes are declared and await physical migration")
                } else {
                    Finding::new(Severity::Error, "ILLEGAL_ROOT", &e, "a root that is neither a canonical role nor declared legacy placement")
                });
                r.unit(false);
            }
        }
    }

    // 2. Nodes.
    for n in &active {
        r.nodes_total += 1;
        let mut ok = true;
        if let Err(why) = check_canonical_path(n.kind, &n.canonical_path) {
            r.findings.push(Finding::new(
                Severity::Error,
                "NONCANONICAL_TARGET",
                &n.key,
                &format!("canonical path `{}`: {why}", n.canonical_path),
            ));
            ok = false;
        }
        if n.lifecycle == NodeLifecycle::Planned {
            if !n.path.is_empty() && files.exists(&n.path) {
                r.findings.push(Finding::new(
                    Severity::Warning,
                    "PLANNED_NODE_EXISTS",
                    &n.key,
                    "a PLANNED node already has a tree; mark it ACTIVE",
                ));
            }
            ok = false;
        } else if !files.exists(&n.path) {
            r.findings.push(Finding::new(
                Severity::Error,
                "NODE_PATH_MISSING",
                &n.key,
                &format!("`{}` does not exist", n.path),
            ));
            ok = false;
        } else if n.path != n.canonical_path {
            r.findings.push(Finding::new(
                Severity::Warning,
                "LEGACY_PLACEMENT",
                &n.key,
                &format!("lives at `{}`; canonical is `{}`", n.path, n.canonical_path),
            ));
            ok = false;
        }
        if ok {
            r.nodes_conformant += 1;
        }
        r.unit(ok);
    }

    // 3. Every workspace member is a declared node.
    for m in &census.members {
        let dir = if m.dir.is_empty() {
            "."
        } else {
            m.dir.as_str()
        };
        if !active.iter().any(|n| n.path == dir) && dir != "." {
            r.findings.push(Finding::new(
                Severity::Error,
                "UNDECLARED_MEMBER",
                dir,
                &format!("workspace package `{}` is not a declared node", m.package),
            ));
            r.unit(false);
        }
    }

    // 4. Code outside every node.
    let index = NodeIndex::new(d);
    let mut unowned: BTreeMap<String, u64> = BTreeMap::new();
    for f in &files.paths {
        if CODE_SUFFIXES.iter().any(|s| f.ends_with(s))
            && !is_excluded(f, &excluded)
            && index.owner(f).is_none()
        {
            let dir = f
                .rsplit_once('/')
                .map(|(d, _)| d.to_string())
                .unwrap_or_else(|| ".".into());
            *unowned.entry(dir).or_default() += 1;
        }
    }
    for (dir, n) in unowned {
        r.findings.push(Finding::new(
            Severity::Error,
            "UNOWNED_CODE",
            &dir,
            &format!("{n} code files belong to no declared node"),
        ));
        r.unit(false);
    }

    // 5. Plane order, judged on canonical (semantic) placement, and research isolation.
    let key_role = |key: &str| -> Option<(NodeKind, Role)> {
        let n = d.node(key)?;
        Some((n.kind, role_of_path(&n.canonical_path)?))
    };
    let mut edges: Vec<(String, String, Scope)> = d
        .repository
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::DependsOn && !e.from.contains(':') && !e.to.contains(':'))
        .map(|e| (e.from.clone(), e.to.clone(), e.scope))
        .collect();
    for (from, to, scope) in &census.internal {
        if let (Some(f), Some(t)) = (
            index.owner(&format!("{from}/Cargo.toml")),
            index.owner(&format!("{to}/Cargo.toml")),
        ) {
            if f != t {
                edges.push((f.to_string(), t.to_string(), *scope));
            }
        }
    }
    edges.sort();
    edges.dedup();
    for (from, to, scope) in edges {
        let (Some((fk, fr)), Some((tk, tr))) = (key_role(&from), key_role(&to)) else {
            continue;
        };
        if tr == Role::Research || tk == NodeKind::Research {
            r.findings.push(Finding::new(
                Severity::Error,
                "RESEARCH_ON_BUILD_PATH",
                &from,
                &format!("depends on research node `{to}`"),
            ));
            r.unit(false);
            continue;
        }
        let test_like = scope == Scope::Test || matches!(fk, NodeKind::Test | NodeKind::Fixture);
        if test_like {
            continue;
        }
        if !role_spec(fr).may_depend_on.contains(&tr) {
            r.findings.push(Finding::new(
                Severity::Error,
                "PLANE_VIOLATION",
                &from,
                &format!(
                    "{fr} node depends on {tr} node `{to}`; {fr} may depend on {:?}",
                    role_spec(fr)
                        .may_depend_on
                        .iter()
                        .map(|x| x.wire())
                        .collect::<Vec<_>>()
                ),
            ));
            r.unit(false);
        }
    }
    for m in &census.members {
        if let Some(owner) = index.owner(&format!("{}/Cargo.toml", m.dir)) {
            if d.node(owner).is_some_and(|n| n.kind == NodeKind::Research) {
                r.findings.push(Finding::new(
                    Severity::Error,
                    "RESEARCH_ON_BUILD_PATH",
                    &m.dir,
                    "a research node is a workspace member",
                ));
                r.unit(false);
            }
        }
    }

    // 6. Generated state is never tracked; generic containers never appear.
    let tracked_generated: Vec<&String> = files
        .paths
        .iter()
        .filter(|p| {
            p.starts_with("target/") || p.contains("/target/") && !is_excluded(p, &excluded)
        })
        .collect();
    if !tracked_generated.is_empty() {
        r.findings.push(Finding::new(
            Severity::Error,
            "GENERATED_TRACKED",
            tracked_generated[0],
            &format!(
                "{} build/generated files are tracked",
                tracked_generated.len()
            ),
        ));
        r.unit(false);
    }
    for f in &files.paths {
        if !is_excluded(f, &excluded) && f.split('/').any(|s| s == "crates") {
            r.findings.push(Finding::new(
                Severity::Error,
                "FORBIDDEN_CONTAINER",
                f,
                "`crates/` is never legal",
            ));
            r.unit(false);
            break;
        }
    }
    r
}
