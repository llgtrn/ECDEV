//! The universe of a repository: everything it knows about — OSS repositories, information
//! sources, authorities, protocol specs, papers, benchmarks, datasets, simulators, standards,
//! reference architectures — as lossless, typed, canonical records. `declared/donors.rs` is not
//! the universe: it materializes the part of it under the donor lifecycle.
//!
//! Storage: `.ecdev/declared/universe/<source>.jsonl`, one file per reconciled legacy
//! registry. Line 1 describes the source (its path, format, digest, size and the text between
//! records); every other line is one record, rendered canonically on one line (fixed member
//! order). JSONL scales to tens of thousands of records per repository, is deterministic (an
//! import of the same bytes is byte-identical) and diffs one record per line; rustc-checked
//! literals would make every build compile thousands of records, and a binary knowledge batch
//! would not diff. Each record keeps its legacy text verbatim, so the source rebuilds byte for
//! byte from the file (proven against the recorded digest whenever the universe is read).
//!
//! Every repository relates to each global donor once: [`RepositoryUsesDonor`], derived from
//! its universe records and its declared donors, grouped by global donor identity (see
//! `identity`). Its lifecycle layer is the universe-record state below CENSUSED and the donor
//! ladder's effective state from CENSUSED up, so one ladder reads
//!
//! ```text
//! DISCOVERED → REGISTERED → RELEVANCE_RESOLVED → SELECTED_FOR_CENSUS      (universe records)
//!   → CENSUSED → TECHNOLOGY_MAPPED → SPECIFIED → NATIVE_TARGETED → NATIVE_SHADOW
//!   → PARITY_PROVEN → CAPABILITY_RELEVANCE_RESOLVED → CUTOVER → EXTINCT        (declared donors)
//! ```
//!
//! A technology a donor taught is never extinct with it.

pub mod identity;
pub mod import;
pub mod summary;

use crate::declare::Declaration;
use crate::donors::Analysis;
use crate::formats::json::{self, Json};
use crate::formats::Value;
use crate::repository::files::Files;
use crate::schema::{DonorState, ExceptionKind, UniverseKind, UniverseState};
use crate::{Finding, Severity};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub const UNIVERSE_DIR: &str = ".ecdev/declared/universe";
pub const UNIVERSE_SCHEMA: &str = "ecdev.universe.v1";

/// The registries ECDEV's universe is reconciled from. One that exists and has no universe file
/// of the same bytes is NOT RECONCILED (or STALE): its records are unknown to governance, never
/// zero. (`*` matches within one path segment.)
pub const LEGACY_SOURCES: &[&str] = &["research/commerce/donors/registry.json"];

/// A record's relevance to its repository.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Relevance {
    #[default]
    Unresolved,
    Relevant(String),
    NotRelevant(String),
}

impl Relevance {
    pub fn word(&self) -> &'static str {
        match self {
            Relevance::Unresolved => "UNRESOLVED",
            Relevance::Relevant(_) => "RELEVANT",
            Relevance::NotRelevant(_) => "NOT_RELEVANT",
        }
    }
    pub fn reason(&self) -> &str {
        match self {
            Relevance::Unresolved => "",
            Relevance::Relevant(r) | Relevance::NotRelevant(r) => r,
        }
    }
    pub fn from_parts(word: &str, reason: &str) -> Option<Relevance> {
        Some(match word {
            "UNRESOLVED" => Relevance::Unresolved,
            "RELEVANT" => Relevance::Relevant(reason.to_string()),
            "NOT_RELEVANT" => Relevance::NotRelevant(reason.to_string()),
            _ => return None,
        })
    }
    pub fn resolved(&self) -> bool {
        !matches!(self, Relevance::Unresolved) && !self.reason().trim().is_empty()
    }
    pub fn relevant(&self) -> bool {
        matches!(self, Relevance::Relevant(r) if !r.trim().is_empty())
    }
    pub fn wire(&self) -> String {
        match self {
            Relevance::Unresolved => "UNRESOLVED".into(),
            r => format!("{}({})", r.word(), r.reason()),
        }
    }
}

