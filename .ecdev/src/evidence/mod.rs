//! Proof evidence. A proof record binds a result to the exact bytes it judged: the proof file and
//! the replacement node's tree. When either changes, the record is stale and proves nothing.
//! Records are content-addressed files in `.ecdev/evidence/`, so concurrent branches never
//! conflict. Evidence is keyed by `(locator, subject)`: one test may prove several subjects. A
//! record whose key no longer names any declared proof (its proof was moved or dropped, or no
//! longer judges that subject) is retired by `compact` into a history file in the same
//! directory: kept with its provenance, never counted as evidence again.

use crate::compact::codec::{DecodeError, Decoder, Encoder};
use crate::declare::{Declaration, Proof};
use crate::digest::{content_digest, hex, Sha256};
use crate::repository::files::Files;
use crate::schema::ProofKind;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub const EVIDENCE_DIR: &str = ".ecdev/evidence";
pub const EVIDENCE_TAG: u8 = 2;
pub const RETIRED_TAG: u8 = 9;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Record {
    pub locator: String,
    pub kind: ProofKind,
    pub passed: bool,
    /// Digest of the proof file when the proof ran.
    pub proof_digest: String,
    /// Replacement node key and the digest of its tree when the proof ran.
    pub subject: String,
    pub subject_digest: String,
    pub command: String,
}

impl Record {
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new(EVIDENCE_TAG);
        e.str(&self.locator)
            .u8(self.kind.rank())
            .bool(self.passed)
            .str(&self.proof_digest)
            .str(&self.subject)
            .str(&self.subject_digest)
            .str(&self.command);
        e.finish()
    }
    pub fn decode(b: &[u8]) -> Result<Record, DecodeError> {
        let mut d = Decoder::open(b, EVIDENCE_TAG)?;
        let r = Record {
            locator: d.str()?,
            kind: d.word(ProofKind::ALL)?,
            passed: d.bool()?,
            proof_digest: d.str()?,
            subject: d.str()?,
            subject_digest: d.str()?,
            command: d.str()?,
        };
        d.end()?;
        Ok(r)
    }
}

/// Verdict on one declared proof.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verdict {
    Pass,
    Fail,
    /// Records exist but none matches the current bytes.
    Stale,
    /// The test exists but has never been recorded.
    Unrecorded,
    /// The locator names no test function.
    Absent,
}

impl Verdict {
    pub const ALL: &'static [Verdict] = &[
        Verdict::Pass,
        Verdict::Fail,
        Verdict::Stale,
        Verdict::Unrecorded,
        Verdict::Absent,
    ];
    pub fn rank(self) -> u8 {
        self as u8
    }
    pub fn wire(self) -> &'static str {
        match self {
            Verdict::Pass => "PASS",
            Verdict::Fail => "FAIL",
            Verdict::Stale => "STALE",
            Verdict::Unrecorded => "UNRECORDED",
            Verdict::Absent => "ABSENT",
        }
    }
}

/// A record retired from evidence into history: what it judged, its verdict and digests (all in
/// `record`), the content address it had, and when it was retired.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Retired {
    /// The compaction that retired it (history sequence, starting at 1).
    pub seq: u64,
    /// The commit checked out when it was retired (empty outside Git).
    pub commit: String,
    /// The record's former file name, `<digest>.ecg`.
    pub address: String,
    pub record: Record,
}

/// Encodes a history of retired records (sorted, so equal history encodes to equal bytes).
pub fn encode_retired(rows: &[Retired]) -> Vec<u8> {
    let mut rows = rows.to_vec();
    rows.sort();
    let mut e = Encoder::new(RETIRED_TAG);
    e.u64(rows.len() as u64);
    for r in &rows {
        e.u64(r.seq)
            .str(&r.commit)
            .str(&r.address)
            .bytes(&r.record.encode());
    }
    e.finish()
}

pub fn decode_retired(b: &[u8]) -> Result<Vec<Retired>, DecodeError> {
    let mut d = Decoder::open(b, RETIRED_TAG)?;
    let mut rows = Vec::new();
    for _ in 0..d.u64()? {
        rows.push(Retired {
            seq: d.u64()?,
            commit: d.str()?,
            address: d.str()?,
            record: Record::decode(d.bytes()?)?,
        });
    }
    d.end()?;
    Ok(rows)
}

/// The content address a state file holding `bytes` has.
pub fn address(bytes: &[u8]) -> String {
    format!(
        "{}.{}",
        &hex(&crate::digest::sha256(bytes))[..32],
        crate::compact::codec::EXTENSION
    )
}

