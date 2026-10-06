//! Decision-record status and supersession. A status is written many ways — `Status: Accepted`,
//! `**Status:** ACCEPTED (2026-09-27)`, `- Status: accepted`, `| Status | Accepted |`, a
//! `## Status` section, frontmatter — and often carries a relation to another decision
//! (`SUPERSEDED (2026-09-27) by 0010`, `Accepted. Supersedes 0004.`, `Superseded by
//! [ADR-0012](0012-x.md)`). This module reads one canonical status word and the referenced
//! decision ids out of such text.

/// Status words and their canonical spelling.
const STATUS_WORDS: &[(&str, &str)] = &[
    ("proposed", "Proposed"),
    ("draft", "Draft"),
    ("accepted", "Accepted"),
    ("approved", "Approved"),
    ("adopted", "Adopted"),
    ("rejected", "Rejected"),
    ("declined", "Declined"),
    ("deprecated", "Deprecated"),
    ("superseded", "Superseded"),
    ("superceded", "Superseded"),
    ("replaced", "Superseded"),
    ("withdrawn", "Withdrawn"),
    ("retired", "Retired"),
    ("obsolete", "Obsolete"),
    ("abandoned", "Abandoned"),
    ("amended", "Amended"),
    ("refined", "Refined"),
    ("implemented", "Implemented"),
    ("active", "Active"),
    ("pending", "Pending"),
    ("final", "Final"),
];

/// Relation keys of a decision, and the spellings that name them (as field terms, frontmatter
/// keys or section headings, lowercased with `_` and spaces as `-`).
const RELATIONS: &[(&str, &[&str])] = &[
    ("supersedes", &["supersedes", "supercedes", "replaces"]),
    (
        "superseded-by",
        &[
            "superseded-by",
            "superceded-by",
            "replaced-by",
            "supersededby",
        ],
    ),
    ("refines", &["refines"]),
    ("refined-by", &["refined-by"]),
    ("amends", &["amends"]),
    ("amended-by", &["amended-by"]),
];

/// Phrases in running text that name a relation: (words, relation). A phrase whose last word is
/// `by` may have a parenthetical between its words (`SUPERSEDED (2026-09-27) by 0010`).
const PHRASES: &[(&[&str], &str)] = &[
    (&["superseded", "by"], "superseded-by"),
    (&["superceded", "by"], "superseded-by"),
    (&["replaced", "by"], "superseded-by"),
    (&["refined", "by"], "refined-by"),
    (&["amended", "by"], "amended-by"),
    (&["supersedes"], "supersedes"),
    (&["supercedes"], "supersedes"),
    (&["replaces"], "supersedes"),
    (&["refines"], "refines"),
    (&["amends"], "amends"),
];

/// The relation key a field term, frontmatter key or section heading names.
pub fn relation(term: &str) -> Option<&'static str> {
    let t: String = term
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c == '_' || c == ' ' { '-' } else { c })
        .collect();
    RELATIONS
        .iter()
        .find(|(_, spellings)| spellings.contains(&t.as_str()))
        .map(|(k, _)| *k)
}