/// A declared alias: another origin of the same upstream (a legacy id, a renamed or
/// redirected URL), legal only with provenance.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Alias {
    pub origin: String,
    pub provenance: String,
}

/// One canonical universe record.
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    /// Its id within the source (the legacy id, or key).
    pub id: String,
    pub kind: UniverseKind,
    pub name: String,
    /// The upstream as the legacy record states it (empty when it names none).
    pub origin: String,
    /// The global donor key derived from `origin` (`None`: unknown identity).
    pub global: Option<String>,
    /// The origin was checked (reachable, fetched or pinned at a revision).
    pub origin_verified: bool,
    pub origin_evidence: String,
    /// The licence, when verified (read from the upstream's own licence file).
    pub license: String,
    /// A licence the legacy record states without verification.
    pub license_claim: String,
    /// The pinned revision, when the record names one.
    pub revision: String,
    pub relevance: Relevance,
    /// Selected for deep census (cloned or pinned for census, or in a donor ledger).
    pub selected: bool,
    /// The universe state its record claims; the effective state is computed.
    pub claimed: UniverseState,
    /// The legacy lifecycle words, verbatim (claims, never evidence).
    pub legacy_state: String,
    pub aliases: Vec<Alias>,
    /// Where it came from: `<legacy path>#<id>`.
    pub provenance: String,
    /// The record's legacy text, verbatim.
    pub legacy: String,
}

impl Record {
    /// The effective universe state.
    pub fn effective(&self) -> UniverseState {
        let registered = self.claimed >= UniverseState::Registered && self.global.is_some();
        if !registered {
            UniverseState::Discovered
        } else if !self.relevance.resolved() {
            UniverseState::Registered
        } else if !(self.relevance.relevant() && self.selected) {
            UniverseState::RelevanceResolved
        } else {
            UniverseState::SelectedForCensus
        }
    }

    pub fn to_json(&self) -> Json {
        Json::obj()
            .with("id", &self.id)
            .with("kind", self.kind.wire())
            .with("name", &self.name)
            .with("origin", &self.origin)
            .with("global_donor", self.global.clone().unwrap_or_default())
            .with("origin_verified", self.origin_verified)
            .with("origin_evidence", &self.origin_evidence)
            .with("license", &self.license)
            .with("license_claim", &self.license_claim)
            .with("revision", &self.revision)
            .with("relevance", self.relevance.word())
            .with("relevance_reason", self.relevance.reason())
            .with("selected", self.selected)
            .with("claimed", self.claimed.wire())
            .with("legacy_state", &self.legacy_state)
            .with(
                "aliases",
                Json::Array(
                    self.aliases
                        .iter()
                        .map(|a| {
                            Json::obj()
                                .with("origin", &a.origin)
                                .with("provenance", &a.provenance)
                        })
                        .collect(),
                ),
            )
            .with("provenance", &self.provenance)
            .with("legacy", &self.legacy)
    }

