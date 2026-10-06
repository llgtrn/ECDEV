//! THE donor lifecycle, executable. Declared states are claims; the effective state of every
//! donor is computed from observations and evidence, one guard per rung:
//!
//! ```text
//! DISCOVERED → REGISTERED → CENSUSED → TECHNOLOGY_MAPPED → SPECIFIED → NATIVE_TARGETED
//!   → NATIVE_SHADOW → PARITY_PROVEN → CAPABILITY_RELEVANCE_RESOLVED → CUTOVER → EXTINCT
//! ```
//!
//! A claim above the effective state is a false-green violation. Exceptional states (BLOCKED,
//! REJECTED, SUPERSEDED) have exact guards; an illegal exception is ignored and reported.

use crate::census::{Census, Observation, Via};
use crate::compact::history::History;
use crate::declare::{Declaration, Donor, Package};
use crate::evidence::{judge, Store, Verdict};
use crate::extinction::{gates, CapabilityVerdict, DonorFacts, GateResult};
use crate::repository::check_canonical_path;
use crate::repository::files::Files;
use crate::schema::{
    is_native_role, is_physical, DonorState, Ecosystem, EdgeKind, ExceptionKind, NativeStatus,
    NodeKind, NodeLifecycle, ProofKind, Scope, ShimKind,
};
use crate::{Finding, Severity};
use std::collections::{BTreeMap, BTreeSet};

/// Operating-system libraries (the platform ABI Rust's std itself links); linking them is not
/// a foreign dependency.
pub const PLATFORM_LIBRARIES: &[&str] = &[
    "c",
    "m",
    "dl",
    "pthread",
    "rt",
    "util",
    "System",
    "kernel32",
    "advapi32",
    "ntdll",
    "user32",
    "ws2_32",
    "bcrypt",
    "userenv",
    "synchronization",
    "Security",
    "CoreFoundation",
];

/// Programs of the Rust/Git toolchain itself; invoking them is not a foreign dependency.
pub const TOOLCHAIN_PROGRAMS: &[&str] = &[
    "cargo", "rustc", "rustfmt", "rustup", "git", "sh", "bash", "env",
];

#[derive(Clone, Debug)]
pub struct DonorAssessment {
    pub key: String,
    pub claimed: DonorState,
    pub effective: DonorState,
    /// The exception, if it is legal.
    pub exception: Option<(ExceptionKind, String)>,
    pub capabilities: Vec<CapabilityVerdict>,
    pub gates: Vec<GateResult>,
    pub facts: DonorFacts,
    /// Why the effective state stops where it does.
    pub stopped_by: String,
}

impl DonorAssessment {
    pub fn resolved(&self) -> bool {
        matches!(
            self.exception,
            Some((ExceptionKind::Rejected, _)) | Some((ExceptionKind::Superseded, _))
        )
    }
    pub fn extinct(&self) -> bool {
        self.effective == DonorState::Extinct
    }
}

#[derive(Clone, Debug, Default)]
pub struct Analysis {
    pub donors: Vec<DonorAssessment>,
    /// External packages observed but not registered as donors.
    pub unregistered: BTreeMap<(Ecosystem, String), Vec<Observation>>,
    /// Donors registered in an earlier census and absent now.
    pub vanished: Vec<String>,
    pub node_status: BTreeMap<String, NativeStatus>,
    pub findings: Vec<Finding>,
}

/// Maps repository paths to the declared node that owns them (longest path prefix).
pub struct NodeIndex {
    by_path: Vec<(String, String)>,
}

impl NodeIndex {
    pub fn new(d: &Declaration) -> NodeIndex {
        let mut by_path: Vec<(String, String)> = d
            .repository
            .nodes
            .iter()
            .filter(|n| {
                is_physical(n.kind) && !n.path.is_empty() && n.lifecycle != NodeLifecycle::Retired
            })
            .map(|n| (n.path.trim_end_matches('/').to_string(), n.key.clone()))
            .collect();
        by_path.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(&b.0)));
        NodeIndex { by_path }
    }
    pub fn owner(&self, file: &str) -> Option<&str> {
        self.by_path
            .iter()
            .find(|(p, _)| p == "." || file == p || file.starts_with(&format!("{p}/")))
            .map(|(_, k)| k.as_str())
    }
}

fn normalize(eco: Ecosystem, name: &str) -> String {
    crate::census::package_key(eco, name)
}

