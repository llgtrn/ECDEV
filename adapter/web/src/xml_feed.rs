//! Bounded reading of XML syndication feeds (RSS 2.0, RSS 1.0/RDF, Atom 1.0) without an XML
//! library. The reader is deliberately strict and small: UTF-8 only, no DTD (a DOCTYPE or
//! ENTITY declaration is refused, so no external or expanding entities exist), the five
//! predefined entities and numeric character references only, matched end tags, a depth and
//! node limit. A feed it cannot read is refused with a reason, never guessed.

use serde_json::{Value, json};

const MAX_DEPTH: usize = 64;
const MAX_NODES: usize = 200_000;
pub const MAX_ITEMS: usize = 1000;

#[derive(Debug, Default)]
pub struct Node {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
    pub text: String,
}

impl Node {
    /// Local name: the part after a namespace prefix.
    pub fn local(&self) -> &str {
        self.name.rsplit(':').next().unwrap_or(&self.name)
    }
    fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }
    fn children_named<'a>(&'a self, local: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.local() == local)
    }
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
    fn text_of(&self, name: &str) -> Option<String> {
        self.child(name)
            .map(|c| c.text.trim().to_string())
            .filter(|t| !t.is_empty())
    }
}

fn decode_entities(s: &str) -> Result<String, String> {
    if !s.contains('&') {
        return Ok(s.to_string());
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i + 1..];
        let end = tail
            .find(';')
            .filter(|e| *e <= 12)
            .ok_or("XML_MALFORMED_ENTITY")?;
        let name = &tail[..end];
        let c = match name {
            "lt" => '<',
            "gt" => '>',
            "amp" => '&',
            "quot" => '"',
            "apos" => '\'',
            _ => {
                let code = if let Some(h) = name.strip_prefix("#x").or(name.strip_prefix("#X")) {
                    u32::from_str_radix(h, 16).ok()
                } else if let Some(d) = name.strip_prefix('#') {
                    d.parse().ok()
                } else {
                    return Err("XML_UNDECLARED_ENTITY".into());
                };
                code.and_then(char::from_u32)
                    .ok_or("XML_INVALID_CHARACTER_REFERENCE")?
            }
        };
        out.push(c);
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, ':' | '_' | '-' | '.')
}

