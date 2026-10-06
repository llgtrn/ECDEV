//! `ecdev-gov migrate universe --from <legacy registry> [--kind <KIND>] [--table <name>]`:
//! a lossless import of one legacy registry into `.ecdev/declared/universe/`.
//!
//! Formats: JSONL (one record per line), TOML (an array of tables, with or without a header
//! table) and JSON (an array of records, at the top or under one key, such as ECDEV's
//! `research/commerce/donors/registry.json`). Every record keeps its legacy text
//! verbatim and every byte between records is kept as a segment, so the universe file rebuilds
//! the registry byte for byte (its digest is recorded and re-proven on every read). The typed
//! fields are derived from the legacy fields by the rules below; the legacy lifecycle words are
//! kept as `legacy_state` and are claims, never evidence. A re-import of the same bytes writes
//! the same file (idempotent); declared aliases of a record survive re-import. The legacy
//! registry is never modified.

use super::{identity, Alias, Record, Relevance, Source, UNIVERSE_DIR};
use crate::declare::Declaration;
use crate::formats::{json, toml, Value};
use crate::schema::{UniverseKind, UniverseState};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Names of arrays of tables that hold records, in the legacy registries.
pub const RECORD_TABLES: &[&str] = &[
    "repo",
    "repository",
    "donor",
    "donors",
    "spec",
    "paper",
    "entry",
    "simulator",
    "record",
    "records",
    "source",
    "repositories",
    "entries",
];

/// Fields naming a record's upstream, in order of preference.
const ORIGIN_FIELDS: &[&str] = &[
    "repository",
    "remote",
    "resolved_url",
    "upstream",
    "source_url",
    "url",
    "homepage",
];
/// Fields stating why a record is relevant to its repository.
const RELEVANCE_FIELDS: &[&str] = &[
    "capability",
    "capabilities",
    "mechatron_subsystems",
    "subsystems",
    "technology_domains",
    "serves",
    "target_capabilities",
    "targets",
    "teaches",
    "needs",
    "relevance",
    "terminal_state",
    "reason",
];
/// Legacy lifecycle fields (kept verbatim as `legacy_state`).
const STATE_FIELDS: &[&str] = &[
    "lifecycle",
    "status",
    "ingestion_status",
    "census_status",
    "decision_status",
    "replay_status",
    "terminal_state",
    "disposition",
];
/// Legacy words meaning the record was selected for deep census (cloned, acquired, censused).
const SELECTED_WORDS: &[&str] = &[
    "CLONED",
    "ACQUIRED",
    "CENSUSED",
    "DEEP_CENSUSED",
    "DEPENDENCY_CENSUSED",
    "MAPPED",
    "PARTIALLY_EXTRACTED",
    "NATIVE_REPLACEMENT_STARTED",
    "NATIVE_EQUIVALENCE_PARTIAL",
    "DISPOSITIONED",
    "ABSORBING",
    "ABSORBED",
    "EXTINCTION_READY",
    "EXTINCT",
    "PROCESSED",
    "QUEUED",
    "MATERIALIZED",
];
const UNREGISTERED_WORDS: &[&str] = &["DISCOVERED", "CANDIDATE"];
// A legacy "REJECTED" was a runtime decision, never a knowledge verdict, so it does not map here.
const NOT_RELEVANT_WORDS: &[&str] = &["NOT_RELEVANT", "DROP", "OUT_OF_SCOPE"];

/// The universe file of a legacy registry path.
pub fn universe_file(from: &str) -> String {
    format!(
        "{UNIVERSE_DIR}/{}.jsonl",
        from.trim_matches('/').replace('/', ".")
    )
}

