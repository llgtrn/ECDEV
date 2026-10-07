//! Registrable domains (eTLD+1) from the Public Suffix List, so listings on www.example.com and
//! shop.example.com count as one site while two stores on a shared platform suffix
//! (a.myshopify.com, b.myshopify.com) count as two. The list is vendored unmodified
//! (data/public_suffix_list.dat, MPL-2.0, version in its header) and matched natively: the
//! prevailing rule is an exception if one matches, else the longest normal or wildcard rule,
//! else "*". A same registrable domain is one site; different ones are still not proven to be
//! independent publishers.

use std::collections::HashSet;
use std::sync::OnceLock;

const LIST: &str = include_str!("../data/public_suffix_list.dat");

struct Rules {
    normal: HashSet<String>,
    /// "*.ck" is stored as "ck".
    wildcard: HashSet<String>,
    /// "!www.ck" is stored as "www.ck".
    exception: HashSet<String>,
    version: String,
}

/// A rule or host in the ASCII (punycode) form the url crate gives hosts.
fn ascii(name: &str) -> Option<String> {
    if name.is_ascii() {
        return Some(name.to_ascii_lowercase());
    }
    url::Url::parse(&format!("http://{name}/"))
        .ok()?
        .host_str()
        .map(str::to_string)
}

fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| {
        let mut r = Rules {
            normal: HashSet::new(),
            wildcard: HashSet::new(),
            exception: HashSet::new(),
            version: String::new(),
        };
        for line in LIST.lines() {
            if let Some(v) = line.strip_prefix("// VERSION: ") {
                r.version = v.trim().to_string();
            }
            let Some(rule) = line.split_whitespace().next() else {
                continue;
            };
            if rule.starts_with("//") {
                continue;
            }
            if let Some(e) = rule.strip_prefix('!') {
                r.exception.extend(ascii(e));
            } else if let Some(w) = rule.strip_prefix("*.") {
                r.wildcard.extend(ascii(w));
            } else {
                r.normal.extend(ascii(rule));
            }
        }
        r
    })
}

pub fn list_version() -> &'static str {
    &rules().version
}

/// The registrable domain of a host name; None for a public suffix itself, an IP address, or a
/// malformed name (empty labels, a leading dot).
pub fn registrable_domain(host: &str) -> Option<String> {
    let host = ascii(host.strip_suffix('.').unwrap_or(host))?;
    if host.is_empty()
        || host.split('.').any(str::is_empty)
        || host.parse::<std::net::IpAddr>().is_ok()
        || host.starts_with('[')
    {
        return None;
    }
    let labels: Vec<&str> = host.split('.').collect();
    let n = labels.len();
    let r = rules();
    let suffix_len = (0..n)
        .find(|&i| r.exception.contains(&labels[i..].join(".")))
        .map(|i| n - i - 1)
        .or_else(|| {
            (0..n).find_map(|i| {
                let s = labels[i..].join(".");
                let wild = i + 1 < n && r.wildcard.contains(&labels[i + 1..].join("."));
                (r.normal.contains(&s) || wild).then_some(n - i)
            })
        })
        .unwrap_or(1);
    (n > suffix_len).then(|| labels[n - suffix_len - 1..].join("."))
}

/// The site a URL's listing belongs to: its registrable domain, else its host (IP addresses,
/// public suffixes used as hosts).
pub fn site_of(url: &url::Url) -> Option<String> {
    let host = url.host_str()?;
    Some(registrable_domain(host).unwrap_or_else(|| host.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sites_group_subdomains_and_split_platform_stores() {
        assert_eq!(
            registrable_domain("www.example.co.uk").as_deref(),
            Some("example.co.uk")
        );
        assert_eq!(
            registrable_domain("shop.example.com").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            registrable_domain("a.myshopify.com").as_deref(),
            Some("a.myshopify.com")
        );
        assert_eq!(registrable_domain("co.uk"), None);
        assert_eq!(registrable_domain("127.0.0.1"), None);
        let u = url::Url::parse("http://192.0.2.7/x").unwrap();
        assert_eq!(site_of(&u).as_deref(), Some("192.0.2.7"));
        assert!(list_version().starts_with("20"));
    }
}