    fn from_value(v: &Value) -> Result<Record, String> {
        let s = |k: &str| -> Result<String, String> {
            match v.get(k) {
                Some(Value::Str(x)) => Ok(x.clone()),
                _ => Err(format!("missing string `{k}`")),
            }
        };
        let b = |k: &str| -> Result<bool, String> {
            match v.get(k) {
                Some(Value::Bool(x)) => Ok(*x),
                _ => Err(format!("missing boolean `{k}`")),
            }
        };
        let kind = s("kind")?;
        let claimed = s("claimed")?;
        let relevance = Relevance::from_parts(&s("relevance")?, &s("relevance_reason")?)
            .ok_or("`relevance` is not UNRESOLVED | RELEVANT | NOT_RELEVANT")?;
        let origin = s("origin")?;
        let global = s("global_donor")?;
        let r = Record {
            id: s("id")?,
            kind: UniverseKind::from_wire(&kind).ok_or(format!("`{kind}` is not a kind"))?,
            name: s("name")?,
            global: (!global.is_empty()).then_some(global),
            origin,
            origin_verified: b("origin_verified")?,
            origin_evidence: s("origin_evidence")?,
            license: s("license")?,
            license_claim: s("license_claim")?,
            revision: s("revision")?,
            relevance,
            selected: b("selected")?,
            claimed: UniverseState::from_wire(&claimed)
                .ok_or(format!("`{claimed}` is not a universe state"))?,
            legacy_state: s("legacy_state")?,
            aliases: v
                .items_of("aliases")
                .iter()
                .map(|a| Alias {
                    origin: a.text("origin"),
                    provenance: a.text("provenance"),
                })
                .collect(),
            provenance: s("provenance")?,
            legacy: s("legacy")?,
        };
        let expected = identity::global_donor_key(&r.origin);
        if r.global != expected {
            return Err(format!(
                "record `{}`: global_donor `{}` is not the identity of origin `{}` (`{}`): re-run `ecdev-gov migrate universe`",
                r.id,
                r.global.clone().unwrap_or_default(),
                r.origin,
                expected.unwrap_or_default()
            ));
        }
        Ok(r)
    }
}

/// One reconciled legacy registry.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    /// The universe file, repository-relative.
    pub file: String,
    /// The legacy registry it reconciles, repository-relative.
    pub source: String,
    /// `JSONL`, `TOML` or `JSON`.
    pub format: String,
    /// The record table (TOML array of tables, JSON array key); empty for JSONL.
    pub table: String,
    /// `sha256:` digest of the legacy bytes.
    pub digest: String,
    pub bytes: u64,
    /// The legacy text that is not a record, as (number of records before it, text).
    pub segments: Vec<(u64, String)>,
    pub records: Vec<Record>,
}

impl Source {
    /// The legacy registry, byte for byte.
    pub fn rebuild(&self) -> String {
        let mut out = String::new();
        let mut seg = self.segments.iter().peekable();
        for (i, r) in self.records.iter().enumerate() {
            while let Some((_, t)) = seg.next_if(|(p, _)| *p as usize <= i) {
                out.push_str(t);
            }
            out.push_str(&r.legacy);
        }
        for (_, t) in seg {
            out.push_str(t);
        }
        out
    }

    pub fn render(&self) -> String {
        let header = Json::obj()
            .with("universe", UNIVERSE_SCHEMA)
            .with("source", &self.source)
            .with("format", &self.format)
            .with("table", &self.table)
            .with("digest", &self.digest)
            .with("bytes", self.bytes)
            .with("records", self.records.len())
            .with(
                "segments",
                Json::Array(
                    self.segments
                        .iter()
                        .map(|(p, t)| Json::Array(vec![Json::Int(*p as i64), Json::Str(t.clone())]))
                        .collect(),
                ),
            );
        let mut s = header.render_line();
        s.push('\n');
        for r in &self.records {
            s.push_str(&r.to_json().render_line());
            s.push('\n');
        }
        s
    }

