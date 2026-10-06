//! Compacted knowledge: facts with stable identity and provenance, stored as content-addressed
//! batches in `.ecdev/knowledge/`. Twenty copies of one statement are one fact with twenty
//! provenances; a newer value supersedes an older one instead of accumulating beside it. Facts
//! extracted from documents are keyed so that different documents never supersede each other
//! (`audit::extract_at`): only re-extracting the same document's same item does.

use super::codec::{DecodeError, Decoder, Encoder};
use crate::digest::{hex, Sha256};
use crate::schema::FactKind;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub const KNOWLEDGE_DIR: &str = ".ecdev/knowledge";
pub const KNOWLEDGE_TAG: u8 = 4;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Fact {
    pub kind: FactKind,
    pub subject: String,
    pub key: String,
    pub value: String,
    pub provenance: BTreeSet<String>,
    /// Logical time of the value; the highest wins.
    pub seq: u64,
    /// Earlier values this fact superseded.
    pub superseded: Vec<String>,
}

impl Fact {
    pub fn new(
        kind: FactKind,
        subject: &str,
        key: &str,
        value: &str,
        provenance: &str,
        seq: u64,
    ) -> Fact {
        Fact {
            kind,
            subject: subject.to_string(),
            key: key.to_string(),
            value: value.to_string(),
            provenance: [provenance.to_string()].into_iter().collect(),
            seq,
            superseded: Vec::new(),
        }
    }

    /// Stable identity: kind, subject and key — never the value or where it was found.
    pub fn id(&self) -> String {
        let mut h = Sha256::new();
        h.field(b"ecdev.fact.v1");
        h.field(self.kind.wire().as_bytes());
        h.field(self.subject.as_bytes());
        h.field(self.key.as_bytes());
        format!("f1:{}", &hex(&h.finish())[..32])
    }
}

/// Normalises text for identity: case-folded, whitespace-collapsed, markup-stripped.
pub fn normalize(text: &str) -> String {
    let stripped: String = text
        .chars()
        .map(|c| {
            if matches!(c, '*' | '_' | '`' | '#' | '>' | '|') {
                ' '
            } else {
                c
            }
        })
        .collect();
    stripped
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(|c: char| c == '-' || c == '.' || c == ':' || c == ' ')
        .to_lowercase()
}

/// Folds facts: one per identity, newest value wins, older values are marked superseded,
/// provenances are united. Order-independent. A STATEMENT is keyed by its normalised text, so
/// its spellings in several documents are one statement, not values superseding each other.
pub fn fold(facts: impl IntoIterator<Item = Fact>) -> Vec<Fact> {
    let mut by_id: BTreeMap<String, Vec<Fact>> = BTreeMap::new();
    for f in facts {
        by_id.entry(f.id()).or_default().push(f);
    }
    by_id
        .into_values()
        .map(|mut group| {
            group.sort_by(|a, b| (a.seq, &a.value).cmp(&(b.seq, &b.value)));
            let mut winner = group.pop().unwrap();
            for f in group {
                winner.provenance.extend(f.provenance);
                if f.value != winner.value {
                    winner.superseded.push(f.value);
                }
                winner.superseded.extend(f.superseded);
            }
            let (value, statement) = (winner.value.clone(), winner.kind == FactKind::Statement);
            winner
                .superseded
                .retain(|v| *v != value && !(statement && normalize(v) == normalize(&value)));
            winner.superseded.sort();
            winner.superseded.dedup();
            winner
        })
        .collect()
}

pub fn encode(facts: &[Fact]) -> Vec<u8> {
    let mut facts = facts.to_vec();
    facts.sort_by_key(|f| f.id());
    let mut e = Encoder::new(KNOWLEDGE_TAG);
    e.u64(facts.len() as u64);
    for f in &facts {
        e.u8(f.kind.rank())
            .str(&f.subject)
            .str(&f.key)
            .str(&f.value)
            .strs(&f.provenance.iter().cloned().collect::<Vec<_>>())
            .u64(f.seq)
            .strs(&f.superseded);
    }
    e.finish()
}

pub fn decode(b: &[u8]) -> Result<Vec<Fact>, DecodeError> {
    let mut d = Decoder::open(b, KNOWLEDGE_TAG)?;
    let mut out = Vec::new();
    for _ in 0..d.u64()? {
        out.push(Fact {
            kind: d.word(FactKind::ALL)?,
            subject: d.str()?,
            key: d.str()?,
            value: d.str()?,
            provenance: d.strs()?.into_iter().collect(),
            seq: d.u64()?,
            superseded: d.strs()?,
        });
    }
    d.end()?;
    Ok(out)
}

