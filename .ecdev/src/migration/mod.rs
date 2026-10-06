//! Incremental migration under concurrent development.
//!
//! * Semantic canonicalization comes first: a repository is graph-canonical as soon as every
//!   node is declared with its canonical path, while files stay where they are.
//! * Physical canonicalization follows in atomic waves: `plan` groups legacy-placed nodes by
//!   their current root, `apply` moves one wave (directories, Cargo members and path
//!   dependencies, declarations) and leaves the workspace buildable. Node ids never change.
//! * Compatibility is a shim with an expiry; an expired shim that still exists is an error, so
//!   scaffolding cannot become architecture.

pub mod pathmap;
pub mod register;

use crate::declare::Declaration;
use crate::donors::Analysis;
use crate::repository::files::Files;
use crate::schema::{DonorState, ExpiryKind, NodeKind, ShimKind, WaveStatus};
use crate::{Finding, Severity};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct ShimStatus {
    pub key: String,
    pub expired: bool,
    pub present: bool,
    pub detail: String,
}

#[derive(Clone, Debug)]
pub struct WaveState {
    pub key: String,
    pub status: WaveStatus,
    pub canonical: usize,
    pub total: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub shims: Vec<ShimStatus>,
    pub waves: Vec<WaveState>,
    pub findings: Vec<Finding>,
}

pub fn status(files: &Files, d: &Declaration, a: &Analysis) -> Status {
    let mut s = Status::default();
    let canonical = |k: &str| d.node(k).is_some_and(|n| n.path == n.canonical_path);

    let mut in_wave: BTreeMap<&str, &str> = BTreeMap::new();
    for w in &d.migration.waves {
        let mut can = 0;
        for n in &w.nodes {
            if d.node(n).is_none() {
                s.findings.push(Finding::new(
                    Severity::Error,
                    "WAVE_UNKNOWN_NODE",
                    &w.key,
                    &format!("names undeclared node `{n}`"),
                ));
            }
            if let Some(prev) = in_wave.insert(n, &w.key) {
                s.findings.push(Finding::new(
                    Severity::Error,
                    "NODE_IN_TWO_WAVES",
                    n,
                    &format!("in waves `{prev}` and `{}`", w.key),
                ));
            }
            if canonical(n) {
                can += 1;
            }
        }
        if w.status == WaveStatus::Applied && can != w.nodes.len() {
            s.findings.push(Finding::new(
                Severity::Error,
                "WAVE_NOT_APPLIED",
                &w.key,
                &format!(
                    "marked APPLIED but {} of {} nodes are not at their canonical path",
                    w.nodes.len() - can,
                    w.nodes.len()
                ),
            ));
        }
        s.waves.push(WaveState {
            key: w.key.clone(),
            status: w.status,
            canonical: can,
            total: w.nodes.len(),
        });
    }

    for sh in &d.migration.shims {
        let (kind, target) = &sh.expires;
        let (valid, expired, detail) = match kind {
            ExpiryKind::WaveApplied => match d.migration.waves.iter().find(|w| &w.key == target) {
                None => (false, false, format!("wave `{target}` is not declared")),
                Some(w) => {
                    let done =
                        w.status == WaveStatus::Applied && w.nodes.iter().all(|n| canonical(n));
                    (
                        true,
                        done,
                        format!(
                            "wave `{target}` is {}",
                            if done { "applied" } else { "pending" }
                        ),
                    )
                }
            },
            ExpiryKind::DonorExtinct => match a.donors.iter().find(|x| &x.key == target) {
                None => (false, false, format!("donor `{target}` is not declared")),
                Some(x) => (
                    true,
                    x.effective == DonorState::Extinct,
                    format!("donor `{target}` is {}", x.effective),
                ),
            },
            ExpiryKind::NodeCanonical => match d.node(target) {
                None => (false, false, format!("node `{target}` is not declared")),
                Some(n) => (
                    true,
                    n.path == n.canonical_path,
                    format!(
                        "node `{target}` at `{}`, canonical `{}`",
                        n.path, n.canonical_path
                    ),
                ),
            },
        };
        let present = !sh.path.is_empty() && files.exists(&sh.path);
        if !valid {
            s.findings.push(Finding::new(
                Severity::Error,
                "SHIM_WITHOUT_VALID_EXPIRY",
                &sh.key,
                &detail,
            ));
        } else if expired && present {
            s.findings.push(Finding::new(
                Severity::Error,
                "EXPIRED_SHIM_PRESENT",
                &sh.key,
                &format!(
                    "its expiry {kind}({target}) holds ({detail}); delete `{}`",
                    sh.path
                ),
            ));
        } else if present {
            s.findings.push(Finding::new(
                Severity::Warning,
                "SHIM_ACTIVE",
                &sh.key,
                &format!("{} `{}` until {kind}({target}): {detail}", sh.kind, sh.path),
            ));
        }
        if sh.kind == ShimKind::DonorFallback && d.donor(&sh.serves).is_none() {
            s.findings.push(Finding::new(
                Severity::Error,
                "SHIM_WITHOUT_VALID_EXPIRY",
                &sh.key,
                "a DONOR_FALLBACK shim must serve a declared donor",
            ));
        }
        s.shims.push(ShimStatus {
            key: sh.key.clone(),
            expired,
            present,
            detail,
        });
    }

    for n in &d.repository.nodes {
        if n.kind == NodeKind::Compat
            && !d
                .migration
                .shims
                .iter()
                .any(|sh| sh.serves == n.key || sh.path == n.path)
        {
            s.findings.push(Finding::new(
                Severity::Error,
                "COMPAT_WITHOUT_EXPIRY",
                &n.key,
                "a COMPAT node needs a shim declaring when it expires",
            ));
        }
    }
    s
}