    /// Reads a universe file; every record is checked, and the records and segments must
    /// rebuild the legacy bytes the header's digest names (the import was lossless).
    pub fn parse(file: &str, text: &str) -> Result<Source, String> {
        let mut lines = text.lines();
        let head = json::parse(lines.next().ok_or("empty universe file")?)
            .map_err(|e| format!("header: {e}"))?;
        if head.str("universe") != Some(UNIVERSE_SCHEMA) {
            return Err(format!("header is not `{UNIVERSE_SCHEMA}`"));
        }
        let int = |k: &str| match head.get(k) {
            Some(Value::Int(n)) if *n >= 0 => Ok(*n as u64),
            _ => Err(format!("header: missing count `{k}`")),
        };
        let mut segments = Vec::new();
        for s in head.items_of("segments") {
            match s.items() {
                [Value::Int(p), Value::Str(t)] if *p >= 0 => segments.push((*p as u64, t.clone())),
                _ => return Err("header: a segment is not [position, text]".into()),
            }
        }
        let mut records = Vec::new();
        let mut ids = BTreeSet::new();
        for (n, line) in lines.enumerate() {
            let v = json::parse(line).map_err(|e| format!("line {}: {e}", n + 2))?;
            let r = Record::from_value(&v).map_err(|e| format!("line {}: {e}", n + 2))?;
            if !ids.insert(r.id.clone()) {
                return Err(format!("line {}: duplicate record id `{}`", n + 2, r.id));
            }
            records.push(r);
        }
        let src = Source {
            file: file.to_string(),
            source: head.text("source"),
            format: head.text("format"),
            table: head.text("table"),
            digest: head.text("digest"),
            bytes: int("bytes")?,
            segments,
            records,
        };
        if int("records")? != src.records.len() as u64 {
            return Err(format!(
                "header counts {} records, the file holds {}",
                int("records")?,
                src.records.len()
            ));
        }
        let rebuilt = src.rebuild();
        let digest = crate::digest::content_digest(rebuilt.as_bytes());
        if digest != src.digest || rebuilt.len() as u64 != src.bytes {
            return Err(format!(
                "records do not rebuild `{}` ({} bytes, {digest}; recorded {} bytes, {}): the universe is not lossless",
                src.source,
                rebuilt.len(),
                src.bytes,
                src.digest
            ));
        }
        Ok(src)
    }
}

/// Every universe file of a repository.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Universe {
    pub sources: Vec<Source>,
    /// (file, why it cannot be read).
    pub unreadable: Vec<(String, String)>,
}

impl Universe {
    pub fn load(root: &Path) -> Universe {
        let mut u = Universe::default();
        let Ok(rd) = std::fs::read_dir(root.join(UNIVERSE_DIR)) else {
            return u;
        };
        let mut names: Vec<String> = rd
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".jsonl"))
            .collect();
        names.sort();
        for n in names {
            let file = format!("{UNIVERSE_DIR}/{n}");
            match std::fs::read_to_string(root.join(&file)) {
                Ok(t) => match Source::parse(&file, &t) {
                    Ok(s) => u.sources.push(s),
                    Err(e) => u.unreadable.push((file, e)),
                },
                Err(e) => u.unreadable.push((file, e.to_string())),
            }
        }
        u
    }

    pub fn records(&self) -> impl Iterator<Item = (&Source, &Record)> {
        self.sources
            .iter()
            .flat_map(|s| s.records.iter().map(move |r| (s, r)))
    }
}

/// Whether `path` matches a legacy-source pattern (`*` within one segment).
pub fn matches_pattern(pattern: &str, path: &str) -> bool {
    let (ps, xs): (Vec<&str>, Vec<&str>) =
        (pattern.split('/').collect(), path.split('/').collect());
    ps.len() == xs.len()
        && ps.iter().zip(&xs).all(|(p, x)| match p.split_once('*') {
            Some((a, b)) => x.starts_with(a) && x.ends_with(b) && x.len() >= a.len() + b.len(),
            None => p == x,
        })
}

/// The legacy registries present in a repository.
pub fn legacy_sources(files: &Files) -> Vec<String> {
    files
        .paths
        .iter()
        .filter(|p| LEGACY_SOURCES.iter().any(|pat| matches_pattern(pat, p)))
        .cloned()
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Relations: a repository and each global donor it knows.

/// A lifecycle layer: a universe-record state below CENSUSED, the donor ladder from CENSUSED.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    Universe(UniverseState),
    Donor(DonorState),
}

