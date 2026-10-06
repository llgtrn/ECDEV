//! Native robots evaluator, tested against the locked Reppy C++ oracle.
//! Exact agent selection follows the reviewed donor contract. Ambiguous ties deny.
use ecdev_core::frontier::{UrlPolicy, canonicalize};
use serde_json::{Value, json};
use url::Url;
#[derive(Debug)]
pub struct Decision {
    pub allowed: bool,
    pub crawl_delay_seconds: Option<f64>,
    pub evidence: Value,
}
#[derive(Default)]
struct Group {
    agents: Vec<String>,
    rules: Vec<(bool, String, usize)>,
    delay: Option<f64>,
    unknown: bool,
    started: bool,
}

fn normalized_path(raw: &str, base: &str) -> Option<String> {
    let normalized = canonicalize(raw, Some(base), &UrlPolicy::default(), 0).ok()?;
    let url = Url::parse(&normalized.canonical_url).ok()?;
    if url.host_str() != Url::parse(base).ok()?.host_str() {
        return None;
    }
    Some(format!(
        "{}{}",
        url.path(),
        url.query().map(|q| format!("?{q}")).unwrap_or_default()
    ))
}
fn pattern(raw: &str, base: &str) -> Option<String> {
    if raw.len() > 4096 {
        return None;
    }
    // A rule may begin with a wildcard ("*/collections/*filter*", "*filters=*"): the wildcard
    // matches any prefix of the path, so the rest is normalised as a path fragment and keeps
    // its leading wildcard. Such rules are honoured, Disallow included, rather than voiding the
    // whole file.
    let p = match raw.strip_prefix('*') {
        Some(rest) => {
            let rest = rest.trim_start_matches('*');
            if rest.is_empty() {
                "/".to_string()
            } else if rest.starts_with('/') {
                format!("*{}", normalized_path(rest, base)?)
            } else {
                format!(
                    "*{}",
                    normalized_path(&format!("/{rest}"), base)?.strip_prefix('/')?
                )
            }
        }
        None => normalized_path(raw, base)?,
    };
    if p.contains('$') && !p.ends_with('$') {
        return None;
    }
    let mut result = String::new();
    for c in p.chars() {
        if c != '*' || !result.ends_with('*') {
            result.push(c);
        }
    }
    while result.ends_with('*') {
        result.pop();
    }
    Some(result)
}
fn matches(pattern: &str, path: &str) -> bool {
    let anchored = pattern.ends_with('$');
    let p = pattern.strip_suffix('$').unwrap_or(pattern).as_bytes();
    let s = path.as_bytes();
    let (mut i, mut j) = (0, 0);
    let mut star = None;
    let mut restart = 0;
    loop {
        if i == p.len() {
            return !anchored || j == s.len();
        }
        if p[i] == b'*' {
            star = Some(i);
            i += 1;
            restart = j;
            continue;
        }
        if j < s.len() && p[i] == s[j] {
            i += 1;
            j += 1;
            continue;
        }
        if let Some(mark) = star
            && restart < s.len()
        {
            restart += 1;
            j = restart;
            i = mark + 1;
            continue;
        }
        return false;
    }
}
/// The robots.txt governing `url`: same scheme, host and port, with credentials, query and
/// fragment dropped (Rep::Robots::robotsUrl).
/// Sitemaps a robots.txt declares (`Sitemap:` lines are not group-scoped; RFC 9309 section
/// 2.2.4). Relative locations resolve against the robots URL; only http(s) URLs are kept,
/// duplicates once, at most `MAX_DECLARED_SITEMAPS`. A sitemap on another host is kept: robots
/// cross-submission is how a site declares it.
pub const MAX_DECLARED_SITEMAPS: usize = 50;

pub fn sitemaps(text: &str, robots: &Url) -> Vec<Url> {
    let mut out: Vec<Url> = vec![];
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if !key.trim().eq_ignore_ascii_case("sitemap") {
            continue;
        }
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        let Ok(u) = robots.join(value) else {
            continue;
        };
        if matches!(u.scheme(), "http" | "https") && !out.contains(&u) {
            out.push(u);
            if out.len() == MAX_DECLARED_SITEMAPS {
                break;
            }
        }
    }
    out
}

/// Origins whose robots.txt outcome a process keeps at once.
pub const ROBOTS_CACHE_ORIGINS: usize = 256;