/// Every evidence record found on disk, by locator, and the retired history beside them.
#[derive(Clone, Debug, Default)]
pub struct Store {
    pub records: BTreeMap<String, Vec<Record>>,
    /// Retired records: history, never evidence.
    pub retired: Vec<Retired>,
    /// The files holding retired history.
    pub retired_files: Vec<String>,
    pub unreadable: Vec<String>,
}

impl Store {
    pub fn load(root: &Path) -> Store {
        let mut s = Store::default();
        for (name, bytes) in crate::compact::read_addressed(root, EVIDENCE_DIR, &mut s.unreadable) {
            if Decoder::tag_of(&bytes) == Some(RETIRED_TAG) {
                match decode_retired(&bytes) {
                    Ok(rows) => {
                        s.retired.extend(rows);
                        s.retired_files.push(name);
                    }
                    Err(e) => s.unreadable.push(format!("{EVIDENCE_DIR}/{name}: {e}")),
                }
                continue;
            }
            match Record::decode(&bytes) {
                Ok(r) => s.records.entry(r.locator.clone()).or_default().push(r),
                Err(e) => s.unreadable.push(format!("{EVIDENCE_DIR}/{name}: {e}")),
            }
        }
        s.retired.sort();
        s
    }

    /// Persists a record under its content address; returns the file path.
    pub fn write(root: &Path, r: &Record) -> std::io::Result<String> {
        crate::compact::write_addressed(root, EVIDENCE_DIR, &r.encode())
    }
}

/// Governance's own state (evidence, history, knowledge, locks) never counts as a node's content:
/// recording evidence must not invalidate the evidence just recorded.
pub fn is_state(path: &str) -> bool {
    ["evidence", "history", "knowledge", "materialized"]
        .iter()
        .any(|d| path.starts_with(&format!(".ecdev/{d}/")))
        || path.starts_with("target/")
}

/// Digest of a node's tree: every tracked file below `path`, with its path.
pub fn tree_digest(files: &Files, path: &str) -> String {
    let mut h = Sha256::new();
    let mut any = false;
    for f in files.under(path).filter(|f| !is_state(f)) {
        if let Some(b) = files.read_bytes(f) {
            h.field(f.as_bytes());
            h.field(&b);
            any = true;
        }
    }
    if !any {
        return String::new();
    }
    format!("sha256:{}", hex(&h.finish()))
}

/// Whether `file` defines a test function named `name`.
pub fn test_exists(files: &Files, file: &str, name: &str) -> bool {
    if name.is_empty() || !files.paths.contains(file) {
        return false;
    }
    let Some(text) = files.read(file) else {
        return false;
    };
    let needle = format!("fn {name}(");
    text.match_indices(&needle).any(|(i, _)| {
        let mut start = i.saturating_sub(300);
        while !text.is_char_boundary(start) {
            start += 1;
        }
        let window = &text[start..i];
        window.contains("#[test]") || window.contains("#[tokio::test") || window.contains("_test]")
    })
}

/// The current bytes a proof about `subject` depends on.
pub fn current_digests(
    files: &Files,
    d: &Declaration,
    proof: &Proof,
    subject: &str,
) -> (String, String) {
    let (file, _) = proof.target();
    let proof_digest = files
        .read_bytes(file)
        .map(|b| content_digest(&b))
        .unwrap_or_default();
    // A technology's proofs judge its canonical sources; a node's proofs judge its tree.
    let subject_digest = if let Some(key) = subject.strip_prefix("technology/") {
        d.technology(key)
            .map(|t| crate::technology::source_digest(files, t))
            .unwrap_or_default()
    } else {
        d.node(subject)
            .map(|n| tree_digest(files, &n.path))
            .unwrap_or_default()
    };
    (proof_digest, subject_digest)
}

pub fn judge(
    store: &Store,
    files: &Files,
    d: &Declaration,
    proof: &Proof,
    subject: Option<&str>,
) -> Verdict {
    let (file, name) = proof.target();
    if !test_exists(files, file, name) {
        return Verdict::Absent;
    }
    let Some(subject) = subject else {
        return Verdict::Unrecorded;
    };
    let (pd, sd) = current_digests(files, d, proof, subject);
    let Some(records) = store.records.get(&proof.locator) else {
        return Verdict::Unrecorded;
    };
    let fresh: Vec<&Record> = records
        .iter()
        .filter(|r| {
            r.kind == proof.kind
                && r.subject == subject
                && r.proof_digest == pd
                && r.subject_digest == sd
                && !sd.is_empty()
        })
        .collect();
    if fresh.is_empty() {
        Verdict::Stale
    } else if fresh.iter().any(|r| !r.passed) {
        // Any failing run of the exact same bytes outweighs a passing one.
        Verdict::Fail
    } else {
        Verdict::Pass
    }
}

/// Runs proofs. The real runner shells out to cargo in the repository; tests substitute one.
pub trait Runner {
    fn run(&mut self, root: &Path, files: &Files, proof: &Proof) -> (bool, String);
}