impl Layer {
    pub fn rank(self) -> u8 {
        match self {
            Layer::Universe(s) => s.rank(),
            Layer::Donor(s) => {
                UniverseState::ALL.len() as u8
                    + s.rank().saturating_sub(DonorState::Censused.rank())
            }
        }
    }
    pub fn wire(self) -> &'static str {
        match self {
            Layer::Universe(s) => s.wire(),
            Layer::Donor(s) => s.wire(),
        }
    }
    pub fn from_wire(w: &str) -> Option<Layer> {
        UniverseState::from_wire(w)
            .map(Layer::Universe)
            .or_else(|| {
                DonorState::from_wire(w)
                    .filter(|s| *s >= DonorState::Censused)
                    .map(Layer::Donor)
            })
    }
    pub fn at_least(self, other: Layer) -> bool {
        self.rank() >= other.rank()
    }
}

/// The one lifecycle ladder of the universe and the donor lifecycle.
pub fn layers() -> Vec<Layer> {
    UniverseState::ALL
        .iter()
        .map(|s| Layer::Universe(*s))
        .chain(
            DonorState::ALL
                .iter()
                .filter(|s| **s >= DonorState::Censused)
                .map(|s| Layer::Donor(*s)),
        )
        .collect()
}

/// A repository's relation to one global donor (or, with no global identity, to one record).
#[derive(Clone, Debug, PartialEq)]
pub struct RepositoryUsesDonor {
    pub repository: String,
    /// The global donor key (`None`: unknown identity).
    pub global_donor: Option<String>,
    /// The global key, or `<source>#<id>` / `donors.rs#<key>` for an unknown identity.
    pub label: String,
    /// The primary kind (the first, in vocabulary order, of its records' kinds).
    pub kind: UniverseKind,
    pub relevance: Relevance,
    /// Selected for deep census (effective).
    pub selected: bool,
    pub lifecycle: Layer,
    pub origin_verified: bool,
    pub license_verified: bool,
    /// Declared donor keys of this repository with this upstream.
    pub donors: Vec<String>,
    /// Universe records: `<legacy path>#<id>`.
    pub records: Vec<String>,
    /// Declared aliases (alias origin, provenance).
    pub aliases: Vec<(String, String)>,
    /// Graph identities of its declared donors' nodes (v1 keys).
    pub node_ids: Vec<String>,
}

/// The state of one legacy registry of the repository.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SourceState {
    /// A universe file holds exactly its bytes.
    Reconciled,
    /// Imported, but the registry changed since.
    Stale,
    /// No universe file: its records are unknown to governance.
    NotReconciled,
}

impl SourceState {
    pub fn wire(self) -> &'static str {
        match self {
            SourceState::Reconciled => "RECONCILED",
            SourceState::Stale => "STALE",
            SourceState::NotReconciled => "NOT_RECONCILED",
        }
    }
    pub fn from_wire(w: &str) -> Option<SourceState> {
        [
            SourceState::Reconciled,
            SourceState::Stale,
            SourceState::NotReconciled,
        ]
        .into_iter()
        .find(|s| s.wire() == w)
    }
}

/// The universe as one repository judges it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub relations: Vec<RepositoryUsesDonor>,
    /// (legacy path, state); a universe file whose registry is gone is RECONCILED.
    pub sources: Vec<(String, SourceState)>,
    /// (look-alike name, distinct global keys).
    pub ambiguous: Vec<(String, Vec<String>)>,
    pub findings: Vec<Finding>,
}

#[derive(Default)]
struct Acc {
    kinds: BTreeSet<UniverseKind>,
    relevance: Vec<Relevance>,
    selected: bool,
    layer: Option<Layer>,
    origin_verified: bool,
    license_verified: bool,
    donors: Vec<String>,
    records: Vec<String>,
    aliases: Vec<(String, String)>,
    node_ids: Vec<String>,
}