/// The kind a legacy word names (a record `type`, a registry `kind`, a table name).
pub fn kind_of_word(w: &str) -> Option<UniverseKind> {
    let w = w.trim().to_ascii_uppercase().replace(['-', ' ', '.'], "_");
    if let Some(k) = UniverseKind::from_wire(&w) {
        return Some(k);
    }
    use UniverseKind as K;
    Some(match w.as_str() {
        "OSS_DONOR" | "OSS_UNIVERSE" | "OSS" | "REPO" | "REPOS" | "REPOSITORY" | "REPOSITORIES"
        | "DONOR" | "DONORS" => K::OssRepository,
        "PROTOCOL_SPECS" | "PROTOCOLS" | "PROTOCOL" => K::ProtocolSpec,
        "BENCHMARKS" | "BENCHMARK_REFERENCE" => K::Benchmark,
        "DATASETS" => K::Dataset,
        "SIMULATORS" => K::Simulator,
        "STANDARDS" => K::Standard,
        "REFERENCE_ARCHITECTURES" => K::ReferenceArchitecture,
        "PAPERS" => K::Paper,
        "OFFICIAL_AUTHORITIES" | "AUTHORITY" => K::OfficialAuthority,
        "ACADEMIC_REFERENCES" | "ACADEMIC" => K::AcademicReference,
        w if w.ends_with("_SOURCE") || w.ends_with("_SOURCES") => K::InformationSource,
        _ => return None,
    })
}