pub struct CargoRunner;

impl Runner for CargoRunner {
    fn run(&mut self, root: &Path, files: &Files, proof: &Proof) -> (bool, String) {
        let (file, name) = proof.target();
        let members = crate::census::cargo::members(files);
        let member = members
            .iter()
            .filter(|m| m.dir.is_empty() || file.starts_with(&format!("{}/", m.dir)))
            .max_by_key(|m| m.dir.len());
        let mut args: Vec<String> = vec!["test".into()];
        if let Some(m) = member {
            args.push("-p".into());
            args.push(m.package.clone());
            let rel = file.strip_prefix(&format!("{}/", m.dir)).unwrap_or(file);
            if rel.starts_with("src/") {
                // A unit test: build only the library, not unrelated integration tests.
                args.push("--lib".into());
            } else if let Some(stem) = rel
                .strip_prefix("tests/")
                .and_then(|t| t.strip_suffix(".rs"))
            {
                if !stem.contains('/') {
                    args.push("--test".into());
                    args.push(stem.into());
                }
            }
        }
        args.push("--".into());
        args.push(name.into());
        let manifest = if member.is_some_and(|m| m.dir == ".ecdev") {
            root.join(".ecdev/Cargo.toml")
        } else {
            root.join("Cargo.toml")
        };
        let command = format!("cargo {} (manifest {})", args.join(" "), manifest.display());
        let out = std::process::Command::new("cargo")
            .arg(&args[0])
            .arg("--manifest-path")
            .arg(&manifest)
            .args(&args[1..])
            .output();
        let ok = match out {
            Ok(o) => {
                let text = String::from_utf8_lossy(&o.stdout);
                o.status.success()
                    && text
                        .lines()
                        .any(|l| l.starts_with("test ") && l.contains(name) && l.ends_with(" ok"))
            }
            Err(_) => false,
        };
        (ok, command)
    }
}

/// Every declared proof with the subject it judges, as `(proof, subject)`: donor capabilities'
/// proofs judge their replacement node (a capability without one has no subject and nothing to
/// record), technologies' proofs and claims' evidence judge `technology/<key>`.
pub fn declared_proofs(d: &Declaration) -> Vec<(Proof, String)> {
    let mut out = Vec::new();
    for c in d.donors.iter().flat_map(|dn| &dn.capabilities) {
        if let Some(subject) = &c.replacement {
            out.extend(c.proofs.iter().map(|p| (p.clone(), subject.clone())));
        }
    }
    for t in &d.technologies {
        let subject = format!("technology/{}", t.key);
        for p in t
            .proofs
            .iter()
            .chain(t.claims.iter().flat_map(|c| c.evidence.iter()))
        {
            out.push((p.clone(), subject.clone()));
        }
    }
    out
}

/// The evidence keys of every declared proof: `(locator, subject)`. One locator can prove
/// several subjects (one test shared by two replacement nodes); a record counts only for its own.
pub fn declared_keys(d: &Declaration) -> BTreeSet<(String, String)> {
    declared_proofs(d)
        .into_iter()
        .map(|(p, s)| (p.locator.clone(), s))
        .collect()
}

/// Removes every stale record of a declared proof once a fresh record of the same evidence key
/// `(locator, subject)` exists. Records of another subject under the same locator are never
/// judged against this subject's bytes, so no verdict changes. Returns the records removed.
pub fn prune_stale(root: &Path, files: &Files, d: &Declaration, store: &Store) -> usize {
    let mut done = BTreeSet::new();
    let mut pruned = 0;
    for (p, subject) in declared_proofs(d) {
        if !done.insert((p.locator.clone(), subject.clone())) {
            continue;
        }
        let Some(records) = store.records.get(&p.locator) else {
            continue;
        };
        let (pd, sd) = current_digests(files, d, &p, &subject);
        let own: Vec<&Record> = records.iter().filter(|r| r.subject == subject).collect();
        let fresh = |r: &Record| !sd.is_empty() && r.proof_digest == pd && r.subject_digest == sd;
        if !own.iter().any(|r| fresh(r)) {
            continue;
        }
        for r in own.into_iter().filter(|r| !fresh(r)) {
            let file = root.join(EVIDENCE_DIR).join(address(&r.encode()));
            if std::fs::remove_file(file).is_ok() {
                pruned += 1;
            }
        }
    }
    pruned
}