/// A fetched robots.txt outcome kept for its HTTP freshness lifetime (RFC 9309 section 2.4:
/// at most 24 hours, via `freshness_lifetime`). Only 200 and 404 are kept: 429, 5xx and anything
/// else are refetched every time, so an outage never becomes a cached allow or deny.
#[derive(Clone, Debug, PartialEq)]
pub struct RobotsEntry {
    pub status: u16,
    pub body: Vec<u8>,
    pub fetched_at: u64,
    pub expires_at: u64,
    pub basis: &'static str,
}

#[derive(Default)]
pub struct RobotsCache {
    entries: std::collections::BTreeMap<String, RobotsEntry>,
}

impl RobotsCache {
    /// The kept outcome for this robots URL while it is fresh (`now < expires_at`).
    pub fn get(&mut self, robots: &Url, now: u64) -> Option<RobotsEntry> {
        let key = robots.as_str();
        match self.entries.get(key) {
            Some(e) if now < e.expires_at => Some(e.clone()),
            Some(_) => {
                self.entries.remove(key);
                None
            }
            None => None,
        }
    }
    /// Keeps a 200 or 404 outcome for its header-derived lifetime; returns whether it was kept.
    pub fn put(
        &mut self,
        robots: &Url,
        status: u16,
        body: &[u8],
        headers: &Value,
        now: u64,
    ) -> bool {
        if status != 200 && status != 404 {
            return false;
        }
        let (lifetime, basis) = ecdev_core::research::freshness_lifetime(headers);
        if lifetime == 0 {
            return false;
        }
        self.entries.retain(|_, e| now < e.expires_at);
        while self.entries.len() >= ROBOTS_CACHE_ORIGINS {
            let soonest = self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.expires_at)
                .map(|(k, _)| k.clone());
            match soonest {
                Some(k) => self.entries.remove(&k),
                None => break,
            };
        }
        self.entries.insert(
            robots.as_str().to_string(),
            RobotsEntry {
                status,
                body: body.to_vec(),
                fetched_at: now,
                expires_at: now.saturating_add(lifetime),
                basis,
            },
        );
        true
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

pub fn robots_url(url: &Url) -> Url {
    let mut robots = url.clone();
    robots.set_path("/robots.txt");
    robots.set_query(None);
    robots.set_fragment(None);
    let _ = robots.set_username("");
    let _ = robots.set_password(None);
    robots
}
pub fn evaluate(text: &str, url: &str, agent: &str) -> Decision {
    let denied = |reason: &str| Decision {
        allowed: false,
        crawl_delay_seconds: None,
        evidence: json!({"reason":reason}),
    };
    if text.len() > 1024 * 1024 || url.len() > 8192 {
        return denied("ROBOTS_INPUT_LIMIT");
    }
    let Some(path) = normalized_path(url, url) else {
        return denied("ROBOTS_INVALID_URL");
    };
    let mut groups = vec![];
    let mut group = Group {
        agents: vec!["*".into()],
        ..Default::default()
    };
    for (number, line) in text.trim_start_matches('\u{feff}').lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        if key == "user-agent" {
            if group.started {
                groups.push(group);
                group = Group::default();
            }
            if group.agents == ["*"] && group.rules.is_empty() && !group.started {
                group.agents.clear();
            }
            group.agents.push(value.to_ascii_lowercase());
            continue;
        }
        group.started = true;
        match key.as_str() {
            "allow" | "disallow" if !value.is_empty() => {
                // Foreign host rules are ignored by the reviewed Reppy agent implementation.
                if let Ok(other) = Url::parse(value)
                    && other.host_str()
                        != Url::parse(url)
                            .ok()
                            .and_then(|u| u.host_str().map(str::to_owned))
                            .as_deref()
                {
                    continue;
                }
                match pattern(value, url) {
                    Some(p) => group.rules.push((key == "allow", p, number + 1)),
                    None => group.unknown = true,
                }
            }
            "crawl-delay" => match value.parse::<f32>() {
                Ok(v) if v.is_finite() && v >= 0.0 => group.delay = Some(v as f64),
                _ => group.unknown = true,
            },
            _ => {}
        }
    }
    groups.push(group);
    let agent = agent.to_ascii_lowercase();
    let specific = groups.iter().any(|g| g.agents.contains(&agent));
    let mut selected_rules = vec![];
    let mut priority = 0;
    let mut allowed = true;
    let mut delay: Option<f64> = None;
    for g in groups.iter().filter(|g| {
        g.agents
            .iter()
            .any(|a| a == if specific { &agent } else { "*" })
    }) {
        if g.unknown {
            return denied("ROBOTS_UNSUPPORTED_OR_INVALID_DIRECTIVE");
        }
        if let Some(d) = g.delay {
            delay = Some(delay.unwrap_or(0.0).max(d));
        }
        for (allow, p, line) in &g.rules {
            if matches(p, &path) {
                selected_rules.push(json!({"line":line,"directive":if *allow{"Allow"}else{"Disallow"},"pattern":p,"priority":p.len()}));
                if p.len() > priority {
                    priority = p.len();
                    allowed = *allow;
                } else if p.len() == priority {
                    allowed &= *allow;
                }
            }
        }
    }
    if path == "/robots.txt" {
        allowed = true;
    }
    Decision {
        allowed,
        crawl_delay_seconds: delay,
        evidence: json!({"agent":agent,"selection":if specific{"EXACT_AGENT"}else{"WILDCARD_FALLBACK"},"path_and_query":path,"matched_rules":selected_rules,"tie_policy":"DENY_IF_AMBIGUOUS","contract":"REPPY_ORACLE_TESTED_SUBSET"}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn locked_reppy_oracle() {
        let oracle: Value =
            serde_json::from_str(include_str!("../tests/fixtures/reppy-robots.json")).unwrap();
        let cases = oracle["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 1620);
        for c in cases {
            let d = evaluate(
                c["robots"].as_str().unwrap(),
                c["url"].as_str().unwrap(),
                c["agent"].as_str().unwrap(),
            );
            assert_eq!(
                d.allowed,
                c["allowed"].as_bool().unwrap(),
                "{c}\n{:?}",
                d.evidence
            );
            let expected = c["crawl_delay_seconds"].as_f64();
            assert!(
                d.crawl_delay_seconds
                    .zip(expected)
                    .is_some_and(|(a, b)| (a - b).abs() < 0.00001)
                    || d.crawl_delay_seconds.is_none() && expected.is_none(),
                "Delay differs: {c} {d:?}"
            );
        }
    }
    #[test]
    fn rules_beginning_with_a_wildcard_are_honoured_not_fatal() {
        // Shapes seen in real retail robots.txt files (allbirds.com, ikea.com).
        let text = "User-agent: *\nDisallow: */collections/*filter*&*filter*\nDisallow: *sort_by=*\nDisallow: /checkout\nAllow: *filters=*\nDisallow: /search\n";
        let allowed = |u: &str| evaluate(text, u, "ECDEV").allowed;
        assert!(allowed("https://shop.example/products/mens-wool-runners"));
        assert!(!allowed(
            "https://shop.example/collections/shoes?filter=a&filter=b"
        ));
        assert!(!allowed(
            "https://shop.example/en/collections/shoes?filter=a&x=1&filter=b"
        ));
        assert!(allowed("https://shop.example/collections/shoes?filter=a"));
        assert!(!allowed(
            "https://shop.example/collections/shoes?sort_by=price"
        ));
        assert!(!allowed("https://shop.example/checkout"));
        // The longer, more specific Allow wins over a shorter Disallow.
        assert!(allowed("https://shop.example/search?filters=red"));
        assert!(!allowed("https://shop.example/search?q=red"));
        // A bare wildcard disallows everything, as "/" does.
        assert!(
            !evaluate(
                "User-agent: *\nDisallow: *\n",
                "https://shop.example/a",
                "ECDEV"
            )
            .allowed
        );
    }

    #[test]
    fn declared_sitemaps_are_resolved_and_deduplicated() {
        let r = Url::parse("https://www.shop.example/robots.txt").unwrap();
        let text = "User-agent: *\nDisallow: /cart\nSitemap: https://shop.example/sitemap.xml\nsitemap:/s2.xml # comment\nSITEMAP: https://shop.example/sitemap.xml\nSitemap: ftp://x.example/s.xml\nSitemap:\n";
        let got: Vec<_> = sitemaps(text, &r).iter().map(Url::to_string).collect();
        assert_eq!(
            got,
            [
                "https://shop.example/sitemap.xml",
                "https://www.shop.example/s2.xml"
            ]
        );
        // A colon inside the URL survives; comments do not.
        assert_eq!(
            sitemaps("Sitemap: https://a.example:8443/s.xml", &r)[0].port(),
            Some(8443)
        );
    }

    #[test]
    fn robots_outcomes_are_kept_for_their_http_lifetime_only() {
        let u = Url::parse("https://shop.example/robots.txt").unwrap();
        let mut c = RobotsCache::default();
        assert!(c.put(
            &u,
            200,
            b"User-agent: *\nDisallow: /x",
            &json!({"cache_control":"max-age=600"}),
            1000
        ));
        assert_eq!(c.get(&u, 1599).unwrap().basis, "MAX_AGE");
        assert!(c.get(&u, 1600).is_none());
        assert!(c.is_empty());
        // No freshness information: the default hour; never beyond 24 hours.
        assert!(c.put(&u, 404, b"", &json!({}), 0));
        assert_eq!(c.get(&u, 0).unwrap().expires_at, 3600);
        assert!(c.put(&u, 200, b"", &json!({"cache_control":"max-age=999999"}), 0));
        assert_eq!(c.get(&u, 0).unwrap().expires_at, 86_400);
        // Outages and no-store are never kept.
        let mut d = RobotsCache::default();
        for s in [429, 500, 503, 401, 403] {
            assert!(
                !d.put(&u, s, b"", &json!({"cache_control":"max-age=600"}), 0),
                "{s}"
            );
        }
        assert!(!d.put(&u, 200, b"", &json!({"cache_control":"no-store"}), 0));
        assert!(d.is_empty());
    }

    #[test]
    fn robots_cache_is_bounded_and_evicts_the_soonest_expiry() {
        let mut c = RobotsCache::default();
        for i in 0..ROBOTS_CACHE_ORIGINS as u64 + 5 {
            let u = Url::parse(&format!("https://s{i}.example/robots.txt")).unwrap();
            let age = json!({"cache_control": format!("max-age={}", 1000 + i)});
            assert!(c.put(&u, 200, b"", &age, 0));
        }
        assert_eq!(c.len(), ROBOTS_CACHE_ORIGINS);
        let first = Url::parse("https://s0.example/robots.txt").unwrap();
        let last = Url::parse(&format!(
            "https://s{}.example/robots.txt",
            ROBOTS_CACHE_ORIGINS + 4
        ))
        .unwrap();
        assert!(c.get(&first, 1).is_none());
        assert!(c.get(&last, 1).is_some());
    }

    #[test]
    fn locked_reppy_robots_url_and_gpp_reproduction() {
        let fixture: Value =
            serde_json::from_str(include_str!("../tests/fixtures/reppy-robots-gpp.json")).unwrap();
        let reproduction = &fixture["reproduction_of_msvc_fixture"];
        assert_eq!(reproduction["fixture_cases"], 1620);
        assert_eq!(reproduction["gpp_agreeing"], 1620);
        for case in fixture["robots_url_cases"].as_array().unwrap() {
            let url = Url::parse(case["url"].as_str().unwrap()).unwrap();
            // The donor keeps IDN hosts in Unicode; parsing gives the same host in ASCII form.
            let expected = Url::parse(case["robots_url"].as_str().unwrap()).unwrap();
            assert_eq!(robots_url(&url), expected, "{}", case["url"]);
        }
    }
    #[test]
    fn crawl_delay_regressions() {
        let delay =
            |text: &str| evaluate(text, "https://shop.example/a", "ECDEV").crawl_delay_seconds;
        assert_eq!(delay("User-agent: *\nCrawl-delay: 2.5\n"), Some(2.5));
        assert_eq!(
            delay("User-agent: ECDEV\nCrawl-delay: 1\nUser-agent: *\nCrawl-delay: 9\n"),
            Some(1.0)
        );
        assert_eq!(delay("User-agent: *\nDisallow:\n"), None);
    }
    #[test]
    fn invalid_and_ambiguous_policy_remains_denied() {
        assert!(
            !evaluate(
                "User-agent: *\nAllow: /x\nDisallow: /x",
                "https://shop.example/x",
                "ECDEV"
            )
            .allowed
        );
        assert!(
            !evaluate(
                "User-agent: *\nCrawl-delay: NaN",
                "https://shop.example/x",
                "ECDEV"
            )
            .allowed
        );
        assert!(
            !evaluate(
                "User-agent: *\nDisallow: /x$y",
                "https://shop.example/x",
                "ECDEV"
            )
            .allowed
        );
    }
}