/// Text without Markdown markup: links become their text, struck text (`~~x~~`) is dropped,
/// emphasis and code marks go, whitespace is collapsed.
pub fn plain(text: &str) -> String {
    let mut s = String::new();
    let mut rest = text;
    while let Some(i) = rest.find("~~") {
        s.push_str(&rest[..i]);
        match rest[i + 2..].find("~~") {
            Some(j) => rest = &rest[i + 2 + j + 2..],
            None => {
                rest = &rest[i + 2..];
            }
        }
    }
    s.push_str(rest);
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' | '`' => {}
            '_' if out.ends_with(|p: char| p.is_whitespace()) || out.is_empty() => {}
            '_' if chars
                .peek()
                .is_none_or(|n| n.is_whitespace() || n.is_ascii_punctuation()) => {}
            ']' if chars.peek() == Some(&'(') => {
                // `[text](target)`: keep the text, drop the target.
                for t in chars.by_ref() {
                    if t == ')' {
                        break;
                    }
                }
            }
            '[' => {}
            _ => out.push(c),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The canonical status of a status text and, when the text says more than its status word,
/// the text itself (plain): `ACCEPTED (2026-09-27)` → (`Accepted`, Some(that text)).
pub fn status(raw: &str) -> Option<(String, Option<String>)> {
    let text = plain(raw);
    if text.is_empty() {
        return None;
    }
    let lower = text.to_lowercase();
    let word = lower
        .split(|c: char| !c.is_alphabetic())
        .filter(|w| !w.is_empty())
        .find_map(|w| STATUS_WORDS.iter().find(|(k, _)| *k == w).map(|(_, v)| *v));
    let status = match word {
        Some(w) => w.to_string(),
        None => {
            let first = text.split(['.', ';']).next().unwrap_or(&text).trim();
            if first.is_empty() {
                text.clone()
            } else {
                first.to_string()
            }
        }
    };
    let bare = crate::compact::facts::normalize(&text);
    let extra = (bare != status.to_lowercase()).then_some(text);
    Some((status, extra))
}

/// The id a token names: a link target's file stem, `ADR-0012` / `ADR0012` / `#12` as their
/// digits, or a run of three or more digits; `after_adr` admits shorter digit runs after the
/// word `ADR` (`ADR 12`).
fn id_of(token: &str, after_adr: bool) -> Option<String> {
    let t = token.trim_matches(|c: char| !c.is_alphanumeric());
    let lower = t.to_ascii_lowercase();
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if let Some(d) = lower
        .strip_prefix("adr-")
        .or_else(|| lower.strip_prefix("adr"))
        .filter(|d| digits(d))
    {
        return Some(d.to_string());
    }
    if let Some(d) = token
        .trim()
        .trim_start_matches(['(', '['])
        .strip_prefix('#')
    {
        let d = d.trim_end_matches(|c: char| !c.is_ascii_digit());
        if digits(d) {
            return Some(d.to_string());
        }
    }
    (digits(t) && (t.len() >= 3 || after_adr)).then(|| t.to_string())
}

/// Decision ids named in one clause: link targets to documents by their stem, other ids by
/// [`id_of`]; parenthesised text is skipped.
fn ids_in(clause: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |id: String| {
        if !out.contains(&id) {
            out.push(id);
        }
    };
    let mut depth = 0usize;
    let mut word = String::new();
    let mut prev_adr = false;
    let mut chars = clause.char_indices().peekable();
    let flush = |w: &mut String, prev_adr: &mut bool, push: &mut dyn FnMut(String)| {
        if !w.is_empty() {
            if let Some(id) = id_of(w, *prev_adr) {
                push(id);
            }
            *prev_adr = w
                .trim_matches(|c: char| !c.is_alphanumeric())
                .eq_ignore_ascii_case("adr");
            w.clear();
        }
    };
    while let Some((i, c)) = chars.next() {
        if c == ']' && clause[i + 1..].starts_with('(') {
            // A link: its target names the decision when it is a document.
            let end = clause[i + 2..].find(')').map(|j| i + 2 + j);
            if let Some(end) = end {
                let target = clause[i + 2..end].split('#').next().unwrap_or("");
                let name = target.rsplit('/').next().unwrap_or(target);
                if let Some(stem) = name.strip_suffix(".md").filter(|s| !s.is_empty()) {
                    word.clear();
                    push(stem.to_string());
                } else {
                    flush(&mut word, &mut prev_adr, &mut push);
                }
                while chars.peek().is_some_and(|(j, _)| *j <= end) {
                    chars.next();
                }
                continue;
            }
        }
        match c {
            '(' => {
                flush(&mut word, &mut prev_adr, &mut push);
                depth += 1;
            }
            ')' => depth = depth.saturating_sub(1),
            _ if depth > 0 => {}
            c if c.is_whitespace() || c == ',' || c == '[' => {
                flush(&mut word, &mut prev_adr, &mut push)
            }
            c => word.push(c),
        }
    }
    flush(&mut word, &mut prev_adr, &mut push);
    out
}

/// Where a clause ends: `;`, a sentence end (`.` before whitespace or the end), or a blank line.
fn clause_end(s: &str) -> usize {
    let b = s.as_bytes();
    for i in 0..b.len() {
        match b[i] {
            b';' => return i,
            b'.' if b.get(i + 1).is_none_or(|n| n.is_ascii_whitespace()) => return i,
            b'\n' if b.get(i + 1) == Some(&b'\n') => return i,
            _ => {}
        }
    }
    b.len()
}

/// Every relation `text` states, in order: (relation key, ids). The ids of one relation are the
/// decisions named in the clause after its phrase.
pub fn relations(text: &str) -> Vec<(&'static str, Vec<String>)> {
    let lower = text.to_ascii_lowercase();
    // Words with their byte ranges; parenthesised words are kept so they can be skipped.
    let mut words: Vec<(usize, usize, bool)> = Vec::new();
    let mut depth = 0usize;
    let mut start: Option<usize> = None;
    for (i, c) in lower.char_indices() {
        if c.is_ascii_alphabetic() {
            start.get_or_insert(i);
            continue;
        }
        if let Some(s) = start.take() {
            words.push((s, i, depth > 0));
        }
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    if let Some(s) = start {
        words.push((s, lower.len(), depth > 0));
    }
    let mut out: Vec<(&'static str, Vec<String>)> = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let (s, e, nested) = words[i];
        let mut matched = None;
        if !nested {
            for (phrase, rel) in PHRASES {
                if &lower[s..e] != phrase[0] {
                    continue;
                }
                match phrase.get(1) {
                    None => matched = Some((*rel, e)),
                    Some(next) => {
                        // The next word outside parentheses must be `by`.
                        let k = words[i + 1..].iter().take(8).find(|(_, _, nested)| !nested);
                        if let Some((s2, e2, _)) = k {
                            if &lower[*s2..*e2] == *next {
                                matched = Some((*rel, *e2));
                            }
                        }
                    }
                }
                if matched.is_some() {
                    break;
                }
            }
        }
        if let Some((rel, after)) = matched {
            let tail = &text[after..];
            let ids = ids_in(&tail[..clause_end(tail)]);
            if !ids.is_empty() {
                match out.iter_mut().find(|(r, _)| *r == rel) {
                    Some((_, v)) => {
                        for id in ids {
                            if !v.contains(&id) {
                                v.push(id);
                            }
                        }
                    }
                    None => out.push((rel, ids)),
                }
            }
            while i < words.len() && words[i].0 < after {
                i += 1;
            }
            continue;
        }
        i += 1;
    }
    out
}

/// Whether a block of a decision record states a relation of the record itself: it opens with a
/// relation phrase (`Superseded by …`, `Supersedes …`) or with `This ADR|decision|record …`
/// followed by one.
pub fn states_relation(body: &str) -> bool {
    let p = plain(body).to_ascii_lowercase();
    let p = ["this adr ", "this decision ", "this record ", "this "]
        .iter()
        .find_map(|x| p.strip_prefix(x))
        .unwrap_or(&p);
    PHRASES.iter().any(|(phrase, _)| p.starts_with(phrase[0]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_are_normalised_and_keep_their_text() {
        let s = |t: &str| status(t).unwrap();
        assert_eq!(s("Accepted"), ("Accepted".into(), None));
        assert_eq!(s("accepted."), ("Accepted".into(), None));
        assert_eq!(s("**ACCEPTED**"), ("Accepted".into(), None));
        assert_eq!(
            s("ACCEPTED (2026-09-27)"),
            ("Accepted".into(), Some("ACCEPTED (2026-09-27)".into()))
        );
        assert_eq!(
            s("SUPERSEDED (2026-09-27) by 0010 (two-language production architecture)").0,
            "Superseded"
        );
        assert_eq!(s("~~Proposed~~ Accepted").0, "Accepted");
        assert_eq!(s("In review; see thread").0, "In review");
        assert!(status("  ** ").is_none());
    }

    #[test]
    fn relations_name_the_referenced_decisions() {
        assert_eq!(
            relations("SUPERSEDED (2026-09-27) by 0010 (two-language production architecture)"),
            vec![("superseded-by", vec!["0010".to_string()])]
        );
        assert_eq!(
            relations("ACCEPTED (2026-09-27). Supersedes 0004."),
            vec![("supersedes", vec!["0004".to_string()])]
        );
        assert_eq!(
            relations("Superseded by [ADR-0012](../decisions/0012-use-x.md#status)."),
            vec![("superseded-by", vec!["0012-use-x".to_string()])]
        );
        assert_eq!(
            relations("ACCEPTED — GENERATION-1 HISTORY; ACTIVE FLEET SUPERSEDED BY ADR-0027"),
            vec![("superseded-by", vec!["0027".to_string()])]
        );
        assert_eq!(
            relations("ACCEPTED (2026-09-27); REFINED by 0015 (core/, mcp/)"),
            vec![("refined-by", vec!["0015".to_string()])]
        );
        assert_eq!(
            relations(
                "Accepted. Supersedes, in ADR 0010 and ADR 0015, the rule that apps/ holds code."
            ),
            vec![("supersedes", vec!["0010".to_string(), "0015".to_string()])]
        );
        // Not a decision id: dates, invariant names, plain words.
        assert!(
            relations("Supersedes the outbound-licence rule of INV-D4 and the rest.").is_empty()
        );
        assert!(relations("Accepted on 2026-09-27").is_empty());
        assert_eq!(relation("Superseded_by"), Some("superseded-by"));
        assert_eq!(relation("superseded by"), Some("superseded-by"));
        assert_eq!(relation("context"), None);
        assert!(states_relation("**Superseded by** ADR 12"));
        assert!(states_relation("This ADR supersedes ADR-0018."));
        assert!(!states_relation("ADR-0027 supersedes the allowance."));
    }
}
