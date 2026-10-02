//! Verbatim document outlines. A document is split into spans that partition its bytes: each
//! span is one Markdown block (a heading with its level, a paragraph, a list, a fenced or
//! indented code block, a table, a quote, a rule, frontmatter) together with the blank lines
//! that follow it. Concatenating the spans in order gives back the document byte for byte, so a
//! document's BLOCK facts rebuild it exactly, and their tags give its structure.

/// One block of a document: its structural tag, 1-based line range and verbatim text
/// (line endings and trailing blank lines included).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub tag: String,
    pub first: usize,
    pub last: usize,
    pub raw: String,
}

/// The tag of a plain-text (non-Markdown) document stored as one verbatim span.
pub const TEXT_TAG: &str = "text";

/// The key of the `i`-th block of a document: zero-padded, so keys sort in document order.
pub fn block_key(i: usize) -> String {
    format!("{i:05}")
}

/// A BLOCK fact's value: the tag on the first line, then the verbatim text.
pub fn encode_block(tag: &str, raw: &str) -> String {
    format!("{tag}\n{raw}")
}

/// (tag, verbatim text) of a BLOCK fact's value.
pub fn decode_block(value: &str) -> (&str, &str) {
    value.split_once('\n').unwrap_or((value, ""))
}

fn indent(line: &str) -> usize {
    let mut n = 0;
    for c in line.chars() {
        match c {
            ' ' => n += 1,
            '\t' => n += 4,
            _ => break,
        }
    }
    n
}

fn blank(line: &str) -> bool {
    line.trim().is_empty()
}

/// `h1`…`h6` for an ATX heading line.
fn atx(line: &str) -> Option<usize> {
    if indent(line) > 3 {
        return None;
    }
    let t = line.trim_start();
    let level = t.chars().take_while(|c| *c == '#').count();
    let rest = &t[level..];
    ((1..=6).contains(&level) && (rest.trim().is_empty() || rest.starts_with([' ', '\t'])))
        .then_some(level)
}

/// The fence that opens a fenced code block: (fence character, run length).
fn fence(line: &str) -> Option<(char, usize)> {
    let t = line.trim_start();
    let c = t.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let n = t.chars().take_while(|x| *x == c).count();
    (n >= 3).then_some((c, n))
}

fn closes(line: &str, (c, n): (char, usize)) -> bool {
    let t = line.trim();
    t.chars().take_while(|x| *x == c).count() >= n && t.chars().all(|x| x == c)
}

/// A thematic break: three or more of one of `-`, `*`, `_`, spaces allowed.
fn rule(line: &str) -> bool {
    let t: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    indent(line) <= 3 && t.len() >= 3 && ['-', '*', '_'].iter().any(|m| t.chars().all(|c| c == *m))
}

/// A list item marker: `ul` for `-`, `*`, `+`; `ol` for `1.` / `1)`.
fn item(line: &str) -> Option<&'static str> {
    let t = line.trim_start();
    let rest_ok = |r: &str| r.is_empty() || r.starts_with([' ', '\t']);
    if let Some(r) = t.strip_prefix(['-', '*', '+']) {
        if rest_ok(r.trim_end_matches(['\r', '\n'])) {
            return Some("ul");
        }
    }
    let digits = t.chars().take_while(char::is_ascii_digit).count();
    if (1..=9).contains(&digits) {
        if let Some(r) = t[digits..].strip_prefix(['.', ')']) {
            if rest_ok(r.trim_end_matches(['\r', '\n'])) {
                return Some("ol");
            }
        }
    }
    None
}

fn setext(line: &str) -> Option<usize> {
    let t = line.trim();
    if indent(line) > 3 || t.is_empty() {
        return None;
    }
    if t.chars().all(|c| c == '=') {
        Some(1)
    } else if t.chars().all(|c| c == '-') {
        Some(2)
    } else {
        None
    }
}

/// Whether `line` starts a block that interrupts a paragraph.
fn interrupts(line: &str) -> bool {
    atx(line).is_some()
        || fence(line).is_some()
        || line.trim_start().starts_with('>')
        || (indent(line) <= 3 && item(line).is_some())
        || (rule(line) && !line.trim().chars().all(|c| c == '-'))
}

