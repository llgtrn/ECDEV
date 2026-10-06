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
    if raw.starts_with('*') || raw.len() > 4096 {
        return None;
    }
    let p = normalized_path(raw, base)?;
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
