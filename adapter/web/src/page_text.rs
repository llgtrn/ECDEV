//! The visible text of a page as lines, for monitoring pages that state no structured product
//! data. Script, style, template, noscript, svg and head content is not visible and is skipped;
//! block elements end a line; whitespace inside a line collapses to single spaces.

use scraper::{ElementRef, Html};
use sha2::{Digest, Sha256};

const HIDDEN: [&str; 7] = [
    "script", "style", "template", "noscript", "svg", "head", "iframe",
];
const BLOCK: [&str; 31] = [
    "address",
    "article",
    "aside",
    "blockquote",
    "br",
    "dd",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "li",
    "main",
    "nav",
    "p",
    "section",
    "table",
    "tr",
    "td",
    "th",
    "pre",
];

fn flush(current: &mut String, lines: &mut Vec<String>) {
    let line = current.split_whitespace().collect::<Vec<_>>().join(" ");
    if !line.is_empty() {
        lines.push(line);
    }
    current.clear();
}

fn walk(element: ElementRef, current: &mut String, lines: &mut Vec<String>, depth: usize) {
    let name = element.value().name();
    if HIDDEN.contains(&name) || depth > 512 {
        return;
    }
    let block = BLOCK.contains(&name);
    if block {
        flush(current, lines);
    }
    for child in element.children() {
        if let Some(e) = ElementRef::wrap(child) {
            walk(e, current, lines, depth + 1);
        } else if let Some(t) = child.value().as_text() {
            current.push(' ');
            current.push_str(t);
        }
    }
    if block {
        flush(current, lines);
    }
}

pub fn visible_lines(html: &str) -> Vec<String> {
    let doc = Html::parse_document(html);
    let mut lines = vec![];
    let mut current = String::new();
    walk(doc.root_element(), &mut current, &mut lines, 0);
    flush(&mut current, &mut lines);
    lines
}

/// Lines kept with a capture's text fingerprint, so changes can be shown, not just detected.
pub const MAX_KEPT_LINES: usize = 1_000;
pub const MAX_KEPT_BYTES: usize = 100 * 1024;

/// The fingerprint of the whole visible text, with its first lines up to the bounds.
pub fn summary(html: &str) -> serde_json::Value {
    let lines = visible_lines(html);
    let mut kept = vec![];
    let mut bytes = 0;
    for l in lines.iter().take(MAX_KEPT_LINES) {
        bytes += l.len() + 1;
        if bytes > MAX_KEPT_BYTES {
            break;
        }
        kept.push(l.clone());
    }
    serde_json::json!({"sha256":text_sha256(&lines),"line_count":lines.len(),"lines":kept,"truncated":kept.len() < lines.len(),"method":"VISIBLE_TEXT_LINES_V1"})
}

pub fn text_sha256(lines: &[String]) -> String {
    format!("{:x}", Sha256::digest(lines.join("\n").as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_visible_text_in_lines() {
        let html = r#"<html><head><title>T</title><style>p{}</style></head><body>
            <script>var x = Date.now();</script>
            <h1>Shipping   policy</h1><p>Orders ship in <b>2</b> days.</p>
            <ul><li>Free over 50</li><li>Returns 30 days</li></ul>
            <noscript>enable js</noscript><template><p>hidden</p></template>
            <div>Last <span>updated</span></div></body></html>"#;
        assert_eq!(
            visible_lines(html),
            [
                "Shipping policy",
                "Orders ship in 2 days.",
                "Free over 50",
                "Returns 30 days",
                "Last updated"
            ]
        );
        assert_eq!(
            text_sha256(&visible_lines(html)),
            text_sha256(&visible_lines(&html.replace("Date.now()", "0")))
        );
    }
}