impl Acc {
    fn raise(&mut self, l: Layer) {
        if self.layer.is_none_or(|x| l.rank() > x.rank()) {
            self.layer = Some(l);
        }
    }
    fn finish(
        mut self,
        repository: &str,
        global: Option<String>,
        label: String,
    ) -> RepositoryUsesDonor {
        let relevance = self
            .relevance
            .iter()
            .find(|r| r.relevant())
            .or_else(|| self.relevance.iter().find(|r| r.resolved()))
            .cloned()
            .unwrap_or_default();
        for v in [&mut self.donors, &mut self.records, &mut self.node_ids] {
            v.sort();
            v.dedup();
        }
        self.aliases.sort();
        self.aliases.dedup();
        RepositoryUsesDonor {
            repository: repository.to_string(),
            global_donor: global,
            label,
            kind: self
                .kinds
                .iter()
                .next()
                .copied()
                .unwrap_or(UniverseKind::OssRepository),
            relevance,
            selected: self.selected,
            lifecycle: self
                .layer
                .unwrap_or(Layer::Universe(UniverseState::Discovered)),
            origin_verified: self.origin_verified,
            license_verified: self.license_verified,
            donors: self.donors,
            records: self.records,
            aliases: self.aliases,
            node_ids: self.node_ids,
        }
    }
}

/// Look-alike groups among OSS relations: one name, several unaliased global keys.
pub fn ambiguous_groups<'a>(
    keys: impl IntoIterator<Item = (&'a str, bool)>,
) -> Vec<(String, Vec<String>)> {
    let mut by_name: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (k, oss) in keys {
        let name = identity::look_alike_name(k);
        if oss && name.len() >= 2 {
            by_name.entry(name).or_default().insert(k.to_string());
        }
    }
    by_name
        .into_iter()
        .filter(|(_, ks)| ks.len() > 1)
        .map(|(n, ks)| (n, ks.into_iter().collect()))
        .collect()
}

