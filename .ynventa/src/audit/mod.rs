//! The document budget. Markdown is never a source of truth; a repository keeps only its human
//! entry points. Everything else is detected, classified, extracted into compacted knowledge
//! facts, and only then deleted — never before its knowledge is preserved.

pub mod decision;

use crate::compact::facts::{normalize, Fact, Knowledge};
use crate::compact::outline;
use crate::declare::Declaration;
use crate::digest::content_digest;
use crate::repository::files::Files;
use crate::schema::{is_physical, FactKind};
use std::collections::{BTreeMap, BTreeSet};

/// Documents always within budget.
pub const ALLOWED_DOCUMENTS: &[&str] = &[
    "README.md",
    "AGENTS.md",
    "CLAUDE.md",
    "THIRD-PARTY-NOTICES.md",
    ".ynventa/README.md",
];

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Issue {
    OverBudget,
    GeneratedCommitted,
    SessionLog,
    Duplicate(String),
    NearDuplicate(String, u64),
    Superseded,
    DonorNote,
    CensusDescription,
    StaleArchitecture(u64, u64),
    Represented(u64),
}

impl Issue {
    pub fn wire(&self) -> String {
        match self {
            Issue::OverBudget => "OVER_BUDGET".into(),
            Issue::GeneratedCommitted => "GENERATED_REPORT_COMMITTED".into(),
            Issue::SessionLog => "SESSION_OR_PROGRESS_LOG".into(),
            Issue::Duplicate(o) => format!("DUPLICATE_OF {o}"),
            Issue::NearDuplicate(o, p) => format!("NEAR_DUPLICATE_OF {o} ({p}% shared lines)"),
            Issue::Superseded => "SUPERSEDED_DOCUMENT".into(),
            Issue::DonorNote => "DONOR_NOTE".into(),
            Issue::CensusDescription => "CENSUS_DESCRIPTION".into(),
            Issue::StaleArchitecture(dead, all) => {
                format!("STALE_ARCHITECTURE ({dead}/{all} referenced paths missing)")
            }
            Issue::Represented(p) => {
                format!("REPRESENTED_CANONICALLY ({p}% of statements are knowledge facts)")
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct DocReport {
    pub path: String,
    pub allowed: bool,
    pub issues: Vec<Issue>,
    pub digest: String,
    /// Knowledge was extracted from exactly these bytes and rebuilds them byte for byte from its
    /// facts; the document may be deleted.
    pub extracted: bool,
}

#[derive(Clone, Debug, Default)]
pub struct DocAudit {
    pub docs: Vec<DocReport>,
}

impl DocAudit {
    pub fn over_budget(&self) -> impl Iterator<Item = &DocReport> {
        self.docs.iter().filter(|d| !d.allowed)
    }
}

pub fn is_document(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".md") || lower.ends_with(".markdown") || lower.ends_with(".mdx")
}

/// Directories whose documents are decision records (compared case-insensitively).
pub const DECISION_RECORD_DIRS: &[&str] = &["decisions", "decision-records", "adr", "adrs"];

/// Index documents of a decision directory: living navigation, not decision records.
pub const DECISION_INDEX_NAMES: &[&str] = &["readme.md", "index.md", "_index.md"];

/// The one definition of a decision record (an ADR): a document with a directory segment in
/// [`DECISION_RECORD_DIRS`] (`.atlas/decisions/0015-x.md`, `docs/adr/0003-y.md`), other than
/// that directory's index. A decision record is history: it keeps the paths of its time, so
/// paths it names that were later moved are neither stale architecture nor stale references.
pub fn is_decision_record(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let Some((dir, name)) = lower.rsplit_once('/') else {
        return false;
    };
    is_document(&lower)
        && !DECISION_INDEX_NAMES.contains(&name)
        && dir.split('/').any(|s| DECISION_RECORD_DIRS.contains(&s))
}

fn allowed(path: &str, node_paths: &BTreeSet<String>) -> bool {
    if ALLOWED_DOCUMENTS.contains(&path) || path.starts_with(".github/") {
        return true;
    }
    match path.rsplit_once('/') {
        Some((dir, "README.md")) => node_paths.contains(dir),
        _ => false,
    }
}

/// Non-trivial normalised lines of a document.
fn lines(text: &str) -> BTreeSet<String> {
    text.lines()
        .map(normalize)
        .filter(|l| l.len() >= 12 && l.chars().any(|c| c.is_alphabetic()))
        .collect()
}

pub fn audit(files: &Files, d: &Declaration, knowledge: &Knowledge) -> DocAudit {
    let excluded = crate::census::excluded_roots(d);
    let donor_sources: Vec<String> = excluded
        .iter()
        .filter(|e| *e != ".atlas")
        .cloned()
        .collect();
    let node_paths: BTreeSet<String> = d
        .repository
        .nodes
        .iter()
        .filter(|n| is_physical(n.kind) && !n.path.is_empty())
        .map(|n| n.path.clone())
        .collect();
    let donor_words: BTreeSet<String> = d
        .donors
        .iter()
        .flat_map(|x| [x.key.to_ascii_lowercase(), x.name.to_ascii_lowercase()])
        .filter(|w| w.len() >= 3)
        .collect();
    let statements: BTreeSet<&str> = knowledge
        .facts
        .iter()
        .filter(|f| f.kind == FactKind::Statement)
        .map(|f| f.key.as_str())
        .collect();

    let docs: Vec<&String> = files
        .paths
        .iter()
        .filter(|p| is_document(p) && !crate::census::is_excluded(p, &donor_sources))
        .filter(|p| !p.split('/').any(|s| s == "node_modules"))
        .collect();
    let texts: BTreeMap<&String, String> = docs
        .iter()
        .map(|p| (*p, files.read(p).unwrap_or_default()))
        .collect();
    let line_sets: BTreeMap<&String, BTreeSet<String>> =
        texts.iter().map(|(p, t)| (*p, lines(t))).collect();

    // Exact duplicates by normalised content.
    let mut by_content: BTreeMap<String, Vec<&String>> = BTreeMap::new();
    for (p, t) in &texts {
        by_content
            .entry(content_digest(normalize(t).as_bytes()))
            .or_default()
            .push(p);
    }
    // Near duplicates via an inverted index of lines (common boilerplate lines skipped).
    let mut postings: BTreeMap<&str, Vec<&String>> = BTreeMap::new();
    for (p, ls) in &line_sets {
        for l in ls {
            postings.entry(l.as_str()).or_default().push(p);
        }
    }
    let mut shared: BTreeMap<(&String, &String), u64> = BTreeMap::new();
    for ps in postings.values() {
        if ps.len() < 2 || ps.len() > 50 {
            continue;
        }
        for i in 0..ps.len() {
            for j in i + 1..ps.len() {
                *shared.entry((ps[i], ps[j])).or_default() += 1;
            }
        }
    }
    let mut near: BTreeMap<&String, (String, u64)> = BTreeMap::new();
    for ((a, b), n) in shared {
        let (la, lb) = (line_sets[a].len() as u64, line_sets[b].len() as u64);
        // The smaller document is the redundant one.
        let (small, big, ls) = if la <= lb { (a, b, la) } else { (b, a, lb) };
        if ls >= 3 {
            let pct = n * 100 / ls;
            if pct >= 80 && near.get(small).is_none_or(|(_, p)| pct > *p) {
                near.insert(small, (big.clone(), pct));
            }
        }
    }

    let mut out = DocAudit::default();
    for p in docs {
        let text = &texts[p];
        let digest = content_digest(text.as_bytes());
        let lower = p.to_ascii_lowercase();
        let segs: Vec<&str> = lower.split('/').collect();
        let name = segs.last().copied().unwrap_or("");
        let ok = allowed(p, &node_paths);
        let mut issues = Vec::new();
        if !ok {
            issues.push(Issue::OverBudget);
        }
        let head: String = text
            .lines()
            .take(8)
            .collect::<Vec<_>>()
            .join("\n")
            .to_ascii_lowercase();
        if head.contains("@generated")
            || head.contains("do not edit")
            || head.contains("generated by")
            || segs.contains(&"generated")
        {
            issues.push(Issue::GeneratedCommitted);
        }
        let dated = name.len() > 10
            && name[..10].bytes().enumerate().all(|(i, c)| {
                if i == 4 || i == 7 {
                    c == b'-'
                } else {
                    c.is_ascii_digit()
                }
            });
        if dated
            || [
                "session",
                "progress",
                "handoff",
                "status",
                "iteration",
                "journal",
                "worklog",
                "notes-",
                "standup",
            ]
            .iter()
            .any(|w| name.contains(w))
        {
            issues.push(Issue::SessionLog);
        }
        if let Some(group) = by_content.get(&content_digest(normalize(text).as_bytes())) {
            if group.len() > 1 && group[0] != p {
                issues.push(Issue::Duplicate(group[0].clone()));
            }
        }
        if let Some((other, pct)) = near.get(p) {
            issues.push(Issue::NearDuplicate(other.clone(), *pct));
        }
        let head20: String = text
            .lines()
            .take(20)
            .collect::<Vec<_>>()
            .join("\n")
            .to_ascii_lowercase();
        if head20.contains("superseded")
            || head20.contains("status: deprecated")
            || head20.contains("status: retired")
        {
            issues.push(Issue::Superseded);
        }
        if segs
            .iter()
            .any(|s| matches!(*s, "donors" | "provenance" | "licenses" | "license"))
            || donor_words
                .iter()
                .any(|w| name.trim_end_matches(".md").contains(w.as_str()))
        {
            issues.push(Issue::DonorNote);
        }
        if segs.contains(&"census") {
            issues.push(Issue::CensusDescription);
        }
        // Living architecture must name paths that exist; a decision record keeps its history.
        if (segs.iter().any(|s| {
            matches!(*s, "architecture" | "blueprints") || DECISION_RECORD_DIRS.contains(s)
        }) || name == "architecture.md")
            && !is_decision_record(p)
        {
            let refs = referenced_paths(text);
            let dead = refs.iter().filter(|r| !files.exists(r)).count() as u64;
            if !refs.is_empty() && dead * 2 > refs.len() as u64 {
                issues.push(Issue::StaleArchitecture(dead, refs.len() as u64));
            }
        }
        let ls = statement_units(text);
        if !ls.is_empty() && !statements.is_empty() {
            let represented = ls
                .iter()
                .filter(|l| statements.contains(l.as_str()))
                .count() as u64;
            let pct = represented * 100 / ls.len() as u64;
            if pct >= 90 {
                issues.push(Issue::Represented(pct));
            }
        }
        // Extracted: knowledge rebuilds exactly these bytes from its facts.
        let extracted = knowledge.stores(p, text.as_bytes());
        out.docs.push(DocReport {
            path: p.clone(),
            allowed: ok,
            issues,
            digest,
            extracted,
        });
    }
    out
}

/// Backticked repository-relative paths (`dir/file.ext`) mentioned by a document.
fn referenced_paths(text: &str) -> Vec<String> {
    let mut out = BTreeSet::new();
    for chunk in text.split('`').skip(1).step_by(2) {
        let c = chunk.trim().trim_end_matches('/');
        if c.contains('/')
            && !c.contains(' ')
            && !c.contains("://")
            && !c.starts_with('.')
            && !c.contains("::")
            && c.len() < 120
        {
            out.insert(c.to_string());
        }
    }
    out.into_iter().collect()
}

/// A logical block of a document: a heading, a fenced code block, a table row, or a paragraph
/// or list item together with its wrapped continuation lines. Lines are 1-based.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub first: usize,
    pub last: usize,
    pub kind: BlockKind,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    Heading(usize),
    Code,
    Text,
}

/// Whether a line opens a list item: `- `, `* `, `+ ` or `1. ` / `1) `, after any quote marks.
fn opens_item(line: &str) -> bool {
    let l = line.trim_start_matches(['>', ' ']);
    if l.starts_with("- ") || l.starts_with("* ") || l.starts_with("+ ") || l == "-" {
        return true;
    }
    let digits = l.chars().take_while(char::is_ascii_digit).count();
    digits > 0 && (l[digits..].starts_with(". ") || l[digits..].starts_with(") "))
}

/// Whether a line is a `Field: value` line (`Status: Accepted`, `**Date**: …`), which starts
/// its own block even without a blank line before it.
fn opens_field(line: &str) -> bool {
    let l = line.trim_start_matches(['>', ' ', '*']);
    let Some((term, _)) = l.split_once(": ").or_else(|| l.split_once(":** ")) else {
        return false;
    };
    let term = term.trim_end_matches('*');
    let words = term.split_whitespace().count();
    (1..=3).contains(&words)
        && !term.contains('`')
        && term.chars().next().is_some_and(char::is_uppercase)
}

/// A rule or table separator: only `-`, `|`, `:`, `=` and spaces.
fn is_rule(line: &str) -> bool {
    !line.is_empty()
        && line
            .chars()
            .all(|c| matches!(c, '-' | '|' | ':' | ' ' | '=' | '*' | '_'))
        && line.chars().any(|c| c != ' ')
        && line.len() >= 3
}

/// Splits a document into logical blocks. A wrapped paragraph or list item is one block: its
/// continuation lines are joined with single spaces, so it is classified as one statement.
pub fn blocks(text: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut open: Option<Block> = None;
    let mut in_code = false;
    let mut code = String::new();
    let mut code_start = 0;
    for (i, raw) in text.lines().enumerate() {
        let n = i + 1;
        let line = raw.trim();
        if line.starts_with("```") {
            out.extend(open.take());
            if in_code {
                out.push(Block {
                    first: code_start,
                    last: n,
                    kind: BlockKind::Code,
                    text: std::mem::take(&mut code),
                });
            } else {
                code_start = n;
            }
            in_code = !in_code;
            continue;
        }
        if in_code {
            code.push_str(raw);
            code.push('\n');
            continue;
        }
        if line.is_empty() || is_rule(line) {
            out.extend(open.take());
            continue;
        }
        if let Some(h) = line.strip_prefix('#') {
            out.extend(open.take());
            let level = 1 + h.chars().take_while(|c| *c == '#').count();
            out.push(Block {
                first: n,
                last: n,
                kind: BlockKind::Heading(level),
                text: h.trim_start_matches('#').trim().to_string(),
            });
            continue;
        }
        let row = line.starts_with('|');
        match open.as_mut() {
            Some(b)
                if !row && !b.text.starts_with('|') && !opens_item(line) && !opens_field(line) =>
            {
                b.text.push(' ');
                b.text.push_str(line.trim_start_matches(['>', ' ']).trim());
                b.last = n;
            }
            _ => {
                out.extend(open.take());
                open = Some(Block {
                    first: n,
                    last: n,
                    kind: BlockKind::Text,
                    text: line.to_string(),
                });
            }
        }
    }
    out.extend(open);
    out
}

/// The text of a block without its list or quote marker.
fn block_body(b: &Block) -> &str {
    b.text.trim_start_matches(['-', '*', '+', '>', ' ']).trim()
}

/// Normalised statement keys of a document, as [`extract`] would key them.
fn statement_units(text: &str) -> BTreeSet<String> {
    blocks(text)
        .iter()
        .filter_map(|b| match b.kind {
            BlockKind::Heading(_) => None,
            BlockKind::Code => Some(normalize(&b.text)),
            BlockKind::Text => Some(normalize(block_body(b))),
        })
        .filter(|l| l.len() >= 12 && l.chars().any(|c| c.is_alphabetic()))
        .collect()
}

/// Where a block was found: `doc:<path>#L<a>[-L<b>]`, and with a commit also
/// `git:<sha>:<path>#L<a>[-L<b>]`.
fn provenance(path: &str, first: usize, last: usize, commit: Option<&str>) -> Vec<String> {
    let lines = if first == last {
        format!("#L{first}")
    } else {
        format!("#L{first}-L{last}")
    };
    let mut out = vec![format!("doc:{path}{lines}")];
    if let Some(c) = commit.filter(|c| !c.is_empty()) {
        out.push(format!("git:{c}:{path}{lines}"));
    }
    out
}

fn fact(kind: FactKind, subject: &str, key: &str, value: &str, prov: &[String], seq: u64) -> Fact {
    let mut f = Fact::new(kind, subject, key, value, &prov[0], seq);
    f.provenance.extend(prov.iter().cloned());
    f
}

/// One scalar frontmatter entry: (line, lowercased key, value).
pub type FrontmatterEntry = (usize, String, String);

/// A document's YAML-like frontmatter: `key: value` lines between a first-line `---` fence and
/// the next `---` (or `...`) fence. Returns the closing fence's line (1-based) and the scalar
/// entries as (line, key, value); keys are lowercased, quotes around a value are dropped, and
/// entries without a scalar value (lists, nested maps) are skipped.
pub fn frontmatter(text: &str) -> Option<(usize, Vec<FrontmatterEntry>)> {
    let mut lines = text.lines().enumerate();
    if lines.next()?.1.trim_end() != "---" {
        return None;
    }
    let mut entries = Vec::new();
    for (i, line) in lines {
        let t = line.trim_end();
        if t == "---" || t == "..." {
            return Some((i + 1, entries));
        }
        if line.starts_with([' ', '\t', '-', '#']) {
            continue;
        }
        let Some((key, value)) = t.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
            .unwrap_or(value);
        if key.is_empty()
            || !key
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'))
            || value.is_empty()
            || value == "|"
            || value == ">"
        {
            continue;
        }
        entries.push((i + 1, key.to_ascii_lowercase(), value.to_string()));
    }
    None
}

/// Decision fields read from a decision record's frontmatter and field lines.
const DECISION_FIELDS: &[&str] = &["status", "title", "date"];

/// A `Status: …` / `**Status:** …` / `- status: …` field line, or a `| Status | … |` table row,
/// naming a decision field or relation ([`decision::relation`]) case-insensitively:
/// (field, value).
fn decision_field(body: &str) -> Option<(&'static str, &str)> {
    let (term, value) = match body.strip_prefix('|') {
        Some(row) => {
            let mut cells = row.split('|');
            (cells.next()?, cells.next()?)
        }
        None => body.split_once(':')?,
    };
    let term = term.trim().trim_matches(['*', '_']).trim();
    let value = value.trim_start_matches(['*', '_']).trim();
    let field = DECISION_FIELDS
        .iter()
        .find(|f| f.eq_ignore_ascii_case(term))
        .copied()
        .or_else(|| decision::relation(term))?;
    (!value.is_empty()).then_some((field, value))
}

/// The subject of facts extracted from a document's text: the document itself, then the
/// headings the text sits under. Two documents never share one, so their facts never supersede
/// each other; re-extracting a document supersedes its own earlier values.
pub fn document_subject(path: &str, headings: &[String]) -> String {
    if headings.is_empty() {
        path.to_string()
    } else {
        format!("{path} / {}", headings.join(" / "))
    }
}

/// The key of a decision record's `## <Heading>` section: the heading, lowercased.
fn section_key(heading: &str) -> String {
    heading
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches(':')
        .to_lowercase()
}

/// Extracts a document's knowledge as facts: statements, definitions, decisions, and the
/// document's own digest (which later licenses its deletion).
pub fn extract(path: &str, text: &str, seq: u64) -> Vec<Fact> {
    extract_at(path, text, seq, None)
}

/// Collects the facts of one document, keeping the first value of each decision field.
struct Extraction<'a> {
    path: &'a str,
    commit: Option<&'a str>,
    seq: u64,
    stem: String,
    facts: Vec<Fact>,
    decided: BTreeSet<String>,
    /// Relations of a decision record, in order of first mention: (key, ids, provenance).
    relations: Vec<(&'static str, Vec<String>, Vec<String>)>,
}

impl Extraction<'_> {
    fn provenance(&self, first: usize, last: usize) -> Vec<String> {
        provenance(self.path, first, last, self.commit)
    }
    fn push(&mut self, kind: FactKind, subject: &str, key: &str, value: &str, prov: &[String]) {
        self.facts
            .push(fact(kind, subject, key, value, prov, self.seq));
    }
    /// A DECISION fact of this record; the first value of a key wins (frontmatter, then the
    /// record's own lines, in document order). A status is normalised
    /// ([`decision::status`]): `status` holds the canonical word and `status-text` the text when
    /// it says more. The relations a status or relation field states become `supersedes`,
    /// `superseded-by`, … facts whose values are the referenced decision ids; the prose of a
    /// relation field or section that says more than its ids is kept as `<relation>-text`.
    fn decide(&mut self, key: &str, value: &str, first: usize, last: usize) {
        let value = value.trim();
        if value.is_empty() {
            return;
        }
        if key == "status" {
            self.relate(value, first, last);
            if let Some((status, text)) = decision::status(value) {
                if self.decided.insert("status".into()) {
                    self.record("status", &status, first, last);
                    if let Some(t) = text {
                        self.record("status-text", &t, first, last);
                    }
                }
            }
            return;
        }
        if let Some(rel) = decision::relation(key) {
            let mut found = decision::relations(value);
            if !found.iter().any(|(r, _)| *r == rel) {
                // `Supersedes: 0004` / a `## Supersedes` section: the ids follow the term.
                let ids = decision::relations(&format!("{} {value}", rel.replace('-', " ")))
                    .into_iter()
                    .filter(|(r, _)| *r == rel);
                found.extend(ids);
            }
            let ids: Vec<String> = found
                .iter()
                .flat_map(|(_, ids)| ids.iter().cloned())
                .collect();
            for (r, ids) in found {
                self.link(r, ids, first, last);
            }
            // Words besides the ids and their connectives: the field says more than its ids.
            let prose = decision::plain(value).to_ascii_lowercase();
            let says_more = prose
                .split(|c: char| !c.is_alphanumeric() && c != '-')
                .filter(|w| !w.is_empty() && *w != "-")
                .filter(|w| !matches!(*w, "adr" | "and" | "or" | "by" | "see"))
                .filter(|w| decision::relation(w).is_none())
                .any(|w| {
                    !ids.iter()
                        .any(|id| id.eq_ignore_ascii_case(w) || w.ends_with(id.as_str()))
                });
            if says_more {
                let key = format!("{rel}-text");
                if self.decided.insert(key.clone()) {
                    self.record(&key, value, first, last);
                }
            }
            return;
        }
        if self.decided.insert(key.to_string()) {
            self.record(key, value, first, last);
        }
    }
    fn record(&mut self, key: &str, value: &str, first: usize, last: usize) {
        let prov = self.provenance(first, last);
        let stem = self.stem.clone();
        self.push(FactKind::Decision, &stem, key, value, &prov);
    }
    /// Records every relation `text` states.
    fn relate(&mut self, text: &str, first: usize, last: usize) {
        for (rel, ids) in decision::relations(text) {
            self.link(rel, ids, first, last);
        }
    }
    fn link(&mut self, rel: &'static str, ids: Vec<String>, first: usize, last: usize) {
        let prov = self.provenance(first, last);
        let i = match self.relations.iter().position(|(r, _, _)| *r == rel) {
            Some(i) => i,
            None => {
                self.relations.push((rel, Vec::new(), Vec::new()));
                self.relations.len() - 1
            }
        };
        let (_, have, ps) = &mut self.relations[i];
        for id in ids {
            if !have.contains(&id) {
                have.push(id);
            }
        }
        for p in prov {
            if !ps.contains(&p) {
                ps.push(p);
            }
        }
    }
    /// The relation facts: one per relation, the ids in order of first mention.
    fn finish_relations(&mut self) {
        let stem = self.stem.clone();
        for (rel, ids, prov) in std::mem::take(&mut self.relations) {
            if !ids.is_empty() {
                self.push(FactKind::Decision, &stem, rel, &ids.join(", "), &prov);
            }
        }
    }
    /// A block inside a decision record's section: a status, date or relation field, or a
    /// sentence stating the record's own relation, is read as well as kept in the section.
    fn scan_section_block(&mut self, b: &Block) {
        if b.kind != BlockKind::Text {
            return;
        }
        let body = block_body(b);
        if let Some((field, value)) = decision_field(body).filter(|(f, _)| *f != "title") {
            self.decide(field, value, b.first, b.last);
        } else if decision::states_relation(body) {
            self.relate(body, b.first, b.last);
        }
    }
}

/// [`extract`], with provenance that also names the commit the document was read at.
///
/// Every fact names where it was found (`doc:<path>#L<a>-L<b>`, and `git:<sha>:…` with a
/// commit). Statements are keyed by their normalised text, so one statement in several
/// documents is one fact with every provenance; definitions are keyed by the document
/// ([`document_subject`]). A decision record ([`is_decision_record`]) also yields DECISION
/// facts under its file stem: `title`, `status` and `date` from its frontmatter, `# Title` and
/// field lines (case-insensitive), and one fact per `## <Heading>` section (`context`,
/// `decision`, `consequences`, …) whose value is the section's text, nested lists included.
pub fn extract_at(path: &str, text: &str, seq: u64, commit: Option<&str>) -> Vec<Fact> {
    let decision = is_decision_record(path);
    let name = path.rsplit('/').next().unwrap_or(path);
    let stem = name
        .rsplit_once('.')
        .filter(|(_, ext)| is_document(&format!("x.{ext}")))
        .map_or(name, |(s, _)| s)
        .to_string();
    let mut x = Extraction {
        path,
        commit,
        seq,
        stem,
        facts: Vec::new(),
        decided: BTreeSet::new(),
        relations: Vec::new(),
    };
    let raw: Vec<&str> = text.lines().collect();
    let (front_end, front) = frontmatter(text).unwrap_or((0, Vec::new()));
    for (line, key, value) in &front {
        if decision
            && (DECISION_FIELDS.contains(&key.as_str()) || decision::relation(key).is_some())
        {
            x.decide(key, value, *line, *line);
        } else {
            let prov = x.provenance(*line, *line);
            x.push(FactKind::Definition, path, key, value, &prov);
        }
    }
    let mut headings: Vec<String> = Vec::new();
    // The open `## <Heading>` section of a decision record: (key, heading line, last line).
    let mut section: Option<(String, usize, usize)> = None;
    let close = |x: &mut Extraction, section: &mut Option<(String, usize, usize)>| {
        if let Some((key, head, last)) = section.take() {
            let body = raw[head..last]
                .iter()
                .map(|l| l.trim_end())
                .collect::<Vec<_>>()
                .join("\n");
            x.decide(&key, body.trim_matches('\n'), head, last);
        }
    };
    for b in blocks(text) {
        if b.last <= front_end {
            continue;
        }
        if let BlockKind::Heading(level) = b.kind {
            headings.truncate(level.saturating_sub(1));
            headings.push(b.text.clone());
            if decision && level <= 2 {
                close(&mut x, &mut section);
                if level == 1 {
                    x.decide("title", &b.text, b.first, b.last);
                } else {
                    section = Some((section_key(&b.text), b.first, b.first));
                }
                continue;
            }
        }
        if let Some((_, _, last)) = section.as_mut() {
            // A section's text, sub-headings and nested lists included, is one DECISION fact.
            *last = b.last;
            x.scan_section_block(&b);
            continue;
        }
        let prov = x.provenance(b.first, b.last);
        match b.kind {
            BlockKind::Code => {
                let k = normalize(&b.text);
                if !k.is_empty() {
                    x.push(FactKind::Statement, "", &k, b.text.trim_end(), &prov);
                }
            }
            BlockKind::Heading(_) => {}
            BlockKind::Text => {
                let body = block_body(&b);
                if body.is_empty() || is_rule(body) {
                    continue;
                }
                if decision {
                    if let Some((field, value)) = decision_field(body) {
                        x.decide(field, value, b.first, b.last);
                        continue;
                    }
                    if decision::states_relation(body) {
                        x.relate(body, b.first, b.last);
                    }
                }
                let def = body.split_once(": ").filter(|(t, d)| {
                    !t.is_empty()
                        && t.split_whitespace().count() <= 5
                        && !d.trim().is_empty()
                        && !t.contains('`')
                });
                match def {
                    Some((term, definition)) => x.push(
                        FactKind::Definition,
                        &document_subject(path, &headings),
                        &normalize(term),
                        definition.trim(),
                        &prov,
                    ),
                    None => {
                        let k = normalize(body);
                        if !k.is_empty() {
                            x.push(FactKind::Statement, "", &k, body, &prov);
                        }
                    }
                }
            }
        }
    }
    close(&mut x, &mut section);
    x.finish_relations();
    let mut facts = x.facts;
    facts.extend(stored(path, text, seq, commit, outline::outline(text)));
    facts
}

/// The facts that store a text verbatim: one BLOCK per span, keyed by position, with the
/// span's lines as provenance, and the DOCUMENT digest the blocks rebuild to.
fn stored(
    path: &str,
    text: &str,
    seq: u64,
    commit: Option<&str>,
    spans: Vec<outline::Span>,
) -> Vec<Fact> {
    let mut out: Vec<Fact> = spans
        .iter()
        .enumerate()
        .map(|(i, s)| {
            fact(
                FactKind::Block,
                path,
                &outline::block_key(i),
                &outline::encode_block(&s.tag, &s.raw),
                &provenance(path, s.first, s.last, commit),
                seq,
            )
        })
        .collect();
    let mut whole = vec![format!("doc:{path}")];
    if let Some(c) = commit.filter(|c| !c.is_empty()) {
        whole.push(format!("git:{c}:{path}"));
    }
    let digest = content_digest(text.as_bytes());
    out.push(fact(
        FactKind::Document,
        path,
        "digest",
        &digest,
        &whole,
        seq,
    ));
    out
}

/// Whether a file is a licence or notice text (`LICENSE`, `LICENCE-MIT`, `COPYING`,
/// `NOTICE.txt`, `licenses/apache.txt`, …), which knowledge may store verbatim.
pub fn is_licence_text(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let segs: Vec<&str> = lower.split('/').collect();
    let name = segs.last().copied().unwrap_or("");
    let ext = name.rsplit_once('.').map(|(_, e)| e);
    let text_ext = ext.is_none_or(|e| matches!(e, "txt" | "text" | "md" | "markdown" | "rst"));
    let code_ext = ext.is_some_and(|e| {
        matches!(
            e,
            "rs" | "toml"
                | "json"
                | "jsonl"
                | "yaml"
                | "yml"
                | "js"
                | "ts"
                | "py"
                | "go"
                | "html"
                | "css"
                | "sh"
                | "lock"
        )
    });
    let named = [
        "license",
        "licence",
        "copying",
        "notice",
        "unlicense",
        "copyright",
    ]
    .iter()
    .any(|w| name.starts_with(w));
    let in_dir = segs[..segs.len() - 1]
        .iter()
        .any(|s| matches!(*s, "license" | "licenses" | "licence" | "licences"));
    (named && !code_ext) || (in_dir && text_ext)
}

/// Stores a plain text (a licence, a notice) verbatim: one `text` BLOCK and its DOCUMENT
/// digest. Nothing else is read out of it.
pub fn extract_verbatim(path: &str, text: &str, seq: u64, commit: Option<&str>) -> Vec<Fact> {
    stored(path, text, seq, commit, vec![outline::verbatim(text)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extraction_keeps_definitions_decisions_and_digest() {
        let text = "# Use native hashing\n\nStatus: Accepted\n\n- Census: a deterministic observation\n- The log is append-only.\n\n```\nfn x() {}\n```\n";
        let facts = extract(".atlas/decisions/0001-hash.md", text, 1);
        assert!(facts.iter().any(|f| f.kind == FactKind::Decision
            && f.key == "title"
            && f.value == "Use native hashing"));
        assert!(facts
            .iter()
            .any(|f| f.kind == FactKind::Decision && f.key == "status" && f.value == "Accepted"));
        assert!(facts
            .iter()
            .any(|f| f.kind == FactKind::Definition && f.key == "census"));
        assert!(facts
            .iter()
            .any(|f| f.kind == FactKind::Statement && f.value == "The log is append-only."));
        assert!(facts
            .iter()
            .any(|f| f.kind == FactKind::Statement && f.value == "fn x() {}"));
        assert!(facts
            .iter()
            .any(|f| f.kind == FactKind::Document && f.subject == ".atlas/decisions/0001-hash.md"));
    }
    #[test]
    fn wrapped_bullet_is_one_fact() {
        let text = "# Notes\n\n- The page store is append-only and every\n  record is addressed by its digest.\n- Census: a deterministic observation\n  of committed content.\n1. First numbered item that\n   wraps onto a second line.\n\nA plain paragraph that is\nwrapped by the editor\n> and quoted.\nStatus: Draft\nDate: 2026-10-01\n\n| a | b |\n|---|---|\n| c | d |\n";
        let facts = extract("docs/notes.md", text, 1);
        let statements: Vec<&str> = facts
            .iter()
            .filter(|f| f.kind == FactKind::Statement)
            .map(|f| f.value.as_str())
            .collect();
        assert!(statements.contains(
            &"The page store is append-only and every record is addressed by its digest."
        ));
        assert!(statements.contains(&"1. First numbered item that wraps onto a second line."));
        assert!(statements.contains(&"A plain paragraph that is wrapped by the editor and quoted."));
        assert!(
            !statements
                .iter()
                .any(|s| s.starts_with("record is addressed")),
            "no fragment of a wrapped bullet: {statements:?}"
        );
        let census = facts
            .iter()
            .find(|f| f.kind == FactKind::Definition && f.key == "census")
            .unwrap();
        assert_eq!(
            census.value,
            "a deterministic observation of committed content."
        );
        assert_eq!(
            census.provenance.iter().cloned().collect::<Vec<_>>(),
            vec!["doc:docs/notes.md#L5-L6".to_string()]
        );
        // Field lines start their own block; table rows stay separate.
        assert!(facts
            .iter()
            .any(|f| f.kind == FactKind::Definition && f.key == "status" && f.value == "Draft"));
        assert!(facts
            .iter()
            .any(|f| f.kind == FactKind::Definition && f.key == "date"));
        assert!(statements.contains(&"| a | b |") && statements.contains(&"| c | d |"));
    }

    #[test]
    fn decision_fields_are_read_from_frontmatter_case_insensitively() {
        let text = "---\nid: fi.decision.0007\nStatus: Accepted\nTITLE: \"Native hashing\"\ndate: 2026-09-30\n---\n# 0007 — Use native hashing\n\nstatus: superseded later in the body\n";
        let facts = extract("docs/adr/0007-hash.md", text, 1);
        let get = |k: FactKind, key: &str| {
            facts
                .iter()
                .find(|f| f.kind == k && f.key == key)
                .map(|f| (f.subject.as_str(), f.value.as_str()))
        };
        assert_eq!(
            get(FactKind::Decision, "status"),
            Some(("0007-hash", "Accepted"))
        );
        assert_eq!(
            get(FactKind::Decision, "title"),
            Some(("0007-hash", "Native hashing")),
            "the frontmatter title wins over the heading"
        );
        assert_eq!(
            get(FactKind::Decision, "date"),
            Some(("0007-hash", "2026-09-30"))
        );
        // Other frontmatter keys are definitions of this document.
        assert_eq!(
            get(FactKind::Definition, "id"),
            Some(("docs/adr/0007-hash.md", "fi.decision.0007"))
        );
        let status: Vec<&Fact> = facts.iter().filter(|f| f.key == "status").collect();
        assert_eq!(status.len(), 1, "{status:?}");
        assert!(status[0]
            .provenance
            .contains("doc:docs/adr/0007-hash.md#L3"));
        // Without frontmatter, a lowercase field line is still the status.
        let facts = extract("decisions/0008-x.md", "# X\n\nstatus: Proposed\n", 1);
        assert!(facts
            .iter()
            .any(|f| f.kind == FactKind::Decision && f.key == "status" && f.value == "Proposed"));
    }

    #[test]
    fn decision_sections_are_decision_facts_with_their_nested_lists() {
        let sha = "0123456789abcdef0123456789abcdef01234567";
        let text = "# 0003 — One knowledge authority\n\n## Status\n\nAccepted\n\n## Context\n\nTwo authorities drift:\n- the legacy tree\n  - its registries\n  - its ledger\n- the subsystem\n\n## Decision\n\nOne authority.\n\n### Detail\n\n- Knowledge enters through extraction.\n\n## Consequences\n\n- Nothing reads the legacy tree.\n\n## Alternatives Considered\n\nKeeping both.\n";
        let facts = extract_at(".atlas/decisions/0003-authority.md", text, 4, Some(sha));
        let decision = |key: &str| {
            facts
                .iter()
                .find(|f| f.kind == FactKind::Decision && f.key == key)
                .unwrap_or_else(|| panic!("no {key}: {facts:#?}"))
        };
        assert_eq!(decision("title").value, "0003 — One knowledge authority");
        assert_eq!(decision("status").value, "Accepted");
        let context = decision("context");
        assert_eq!(
            context.value,
            "Two authorities drift:\n- the legacy tree\n  - its registries\n  - its ledger\n- the subsystem"
        );
        assert_eq!(context.subject, "0003-authority");
        assert!(context
            .provenance
            .contains("doc:.atlas/decisions/0003-authority.md#L7-L13"));
        assert!(context.provenance.contains(&format!(
            "git:{sha}:.atlas/decisions/0003-authority.md#L7-L13"
        )));
        assert_eq!(
            decision("decision").value,
            "One authority.\n\n### Detail\n\n- Knowledge enters through extraction."
        );
        assert_eq!(
            decision("consequences").value,
            "- Nothing reads the legacy tree."
        );
        assert_eq!(decision("alternatives considered").value, "Keeping both.");
        // Section text stays in its section: no separate statements or definitions.
        assert!(
            facts
                .iter()
                .all(|f| !matches!(f.kind, FactKind::Statement | FactKind::Definition)),
            "{facts:#?}"
        );
        // Outside a decision record, the same text is statements as before.
        let plain = extract("docs/notes.md", text, 1);
        assert!(plain.iter().all(|f| f.kind != FactKind::Decision));
        assert!(plain
            .iter()
            .any(|f| f.kind == FactKind::Statement && f.value == "its registries"));
    }

    #[test]
    fn definitions_of_different_documents_never_supersede_each_other() {
        use crate::compact::facts::fold;
        let a = extract(
            "docs/a.md",
            "---\nid: doc.a\n---\n# Terms\n\n- Owner: team a\n",
            1,
        );
        let b = extract(
            "docs/b.md",
            "---\nid: doc.b\n---\n# Terms\n\n- Owner: team b\n",
            2,
        );
        let folded = fold(a.iter().chain(&b).cloned());
        let defs: Vec<(&str, &str, &str)> = folded
            .iter()
            .filter(|f| f.kind == FactKind::Definition)
            .map(|f| (f.subject.as_str(), f.key.as_str(), f.value.as_str()))
            .collect();
        assert_eq!(defs.len(), 4, "{defs:?}");
        for want in [
            ("docs/a.md", "id", "doc.a"),
            ("docs/b.md", "id", "doc.b"),
            ("docs/a.md / Terms", "owner", "team a"),
            ("docs/b.md / Terms", "owner", "team b"),
        ] {
            assert!(defs.contains(&want), "{want:?} in {defs:?}");
        }
        assert!(
            folded.iter().all(|f| f.superseded.is_empty()),
            "{folded:#?}"
        );
        // One statement in two documents is one fact with both provenances, superseding nothing.
        let c = extract("docs/c.md", "- The log is **append-only**.\n", 3);
        let d = extract("docs/d.md", "- The log is append-only.\n", 4);
        let folded = fold(c.into_iter().chain(d));
        let st = folded
            .iter()
            .find(|f| f.kind == FactKind::Statement)
            .unwrap();
        assert_eq!(st.provenance.len(), 2);
        assert!(st.superseded.is_empty(), "{st:?}");
        // Re-extracting the same document supersedes its own earlier value.
        let again = extract("docs/a.md", "---\nid: doc.a2\n---\n", 5);
        let folded = fold(a.into_iter().chain(b).chain(again));
        let id = |s: &str| {
            folded
                .iter()
                .find(|f| f.kind == FactKind::Definition && f.subject == s && f.key == "id")
                .unwrap()
        };
        assert_eq!(id("docs/a.md").value, "doc.a2");
        assert_eq!(id("docs/a.md").superseded, vec!["doc.a".to_string()]);
        assert!(id("docs/b.md").superseded.is_empty());
    }

    #[test]
    fn extraction_provenance_names_the_commit() {
        let sha = "0123456789abcdef0123456789abcdef01234567";
        let facts = extract_at(
            ".atlas/decisions/0002-x.md",
            "# X\n\nStatus: Accepted\n\n- A statement that\n  wraps.\n",
            3,
            Some(sha),
        );
        let st = facts
            .iter()
            .find(|f| f.kind == FactKind::Statement)
            .unwrap();
        assert!(st
            .provenance
            .contains(&format!("git:{sha}:.atlas/decisions/0002-x.md#L5-L6")));
        assert!(st
            .provenance
            .contains("doc:.atlas/decisions/0002-x.md#L5-L6"));
        let status = facts.iter().find(|f| f.key == "status").unwrap();
        assert!(status
            .provenance
            .contains(&format!("git:{sha}:.atlas/decisions/0002-x.md#L3")));
        let doc = facts.iter().find(|f| f.kind == FactKind::Document).unwrap();
        assert!(doc
            .provenance
            .contains(&format!("git:{sha}:.atlas/decisions/0002-x.md")));
        // Without a commit, provenance is the document alone.
        assert!(extract(".atlas/decisions/0002-x.md", "- a statement\n", 1)
            .iter()
            .all(|f| f.provenance.iter().all(|p| p.starts_with("doc:"))));
    }

    #[test]
    fn decision_status_is_read_in_every_form_and_normalised() {
        let get = |text: &str| {
            let facts = extract_at("docs/adr/0009-x.md", text, 1, None);
            let mut out: Vec<(String, String)> = facts
                .iter()
                .filter(|f| f.kind == FactKind::Decision && f.key != "title")
                .filter(|f| {
                    f.key.starts_with("status")
                        || f.key.starts_with("supersed")
                        || f.key.starts_with("refine")
                })
                .map(|f| (f.key.clone(), f.value.clone()))
                .collect();
            out.sort();
            out
        };
        let kv = |pairs: &[(&str, &str)]| {
            let mut v: Vec<(String, String)> = pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            v.sort();
            v
        };
        // A `## Status` section, its text a paragraph naming the successor.
        assert_eq!(
            get("# X\n\n## Status\n\nSuperseded by [ADR 0012](0012-use-y.md)\n\n## Context\n\nWhy.\n"),
            kv(&[
                ("status", "Superseded"),
                ("status-text", "Superseded by ADR 0012"),
                ("superseded-by", "0012-use-y"),
            ])
        );
        assert_eq!(
            get("# X\n\n## Status\n\nAccepted\n"),
            kv(&[("status", "Accepted")])
        );
        // Bold, list, lowercase and table field forms.
        for text in [
            "# X\n\n**Status:** Accepted\n",
            "# X\n\n**Status**: **ACCEPTED**\n",
            "# X\n\n- Status: accepted\n- Date: 2026-09-27\n",
            "# X\n\n* status: Accepted.\n",
            "# X\n\n| Field | Value |\n|---|---|\n| Status | Accepted |\n",
            "---\nstatus: accepted\n---\n# X\n",
            "# X\n\n## Metadata\n\n- Status: Accepted\n- Owner: core\n",
        ] {
            assert_eq!(get(text), kv(&[("status", "Accepted")]), "{text}");
        }
        // Real forms: a date and a successor in the status line, refinements, supersessions.
        assert_eq!(
            get("# 0004\n\n- Status: SUPERSEDED (2026-09-27) by 0010 (two-language production architecture)\n- Context: x\n"),
            kv(&[
                ("status", "Superseded"),
                ("status-text", "SUPERSEDED (2026-09-27) by 0010 (two-language production architecture)"),
                ("superseded-by", "0010"),
            ])
        );
        assert_eq!(
            get("# 0012\n\n- Status: ACCEPTED (2026-09-27); REFINED by 0015 (core/, mcp/)\n"),
            kv(&[
                ("refined-by", "0015"),
                ("status", "Accepted"),
                (
                    "status-text",
                    "ACCEPTED (2026-09-27); REFINED by 0015 (core/, mcp/)"
                ),
            ])
        );
        assert_eq!(
            get("---\nstatus: superseded\nsuperseded_by: \"0031\"\n---\n# X\n\nThis ADR supersedes ADR-0018.\n"),
            kv(&[
                ("status", "Superseded"),
                ("superseded-by", "0031"),
                ("supersedes", "0018"),
            ])
        );
        assert_eq!(
            get("# X\n\nStatus: Accepted\n\n## Supersedes\n\n- ADR 0003 and ADR 0004\n"),
            kv(&[("status", "Accepted"), ("supersedes", "0003, 0004")])
        );
        // Outside a decision record a status line stays a definition of the document.
        let facts = extract("docs/notes.md", "# N\n\nStatus: Superseded by 0012\n", 1);
        assert!(facts.iter().all(|f| f.kind != FactKind::Decision));
    }
}