/// Parses an XML document into its root element.
pub fn parse(bytes: &[u8]) -> Result<Node, String> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let doc = std::str::from_utf8(bytes).map_err(|_| "XML_ENCODING_UNSUPPORTED_NOT_UTF8")?;
    if let Some(decl) = doc.trim_start().strip_prefix("<?xml") {
        let decl = &decl[..decl.find("?>").ok_or("XML_MALFORMED_DECLARATION")?];
        if let Some(i) = decl.find("encoding") {
            let enc = decl[i + 8..]
                .trim_start_matches([' ', '='])
                .trim_start_matches(['"', '\''])
                .split(['"', '\''])
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            if !matches!(enc.as_str(), "utf-8" | "utf8" | "us-ascii" | "ascii") {
                return Err("XML_ENCODING_UNSUPPORTED_NOT_UTF8".into());
            }
        }
    }
    let mut stack: Vec<Node> = vec![];
    let mut root: Option<Node> = None;
    let mut nodes = 0usize;
    let mut i = 0usize;
    let b = doc.as_bytes();
    while i < b.len() {
        if b[i] != b'<' {
            let end = doc[i..].find('<').map_or(b.len(), |e| i + e);
            let text = &doc[i..end];
            match stack.last_mut() {
                Some(top) => top.text.push_str(&decode_entities(text)?),
                None if text.trim().is_empty() => {}
                None => return Err("XML_TEXT_OUTSIDE_ROOT".into()),
            }
            i = end;
            continue;
        }
        let rest = &doc[i..];
        if rest.starts_with("<!--") {
            i += rest.find("-->").ok_or("XML_UNTERMINATED_COMMENT")? + 3;
        } else if rest.starts_with("<![CDATA[") {
            let end = rest.find("]]>").ok_or("XML_UNTERMINATED_CDATA")?;
            stack
                .last_mut()
                .ok_or("XML_CDATA_OUTSIDE_ROOT")?
                .text
                .push_str(&rest[9..end]);
            i += end + 3;
        } else if rest.starts_with("<!") {
            // DOCTYPE and ENTITY declarations: refused, never expanded.
            return Err("XML_DTD_REFUSED".into());
        } else if rest.starts_with("<?") {
            i += rest
                .find("?>")
                .ok_or("XML_UNTERMINATED_PROCESSING_INSTRUCTION")?
                + 2;
        } else if let Some(close) = rest.strip_prefix("</") {
            let end = close.find('>').ok_or("XML_UNTERMINATED_TAG")?;
            let name = close[..end].trim();
            let node = stack.pop().ok_or("XML_UNEXPECTED_END_TAG")?;
            if node.name != name {
                return Err("XML_MISMATCHED_END_TAG".into());
            }
            match stack.last_mut() {
                Some(parent) => parent.children.push(node),
                None if root.is_none() => root = Some(node),
                None => return Err("XML_MULTIPLE_ROOTS".into()),
            }
            i += 2 + end + 1;
        } else {
            // Start tag: name, attributes, optional self-closing slash.
            let inner = &rest[1..];
            let name_end = inner
                .find(|c: char| !is_name_char(c))
                .ok_or("XML_UNTERMINATED_TAG")?;
            let name = &inner[..name_end];
            if name.is_empty() {
                return Err("XML_MALFORMED_TAG".into());
            }
            let mut j = name_end;
            let mut attrs = vec![];
            let self_closing = loop {
                let t = inner[j..].trim_start();
                j = inner.len() - t.len();
                if let Some(t2) = t.strip_prefix("/>") {
                    let _ = t2;
                    j += 2;
                    break true;
                }
                if t.starts_with('>') {
                    j += 1;
                    break false;
                }
                let key_end = t
                    .find(|c: char| !is_name_char(c))
                    .ok_or("XML_UNTERMINATED_TAG")?;
                if key_end == 0 {
                    return Err("XML_MALFORMED_ATTRIBUTE".into());
                }
                let key = &t[..key_end];
                let after = t[key_end..].trim_start();
                let after = after
                    .strip_prefix('=')
                    .ok_or("XML_MALFORMED_ATTRIBUTE")?
                    .trim_start();
                let quote = after
                    .chars()
                    .next()
                    .filter(|q| *q == '"' || *q == '\'')
                    .ok_or("XML_MALFORMED_ATTRIBUTE")?;
                let body = &after[1..];
                let vend = body.find(quote).ok_or("XML_UNTERMINATED_ATTRIBUTE")?;
                if body[..vend].contains('<') {
                    return Err("XML_MALFORMED_ATTRIBUTE".into());
                }
                attrs.push((key.to_string(), decode_entities(&body[..vend])?));
                j = inner.len() - body[vend + 1..].len();
            };
            nodes += 1;
            if nodes > MAX_NODES || stack.len() >= MAX_DEPTH {
                return Err("XML_LIMIT_EXCEEDED".into());
            }
            let node = Node {
                name: name.to_string(),
                attrs,
                ..Node::default()
            };
            if self_closing {
                match stack.last_mut() {
                    Some(parent) => parent.children.push(node),
                    None if root.is_none() => root = Some(node),
                    None => return Err("XML_MULTIPLE_ROOTS".into()),
                }
            } else {
                if stack.is_empty() && root.is_some() {
                    return Err("XML_MULTIPLE_ROOTS".into());
                }
                stack.push(node);
            }
            i += 1 + j;
        }
    }
    if !stack.is_empty() {
        return Err("XML_UNCLOSED_ELEMENT".into());
    }
    root.ok_or_else(|| "XML_NO_ROOT".into())
}

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

