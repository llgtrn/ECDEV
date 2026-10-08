//! ECDEV's redirect decision against frozen decisions of the pinned Scrapy RedirectMiddleware
//! (BSD-3-Clause, scrapy/scrapy at 54f7ed9c), on the 2,000 cases of the httpx oracle. Compared
//! on the target with its fragment removed. Differences are classified and counted, never hidden.
use ecdev_web::{location_text, redirect_target};
use serde_json::Value;
use std::collections::BTreeMap;
use url::Url;

fn load(s: &str) -> Value {
    serde_json::from_str(s).unwrap()
}

fn bare(u: &str) -> String {
    let b = u.split('#').next().unwrap();
    Url::parse(b).map_or(b.to_string(), |u| u.to_string())
}

fn classify() -> (BTreeMap<String, usize>, BTreeMap<String, Vec<String>>) {
    let httpx = load(include_str!("fixtures/httpx-redirect-oracle.json"));
    let scrapy = load(include_str!("fixtures/scrapy-redirect-oracle.json"));
    assert_eq!(
        scrapy["scrapy_commit"],
        "54f7ed9cccbe19db3fe6cd2252bf3c29326c67d7"
    );
    let (cases, results) = (
        httpx["cases"].as_array().unwrap(),
        scrapy["results"].as_array().unwrap(),
    );
    assert_eq!(cases.len(), results.len());
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut examples: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (c, s) in cases.iter().zip(results) {
        let base = Url::parse(c["base"].as_str().unwrap()).unwrap();
        let (status, location) = (c["status"].as_u64().unwrap() as u16, c["location"].as_str());
        // As the fetch path reads the header: bytes in, text out.
        let header_text = location.map(|l| location_text(l.as_bytes()));
        let ours = redirect_target(&base, status, header_text.as_deref());
        let class = match (s["follows"].as_bool(), s["target"].as_str(), &ours) {
            (Some(false), _, Ok(None)) => "BOTH_STOP".to_string(),
            (Some(false), _, Ok(Some(_))) => "ECDEV_FOLLOWS_SCRAPY_STOPS".to_string(),
            (Some(false), _, Err(e)) => format!("SCRAPY_STOPS_ECDEV_REFUSES_{e}"),
            (Some(true), Some(t), Ok(Some(n))) if *n == bare(t) => {
                "BOTH_FOLLOW_SAME_TARGET".to_string()
            }
            (Some(true), Some(_), Ok(Some(_))) => "BOTH_FOLLOW_DIFFERENT_TARGET".to_string(),
            (Some(true), _, Ok(None)) => "SCRAPY_FOLLOWS_ECDEV_STOPS".to_string(),
            (Some(true), _, Err(e)) => format!("SCRAPY_FOLLOWS_ECDEV_REFUSES_{e}"),
            (None, _, Ok(None)) => "SCRAPY_RAISES_ECDEV_STOPS".to_string(),
            (None, _, Ok(Some(_))) => "SCRAPY_RAISES_ECDEV_FOLLOWS".to_string(),
            (None, _, Err(e)) => format!("SCRAPY_RAISES_ECDEV_REFUSES_{e}"),
            _ => "UNCLASSIFIED".to_string(),
        };
        *counts.entry(class.clone()).or_default() += 1;
        let ex = examples.entry(class).or_default();
        if ex.len() < 3 {
            ex.push(format!(
                "{} {status} {location:?} scrapy={} ecdev={ours:?}",
                c["base"], s
            ));
        }
    }
    (counts, examples)
}

#[test]
fn scrapy_and_ecdev_redirect_decisions_differ_only_where_ecdev_is_stricter() {
    let (counts, _) = classify();
    let want: BTreeMap<&str, usize> = [
        // Same decision, same target (fragment aside): the common ground, 1,700 of 2,000.
        ("BOTH_STOP", 1025),
        ("BOTH_FOLLOW_SAME_TARGET", 675),
        // Scrapy drops a bare "?" from the target; ECDEV keeps the URL as the server sent it.
        ("BOTH_FOLLOW_DIFFERENT_TARGET", 25),
        // ECDEV's public-URL policy: no credentials in a target, no credential-named query
        // parameter, http(s) only. Scrapy follows the first two and stops quietly at ftp.
        ("SCRAPY_FOLLOWS_ECDEV_REFUSES_PUBLIC_HTTP_URL_REQUIRED", 25),
        ("SCRAPY_FOLLOWS_ECDEV_REFUSES_CREDENTIAL_QUERY_DENIED", 25),
        ("SCRAPY_STOPS_ECDEV_REFUSES_PUBLIC_HTTP_URL_REQUIRED", 25),
        // Locations clients resolve differently, which Scrapy resolves by leniency: "//" back to
        // the same URL, "///x" to host x, "http:/x" to an empty host, a host with a space,
        // "https:path" relative to the current host, backslashes as path text.
        ("SCRAPY_FOLLOWS_ECDEV_REFUSES_INVALID_REDIRECT", 145),
        // Scrapy raises on these (javascript: and a port past 65535); ECDEV refuses by name.
        ("SCRAPY_RAISES_ECDEV_REFUSES_INVALID_REDIRECT", 55),
    ]
    .into_iter()
    .collect();
    let got: BTreeMap<&str, usize> = counts.iter().map(|(k, v)| (k.as_str(), *v)).collect();
    assert_eq!(got, want);
    // Never the reverse: ECDEV does not follow where Scrapy stops, nor stop where Scrapy follows
    // without a stated reason.
    assert!(!counts.contains_key("ECDEV_FOLLOWS_SCRAPY_STOPS"));
    assert!(!counts.contains_key("SCRAPY_FOLLOWS_ECDEV_STOPS"));
}

#[test]
fn raw_non_ascii_location_bytes_are_followed_as_received() {
    // Found by this comparison: the fetch path used to drop a non-ASCII Location as if absent.
    let base = Url::parse("https://www.example.com/a/b").unwrap();
    for (raw, want) in [
        (
            "/ü/path".as_bytes().to_vec(),
            "https://www.example.com/%C3%BC/path",
        ),
        (
            "https://bücher.example/p".as_bytes().to_vec(),
            "https://xn--bcher-kva.example/p",
        ),
        // Shift_JIS bytes are kept as they were received, not guessed into another encoding.
        (
            b"/\x96\x95\x92\x83".to_vec(),
            "https://www.example.com/%96%95%92%83",
        ),
        (b"/caf\xe9".to_vec(), "https://www.example.com/caf%E9"),
    ] {
        let text = location_text(&raw);
        assert!(text.is_ascii());
        assert_eq!(
            redirect_target(&base, 301, Some(&text)).unwrap().as_deref(),
            Some(want),
            "{text}"
        );
    }
    // A host that is not decodable from its bytes is refused, never guessed.
    let host = location_text(b"https://b\xfccher.example/p");
    assert_eq!(
        redirect_target(&base, 301, Some(&host)),
        Err("INVALID_REDIRECT".to_string())
    );
}
