//! Generated views of the knowledge store. Rendering reads facts and returns text; it never
//! writes anything, so a view can be produced at any time without changing state. Markdown
//! produced here is a view, never a source of truth.

use super::facts::{Fact, Knowledge};
use crate::schema::FactKind;
use std::collections::BTreeMap;

/// Fields shown first, in this order; other keys follow alphabetically.
const LEADING_KEYS: &[&str] = &["title", "status", "rank", "scope", "evidence"];

fn key_order(key: &str) -> (usize, &str) {
    (
        LEADING_KEYS
            .iter()
            .position(|k| *k == key)
            .unwrap_or(LEADING_KEYS.len()),
        key,
    )
}

fn one_line(v: &str) -> String {
    v.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// At most `max` characters of `v` on one line, `…` marking a cut.
pub fn clip(v: &str, max: usize) -> String {
    let v = one_line(v);
    if v.chars().count() <= max {
        v
    } else {
        format!("{}…", v.chars().take(max).collect::<String>())
    }
}

/// Where a fact comes from, compactly, as a trailing ` — <provenance>`: its first provenance
/// and how many more it has. A `git:<sha>:<path>…` provenance twinned with the same
/// `doc:<path>…` one is not counted again.
pub fn provenance_brief(f: &Fact) -> String {
    let twin = |p: &str| {
        p.strip_prefix("git:")
            .and_then(|r| r.split_once(':'))
            .is_some_and(|(_, rest)| f.provenance.contains(&format!("doc:{rest}")))
    };
    let ps: Vec<&String> = f.provenance.iter().filter(|p| !twin(p)).collect();
    match ps.split_first() {
        None => String::new(),
        Some((first, [])) => format!(" — {first}"),
        Some((first, rest)) => format!(" — {first} (+{} more)", rest.len()),
    }
}

/// Current facts of one kind grouped by subject, subjects in roadmap order for MILESTONE
/// (milestones before gaps, then numeric `rank`, then subject) and by subject otherwise; the
/// keys of a subject in [`LEADING_KEYS`] order.
pub fn by_subject(k: &Knowledge, kind: FactKind) -> Vec<(String, Vec<&Fact>)> {
    let mut groups: BTreeMap<&str, Vec<&Fact>> = BTreeMap::new();
    for f in k.facts.iter().filter(|f| f.kind == kind) {
        groups.entry(f.subject.as_str()).or_default().push(f);
    }
    let mut out: Vec<(String, Vec<&Fact>)> = groups
        .into_iter()
        .map(|(s, mut fs)| {
            fs.sort_by(|a, b| key_order(&a.key).cmp(&key_order(&b.key)));
            (s.to_string(), fs)
        })
        .collect();
    if kind == FactKind::Milestone {
        let rank = |fs: &[&Fact]| {
            fs.iter()
                .find(|f| f.key == "rank")
                .and_then(|f| f.value.trim().parse::<u64>().ok())
                .unwrap_or(u64::MAX)
        };
        out.sort_by(|(a, fa), (b, fb)| {
            (a.starts_with("gap/"), rank(fa), a).cmp(&(b.starts_with("gap/"), rank(fb), b))
        });
    }
    out
}

/// Keys shown per subject in [`summary`]; the rest are counted.
pub const SUMMARY_FIELDS: usize = 3;

/// The compact knowledge section of `status` and `context`: counts per kind and the current
/// MILESTONE and DECISION facts, at most `limit` subjects per kind, values clipped.
pub fn summary(k: &Knowledge, limit: usize) -> String {
    let superseded: usize = k.facts.iter().map(|f| f.superseded.len()).sum();
    let mut s = format!(
        "KNOWLEDGE ({} current facts, {} superseded values; `ecdev-gov fact list`, `ecdev-gov knowledge view`)\n",
        k.facts.len(),
        superseded
    );
    if k.facts.is_empty() {
        s.push_str("  (none: `ecdev-gov fact add <kind> <subject> <key> <value> --provenance <p>` or `ecdev-gov knowledge extract <doc>`)\n");
        return s;
    }
    s.push_str(&format!(
        "  {}\n",
        k.counts()
            .iter()
            .map(|(kind, n)| format!("{kind} {n}"))
            .collect::<Vec<_>>()
            .join(" · ")
    ));
    for (kind, title) in [
        (FactKind::Milestone, "MILESTONES"),
        (FactKind::Decision, "DECISIONS"),
    ] {
        let groups = by_subject(k, kind);
        s.push_str(&format!("  {title} ({})\n", groups.len()));
        if groups.is_empty() {
            s.push_str(&format!(
                "    (none: `ecdev-gov fact add {kind} <subject> <key> <value> --provenance <p>`)\n"
            ));
        }
        for (subject, fs) in groups.iter().take(limit) {
            let mut fields: Vec<String> = fs
                .iter()
                .take(SUMMARY_FIELDS)
                .map(|f| format!("{}={}", f.key, clip(&f.value, 48)))
                .collect();
            if fs.len() > SUMMARY_FIELDS {
                fields.push(format!("(+{} keys)", fs.len() - SUMMARY_FIELDS));
            }
            s.push_str(&format!("    {:<32} {}\n", subject, fields.join("  ")));
        }
        if groups.len() > limit {
            s.push_str(&format!(
                "    … {} more (ecdev-gov fact list --kind {kind})\n",
                groups.len() - limit
            ));
        }
    }
    s
}

/// Kinds shown only when asked for by kind: DOCUMENT digests are bookkeeping, and BLOCK facts
/// are documents' verbatim text, viewed per document (`knowledge view --document <path>`).
fn shown_by_default(kind: FactKind) -> bool {
    !matches!(kind, FactKind::Document | FactKind::Block)
}

/// Current facts as a generated Markdown view, optionally of one kind. DOCUMENT digests and
/// BLOCK text are shown only when asked for by kind.
pub fn markdown(k: &Knowledge, kind: Option<FactKind>) -> String {
    let mut s = String::from("<!-- GENERATED by `ecdev-gov knowledge view`: a view, not a source of truth. Do not edit or commit. -->\n# Knowledge\n");
    for kd in FactKind::ALL
        .iter()
        .copied()
        .filter(|kd| kind.map_or(shown_by_default(*kd), |k| k == *kd))
    {
        let groups = by_subject(k, kd);
        if groups.is_empty() {
            continue;
        }
        s.push_str(&format!("\n## {kd}\n\n{}\n", kd.meaning()));
        for (subject, fs) in groups {
            if !subject.is_empty() {
                s.push_str(&format!("\n### {subject}\n\n"));
            } else {
                s.push('\n');
            }
            for f in fs {
                let label = if f.kind == FactKind::Statement {
                    String::new()
                } else {
                    format!("**{}**: ", f.key)
                };
                s.push_str(&format!(
                    "- {label}{}{}{}\n",
                    one_line(&f.value),
                    if f.superseded.is_empty() {
                        String::new()
                    } else {
                        format!(" _(supersedes {})_", f.superseded.len())
                    },
                    provenance_brief(f)
                ));
            }
        }
    }
    s
}

/// Current facts as plain text, one per line: `KIND subject key = value`.
pub fn text(k: &Knowledge, kind: Option<FactKind>) -> String {
    let mut s = String::new();
    for f in k.current(kind, None) {
        if kind.is_none() && !shown_by_default(f.kind) {
            continue;
        }
        s.push_str(&line(f));
    }
    s
}

/// One fact on one line, with its provenance and supersession counts and where it comes from.
pub fn line(f: &Fact) -> String {
    format!(
        "{} {} {} = {}  [seq {}, {} provenance{}]{}\n",
        f.kind,
        if f.subject.is_empty() {
            "-"
        } else {
            &f.subject
        },
        f.key,
        one_line(&f.value),
        f.seq,
        f.provenance.len(),
        if f.superseded.is_empty() {
            String::new()
        } else {
            format!(", supersedes {}", f.superseded.len())
        },
        provenance_brief(f)
    )
}

/// Why knowledge holds no verbatim text of `path`.
fn not_stored(k: &Knowledge, path: &str) -> String {
    if k.document_digest(path).is_some() {
        format!(
            "{path} was extracted before documents were stored verbatim (its digest, not its \
             text): extract it again with `ecdev-gov knowledge extract {path}` while the file \
             exists, or restore it from git history"
        )
    } else if k.get(FactKind::LegacyRecord, path, "imported").is_some() {
        format!(
            "{path} was imported line by line before legacy files were stored verbatim: import \
             it again with `ecdev-gov compact --extract-legacy` while the file exists"
        )
    } else {
        format!("no stored text of {path} (`ecdev-gov knowledge extract {path}`)")
    }
}

/// The document or legacy file `path` rebuilt from its facts, byte for byte, proven against
/// its recorded digest.
pub fn document(k: &Knowledge, path: &str) -> Result<String, String> {
    match k.stored_text(path) {
        Some(Ok(t)) => Ok(t.clone()),
        Some(Err(e)) => Err(e.clone()),
        None => Err(not_stored(k, path)),
    }
}

/// The structure of a stored document: one line per block, `<key> <tag> L<a>-L<b> <first
/// line>`.
pub fn outline(k: &Knowledge, path: &str) -> Result<String, String> {
    document(k, path)?;
    let digest = k.document_digest(path);
    let seq = k
        .get(FactKind::Document, path, "digest")
        .or_else(|| k.get(FactKind::LegacyRecord, path, "imported"))
        .map(|f| f.seq);
    let mut blocks: Vec<&Fact> = k
        .facts
        .iter()
        .filter(|f| f.kind == FactKind::Block && f.subject == path && Some(f.seq) == seq)
        .collect();
    blocks.sort_by(|a, b| a.key.cmp(&b.key));
    let mut s = format!(
        "{path} {}\n",
        digest.unwrap_or("(legacy content: one verbatim block)")
    );
    for b in blocks {
        let (tag, raw) = super::outline::decode_block(&b.value);
        let lines = b
            .provenance
            .iter()
            .find_map(|p| p.strip_prefix(&format!("doc:{path}#")))
            .unwrap_or("");
        s.push_str(&format!(
            "{} {:<11} {:<11} {}\n",
            b.key,
            tag,
            lines,
            clip(raw.lines().next().unwrap_or(""), 60)
        ));
    }
    Ok(s)
}