#[derive(Clone, Debug, Default)]
pub struct Knowledge {
    pub facts: Vec<Fact>,
    pub files: Vec<String>,
    pub unreadable: Vec<String>,
    /// The text of every document or legacy file that knowledge stores verbatim, rebuilt from
    /// its facts ([`stored_texts`]); an error when the stored text does not match its digest.
    pub stored: BTreeMap<String, Result<String, String>>,
}

/// Rebuilds every text that facts store verbatim, by path, and proves each against its
/// recorded digest.
///
/// - A document's BLOCK facts of its latest extraction (the `seq` of its DOCUMENT `digest`),
///   keyed `00000`, `00001`, …, concatenated in key order.
/// - A legacy file's LEGACY_RECORD `content` of its latest import (the `seq` of its `imported`
///   digest).
///
/// A document extracted before outlines were stored (a digest and no blocks of that
/// extraction) or a legacy file imported line by line is absent: knowledge holds its facts,
/// not its text.
pub fn stored_texts(facts: &[Fact]) -> BTreeMap<String, Result<String, String>> {
    let mut digest: BTreeMap<&str, &Fact> = BTreeMap::new();
    let mut imported: BTreeMap<&str, &Fact> = BTreeMap::new();
    let mut blocks: BTreeMap<&str, Vec<&Fact>> = BTreeMap::new();
    let mut content: BTreeMap<&str, &Fact> = BTreeMap::new();
    for f in facts {
        match (f.kind, f.key.as_str()) {
            (FactKind::Document, "digest") => {
                digest.insert(&f.subject, f);
            }
            (FactKind::LegacyRecord, "imported") => {
                imported.insert(&f.subject, f);
            }
            (FactKind::LegacyRecord, "content") => {
                content.insert(&f.subject, f);
            }
            (FactKind::Block, _) => blocks.entry(&f.subject).or_default().push(f),
            _ => {}
        }
    }
    let check = |path: &str, text: String, d: &Fact| {
        let got = crate::digest::content_digest(text.as_bytes());
        if got == d.value {
            Ok(text)
        } else {
            Err(format!(
                "{path}: stored text rebuilds to {got}, not its recorded {}",
                d.value
            ))
        }
    };
    let mut out = BTreeMap::new();
    for (path, d) in &digest {
        let mut bs: Vec<(usize, &Fact)> = blocks
            .get(path)
            .into_iter()
            .flatten()
            .filter(|b| b.seq == d.seq)
            .map(|b| (b.key.parse::<usize>().unwrap_or(usize::MAX), *b))
            .collect();
        if bs.is_empty() {
            continue;
        }
        bs.sort_by_key(|(i, _)| *i);
        let r = match bs.iter().enumerate().find(|(n, (i, _))| n != i) {
            Some((n, _)) => Err(format!("{path}: block {n} of its outline is missing")),
            None => check(
                path,
                bs.iter()
                    .map(|(_, b)| super::outline::decode_block(&b.value).1)
                    .collect(),
                d,
            ),
        };
        out.insert(path.to_string(), r);
    }
    for (path, c) in &content {
        match imported.get(path) {
            Some(d) if d.seq == c.seq => {
                out.insert(path.to_string(), check(path, c.value.clone(), d));
            }
            _ => {}
        }
    }
    out
}

impl Knowledge {
    pub fn load(root: &Path) -> Knowledge {
        let mut k = Knowledge::default();
        let mut all = Vec::new();
        for (name, bytes) in super::read_addressed(root, KNOWLEDGE_DIR, &mut k.unreadable) {
            match decode(&bytes) {
                Ok(f) => {
                    all.extend(f);
                    k.files.push(name);
                }
                Err(e) => k.unreadable.push(format!("{KNOWLEDGE_DIR}/{name}: {e}")),
            }
        }
        k.facts = fold(all);
        k.stored = stored_texts(&k.facts);
        k
    }

    /// The verbatim text knowledge stores for `path`, proven against its digest.
    pub fn stored_text(&self, path: &str) -> Option<&Result<String, String>> {
        self.stored.get(path)
    }