/// Retires every record whose evidence key `(locator, subject)` names no declared proof (its
/// proof was moved or dropped, or no longer judges that subject): it is folded, with its
/// provenance, into one history file (earlier history files are folded in too) and its own file
/// is removed. A record stays evidence while any declared proof of its locator judges its
/// subject, so no verdict changes. Returns (records retired, history file).
pub fn retire(
    root: &Path,
    d: &Declaration,
    commit: &str,
) -> std::io::Result<(usize, Option<String>)> {
    let store = Store::load(root);
    let declared = declared_keys(d);
    let seq = store
        .retired
        .iter()
        .map(|r| r.seq)
        .max()
        .map_or(1, |m| m + 1);
    let mut rows = store.retired.clone();
    let mut retiring = Vec::new();
    for r in store.records.values().flatten() {
        if declared.contains(&(r.locator.clone(), r.subject.clone())) {
            continue;
        }
        let address = address(&r.encode());
        retiring.push(address.clone());
        rows.push(Retired {
            seq,
            commit: commit.to_string(),
            address,
            record: r.clone(),
        });
    }
    let history = |f: &String| format!("{EVIDENCE_DIR}/{f}");
    if retiring.is_empty() && store.retired_files.len() <= 1 {
        return Ok((0, store.retired_files.first().map(history)));
    }
    // A record retired twice (a crash between writing history and removing it) keeps its first
    // retirement.
    rows.sort_by(|a, b| (&a.address, a.seq).cmp(&(&b.address, b.seq)));
    rows.dedup_by(|b, a| a.address == b.address);
    let written = crate::compact::write_addressed(root, EVIDENCE_DIR, &encode_retired(&rows))?;
    for f in retiring.iter().chain(&store.retired_files) {
        let p = history(f);
        if p != written {
            std::fs::remove_file(root.join(&p))?;
        }
    }
    Ok((retiring.len(), Some(written)))
}

/// Runs every declared proof (of donor capabilities and of technologies, optionally only those
/// of one donor or technology key) and records content-bound results.
/// Every declared proof's verdict, keyed by (owner, locator): owner is a donor key or
/// `technology/<key>`. Comparing two snapshots shows which evidence a change made stale.
pub fn verdicts(
    files: &Files,
    d: &Declaration,
    store: &Store,
) -> BTreeMap<(String, String), Verdict> {
    let mut out = BTreeMap::new();
    for dn in &d.donors {
        for c in &dn.capabilities {
            for p in &c.proofs {
                out.insert(
                    (dn.key.clone(), p.locator.clone()),
                    judge(store, files, d, p, c.replacement.as_deref()),
                );
            }
        }
    }
    for t in &d.technologies {
        let subject = format!("technology/{}", t.key);
        for p in t
            .proofs
            .iter()
            .chain(t.claims.iter().flat_map(|c| c.evidence.iter()))
        {
            out.insert(
                (subject.clone(), p.locator.clone()),
                judge(store, files, d, p, Some(&subject)),
            );
        }
    }
    out
}

pub fn prove(
    root: &Path,
    files: &Files,
    d: &Declaration,
    only: Option<&str>,
    runner: &mut dyn Runner,
) -> Vec<(String, bool, String)> {
    // Donor-capability proofs judge the replacement node; technology proofs judge the
    // technology's canonical sources (subject `technology/<key>`).
    let subjects: Vec<(String, &crate::declare::Technology)> = d
        .technologies
        .iter()
        .filter(|t| only.is_none_or(|k| k == t.key))
        .map(|t| (format!("technology/{}", t.key), t))
        .collect();
    let mut jobs: Vec<(&Proof, &str)> = Vec::new();
    for dn in d
        .donors
        .iter()
        .filter(|dn| only.is_none_or(|k| k == dn.key))
    {
        for c in &dn.capabilities {
            if let Some(subject) = &c.replacement {
                jobs.extend(c.proofs.iter().map(|p| (p, subject.as_str())));
            }
        }
    }
    for (subject, t) in &subjects {
        jobs.extend(t.proofs.iter().map(|p| (p, subject.as_str())));
        for c in &t.claims {
            jobs.extend(c.evidence.iter().map(|p| (p, subject.as_str())));
        }
    }
    let mut done = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for (p, subject) in jobs {
        if !done.insert((p.locator.clone(), p.kind, subject.to_string())) {
            continue;
        }
        let (file, name) = p.target();
        if !test_exists(files, file, name) {
            out.push((
                p.locator.clone(),
                false,
                "ABSENT: no such test function".into(),
            ));
            continue;
        }
        let (passed, command) = runner.run(root, files, p);
        let (pd, sd) = current_digests(files, d, p, subject);
        let rec = Record {
            locator: p.locator.clone(),
            kind: p.kind,
            passed,
            proof_digest: pd,
            subject: subject.to_string(),
            subject_digest: sd,
            command,
        };
        match Store::write(root, &rec) {
            Ok(path) => out.push((p.locator.clone(), passed, path)),
            Err(e) => out.push((p.locator.clone(), false, format!("cannot record: {e}"))),
        }
    }
    out
}