/// Splits Markdown into spans that partition it exactly; see the module documentation.
pub fn outline(text: &str) -> Vec<Span> {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut spans: Vec<(String, usize, usize)> = Vec::new(); // (tag, first, last) 0-based
    let mut i = 0;
    if let Some((end, _)) = crate::audit::frontmatter(text) {
        spans.push(("frontmatter".into(), 0, end - 1));
        i = end;
    }
    let n = lines.len();
    while i < n {
        let line = lines[i];
        if blank(line) {
            match spans.last_mut() {
                Some(s) => s.2 = i,
                None => spans.push(("blank".into(), i, i)),
            }
            i += 1;
            continue;
        }
        let start = i;
        let tag: String;
        if let Some(f) = fence(line).filter(|_| indent(line) <= 3) {
            i += 1;
            while i < n && !closes(lines[i], f) {
                i += 1;
            }
            i = (i + 1).min(n);
            tag = "code".into();
        } else if let Some(level) = atx(line) {
            i += 1;
            tag = format!("h{level}");
        } else if indent(line) >= 4 {
            // Indented code: until a non-blank line indented less, blank lines inside kept.
            i += 1;
            while i < n {
                if blank(lines[i]) {
                    let next = (i..n).find(|j| !blank(lines[*j]));
                    if next.is_some_and(|j| indent(lines[j]) >= 4) {
                        i += 1;
                        continue;
                    }
                    break;
                }
                if indent(lines[i]) < 4 {
                    break;
                }
                i += 1;
            }
            tag = "code".into();
        } else if line.trim_start().starts_with('|') {
            i += 1;
            while i < n && lines[i].trim_start().starts_with('|') {
                i += 1;
            }
            tag = "table".into();
        } else if line.trim_start().starts_with('>') {
            i += 1;
            while i < n
                && !blank(lines[i])
                && (lines[i].trim_start().starts_with('>') || !interrupts(lines[i]))
            {
                i += 1;
            }
            tag = "quote".into();
        } else if let Some(kind) = item(line) {
            // A list: its items, their continuation and nested lines, and blank lines between
            // items (a loose list), as long as what follows a blank line still belongs to it.
            let base = indent(line);
            i += 1;
            let mut code: Option<(char, usize)> = None;
            while i < n {
                let l = lines[i];
                if let Some(f) = code {
                    if closes(l, f) {
                        code = None;
                    }
                    i += 1;
                    continue;
                }
                if blank(l) {
                    let next = (i..n).find(|j| !blank(lines[*j]));
                    match next {
                        Some(j)
                            if indent(lines[j]) > base
                                || indent(lines[j]) == base && item(lines[j]) == Some(kind) =>
                        {
                            i += 1;
                            continue;
                        }
                        _ => break,
                    }
                }
                if indent(l) <= base && item(l).is_some_and(|k| k != kind) {
                    break;
                }
                if indent(l) <= base && item(l).is_none() && interrupts(l) {
                    break;
                }
                if indent(l) < base {
                    break;
                }
                if let Some(f) = fence(l) {
                    code = Some(f);
                }
                i += 1;
            }
            tag = kind.into();
        } else if rule(line) {
            i += 1;
            tag = "hr".into();
        } else {
            // A paragraph, or a setext heading when its last line is underlined.
            i += 1;
            let mut t = "p".to_string();
            while i < n && !blank(lines[i]) {
                if let Some(level) = setext(lines[i]) {
                    i += 1;
                    t = format!("h{level}");
                    break;
                }
                if interrupts(lines[i]) || lines[i].trim_start().starts_with('|') {
                    break;
                }
                i += 1;
            }
            tag = t;
        }
        spans.push((tag, start, i - 1));
    }
    let mut out: Vec<Span> = spans
        .into_iter()
        .map(|(tag, a, b)| Span {
            tag,
            first: a + 1,
            last: b + 1,
            raw: lines[a..=b].concat(),
        })
        .collect();
    if out.is_empty() {
        out.push(Span {
            tag: "empty".into(),
            first: 1,
            last: 1,
            raw: String::new(),
        });
    }
    // The spans partition the text by construction; never store an outline that does not.
    if out.iter().map(|s| s.raw.as_str()).collect::<String>() != text {
        return vec![verbatim(text)];
    }
    out
}

/// A whole text as one verbatim span.
pub fn verbatim(text: &str) -> Span {
    Span {
        tag: TEXT_TAG.into(),
        first: 1,
        last: text.split_inclusive('\n').count().max(1),
        raw: text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(text: &str) -> Vec<String> {
        outline(text).into_iter().map(|s| s.tag).collect()
    }

    #[test]
    fn spans_partition_the_text_and_name_its_structure() {
        let text = "---\ntitle: x\n---\n\n# One\n\nA paragraph\nthat wraps.\n\n## Two\n\n- a\n  - nested\n\n- loose\n\n1. first\n2. second\n\n```rust\nfn x() {\n\n}\n```\n\n    indented code\n\n> quoted\n> more\n\n| a | b |\n|---|---|\n\n---\n\nSetext\n======\n\n### Three ###\nlast line without newline";
        let spans = outline(text);
        assert_eq!(
            spans.iter().map(|s| s.raw.as_str()).collect::<String>(),
            text
        );
        assert_eq!(
            tags(text),
            [
                "frontmatter",
                "h1",
                "p",
                "h2",
                "ul",
                "ol",
                "code",
                "code",
                "quote",
                "table",
                "hr",
                "h1",
                "h3",
                "p"
            ]
        );
        let h2 = spans.iter().find(|s| s.tag == "h2").unwrap();
        assert_eq!((h2.first, h2.last), (10, 11));
        assert_eq!(h2.raw, "## Two\n\n");
    }

    #[test]
    fn crlf_leading_blanks_and_empty_text_round_trip() {
        for text in [
            "",
            "\n\n",
            "\n\n# T\r\n\r\nbody\r\n",
            "no newline",
            "#hashtag is a paragraph\n",
        ] {
            let s = outline(text);
            assert_eq!(s.iter().map(|s| s.raw.as_str()).collect::<String>(), text);
        }
        assert_eq!(tags("#hashtag is a paragraph\n"), ["p"]);
        assert_eq!(tags(""), ["empty"]);
        let v = encode_block("h2", "## X\n\n");
        let (tag, raw) = decode_block(&v);
        assert_eq!((tag, raw), ("h2", "## X\n\n"));
    }
}