fn looks_like_path(spec: &str) -> bool {
    !spec.contains(char::is_whitespace)
        && (spec.contains('/') || spec.ends_with(".rs") || spec.ends_with(".md"))
}

pub fn analyze(
    d: &Declaration,
    files: &Files,
    census: &Census,
    store: &Store,
    history: &History,
) -> Analysis {
    let mut a = Analysis::default();
    let index = NodeIndex::new(d);
    let donor_of_package: BTreeMap<(Ecosystem, String), &str> = d
        .donors
        .iter()
        .flat_map(|dn| {
            dn.packages.iter().map(move |p| {
                (
                    (p.ecosystem, normalize(p.ecosystem, &p.name)),
                    dn.key.as_str(),
                )
            })
        })
        .collect();
    let source_owner = |path: &str| -> Option<&str> {
        d.donors.iter().find_map(|dn| {
            dn.source_paths.iter().find_map(|sp| {
                let sp = sp.trim_end_matches('/');
                (!sp.is_empty() && (path == sp || path.starts_with(&format!("{sp}/"))))
                    .then_some(dn.key.as_str())
            })
        })
    };

    let mut facts: BTreeMap<&str, DonorFacts> = d
        .donors
        .iter()
        .map(|dn| (dn.key.as_str(), DonorFacts::default()))
        .collect();
    // Per node: donors it uses outside tests, and whether it uses non-donor externals.
    let mut node_donors: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut node_external: BTreeSet<String> = BTreeSet::new();
    // Identifier → donor, for source imports (Cargo package names and manifest renames).
    let mut ident_donor: BTreeMap<String, &str> = BTreeMap::new();
    for dn in &d.donors {
        for p in &dn.packages {
            if p.ecosystem == Ecosystem::Cargo {
                ident_donor.insert(p.name.replace('-', "_"), dn.key.as_str());
            }
        }
    }

    for o in &census.observations {
        let donor = match o.via {
            Via::SourceReference => source_owner(&o.name),
            _ => donor_of_package
                .get(&(o.ecosystem, normalize(o.ecosystem, &o.name)))
                .copied(),
        };
        let owner = index.owner(&o.file).map(str::to_string);
        match donor {
            Some(k) => {
                if o.via == Via::Manifest && o.ecosystem == Ecosystem::Cargo {
                    ident_donor.insert(o.ident.clone(), k);
                }
                facts.get_mut(k).unwrap().push(o.clone());
                if o.scope != Scope::Test {
                    if let Some(n) = owner {
                        node_donors.entry(n).or_default().insert(k.to_string());
                    }
                }
            }
            None => {
                if matches!(o.via, Via::SourceReference | Via::Lock) {
                    continue;
                }
                if o.via == Via::Process && TOOLCHAIN_PROGRAMS.contains(&o.name.as_str()) {
                    continue;
                }
                if o.via == Via::Link && PLATFORM_LIBRARIES.contains(&o.name.as_str()) {
                    continue;
                }
                a.unregistered
                    .entry((o.ecosystem, normalize(o.ecosystem, &o.name)))
                    .or_default()
                    .push(o.clone());
                if o.scope != Scope::Test {
                    if let Some(n) = owner {
                        node_external.insert(n);
                    }
                }
            }
        }
    }

    for (file, roots) in &census.imports {
        for r in roots {
            if let Some(k) = ident_donor.get(r) {
                facts
                    .get_mut(k)
                    .unwrap()
                    .imports
                    .push((file.clone(), r.clone()));
                if crate::census::scope_of_file(file) != Scope::Test {
                    if let Some(n) = index.owner(file) {
                        node_donors
                            .entry(n.to_string())
                            .or_default()
                            .insert(k.to_string());
                    }
                }
            }
        }
    }

    for dn in &d.donors {
        let f = facts.get_mut(dn.key.as_str()).unwrap();
        for sp in &dn.source_paths {
            let sp = sp.trim_end_matches('/');
            if sp.is_empty() {
                continue;
            }
            f.resident.extend(files.under(sp).cloned());
            if files.paths.contains(sp) {
                f.resident.push(sp.to_string());
            }
            f.resident.extend(
                files
                    .gitlinks
                    .iter()
                    .filter(|g| *g == sp || g.starts_with(&format!("{sp}/")))
                    .cloned(),
            );
        }
        f.resident.sort();
        f.resident.dedup();
    }

    // Internal dependency adjacency (node keys), declared and observed, non-test.
    let mut deps: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for e in &d.repository.edges {
        if e.kind == EdgeKind::DependsOn
            && e.scope != Scope::Test
            && !e.from.contains(':')
            && !e.to.contains(':')
        {
            deps.entry(e.from.clone()).or_default().insert(e.to.clone());
        }
    }
    for (from, to, scope) in &census.internal {
        if *scope == Scope::Test {
            continue;
        }
        if let (Some(f), Some(t)) = (
            index.owner(&format!("{from}/Cargo.toml")),
            index.owner(&format!("{to}/Cargo.toml")),
        ) {
            if f != t {
                deps.entry(f.to_string()).or_default().insert(t.to_string());
            }
        }
    }
    let uses_donor = |node: &str, donor: &str| -> Option<Vec<String>> {
        // Breadth-first over internal dependencies; returns the path to a node using the donor.
        let mut seen = BTreeSet::new();
        let mut queue = vec![vec![node.to_string()]];
        while let Some(path) = queue.pop() {
            let last = path.last().unwrap().clone();
            if !seen.insert(last.clone()) {
                continue;
            }
            if node_donors.get(&last).is_some_and(|s| s.contains(donor)) {
                return Some(path);
            }
            for n in deps.get(&last).into_iter().flatten() {
                let mut p = path.clone();
                p.push(n.clone());
                queue.insert(0, p);
            }
        }
        None
    };

    // Node nativeness.
    for n in &d.repository.nodes {
        let status = if !is_physical(n.kind) || n.lifecycle == NodeLifecycle::Planned {
            NativeStatus::Abstract
        } else if n.kind == NodeKind::Compat {
            NativeStatus::Compat
        } else if node_donors.contains_key(&n.key) {
            NativeStatus::Wrapper
        } else if node_external.contains(&n.key) {
            NativeStatus::Dependent
        } else {
            NativeStatus::Native
        };
        a.node_status.insert(n.key.clone(), status);
    }

    // Dependents of each donor: declared edges, shims, and nodes that use it outside tests.
    for dn in &d.donors {
        let f = facts.get_mut(dn.key.as_str()).unwrap();
        let target = format!("donor:{}", dn.key);
        for e in &d.repository.edges {
            if e.to == target
                && !matches!(
                    e.kind,
                    EdgeKind::DerivesFrom | EdgeKind::Replaces | EdgeKind::Verifies
                )
            {
                f.dependents.push(format!("{} ({})", e.from, e.kind));
            }
        }
        for s in &d.migration.shims {
            if s.serves == dn.key || (s.kind == ShimKind::DonorFallback && s.serves == target) {
                f.dependents.push(format!("shim {} ({})", s.key, s.kind));
            }
        }
        for (node, ds) in &node_donors {
            if ds.contains(&dn.key) {
                f.dependents.push(format!("{node} (uses it)"));
            }
        }
        f.dependents.sort();
        f.dependents.dedup();
    }

    let declared_keys: BTreeSet<&str> = d.donors.iter().map(|x| x.key.as_str()).collect();
    for k in &history.registered {
        if !declared_keys.contains(k.as_str()) {
            a.vanished.push(k.clone());
            a.findings.push(Finding::new(
                Severity::Error,
                "DONOR_VANISHED",
                k,
                "registered in an earlier census and no longer declared; donors are never unregistered, they end EXTINCT, REJECTED or SUPERSEDED",
            ));
        }
    }

    for dn in &d.donors {
        let f = facts.remove(dn.key.as_str()).unwrap();
        let caps = capability_verdicts(d, files, store, dn, &|node| uses_donor(node, &dn.key));
        let assessment = assess(d, dn, f, caps, history, &mut a.findings);
        a.donors.push(assessment);
    }

    for ((eco, name), obs) in &a.unregistered {
        let mut sample: Vec<String> = obs.iter().map(|o| o.file.clone()).collect();
        sample.sort();
        sample.dedup();
        a.findings.push(Finding::new(
            Severity::Error,
            "UNREGISTERED_EXTERNAL",
            &format!("{eco}:{name}"),
            &format!(
                "external technology participates without a donor record (in {})",
                sample.join(", ")
            ),
        ));
    }
    a
}