/// RFC 822/1123 dates as RSS 2.0 writes them ("Tue, 07 Oct 2026 12:00:00 GMT", "+0900"),
/// rewritten as RFC 3339; None when not that form.
pub fn rfc822_to_rfc3339(s: &str) -> Option<String> {
    let s = s.trim();
    let s = s.split_once(',').map_or(s, |(_, r)| r.trim());
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() != 5 {
        return None;
    }
    let day: u32 = parts[0].parse().ok().filter(|d| (1..=31).contains(d))?;
    let month = MONTHS
        .iter()
        .position(|m| parts[1].len() >= 3 && parts[1][..3].eq_ignore_ascii_case(m))?
        + 1;
    let mut year: u32 = parts[2].parse().ok()?;
    if parts[2].len() == 2 {
        year += if year < 50 { 2000 } else { 1900 };
    }
    let hms: Vec<&str> = parts[3].split(':').collect();
    if !(2..=3).contains(&hms.len())
        || hms
            .iter()
            .any(|p| p.len() != 2 || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let sec = hms.get(2).copied().unwrap_or("00");
    let zone = match parts[4].to_ascii_uppercase().as_str() {
        "GMT" | "UT" | "UTC" | "Z" => "+00:00".to_string(),
        "EST" => "-05:00".into(),
        "EDT" => "-04:00".into(),
        "CST" => "-06:00".into(),
        "CDT" => "-05:00".into(),
        "MST" => "-07:00".into(),
        "MDT" => "-06:00".into(),
        "PST" => "-08:00".into(),
        "PDT" => "-07:00".into(),
        z if z.len() == 5
            && (z.starts_with('+') || z.starts_with('-'))
            && z[1..].bytes().all(|b| b.is_ascii_digit()) =>
        {
            format!("{}:{}", &z[..3], &z[3..])
        }
        _ => return None,
    };
    Some(format!(
        "{year:04}-{month:02}-{day:02}T{}:{}:{sec}{zone}",
        hms[0], hms[1]
    ))
}

/// A feed document as the JSON Feed item shape ECDEV already normalizes: id, url, title,
/// content_html, date_published (RFC 3339) and authors. The source's own form is kept in
/// `xml_kind`; dates that are neither RFC 822 nor RFC 3339 are left unknown and counted.
pub fn to_feed_items(root: &Node) -> Result<Value, String> {
    let (kind, title, entries): (&str, Option<String>, Vec<&Node>) = match root.local() {
        "rss" => {
            let channel = root
                .children_named("channel")
                .next()
                .ok_or("RSS_CHANNEL_REQUIRED")?;
            (
                "RSS_2_0",
                channel.text_of("title"),
                channel.children_named("item").collect(),
            )
        }
        "RDF" => (
            "RSS_1_0",
            root.children_named("channel")
                .next()
                .and_then(|c| c.text_of("title")),
            root.children_named("item").collect(),
        ),
        "feed" => (
            "ATOM_1_0",
            root.text_of("title"),
            root.children_named("entry").collect(),
        ),
        _ => return Err("XML_FEED_ROOT_UNSUPPORTED".into()),
    };
    if entries.len() > MAX_ITEMS {
        return Err("SOCIAL_ITEM_LIMIT_EXCEEDED".into());
    }
    let mut undated = 0;
    let items: Vec<Value> = entries
        .iter()
        .map(|e| {
            let (id, url, date, content, author) = match kind {
                "ATOM_1_0" => {
                    let link = e
                        .children_named("link")
                        .find(|l| matches!(l.attr("rel"), None | Some("alternate")))
                        .and_then(|l| l.attr("href"))
                        .map(str::to_string);
                    (
                        e.text_of("id"),
                        link,
                        e.text_of("published").or(e.text_of("updated")),
                        e.text_of("content").or(e.text_of("summary")),
                        e.children_named("author").next().and_then(|a| a.text_of("name")),
                    )
                }
                _ => {
                    let link = e.text_of("link");
                    let id = e.text_of("guid").or(e.attr("rdf:about").map(str::to_string)).or(link.clone());
                    let date = e
                        .text_of("pubDate")
                        .and_then(|d| rfc822_to_rfc3339(&d))
                        .or(e.text_of("dc:date"));
                    (
                        id,
                        link,
                        date,
                        e.text_of("content:encoded").or(e.text_of("description")),
                        e.text_of("author").or(e.text_of("dc:creator")),
                    )
                }
            };
            if date.is_none() {
                undated += 1;
            }
            json!({"id":id,"url":url,"title":e.text_of("title"),"content_html":content,"date_published":date,"authors":author.map(|a| vec![json!({"name":a})]).unwrap_or_default()})
        })
        .collect();
    Ok(
        json!({"version":"XML_FEED","xml_kind":kind,"title":title,"items":items,"items_without_parsable_date":undated}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rss_atom_and_rdf_become_feed_items() {
        let rss = br#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:content="http://purl.org/rss/1.0/modules/content/">
<channel><title>Shop &amp; News</title>
<item><title>Matcha whisk restock</title><link>https://shop.example/p/1</link><guid isPermaLink="false">a-1</guid>
<pubDate>Tue, 06 Oct 2026 09:30:00 +0900</pubDate><description><![CDATA[<p>New <b>chasen</b> in stock</p>]]></description><dc:creator>Shop</dc:creator></item>
<item><title>Undated &#x2014; note</title><link>https://shop.example/p/2</link><pubDate>sometime</pubDate></item>
</channel></rss>"#;
        let f = to_feed_items(&parse(rss).unwrap()).unwrap();
        assert_eq!(f["xml_kind"], "RSS_2_0");
        assert_eq!(f["title"], "Shop & News");
        let i = &f["items"][0];
        assert_eq!(
            (i["id"].clone(), i["url"].clone()),
            (json!("a-1"), json!("https://shop.example/p/1"))
        );
        assert_eq!(i["date_published"], "2026-10-06T09:30:00+09:00");
        assert_eq!(i["content_html"], "<p>New <b>chasen</b> in stock</p>");
        assert_eq!(i["authors"][0]["name"], "Shop");
        assert_eq!(f["items"][1]["title"], "Undated \u{2014} note");
        assert_eq!(
            f["items"][1]["id"], "https://shop.example/p/2",
            "no guid: the link is the id"
        );
        assert_eq!(f["items_without_parsable_date"], 1);

        let atom = br#"<feed xmlns="http://www.w3.org/2005/Atom"><title>T</title>
<entry><id>tag:x,2026:1</id><title type="html">A &lt;b&gt;</title><link rel="self" href="https://x.example/self"/><link href="https://x.example/1"/>
<updated>2026-10-06T00:00:00Z</updated><summary>s</summary><author><name>N</name></author></entry></feed>"#;
        let f = to_feed_items(&parse(atom).unwrap()).unwrap();
        assert_eq!(f["xml_kind"], "ATOM_1_0");
        assert_eq!(f["items"][0]["url"], "https://x.example/1");
        assert_eq!(f["items"][0]["title"], "A <b>");
        assert_eq!(f["items"][0]["date_published"], "2026-10-06T00:00:00Z");

        let rdf = br#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns="http://purl.org/rss/1.0/" xmlns:dc="http://purl.org/dc/elements/1.1/">
<channel rdf:about="https://r.example/"><title>R</title></channel>
<item rdf:about="https://r.example/1"><title>One</title><link>https://r.example/1</link><dc:date>2026-10-05T10:00:00+09:00</dc:date></item></rdf:RDF>"#;
        let f = to_feed_items(&parse(rdf).unwrap()).unwrap();
        assert_eq!(
            (f["xml_kind"].clone(), f["title"].clone()),
            (json!("RSS_1_0"), json!("R"))
        );
        assert_eq!(f["items"][0]["id"], "https://r.example/1");
        assert_eq!(f["items"][0]["date_published"], "2026-10-05T10:00:00+09:00");
    }

    #[test]
    fn hostile_or_malformed_documents_are_refused() {
        let cases: [(&[u8], &str); 10] = [
            (
                b"<!DOCTYPE r [<!ENTITY a \"aaaa\">]><rss>&a;</rss>",
                "XML_DTD_REFUSED",
            ),
            (
                b"<rss><channel>&nbsp;</channel></rss>",
                "XML_UNDECLARED_ENTITY",
            ),
            (b"<rss><channel></rss>", "XML_MISMATCHED_END_TAG"),
            (b"<rss><channel>", "XML_UNCLOSED_ELEMENT"),
            (
                b"<?xml version=\"1.0\" encoding=\"Shift_JIS\"?><rss/>",
                "XML_ENCODING_UNSUPPORTED_NOT_UTF8",
            ),
            (b"\xff\xfe<rss/>", "XML_ENCODING_UNSUPPORTED_NOT_UTF8"),
            (b"<a/><b/>", "XML_MULTIPLE_ROOTS"),
            (b"<rss a=1/>", "XML_MALFORMED_ATTRIBUTE"),
            (b"<rss>&#xD800;</rss>", "XML_INVALID_CHARACTER_REFERENCE"),
            (b"", "XML_NO_ROOT"),
        ];
        for (doc, reason) in cases {
            assert_eq!(
                parse(doc).unwrap_err(),
                reason,
                "{}",
                String::from_utf8_lossy(doc)
            );
        }
        let deep = "<a>".repeat(MAX_DEPTH + 1) + &"</a>".repeat(MAX_DEPTH + 1);
        assert_eq!(parse(deep.as_bytes()).unwrap_err(), "XML_LIMIT_EXCEEDED");
        assert_eq!(
            to_feed_items(&parse(b"<html/>").unwrap()).unwrap_err(),
            "XML_FEED_ROOT_UNSUPPORTED"
        );
        assert_eq!(
            to_feed_items(&parse(b"<rss/>").unwrap()).unwrap_err(),
            "RSS_CHANNEL_REQUIRED"
        );
    }

    #[test]
    fn rfc822_dates_keep_their_offset() {
        assert_eq!(
            rfc822_to_rfc3339("Mon, 5 Oct 2026 23:59 PDT").as_deref(),
            Some("2026-10-05T23:59:00-07:00")
        );
        assert_eq!(
            rfc822_to_rfc3339("06 Oct 26 01:02:03 GMT").as_deref(),
            Some("2026-10-06T01:02:03+00:00")
        );
        for bad in [
            "",
            "2026-10-06",
            "Tue, 32 Oct 2026 00:00:00 GMT",
            "Tue, 06 Foo 2026 00:00:00 GMT",
            "Tue, 06 Oct 2026 0:0 GMT",
            "Tue, 06 Oct 2026 00:00:00 XYZ",
        ] {
            assert!(rfc822_to_rfc3339(bad).is_none(), "{bad}");
        }
    }
}
