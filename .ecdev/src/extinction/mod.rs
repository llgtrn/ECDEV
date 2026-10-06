//! The extinction ruler. A donor is EXTINCT iff every gate holds; each gate is computed from
//! census observations, the graph and fresh evidence — never from a declared status, never from a
//! deleted directory.
//!
//! Classification of test, reference and oracle use (protocol v1):
//! * Running donor code in any build profile, tests included (dev-dependencies, oracle binaries,
//!   oracle build scripts), is an edge: TEST_EDGES_ZERO fails. A donor used as a live oracle is
//!   at most PARITY_PROVEN or CUTOVER, never EXTINCT.
//! * Oracle *outputs* frozen as fixture data (with provenance) are not donor code and may remain.
//! * Provenance, specifications, licences and learned descriptions may remain.
//! * Donor source tracked anywhere in the repository (vendor/, temporary/, .atlas/, research/,
//!   fixtures, generated code) fails RESIDENT_SOURCE_ZERO whether or not it is built.

use crate::census::{Observation, Via};
use crate::evidence::Verdict;
use crate::schema::{Gate, Scope};

/// Everything observed about one donor.
#[derive(Clone, Debug, Default)]
pub struct DonorFacts {
    pub runtime: Vec<Observation>,
    pub build: Vec<Observation>,
    pub linked: Vec<Observation>,
    pub test: Vec<Observation>,
    /// (file, identifier) pairs importing the donor.
    pub imports: Vec<(String, String)>,
    /// Tracked donor source files (and gitlinks).
    pub resident: Vec<String>,
    /// Nodes that consume, call, control or shim the donor, by key.
    pub dependents: Vec<String>,
}

impl DonorFacts {
    pub fn push(&mut self, o: Observation) {
        let v = match o.scope {
            Scope::Runtime | Scope::Semantic | Scope::Architectural => &mut self.runtime,
            Scope::Build => &mut self.build,
            Scope::Linked => &mut self.linked,
            Scope::Test => &mut self.test,
        };
        if !v.contains(&o) {
            v.push(o);
        }
    }
    /// Whether the donor participates in the repository in any way.
    pub fn active(&self) -> bool {
        !(self.runtime.is_empty()
            && self.build.is_empty()
            && self.linked.is_empty()
            && self.test.is_empty()
            && self.imports.is_empty()
            && self.resident.is_empty())
    }
}

#[derive(Clone, Debug)]
pub struct CapabilityVerdict {
    pub key: String,
    pub required: bool,
    pub specified: bool,
    /// Its replacement names a declared node (any lifecycle: a PLANNED node is a target).
    pub targeted: bool,
    pub replacement_exists: bool,
    pub replacement_canonical: bool,
    /// It maps to a capability or technology of the repository graph (`maps_to` is non-empty).
    pub mapped: bool,
    /// Its relevance to ECDEV is resolved and agrees with `required`: relied on (and required),
    /// or justified not relevant to ECDEV (and not required).
    pub relevance_resolved: bool,
    /// Replacement exists, is canonical, and neither it nor anything it depends on uses the donor.
    pub native: bool,
    pub native_detail: String,
    pub parity: Vec<(String, Verdict)>,
    pub regression: Vec<(String, Verdict)>,
}