    /// Whether knowledge stores exactly `bytes` as the text of `path`.
    pub fn stores(&self, path: &str, bytes: &[u8]) -> bool {
        matches!(self.stored.get(path), Some(Ok(t)) if t.as_bytes() == bytes)
    }

    pub fn next_seq(&self) -> u64 {
        self.facts.iter().map(|f| f.seq).max().map_or(1, |m| m + 1)
    }

    pub fn add(root: &Path, facts: &[Fact]) -> std::io::Result<String> {
        super::write_addressed(root, KNOWLEDGE_DIR, &encode(&fold(facts.iter().cloned())))
    }

    /// Folds every batch into one; returns (files removed, file written).
    pub fn compact(root: &Path) -> std::io::Result<(usize, Option<String>)> {
        let k = Knowledge::load(root);
        if k.files.len() <= 1 {
            return Ok((0, k.files.first().map(|f| format!("{KNOWLEDGE_DIR}/{f}"))));
        }
        let written = super::write_addressed(root, KNOWLEDGE_DIR, &encode(&k.facts))?;
        let mut removed = 0;
        for f in &k.files {
            let p = format!("{KNOWLEDGE_DIR}/{f}");
            if p != written {
                std::fs::remove_file(root.join(&p))?;
                removed += 1;
            }
        }
        Ok((removed, Some(written)))
    }

    /// Current facts, optionally of one kind and under a subject prefix, in a deterministic
    /// order: vocabulary rank, subject, key.
    pub fn current(&self, kind: Option<FactKind>, subject_prefix: Option<&str>) -> Vec<&Fact> {
        let mut out: Vec<&Fact> = self
            .facts
            .iter()
            .filter(|f| kind.is_none_or(|k| f.kind == k))
            .filter(|f| subject_prefix.is_none_or(|p| f.subject.starts_with(p)))
            .collect();
        out.sort_by(|a, b| (a.kind, &a.subject, &a.key).cmp(&(b.kind, &b.subject, &b.key)));
        out
    }

    /// The current fact of one identity.
    pub fn get(&self, kind: FactKind, subject: &str, key: &str) -> Option<&Fact> {
        self.facts
            .iter()
            .find(|f| f.kind == kind && f.subject == subject && f.key == key)
    }

    /// Current facts per kind, in vocabulary order, kinds without facts omitted.
    pub fn counts(&self) -> Vec<(FactKind, usize)> {
        FactKind::ALL
            .iter()
            .map(|k| (*k, self.facts.iter().filter(|f| f.kind == *k).count()))
            .filter(|(_, n)| *n > 0)
            .collect()
    }

    /// The recorded digest of a document whose knowledge was extracted.
    pub fn document_digest(&self, path: &str) -> Option<&str> {
        self.facts
            .iter()
            .find(|f| f.kind == FactKind::Document && f.subject == path && f.key == "digest")
            .map(|f| f.value.as_str())
    }
}

/// How [`assert_fact`] treats an existing fact of the same identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Assert {
    /// Record the value: new, the same (provenance is united) or a newer one (supersedes).
    Add,
    /// Record a newer value of an existing fact; refused when there is none or it is equal.
    Supersede,
}

/// Validates one asserted fact. Provenance is mandatory: every fact says where it came from.
pub fn validate(
    kind: FactKind,
    subject: &str,
    key: &str,
    value: &str,
    provenance: &[String],
) -> Result<(), String> {
    if !crate::schema::is_assertable(kind) {
        return Err(format!(
            "{kind} facts are written only by extraction and import, never asserted"
        ));
    }
    if key.trim().is_empty() {
        return Err("a fact needs a non-empty key".into());
    }
    if value.trim().is_empty() {
        return Err("a fact needs a non-empty value".into());
    }
    if provenance.is_empty() || provenance.iter().any(|p| p.trim().is_empty()) {
        return Err(
            "provenance is mandatory: --provenance <where this fact comes from> (repeatable)"
                .into(),
        );
    }
    match kind {
        FactKind::Milestone => {
            let id = subject
                .strip_prefix("milestone/")
                .or_else(|| subject.strip_prefix("gap/"));
            if id.is_none_or(|id| id.trim().is_empty()) {
                return Err(format!(
                    "a MILESTONE subject is milestone/<id> or gap/<id>, not `{subject}`"
                ));
            }
        }
        FactKind::Decision | FactKind::Definition if subject.trim().is_empty() => {
            return Err(format!("a {kind} fact needs a subject"));
        }
        _ => {}
    }
    Ok(())
}