/// A string field, or a list of strings joined; empty when absent.
fn text(v: &Value, k: &str) -> String {
    match v.get(k) {
        Some(Value::Str(s)) => s.trim().to_string(),
        Some(Value::Int(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_str())
            .map(str::trim)
            .filter(|x| !x.is_empty())
            .collect::<Vec<_>>()
            .join(", "),
        Some(Value::Table(t)) => t
            .iter()
            .map(|(k, v)| match v {
                Value::Str(s) => format!("{k}: {s}"),
                other => format!("{k}: {other:?}"),
            })
            .collect::<Vec<_>>()
            .join(", "),
        _ => String::new(),
    }
}

fn first(v: &Value, fields: &[&str]) -> Option<(String, String)> {
    fields.iter().find_map(|f| {
        let t = text(v, f);
        (!t.is_empty()).then(|| (f.to_string(), t))
    })
}

fn is_hex(s: &str) -> bool {
    s.len() >= 7 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// The words of a record's legacy lifecycle fields.
fn state_words(v: &Value) -> Vec<String> {
    STATE_FIELDS
        .iter()
        .filter_map(|f| v.str(f))
        .map(|s| s.trim().to_ascii_uppercase().replace(['-', ' '], "_"))
        .collect()
}

/// Derives the canonical record of one legacy record.
fn derive(
    v: &Value,
    id: String,
    kind: UniverseKind,
    legacy: String,
    from: &str,
    declaration: Option<&Declaration>,
) -> Record {
    let name = first(v, &["name", "repo", "key", "id"])
        .map(|x| x.1)
        .unwrap_or_else(|| id.clone());
    let mut origin_evidence = String::new();
    let mut origin = first(v, ORIGIN_FIELDS).map(|x| x.1).unwrap_or_default();
    if origin.contains(", ") {
        // A list of upstreams: the first one is the record's.
        origin = origin.split(", ").next().unwrap_or("").to_string();
    }
    if origin.is_empty() {
        // A registry that relies on the declaration for its upstream (the same key there).
        if let Some(dn) = declaration.and_then(|d| d.donor(&id)) {
            origin = dn.origin.clone();
            origin_evidence = format!("origin from .ecdev/declared/donors.rs#{}", dn.key);
        }
    }
    let global = identity::global_donor_key(&origin);

    // Origin verification: a reachability check, a remote HEAD, or a fetch at a revision.
    let verification = text(v, "verification");
    let url_check = text(v, "url_check");
    let mut verified = false;
    let mut evidence = Vec::new();
    if !verification.is_empty() {
        verified |= verification.starts_with("HTTP 2")
            || verification.starts_with("HEAD ")
            || verification.starts_with("CROSSREF MATCH");
        evidence.push(format!("verification: {verification}"));
    }
    if v.get("verified") == Some(&Value::Bool(true)) {
        verified = true;
        evidence.push(format!(
            "verified head {} on {}",
            text(v, "verified_head"),
            text(v, "verified_on")
        ));
    }
    if !url_check.is_empty() {
        verified |= url_check.starts_with("HTTP 2") || url_check.starts_with("DOI registered");
        evidence.push(format!("url_check: {url_check}"));
    }
    // A pinned revision (a remote HEAD seen by a reachability check is not a pin).
    let revision = ["revision", "pinned_commit", "commit_sha"]
        .iter()
        .map(|f| text(v, f))
        .find(|r| is_hex(r))
        .unwrap_or_default();
    if !revision.is_empty() {
        verified = true;
        evidence.push(format!("fetched at {revision}"));
    }
    let observed = text(v, "observed_head");
    if is_hex(&observed) {
        verified = true;
        evidence.push(format!(
            "upstream HEAD {observed} on {}",
            text(v, "observed_at")
        ));
    }
    let pins = text(v, "historical_pins");
    if !pins.is_empty() && revision.is_empty() {
        verified = true;
        evidence.push(format!("historical pins {pins}"));
    }
    if !origin_evidence.is_empty() {
        evidence.insert(0, origin_evidence);
    }
    if global.is_none() {
        verified = false;
    }

    // Licence: verified only when read from the upstream's own licence file.
    let mut license = String::new();
    let mut license_claim = String::new();
    let claim = first(v, &["license", "license_hint", "license_claim"])
        .map(|x| x.1)
        .unwrap_or_default();
    let observed_license = text(v, "license_observed");
    if !observed_license.is_empty() {
        license = observed_license;
    } else if text(v, "license_state").eq_ignore_ascii_case("VERIFIED")
        || !text(v, "license_status").is_empty()
        || (v.get("license_verified") == Some(&Value::Bool(true)))
        || matches!(v.get("license"), Some(Value::Array(a)) if !a.is_empty())
    {
        license = if claim.is_empty() {
            "verified (licence not restated in the record)".into()
        } else {
            claim.clone()
        };
    } else {
        license_claim = claim;
    }

    let words = state_words(v);
    let relevance = match words
        .iter()
        .find(|w| NOT_RELEVANT_WORDS.contains(&w.as_str()))
    {
        Some(w) => Relevance::NotRelevant(format!("legacy state {w}")),
        None => match first(v, RELEVANCE_FIELDS) {
            Some((f, t)) => Relevance::Relevant(format!("{f}: {t}")),
            None => Relevance::Unresolved,
        },
    };
    let selected = !revision.is_empty()
        || ["inventory", "donor_ref", "clone_dir"]
            .iter()
            .any(|f| !text(v, f).is_empty() && text(v, f) != "null")
        || words.iter().any(|w| SELECTED_WORDS.contains(&w.as_str()));
    let claimed = if words
        .iter()
        .any(|w| UNREGISTERED_WORDS.contains(&w.as_str()))
    {
        UniverseState::Discovered
    } else {
        UniverseState::Registered
    };
    let legacy_state = STATE_FIELDS
        .iter()
        .filter_map(|f| {
            let t = text(v, f);
            (!t.is_empty()).then(|| format!("{f}={t}"))
        })
        .collect::<Vec<_>>()
        .join("; ");
    Record {
        provenance: format!("{from}#{id}"),
        id,
        kind,
        name,
        origin,
        global,
        origin_verified: verified,
        origin_evidence: evidence.join("; "),
        license,
        license_claim,
        revision,
        relevance,
        selected,
        claimed,
        legacy_state,
        aliases: Vec::new(),
        legacy,
    }
}

/// What to import and how to read it.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Every record is of this kind.
    pub kind: Option<UniverseKind>,
    /// The array of tables (TOML) or array key (JSON) holding the records.
    pub table: Option<String>,
}

/// Reads one legacy registry into a universe source. Nothing is written.
pub fn read(
    root: &Path,
    from: &str,
    opts: &Options,
    declaration: Option<&Declaration>,
) -> Result<Source, String> {
    let from = from.trim_start_matches("./").to_string();
    let bytes = std::fs::read(root.join(&from)).map_err(|e| format!("{from}: {e}"))?;
    let text = String::from_utf8(bytes).map_err(|_| format!("{from}: not UTF-8"))?;
    let (format, table, raw) = if from.ends_with(".jsonl") {
        ("JSONL", String::new(), split_jsonl(&text)?)
    } else if from.ends_with(".toml") {
        let (t, r) = split_toml(&text, opts.table.as_deref())?;
        ("TOML", t, r)
    } else if from.ends_with(".json") {
        let (t, r) = split_json(&text, opts.table.as_deref())?;
        ("JSON", t, r)
    } else {
        return Err(format!(
            "{from}: unknown registry format (JSONL, TOML or JSON)"
        ));
    };
    let Split {
        records,
        segments,
        head,
    } = raw;
    let file_kind = opts.kind.or_else(|| {
        [
            head.at("registry.kind").and_then(Value::as_str),
            head.get("ledger").map(|_| "donor"),
            Some(table.as_str()),
            head.str("kind"),
        ]
        .into_iter()
        .flatten()
        .find_map(kind_of_word)
    });
    let mut ids = BTreeSet::new();
    let mut out = Vec::new();
    for (n, (v, legacy)) in records.into_iter().enumerate() {
        let kind = match opts
            .kind
            .or_else(|| v.str("type").and_then(kind_of_word))
            .or(file_kind)
        {
            Some(k) => k,
            None => {
                return Err(format!(
                    "{from}: record {} has no kind I can read (no `type`, no registry kind): pass --kind <{}>",
                    n + 1,
                    UniverseKind::ALL
                        .iter()
                        .map(|k| k.wire())
                        .collect::<Vec<_>>()
                        .join("|")
                ))
            }
        };
        let base = first(&v, &["id", "key"])
            .map(|x| x.1)
            .unwrap_or_else(|| format!("#{}", n + 1));
        let mut id = base.clone();
        let mut k = 2;
        while !ids.insert(id.clone()) {
            id = format!("{base}~{k}");
            k += 1;
        }
        out.push(derive(&v, id, kind, legacy, &from, declaration));
    }
    let src = Source {
        file: universe_file(&from),
        source: from.clone(),
        format: format.into(),
        table,
        digest: crate::digest::content_digest(text.as_bytes()),
        bytes: text.len() as u64,
        segments,
        records: out,
    };
    if src.rebuild() != text {
        return Err(format!(
            "{from}: the split does not rebuild the registry byte for byte; nothing written"
        ));
    }
    Ok(src)
}

/// What an import did.
#[derive(Clone, Debug)]
pub struct Outcome {
    pub source: Source,
    /// Whether the universe file was (or, in a dry run, would be) written.
    pub changed: bool,
    pub added: usize,
    pub removed: usize,
    pub modified: usize,
}

/// Imports `from` into the universe of `root` (or only reports, with `dry_run`).
pub fn import(
    root: &Path,
    from: &str,
    opts: &Options,
    declaration: Option<&Declaration>,
    dry_run: bool,
) -> Result<Outcome, String> {
    let mut src = read(root, from, opts, declaration)?;
    let path = root.join(&src.file);
    let previous = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| Source::parse(&src.file, &t).ok());
    let mut added = src.records.len();
    let (mut removed, mut modified) = (0, 0);
    if let Some(p) = &previous {
        let old: BTreeMap<&str, &Record> = p.records.iter().map(|r| (r.id.as_str(), r)).collect();
        // Declared aliases are the repository's own evidence: they survive re-import.
        for r in src.records.iter_mut() {
            if let Some(o) = old.get(r.id.as_str()) {
                r.aliases = o.aliases.clone();
            }
        }
        let new: BTreeMap<&str, &Record> = src.records.iter().map(|r| (r.id.as_str(), r)).collect();
        added = new.keys().filter(|k| !old.contains_key(*k)).count();
        removed = old.keys().filter(|k| !new.contains_key(*k)).count();
        modified = new
            .iter()
            .filter(|(k, r)| old.get(*k).is_some_and(|o| o != *r))
            .count();
    }
    let text = src.render();
    let changed = std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str());
    if changed && !dry_run {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, &text).map_err(|e| e.to_string())?;
    }
    // A sanity read-back: what was written reads as the same source.
    if !dry_run {
        let back = Source::parse(&src.file, &text)?;
        if back != src {
            return Err("the universe file does not read back as written".into());
        }
    }
    Ok(Outcome {
        source: src,
        changed,
        added,
        removed,
        modified,
    })
}