/// Derives every relation of the repository, the state of its legacy registries and the
/// identity findings.
pub fn assess(d: &Declaration, analysis: &Analysis, universe: &Universe, files: &Files) -> Report {
    let repo_id = d.repository.id.as_str();
    let mut findings = Vec::new();
    for (file, why) in &universe.unreadable {
        findings.push(Finding::new(
            Severity::Error,
            "UNIVERSE_INVALID",
            file,
            &format!("{why} (the universe is not read; its records count as not reconciled)"),
        ));
    }

    // Declared aliases join global keys; an alias without provenance is no evidence.
    let mut aliases = identity::Aliases::default();
    for (s, r) in universe.records() {
        let Some(g) = &r.global else { continue };
        for a in &r.aliases {
            match identity::global_donor_key(&a.origin) {
                Some(k) if !a.provenance.trim().is_empty() => aliases.join(g, &k),
                Some(_) => findings.push(Finding::new(
                    Severity::Error,
                    "UNPROVEN_ALIAS",
                    &format!("{}#{}", s.source, r.id),
                    &format!(
                        "alias `{}` declares no provenance: an alias is evidence that two origins are one upstream, never a guess",
                        a.origin
                    ),
                )),
                None => findings.push(Finding::new(
                    Severity::Error,
                    "UNIVERSE_INVALID",
                    &format!("{}#{}", s.source, r.id),
                    &format!("alias `{}` names no upstream", a.origin),
                )),
            }
        }
    }

    let mut known: BTreeMap<String, Acc> = BTreeMap::new();
    let mut unknown: Vec<(String, Acc)> = Vec::new();
    for (s, r) in universe.records() {
        let mut acc = Acc::default();
        acc.kinds.insert(r.kind);
        acc.relevance.push(r.relevance.clone());
        let eff = r.effective();
        acc.selected = eff == UniverseState::SelectedForCensus;
        acc.raise(Layer::Universe(eff));
        acc.origin_verified = r.origin_verified;
        acc.license_verified = !r.license.trim().is_empty();
        let label = format!("{}#{}", s.source, r.id);
        acc.records.push(label.clone());
        acc.aliases.extend(
            r.aliases
                .iter()
                .map(|a| (a.origin.clone(), a.provenance.clone())),
        );
        match &r.global {
            Some(g) => merge(known.entry(aliases.canonical(g)).or_default(), acc),
            None => unknown.push((label, acc)),
        }
    }
    for da in &analysis.donors {
        let Some(dn) = d.donor(&da.key) else { continue };
        let mut acc = Acc::default();
        acc.kinds.insert(UniverseKind::OssRepository);
        acc.donors.push(dn.key.clone());
        let (ns, key) = crate::graph::Graph::donor_id(repo_id, &dn.key, &dn.origin);
        acc.node_ids
            .push(crate::graph::NodeId::of(&ns, &key).to_string());
        match &da.exception {
            Some((ExceptionKind::Rejected, why)) => {
                // A runtime rejection is not a knowledge verdict: the relation stays relevant
                // unless every extracted capability has no research value.
                let all_without_value = !dn.capabilities.is_empty()
                    && dn.capabilities.iter().all(|c| {
                        matches!(c.knowledge, crate::declare::Knowledge::NoResearchValue(..))
                    });
                acc.relevance.push(if all_without_value {
                    Relevance::NotRelevant(format!("runtime rejected and no research value: {why}"))
                } else {
                    Relevance::Relevant(format!("runtime rejected, knowledge retained: {why}"))
                });
                acc.raise(Layer::Universe(if da.effective >= DonorState::Registered {
                    UniverseState::RelevanceResolved
                } else {
                    UniverseState::Discovered
                }));
            }
            _ if da.effective < DonorState::Registered => {
                acc.raise(Layer::Universe(UniverseState::Discovered));
            }
            _ => {
                acc.relevance
                    .push(Relevance::Relevant(format!("declared donor `{}`", dn.key)));
                acc.selected = true;
                acc.raise(if da.effective >= DonorState::Censused {
                    Layer::Donor(da.effective)
                } else {
                    Layer::Universe(UniverseState::SelectedForCensus)
                });
            }
        }
        match identity::global_donor_key(&dn.origin) {
            Some(g) => merge(known.entry(aliases.canonical(&g)).or_default(), acc),
            None => unknown.push((format!("donors.rs#{}", dn.key), acc)),
        }
    }

    let mut relations: Vec<RepositoryUsesDonor> = known
        .into_iter()
        .map(|(k, acc)| acc.finish(repo_id, Some(k.clone()), k))
        .collect();
    relations.extend(
        unknown
            .into_iter()
            .map(|(label, acc)| acc.finish(repo_id, None, label)),
    );

    let ambiguous = ambiguous_groups(relations.iter().filter_map(|r| {
        r.global_donor
            .as_deref()
            .map(|k| (k, r.kind == UniverseKind::OssRepository))
    }));
    for (name, keys) in &ambiguous {
        findings.push(Finding::new(
            Severity::Warning,
            "AMBIGUOUS_DONOR_IDENTITY",
            name,
            &format!(
                "{} distinct upstreams look alike ({}); they stay distinct until an alias with provenance proves them one",
                keys.len(),
                keys.join(", ")
            ),
        ));
    }

    // Legacy registries: reconciled when a universe file holds exactly their bytes.
    let mut sources: Vec<(String, SourceState)> = Vec::new();
    for path in legacy_sources(files) {
        let imported = universe.sources.iter().find(|s| s.source == path);
        let state = match (imported, files.read_bytes(&path)) {
            (None, _) => SourceState::NotReconciled,
            (Some(s), Some(b)) if crate::digest::content_digest(&b) != s.digest => {
                SourceState::Stale
            }
            (Some(_), _) => SourceState::Reconciled,
        };
        if state != SourceState::Reconciled {
            findings.push(Finding::new(
                Severity::Warning,
                "UNIVERSE_SOURCE_NOT_RECONCILED",
                &path,
                &format!(
                    "{}: its records are not reconciled into the universe (unknown, never 0): `ecdev-gov migrate universe --from {path}`",
                    if state == SourceState::Stale {
                        "changed since its import"
                    } else {
                        "a legacy registry with no universe file"
                    }
                ),
            ));
        }
        sources.push((path, state));
    }
    for s in &universe.sources {
        if !sources.iter().any(|(p, _)| *p == s.source) {
            sources.push((s.source.clone(), SourceState::Reconciled));
        }
    }
    sources.sort();
    Report {
        relations,
        sources,
        ambiguous,
        findings,
    }
}