impl CapabilityVerdict {
    pub fn parity_pass(&self) -> bool {
        !self.parity.is_empty() && self.parity.iter().all(|(_, v)| *v == Verdict::Pass)
    }
    pub fn regression_pass(&self) -> bool {
        !self.regression.is_empty() && self.regression.iter().all(|(_, v)| *v == Verdict::Pass)
    }
    pub fn proven(&self) -> bool {
        self.native && self.parity_pass()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GateResult {
    pub gate: Gate,
    pub pass: bool,
    pub detail: String,
}

fn sample(obs: &[Observation]) -> String {
    let mut v: Vec<String> = obs
        .iter()
        .map(|o| match o.via {
            Via::Lock => {
                // `ident` is the path from the direct dependency; its parent pulls it in.
                let path = o.ident.split(" (in ").next().unwrap_or(&o.ident);
                let parts: Vec<&str> = path.split(" → ").collect();
                let via = parts
                    .len()
                    .checked_sub(2)
                    .map_or("no workspace package", |i| parts[i]);
                format!(
                    "LOCK {} {} linked via {via}: {} → {}",
                    o.ecosystem, o.name, o.file, o.ident
                )
            }
            _ => format!("{} {} {} in {}", o.via.wire(), o.ecosystem, o.name, o.file),
        })
        .collect();
    v.sort();
    let n = v.len();
    v.truncate(3);
    if n > 3 {
        v.push(format!("… {} more", n - 3));
    }
    v.join("; ")
}

/// Evaluates every extinction gate for one donor.
pub fn gates(facts: &DonorFacts, caps: &[CapabilityVerdict], cutover: bool) -> Vec<GateResult> {
    let required: Vec<&CapabilityVerdict> = caps.iter().filter(|c| c.required).collect();
    let none_required = required.is_empty();
    let failing = |f: &dyn Fn(&CapabilityVerdict) -> bool| -> Vec<String> {
        required
            .iter()
            .filter(|c| !f(c))
            .map(|c| c.key.clone())
            .collect()
    };
    let cap_gate = |gate: Gate, missing: Vec<String>, what: &str| GateResult {
        gate,
        pass: !none_required && missing.is_empty(),
        detail: if none_required {
            "no required capability declared: the donor is not decomposed".into()
        } else if missing.is_empty() {
            format!("all {} required capabilities {what}", required.len())
        } else {
            format!(
                "{} of {} required capabilities lack it: {}",
                missing.len(),
                required.len(),
                missing.join(", ")
            )
        },
    };
    let edge_gate = |gate: Gate, obs: &[Observation]| GateResult {
        gate,
        pass: obs.is_empty(),
        detail: if obs.is_empty() {
            "none observed".into()
        } else {
            format!("{} observed: {}", obs.len(), sample(obs))
        },
    };
    vec![
        edge_gate(Gate::RuntimeEdges, &facts.runtime),
        edge_gate(Gate::BuildEdges, &facts.build),
        edge_gate(Gate::LinkedEdges, &facts.linked),
        edge_gate(Gate::TestEdges, &facts.test),
        GateResult {
            gate: Gate::SourceImports,
            pass: facts.imports.is_empty(),
            detail: if facts.imports.is_empty() {
                "none observed".into()
            } else {
                let mut v: Vec<String> = facts
                    .imports
                    .iter()
                    .map(|(f, i)| format!("{i} in {f}"))
                    .collect();
                let n = v.len();
                v.truncate(3);
                format!("{n} imports: {}", v.join("; "))
            },
        },
        GateResult {
            gate: Gate::ResidentSource,
            pass: facts.resident.is_empty(),
            detail: if facts.resident.is_empty() {
                "no donor source tracked".into()
            } else {
                format!(
                    "{} tracked donor files, e.g. {}",
                    facts.resident.len(),
                    facts.resident[0]
                )
            },
        },
        cap_gate(
            Gate::CapabilityCoverage,
            failing(&|c| c.native),
            "have a valid native replacement",
        ),
        cap_gate(
            Gate::ParityProofs,
            failing(&|c| c.parity_pass()),
            "have fresh passing parity proofs",
        ),
        cap_gate(
            Gate::RegressionTests,
            failing(&|c| c.regression_pass()),
            "have fresh passing regression proofs",
        ),
        cap_gate(
            Gate::CanonicalReplacement,
            failing(&|c| c.replacement_exists && c.replacement_canonical),
            "have an existing canonical replacement node",
        ),
        cap_gate(
            Gate::TechnologyMapping,
            failing(&|c| c.mapped),
            "map to a capability or technology of the canonical graph",
        ),
        {
            // Every declared capability, required or not: what ECDEV relies on must be native;
            // what it does not must say why.
            let missing: Vec<String> = caps
                .iter()
                .filter(|c| !c.relevance_resolved)
                .map(|c| c.key.clone())
                .collect();
            GateResult {
                gate: Gate::Relevance,
                pass: !caps.is_empty() && missing.is_empty(),
                detail: if caps.is_empty() {
                    "no capability declared: the donor is not decomposed".into()
                } else if missing.is_empty() {
                    format!(
                        "all {} declared capabilities are relied on or justified not relevant to ECDEV",
                        caps.len()
                    )
                } else {
                    format!(
                        "{} of {} declared capabilities have no resolved ECDEV relevance: {}",
                        missing.len(),
                        caps.len(),
                        missing.join(", ")
                    )
                },
            }
        },
        GateResult {
            gate: Gate::CutoverDone,
            pass: cutover,
            detail: if cutover {
                "cutover declared".into()
            } else {
                "no cutover declared".into()
            },
        },
        GateResult {
            gate: Gate::RollbackIndependent,
            pass: facts.dependents.is_empty(),
            detail: if facts.dependents.is_empty() {
                "no node consumes, calls, controls or shims the donor".into()
            } else {
                format!("still depended on by: {}", facts.dependents.join(", "))
            },
        },
    ]
}