/// Writes one asserted fact as its own knowledge batch at the next logical time, so a newer
/// value of the same (kind, subject, key) supersedes the older one, which is kept in
/// `superseded`. Returns the batch written and the fact as it now folds.
pub fn assert_fact(
    root: &Path,
    mode: Assert,
    kind: FactKind,
    subject: &str,
    key: &str,
    value: &str,
    provenance: &[String],
) -> Result<(String, Fact), String> {
    validate(kind, subject, key, value, provenance)?;
    let k = Knowledge::load(root);
    if !k.unreadable.is_empty() {
        return Err(format!(
            "unreadable knowledge; refusing to write: {}",
            k.unreadable.join("; ")
        ));
    }
    if mode == Assert::Supersede {
        match k.get(kind, subject, key) {
            None => {
                return Err(format!(
                    "nothing to supersede: no current {kind} {subject} {key} (use `fact add`)"
                ))
            }
            Some(f) if f.value == value => {
                return Err(format!("{kind} {subject} {key} already has this value"))
            }
            Some(_) => {}
        }
    }
    let mut f = Fact::new(kind, subject, key, value, &provenance[0], k.next_seq());
    f.provenance.extend(provenance.iter().cloned());
    let batch = Knowledge::add(root, &[f.clone()]).map_err(|e| e.to_string())?;
    let now = Knowledge::load(root);
    let folded = now.get(kind, subject, key).cloned().unwrap_or(f);
    Ok((batch, folded))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fold_dedupes_and_supersedes_order_independently() {
        let a = Fact::new(
            FactKind::Statement,
            "",
            &normalize("The log is **append-only**."),
            "The log is append-only.",
            "doc:a.md#L1",
            1,
        );
        let b = Fact::new(
            FactKind::Statement,
            "",
            &normalize("the log is append-only"),
            "the log is append-only",
            "doc:b.md#L9",
            1,
        );
        let d1 = Fact::new(
            FactKind::Definition,
            "",
            "census",
            "a scan",
            "doc:a.md#L3",
            1,
        );
        let d2 = Fact::new(
            FactKind::Definition,
            "",
            "census",
            "a deterministic observation of committed content",
            "doc:c.md#L2",
            2,
        );
        let x = fold(vec![a.clone(), b.clone(), d1.clone(), d2.clone()]);
        let y = fold(vec![d2, b, d1, a]);
        assert_eq!(x, y);
        assert_eq!(x.len(), 2);
        let def = x.iter().find(|f| f.kind == FactKind::Definition).unwrap();
        assert_eq!(
            def.value,
            "a deterministic observation of committed content"
        );
        assert_eq!(def.superseded, vec!["a scan".to_string()]);
        let st = x.iter().find(|f| f.kind == FactKind::Statement).unwrap();
        assert_eq!(st.provenance.len(), 2);
        assert_eq!(decode(&encode(&x)).unwrap(), x);
    }
    #[test]
    fn asserted_facts_require_provenance_and_a_legal_subject() {
        let p = vec!["agent:session".to_string()];
        assert!(validate(FactKind::Milestone, "milestone/v1", "status", "DONE", &p).is_ok());
        assert!(validate(FactKind::Milestone, "gap/x", "status", "OPEN", &p).is_ok());
        let err = validate(FactKind::Milestone, "milestone/v1", "status", "DONE", &[]).unwrap_err();
        assert!(err.contains("provenance is mandatory"), "{err}");
        assert!(validate(FactKind::Decision, "d", "status", "x", &["  ".to_string()]).is_err());
        assert!(validate(FactKind::Milestone, "v1", "status", "DONE", &p).is_err());
        assert!(validate(FactKind::Milestone, "milestone/", "status", "DONE", &p).is_err());
        assert!(validate(FactKind::Decision, "", "status", "x", &p).is_err());
        assert!(validate(FactKind::Decision, "d", "", "x", &p).is_err());
        assert!(validate(FactKind::Decision, "d", "status", " ", &p).is_err());
        for k in [
            FactKind::Document,
            FactKind::LegacyRecord,
            FactKind::LegacyClaim,
        ] {
            assert!(validate(k, "x", "y", "z", &p).is_err(), "{k}");
        }
    }
}