fn capability_verdicts(
    d: &Declaration,
    files: &Files,
    store: &Store,
    dn: &Donor,
    uses_donor: &dyn Fn(&str) -> Option<Vec<String>>,
) -> Vec<CapabilityVerdict> {
    dn.capabilities
        .iter()
        .map(|c| {
            let specified =
                !c.spec.trim().is_empty() && (!looks_like_path(&c.spec) || files.exists(&c.spec));
            let node = c.replacement.as_deref().and_then(|k| d.node(k));
            let replacement_exists = node.is_some_and(|n| {
                n.lifecycle == NodeLifecycle::Active
                    && is_physical(n.kind)
                    && !n.path.is_empty()
                    && files.exists(&n.path)
            });
            let replacement_canonical = node.is_some_and(|n| {
                is_native_role(n.kind) && check_canonical_path(n.kind, &n.canonical_path).is_ok()
            });
            let (native, native_detail) = match (node, c.replacement.as_deref()) {
                (None, None) => (false, "no replacement declared".to_string()),
                (None, Some(k)) => (false, format!("replacement `{k}` is not a declared node")),
                (Some(n), _) if !replacement_exists => (
                    false,
                    format!("replacement `{}` does not exist at `{}`", n.key, n.path),
                ),
                (Some(n), _) if !replacement_canonical => (
                    false,
                    format!(
                        "replacement `{}` is {} at canonical `{}`: not a canonical native role",
                        n.key, n.kind, n.canonical_path
                    ),
                ),
                (Some(n), _) => match uses_donor(&n.key) {
                    Some(path) => (
                        false,
                        format!("WRAPPER: {} uses the donor", path.join(" -> ")),
                    ),
                    None => (true, format!("`{}` is native", n.key)),
                },
            };
            let verdicts = |kind: ProofKind| -> Vec<(String, Verdict)> {
                c.proofs
                    .iter()
                    .filter(|p| p.kind == kind)
                    .map(|p| {
                        (
                            p.locator.clone(),
                            judge(store, files, d, p, c.replacement.as_deref()),
                        )
                    })
                    .collect()
            };
            CapabilityVerdict {
                key: c.key.clone(),
                required: c.required,
                specified,
                targeted: node.is_some(),
                replacement_exists,
                replacement_canonical,
                mapped: c.maps_to.as_deref().is_some_and(|k| !k.trim().is_empty()),
                relevance_resolved: match &c.relevance {
                    crate::declare::Relevance::ReliedOn(_) => c.relevance.resolved() && c.required,
                    crate::declare::Relevance::NotRelevant(_) => {
                        c.relevance.resolved() && !c.required && files.exists(&c.spec)
                    }
                    crate::declare::Relevance::Unresolved => false,
                },
                native,
                native_detail,
                parity: verdicts(ProofKind::Parity),
                regression: verdicts(ProofKind::Regression),
            }
        })
        .collect()
}