/// Declares an alias of a record: another origin of the same upstream, with its evidence.
pub fn add_alias(src: &mut Source, id: &str, origin: &str, provenance: &str) -> Result<(), String> {
    let r = src
        .records
        .iter_mut()
        .find(|r| r.id == id)
        .ok_or(format!("no record `{id}`"))?;
    let a = Alias {
        origin: origin.to_string(),
        provenance: provenance.to_string(),
    };
    if !r.aliases.contains(&a) {
        r.aliases.push(a);
        r.aliases.sort();
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Splitting a registry into records (verbatim) and segments (everything else, verbatim).

struct Split {
    /// (parsed record, its verbatim text).
    records: Vec<(Value, String)>,
    segments: Vec<(u64, String)>,
    /// The parsed document (TOML/JSON) for registry-level fields.
    head: Value,
}

fn push_segment(segments: &mut Vec<(u64, String)>, pos: usize, text: &str) {
    if text.is_empty() {
        return;
    }
    match segments.last_mut() {
        Some((p, t)) if *p == pos as u64 => t.push_str(text),
        _ => segments.push((pos as u64, text.to_string())),
    }
}

fn split_jsonl(text: &str) -> Result<Split, String> {
    let mut records = Vec::new();
    let mut segments = Vec::new();
    for (n, line) in text.split_inclusive('\n').enumerate() {
        if line.trim().is_empty() {
            push_segment(&mut segments, records.len(), line);
            continue;
        }
        let v = json::parse(line.trim_end_matches(['\n', '\r']))
            .map_err(|e| format!("line {}: {e}", n + 1))?;
        if v.table().is_none() {
            return Err(format!("line {}: a record is a JSON object", n + 1));
        }
        records.push((v, line.to_string()));
    }
    Ok(Split {
        records,
        segments,
        head: Value::Table(Default::default()),
    })
}

/// Top-level headers of a TOML text: (byte offset of the line, table name, array of tables).
fn toml_headers(text: &str) -> Vec<(usize, String, bool)> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut ml: Option<&[u8]> = None;
    let mut off = 0;
    for line in text.split_inclusive('\n') {
        let start = off;
        off += line.len();
        let t = line.trim_start();
        if ml.is_none() && depth <= 0 && t.starts_with('[') {
            let array = t.starts_with("[[");
            let inner = if array {
                t[2..].split("]]").next()
            } else {
                t[1..].split(']').next()
            };
            let name: String = inner
                .unwrap_or("")
                .split('.')
                .map(|s| s.trim().trim_matches('"'))
                .collect::<Vec<_>>()
                .join(".");
            out.push((start, name, array));
            depth = 0;
            continue;
        }
        let b = line.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if let Some(q) = ml {
                if b[i..].starts_with(q) {
                    ml = None;
                    i += 3;
                } else if q == b"\"\"\"" && b[i] == b'\\' {
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }
            match b[i] {
                b'#' => break,
                b'"' if b[i..].starts_with(b"\"\"\"") => {
                    ml = Some(b"\"\"\"");
                    i += 3;
                    continue;
                }
                b'\'' if b[i..].starts_with(b"'''") => {
                    ml = Some(b"'''");
                    i += 3;
                    continue;
                }
                b'"' => {
                    i += 1;
                    while i < b.len() && b[i] != b'"' {
                        if b[i] == b'\\' {
                            i += 1;
                        }
                        i += 1;
                    }
                }
                b'\'' => {
                    i += 1;
                    while i < b.len() && b[i] != b'\'' {
                        i += 1;
                    }
                }
                b'[' | b'{' => depth += 1,
                b']' | b'}' => depth -= 1,
                _ => {}
            }
            i += 1;
        }
    }
    out
}

fn split_toml(text: &str, table: Option<&str>) -> Result<(String, Split), String> {
    let doc = toml::parse(text).map_err(|e| e.to_string())?;
    let table = match table {
        Some(t) => t.to_string(),
        None => {
            let mut best: Option<(&str, usize)> = None;
            for t in RECORD_TABLES {
                let n = doc
                    .items_of(t)
                    .iter()
                    .filter(|x| x.table().is_some())
                    .count();
                if n > 0 && best.is_none_or(|(_, m)| n > m) {
                    best = Some((t, n));
                }
            }
            best.map(|(t, _)| t.to_string()).ok_or(format!(
                "no array of record tables ({}): pass --table <name>",
                RECORD_TABLES.join(", ")
            ))?
        }
    };
    let items = doc.items_of(&table).to_vec();
    let headers = toml_headers(text);
    // Block starts: every header except a sub-table of the record table while a record is open.
    let mut starts: Vec<(usize, bool)> = vec![(0, false)];
    let mut open = false;
    let sub = format!("{table}.");
    for (at, name, array) in headers {
        if open && name.starts_with(&sub) {
            continue;
        }
        let record = array && name == table;
        open = record;
        starts.push((at, record));
    }
    starts.dedup_by(|b, a| {
        a.0 == b.0 && {
            a.1 |= b.1;
            true
        }
    });
    let mut records = Vec::new();
    let mut segments = Vec::new();
    for (i, (at, record)) in starts.iter().enumerate() {
        let end = starts.get(i + 1).map(|x| x.0).unwrap_or(text.len());
        let block = &text[*at..end];
        if *record {
            let v = items
                .get(records.len())
                .cloned()
                .ok_or(format!("more `[[{table}]]` headers than records parsed"))?;
            records.push((v, block.to_string()));
        } else {
            push_segment(&mut segments, records.len(), block);
        }
    }
    if records.len() != items.len() {
        return Err(format!(
            "{} `[[{table}]]` headers but {} records parsed: refusing a lossy split",
            records.len(),
            items.len()
        ));
    }
    Ok((
        table,
        Split {
            records,
            segments,
            head: doc,
        },
    ))
}

/// Byte spans of the elements of the record array of a JSON document (the top-level array, or
/// the array under `key` of the top-level object).
fn json_element_spans(text: &str, key: &str) -> Result<Vec<(usize, usize)>, String> {
    let b = text.as_bytes();
    let ws = |mut i: usize| {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        i
    };
    fn skip_string(b: &[u8], mut i: usize) -> usize {
        i += 1;
        while i < b.len() && b[i] != b'"' {
            if b[i] == b'\\' {
                i += 1;
            }
            i += 1;
        }
        i + 1
    }
    fn skip_value(b: &[u8], i: usize) -> usize {
        match b.get(i) {
            Some(b'"') => skip_string(b, i),
            Some(b'[') | Some(b'{') => {
                let mut depth = 0;
                let mut j = i;
                while j < b.len() {
                    match b[j] {
                        b'"' => {
                            j = skip_string(b, j);
                            continue;
                        }
                        b'[' | b'{' => depth += 1,
                        b']' | b'}' => {
                            depth -= 1;
                            if depth == 0 {
                                return j + 1;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                j
            }
            _ => {
                let mut j = i;
                while j < b.len()
                    && !matches!(b[j], b',' | b']' | b'}')
                    && !b[j].is_ascii_whitespace()
                {
                    j += 1;
                }
                j
            }
        }
    }
    let elements = |mut i: usize| -> Result<Vec<(usize, usize)>, String> {
        // `i` is at `[`.
        let mut out = Vec::new();
        i = ws(i + 1);
        if b.get(i) == Some(&b']') {
            return Ok(out);
        }
        loop {
            let end = skip_value(b, i);
            out.push((i, end));
            i = ws(end);
            match b.get(i) {
                Some(b',') => i = ws(i + 1),
                Some(b']') => return Ok(out),
                _ => return Err("malformed array".into()),
            }
        }
    };
    let mut i = ws(0);
    match b.get(i) {
        Some(b'[') if key.is_empty() => elements(i),
        Some(b'{') => {
            i = ws(i + 1);
            while b.get(i) == Some(&b'"') {
                let k_end = skip_string(b, i);
                let k = &text[i + 1..k_end - 1];
                i = ws(k_end);
                if b.get(i) != Some(&b':') {
                    return Err("malformed object".into());
                }
                i = ws(i + 1);
                if k == key && b.get(i) == Some(&b'[') {
                    return elements(i);
                }
                i = ws(skip_value(b, i));
                if b.get(i) == Some(&b',') {
                    i = ws(i + 1);
                }
            }
            Err(format!("no array `{key}`"))
        }
        _ => Err("not a JSON array or object".into()),
    }
}

fn split_json(text: &str, table: Option<&str>) -> Result<(String, Split), String> {
    let doc = json::parse(text)?;
    let key = match (table, &doc) {
        (Some(t), _) => t.to_string(),
        (None, Value::Array(_)) => String::new(),
        (None, Value::Table(t)) => t
            .iter()
            .filter(|(_, v)| v.items().iter().any(|x| x.table().is_some()))
            .max_by_key(|(k, v)| (RECORD_TABLES.contains(&k.as_str()), v.items().len()))
            .map(|(k, _)| k.clone())
            .ok_or("no array of records: pass --table <key>")?,
        _ => return Err("not a JSON array or object".into()),
    };
    let items: Vec<Value> = if key.is_empty() {
        doc.items().to_vec()
    } else {
        doc.items_of(&key).to_vec()
    };
    let spans = json_element_spans(text, &key)?;
    if spans.len() != items.len() {
        return Err(format!(
            "{} array elements found, {} parsed: refusing a lossy split",
            spans.len(),
            items.len()
        ));
    }
    let mut records = Vec::new();
    let mut segments = Vec::new();
    let mut at = 0;
    for (n, ((s, e), v)) in spans.into_iter().zip(items).enumerate() {
        push_segment(&mut segments, n, &text[at..s]);
        records.push((v, text[s..e].to_string()));
        at = e;
    }
    push_segment(&mut segments, records.len(), &text[at..]);
    Ok((
        key,
        Split {
            records,
            segments,
            head: doc,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_headers_ignore_brackets_inside_values_and_strings() {
        let t = "[registry]\nkind = \"papers\"\n\n[[paper]]\nid = \"R1\"\nnotes = \"\"\"\n[[paper]] not a header\n\"\"\"\nlist = [\n  [1, 2],\n  \"]\",\n]\n[paper.extra]\nx = 1\n\n[[paper]]\nid = \"R2\"\n";
        let h = toml_headers(t);
        let names: Vec<(&str, bool)> = h.iter().map(|(_, n, a)| (n.as_str(), *a)).collect();
        assert_eq!(
            names,
            vec![
                ("registry", false),
                ("paper", true),
                ("paper.extra", false),
                ("paper", true)
            ]
        );
        let (table, s) = split_toml(t, None).unwrap();
        assert_eq!(table, "paper");
        assert_eq!(s.records.len(), 2);
        assert!(s.records[0].1.contains("[paper.extra]"));
    }

    #[test]
    fn json_documents_split_by_element() {
        let t = "{\n  \"schema\": \"x\",\n  \"donors\": [\n    {\"id\": \"a\", \"x\": [1, {\"y\": \"]\"}]},\n    {\"id\": \"b\"}\n  ]\n}\n";
        let (key, s) = split_json(t, None).unwrap();
        assert_eq!(key, "donors");
        assert_eq!(s.records.len(), 2);
        assert_eq!(s.records[1].1, "{\"id\": \"b\"}");
        let mut out = String::new();
        let mut seg = s.segments.iter().peekable();
        for (i, (_, r)) in s.records.iter().enumerate() {
            while let Some((_, x)) = seg.next_if(|(p, _)| *p as usize <= i) {
                out.push_str(x);
            }
            out.push_str(r);
        }
        for (_, x) in seg {
            out.push_str(x);
        }
        assert_eq!(out, t);
    }

    #[test]
    fn kinds_are_read_from_legacy_words() {
        assert_eq!(kind_of_word("OSS_DONOR"), Some(UniverseKind::OssRepository));
        assert_eq!(
            kind_of_word("OSINT_SOURCE"),
            Some(UniverseKind::InformationSource)
        );
        assert_eq!(
            kind_of_word("BENCHMARK_REFERENCE"),
            Some(UniverseKind::Benchmark)
        );
        assert_eq!(
            kind_of_word("protocol-specs"),
            Some(UniverseKind::ProtocolSpec)
        );
        assert_eq!(
            kind_of_word("reference-architectures"),
            Some(UniverseKind::ReferenceArchitecture)
        );
        assert_eq!(
            kind_of_word("oss-universe"),
            Some(UniverseKind::OssRepository)
        );
        assert_eq!(kind_of_word("spec"), None);
    }
}
