//! Bounded sitemap reading (sitemaps.org 0.9): a `<urlset>` lists pages, a `<sitemapindex>`
//! lists further sitemaps. Only each entry's own `<loc>` and `<lastmod>` are read; extension
//! elements such as `<image:loc>` are not entries. The protocol's limits (50,000 entries,
//! 50 MiB uncompressed) are enforced, gzip bodies are decoded under that cap (anything that does
//! not decode completely is refused), and an
//! entry whose location is not an absolute http(s) URL is counted as rejected, never repaired.

use serde_json::{Value, json};
use url::Url;

pub const MAX_ENTRIES: usize = 50_000;
pub const MAX_BYTES: usize = 50 * 1024 * 1024;

#[derive(Debug, PartialEq)]
pub struct Sitemap {
    pub kind: &'static str,
    pub entries: Vec<(Url, Option<String>)>,
    pub rejected: usize,
}

/// Decodes XML character data: one CDATA section, or the five predefined entities and numeric
/// character references. Any other entity makes the value undecodable.
fn unescape(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if let Some(inner) = raw
        .strip_prefix("<![CDATA[")
        .and_then(|r| r.strip_suffix("]]>"))
    {
        return Some(inner.trim().to_string());
    }
    let mut out = String::new();
    let mut i = 0;
    while let Some(amp) = raw[i..].find('&') {
        let start = i + amp;
        out.push_str(&raw[i..start]);
        let end = start + raw[start..].find(';')?;
        out.push(match &raw[start + 1..end] {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            n if n.starts_with("#x") || n.starts_with("#X") => {
                char::from_u32(u32::from_str_radix(&n[2..], 16).ok()?)?
            }
            n if n.starts_with('#') => char::from_u32(n[1..].parse().ok()?)?,
            _ => return None,
        });
        i = end + 1;
    }
    out.push_str(&raw[i..]);
    Some(out)
}

fn element<'a>(block: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let start = block.find(&open)? + open.len();
    let end = start + block[start..].find(&close)?;
    Some(&block[start..end])
}

pub fn parse(body: &[u8]) -> Result<Sitemap, String> {
    if body.len() > MAX_BYTES {
        return Err("SITEMAP_TOO_LARGE".into());
    }
    // A gzip sitemap (".xml.gz", the protocol's own compressed form) is read from the stored
    // capture itself, so replay never depends on a second copy; the limit is the protocol's
    // 50 MiB uncompressed, and a stream that does not decode completely is refused whole.
    let decoded;
    let body = if crate::gzip::is_gzip(body) {
        decoded = crate::gzip::gunzip(body, MAX_BYTES).map_err(|e| e.code().to_string())?;
        &decoded[..]
    } else {
        body
    };
    let text = std::str::from_utf8(body).map_err(|_| "SITEMAP_NOT_UTF8")?;
    let text = text.trim_start_matches('\u{feff}');
    let (kind, tag) = match (text.find("<urlset"), text.find("<sitemapindex")) {
        (Some(_), None) => ("URLSET", "url"),
        (None, Some(_)) => ("SITEMAP_INDEX", "sitemap"),
        _ => return Err("SITEMAP_ROOT_UNKNOWN".into()),
    };
    let (open_plain, open_attr, close) =
        (format!("<{tag}>"), format!("<{tag} "), format!("</{tag}>"));
    let mut entries = vec![];
    let mut rejected = 0;
    let mut rest = text;
    loop {
        let start = match (rest.find(&open_plain), rest.find(&open_attr)) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => break,
        };
        let Some(len) = rest[start..].find(&close) else {
            return Err("SITEMAP_ENTRY_UNTERMINATED".into());
        };
        let block = &rest[start..start + len];
        rest = &rest[start + len + close.len()..];
        if entries.len() + rejected >= MAX_ENTRIES {
            return Err("SITEMAP_ENTRY_LIMIT".into());
        }
        let loc = element(block, "loc")
            .and_then(unescape)
            .and_then(|l| Url::parse(&l).ok())
            .filter(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some());
        match loc {
            Some(u) => entries.push((u, element(block, "lastmod").and_then(unescape))),
            None => rejected += 1,
        }
    }
    Ok(Sitemap {
        kind,
        entries,
        rejected,
    })
}

impl Sitemap {
    pub fn evidence(&self) -> Value {
        json!({"kind":self.kind,"entries":self.entries.len(),"rejected":self.rejected,"method":"SITEMAP_LOC_LASTMOD_SCAN_BOUNDED"})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urlset_entries_keep_their_own_loc_and_lastmod() {
        let x = br#"<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9" xmlns:image="http://www.google.com/schemas/sitemap-image/1.1">
<url><loc>https://shop.example/products/cup?a=1&amp;b=2</loc><lastmod>2026-09-01</lastmod>
<image:image><image:loc>https://cdn.example/cup.jpg</image:loc></image:image></url>
<url>
  <loc><![CDATA[ https://shop.example/products/bowl ]]></loc>
</url>
<url><loc>/relative</loc></url>
<url><loc>ftp://shop.example/x</loc></url>
</urlset>"#;
        let s = parse(x).unwrap();
        assert_eq!(s.kind, "URLSET");
        assert_eq!(s.rejected, 2);
        assert_eq!(s.entries.len(), 2);
        assert_eq!(
            s.entries[0].0.as_str(),
            "https://shop.example/products/cup?a=1&b=2"
        );
        assert_eq!(s.entries[0].1.as_deref(), Some("2026-09-01"));
        assert_eq!(
            s.entries[1].0.as_str(),
            "https://shop.example/products/bowl"
        );
        assert_eq!(s.entries[1].1, None);
    }

    #[test]
    fn indexes_and_refusals() {
        let i = br#"<sitemapindex xmlns="http://www.sitemaps.org/schemas/sitemap/0.9"><sitemap><loc>https://shop.example/sitemap_products_1.xml?from=1&#38;to=9</loc></sitemap></sitemapindex>"#;
        let s = parse(i).unwrap();
        assert_eq!(s.kind, "SITEMAP_INDEX");
        assert_eq!(s.entries[0].0.query(), Some("from=1&to=9"));
        // A gzip header with no stream behind it is refused by name, not read as text.
        assert_eq!(parse(&[0x1f, 0x8b, 8]).unwrap_err(), "GZIP_TRUNCATED");
        assert_eq!(parse(b"<html></html>").unwrap_err(), "SITEMAP_ROOT_UNKNOWN");
        assert_eq!(
            parse(b"<urlset><url><loc>https://a.example/</loc>").unwrap_err(),
            "SITEMAP_ENTRY_UNTERMINATED"
        );
        assert_eq!(
            parse(b"<urlset><url><loc>https://a.example/&bogus;</loc></url></urlset>")
                .unwrap()
                .rejected,
            1
        );
    }
}