fn assess(
    d: &Declaration,
    dn: &Donor,
    facts: DonorFacts,
    caps: Vec<CapabilityVerdict>,
    history: &History,
    findings: &mut Vec<Finding>,
) -> DonorAssessment {
    let required: Vec<&CapabilityVerdict> = caps.iter().filter(|c| c.required).collect();
    let gate_results = gates(&facts, &caps, dn.cutover.is_some());
    let all_gates = gate_results.iter().all(|g| g.pass);
    let failed = |pred: &dyn Fn(&CapabilityVerdict) -> bool| -> String {
        required
            .iter()
            .filter(|c| !pred(c))
            .map(|c| c.key.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };

    // One guard per rung; the effective state is the highest rung whose guard and all lower
    // guards hold.
    let ladder: Vec<(DonorState, bool, String)> = vec![
        (
            DonorState::Registered,
            dn.claimed >= DonorState::Registered
                && !dn.origin.trim().is_empty()
                && !dn.license.trim().is_empty(),
            if dn.claimed < DonorState::Registered {
                "declared as a DISCOVERED candidate; registration is an explicit claim".into()
            } else {
                "origin and licence must be recorded".into()
            },
        ),
        (
            DonorState::Censused,
            !required.is_empty(),
            "not decomposed into any required capability".into(),
        ),
        (
            DonorState::TechnologyMapped,
            required.iter().all(|c| c.mapped),
            format!(
                "capabilities not mapped to a capability or technology of the canonical graph (maps_to): {}",
                failed(&|c| c.mapped)
            ),
        ),
        (
            DonorState::Specified,
            required.iter().all(|c| c.specified),
            format!(
                "capabilities without a specification: {}",
                failed(&|c| c.specified)
            ),
        ),
        (
            DonorState::NativeTargeted,
            required.iter().all(|c| c.targeted),
            format!(
                "capabilities naming no declared replacement node (replacement): {}",
                failed(&|c| c.targeted)
            ),
        ),
        (
            DonorState::NativeShadow,
            required.iter().all(|c| c.replacement_exists),
            format!(
                "capabilities without an existing replacement node: {}",
                failed(&|c| c.replacement_exists)
            ),
        ),
        (
            DonorState::ParityProven,
            required.iter().all(|c| c.proven()),
            format!(
                "capabilities not natively replaced with fresh passing parity proofs: {}",
                required
                    .iter()
                    .filter(|c| !c.proven())
                    .map(|c| if c.native {
                        c.key.clone()
                    } else {
                        format!("{} ({})", c.key, c.native_detail)
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ),
        (
            DonorState::RelevanceResolved,
            caps.iter().all(|c| c.relevance_resolved),
            format!(
                "capabilities whose ECDEV relevance is unresolved or inconsistent with `required` (relevance): {}",
                caps.iter()
                    .filter(|c| !c.relevance_resolved)
                    .map(|c| c.key.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ),
        (
            DonorState::Cutover,
            dn.cutover.is_some()
                && facts.runtime.is_empty()
                && facts.linked.is_empty()
                && required.iter().all(|c| c.regression_pass()),
            if dn.cutover.is_none() {
                "no cutover declared".into()
            } else if !(facts.runtime.is_empty() && facts.linked.is_empty()) {
                "runtime or linked edges to the donor remain".into()
            } else {
                format!(
                    "capabilities without fresh passing regression proofs: {}",
                    failed(&|c| c.regression_pass())
                )
            },
        ),
        (
            DonorState::Extinct,
            all_gates,
            format!(
                "failing gates: {}",
                gate_results
                    .iter()
                    .filter(|g| !g.pass)
                    .map(|g| g.gate.wire())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ),
    ];
    let mut effective = DonorState::Discovered;
    let mut stopped_by = String::new();
    for (state, ok, why) in ladder {
        if ok {
            effective = state;
        } else {
            stopped_by = format!("{state}: {why}");
            break;
        }
    }

    // Exceptions.
    let mut exception = None;
    if let Some((kind, arg)) = &dn.exception {
        let legal = match kind {
            ExceptionKind::Blocked => (!arg.trim().is_empty())
                .then_some(())
                .ok_or("a blocker must be named".to_string()),
            ExceptionKind::Rejected => {
                if arg.trim().is_empty() {
                    Err("a rejection must state its reason".into())
                } else if facts.active() {
                    Err("the donor still participates in the repository".into())
                } else if history.active.contains(&dn.key) {
                    Err(
                        "census history observed the donor participating; it can only end EXTINCT"
                            .into(),
                    )
                } else if dn.capabilities.iter().any(|c| c.replacement.is_some()) {
                    Err("capabilities were replaced from it; it was adopted".into())
                } else {
                    Ok(())
                }
            }
            ExceptionKind::Superseded => match d.donor(arg) {
                None => Err(format!("successor `{arg}` is not a declared donor")),
                Some(s) if s.key == dn.key || s.exception.is_some_and_resolving() => Err(format!(
                    "successor `{arg}` is itself rejected, superseded or the donor itself"
                )),
                Some(s) => {
                    let missing_pkgs: Vec<&Package> = dn
                        .packages
                        .iter()
                        .filter(|p| !s.packages.contains(p))
                        .collect();
                    let succ_caps: BTreeSet<&str> =
                        s.capabilities.iter().map(|c| c.key.as_str()).collect();
                    let missing_caps: Vec<&str> = dn
                        .capabilities
                        .iter()
                        .filter(|c| c.required && !succ_caps.contains(c.key.as_str()))
                        .map(|c| c.key.as_str())
                        .collect();
                    if !missing_pkgs.is_empty() {
                        Err(format!(
                            "successor does not carry packages {missing_pkgs:?}"
                        ))
                    } else if !missing_caps.is_empty() {
                        Err(format!(
                            "successor does not cover capabilities {missing_caps:?}"
                        ))
                    } else {
                        Ok(())
                    }
                }
            },
        };
        match legal {
            Ok(()) => exception = Some((*kind, arg.clone())),
            Err(why) => findings.push(Finding::new(
                Severity::Error,
                "ILLEGAL_EXCEPTION",
                &dn.key,
                &format!("{kind} is not legal: {why}; the donor stays active"),
            )),
        }
    }

    let resolved = matches!(
        exception,
        Some((ExceptionKind::Rejected, _)) | Some((ExceptionKind::Superseded, _))
    );
    if dn.claimed > effective && !resolved {
        findings.push(Finding::new(
            Severity::Error,
            "CLAIM_EXCEEDS_EVIDENCE",
            &dn.key,
            &format!(
                "claims {} but evidence supports {}; stopped at {}",
                dn.claimed, effective, stopped_by
            ),
        ));
    }
    if dn.claimed > effective && !resolved {
        // Refinements of CLAIM_EXCEEDS_EVIDENCE: the claimed rung's own evidence is missing,
        // whatever lower rung the ladder stopped at.
        let decomposed = !required.is_empty();
        let unreplaced = failed(&|c| c.replacement_exists);
        if dn.claimed >= DonorState::NativeShadow && (!decomposed || !unreplaced.is_empty()) {
            findings.push(Finding::new(
                Severity::Error,
                "NATIVE_CLAIM_WITHOUT_EVIDENCE",
                &dn.key,
                &format!(
                    "claims {} but {}",
                    dn.claimed,
                    if decomposed {
                        format!("capabilities have no existing native replacement: {unreplaced}")
                    } else {
                        "no required capability is declared".into()
                    }
                ),
            ));
        }
        let unproven: Vec<String> = required
            .iter()
            .filter(|c| !c.proven())
            .map(|c| c.key.clone())
            .collect();
        if dn.claimed >= DonorState::Cutover && (!decomposed || !unproven.is_empty()) {
            findings.push(Finding::new(
                Severity::Error,
                "EXTINCTION_WITHOUT_PARITY",
                &dn.key,
                &format!(
                    "claims {} without native replacements with fresh passing parity proofs for: {}",
                    dn.claimed,
                    if decomposed {
                        unproven.join(", ")
                    } else {
                        "(no required capability)".into()
                    }
                ),
            ));
        }
    }
    if dn.claimed >= DonorState::NativeShadow && !resolved {
        // Every native replacement must carry its lineage back to the donor it learned from:
        // the replacement node's `lineage`, or a technology on it (or the one the capability
        // maps to) that lists the donor in its `lineage`.
        let mut reps: Vec<&str> = dn
            .capabilities
            .iter()
            .filter(|c| c.required)
            .filter_map(|c| c.replacement.as_deref())
            .collect();
        reps.sort();
        reps.dedup();
        let mapped: Vec<&str> = dn
            .capabilities
            .iter()
            .filter_map(|c| c.maps_to.as_deref()?.strip_prefix("technology/"))
            .collect();
        let traced = |rep: &str| {
            d.node(rep).is_some_and(|n| n.lineage.contains(&dn.key))
                || d.technologies.iter().any(|t| {
                    (t.node == rep || mapped.contains(&t.key.as_str()))
                        && t.lineage.contains(&dn.key)
                })
        };
        let untraced: Vec<&str> = reps.iter().copied().filter(|r| !traced(r)).collect();
        if !untraced.is_empty() {
            findings.push(Finding::new(
                Severity::Error,
                "DONOR_WITHOUT_LINEAGE",
                &dn.key,
                &format!(
                    "claims {} but replacements {} carry no lineage (DERIVES_FROM / LEARNED_FROM) back to it: add `{}` to their `lineage` or to a technology's",
                    dn.claimed,
                    untraced.join(", "),
                    dn.key
                ),
            ));
        }
    }
    if effective < DonorState::Registered && facts.active() && !resolved {
        findings.push(Finding::new(
            Severity::Error,
            "DISCOVERED_BUT_ACTIVE",
            &dn.key,
            "an unregistered donor participates in the repository; register it (origin, licence) or remove it",
        ));
    }
    if dn.claimed < effective {
        findings.push(Finding::new(
            Severity::Info,
            "CLAIM_BELOW_EVIDENCE",
            &dn.key,
            &format!(
                "claims {} but evidence supports {}; the claim may be raised",
                dn.claimed, effective
            ),
        ));
    }
    DonorAssessment {
        key: dn.key.clone(),
        claimed: dn.claimed,
        effective,
        exception,
        capabilities: caps,
        gates: gate_results,
        facts,
        stopped_by,
    }
}

trait Resolving {
    fn is_some_and_resolving(&self) -> bool;
}

impl Resolving for Option<(ExceptionKind, String)> {
    fn is_some_and_resolving(&self) -> bool {
        matches!(
            self,
            Some((ExceptionKind::Rejected, _)) | Some((ExceptionKind::Superseded, _))
        )
    }
}