fn merge(into: &mut Acc, from: Acc) {
    into.kinds.extend(from.kinds);
    into.relevance.extend(from.relevance);
    into.selected |= from.selected;
    if let Some(l) = from.layer {
        into.raise(l);
    }
    into.origin_verified |= from.origin_verified;
    into.license_verified |= from.license_verified;
    into.donors.extend(from.donors);
    into.records.extend(from.records);
    into.aliases.extend(from.aliases);
    into.node_ids.extend(from.node_ids);
}

impl Report {
    pub fn unreconciled(&self) -> Vec<&str> {
        self.sources
            .iter()
            .filter(|(_, s)| *s != SourceState::Reconciled)
            .map(|(p, _)| p.as_str())
            .collect()
    }

    /// The universe metrics of one repository (see `metrics`).
    pub fn count(&self, c: &mut crate::metrics::Counts) {
        count_relations(&self.relations, c);
        c.ambiguous_donor_identities = self.ambiguous.len() as u64;
        c.universe_sources_unreconciled = self.unreconciled().len() as u64;
    }
}

/// Counts relations into the universe metrics (per repository; the linker re-derives the
/// deduplicated ones).
pub fn count_relations(rel: &[RepositoryUsesDonor], c: &mut crate::metrics::Counts) {
    let at = |l: Layer| rel.iter().filter(|r| r.lifecycle.at_least(l)).count() as u64;
    let kind = |k: UniverseKind| rel.iter().filter(|r| r.kind == k).count() as u64;
    let known = rel.iter().filter(|r| r.global_donor.is_some());
    c.known_universe_total = rel.len() as u64;
    c.known_oss_repositories = kind(UniverseKind::OssRepository);
    c.known_information_sources = kind(UniverseKind::InformationSource);
    c.known_official_authorities = kind(UniverseKind::OfficialAuthority);
    c.known_protocol_specs = kind(UniverseKind::ProtocolSpec);
    c.known_academic_references = kind(UniverseKind::AcademicReference);
    c.known_benchmarks = kind(UniverseKind::Benchmark);
    c.known_datasets = kind(UniverseKind::Dataset);
    c.known_simulators = kind(UniverseKind::Simulator);
    c.known_standards = kind(UniverseKind::Standard);
    c.known_reference_architectures = kind(UniverseKind::ReferenceArchitecture);
    c.known_papers = kind(UniverseKind::Paper);
    c.registered_universe_total = at(Layer::Universe(UniverseState::Registered));
    c.unique_global_donors = known.clone().count() as u64;
    c.repository_donor_references = known.clone().count() as u64;
    c.selected_for_census = at(Layer::Universe(UniverseState::SelectedForCensus));
    c.censused = at(Layer::Donor(DonorState::Censused));
    c.technology_mapped = at(Layer::Donor(DonorState::TechnologyMapped));
    c.native_targeted = at(Layer::Donor(DonorState::NativeTargeted));
    c.native_shadow = at(Layer::Donor(DonorState::NativeShadow));
    c.parity_proven = at(Layer::Donor(DonorState::ParityProven));
    c.cutover = at(Layer::Donor(DonorState::Cutover));
    c.extinct = at(Layer::Donor(DonorState::Extinct));
    c.unknown_identity = rel.iter().filter(|r| r.global_donor.is_none()).count() as u64;
    c.unverified_origin = known.clone().filter(|r| !r.origin_verified).count() as u64;
    c.unverified_license = known.clone().filter(|r| !r.license_verified).count() as u64;
    c.unresolved_relevance = rel.iter().filter(|r| !r.relevance.resolved()).count() as u64;
}
