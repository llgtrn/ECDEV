//! ECDEV's robots evaluator against Scrapy's default parser, Protego (BSD-3-Clause, 0.7.0 as
//! pinned with scrapy/scrapy at 54f7ed9c). Two frozen sets: the 1,620 reppy-oracle cases ECDEV
//! was built to match, and a probe of 47 handcrafted edge-case texts chosen to find differences.
//! The donor's answers are the donor's, not a standard's; differences are listed, not hidden.
use ecdev_web::robots::evaluate;
use serde_json::Value;
use std::collections::BTreeMap;

fn load(s: &str) -> Value {
    serde_json::from_str(s).unwrap()
}

fn delay_equal(a: Option<f64>, b: Option<f64>) -> bool {
    match (a, b) {
        (Some(x), Some(y)) => (x - y).abs() < 1e-5,
        (None, None) => true,
        _ => false,
    }
}

#[test]
fn ecdev_matches_protego_on_the_reppy_oracle_cases() {
    let reppy = load(include_str!("fixtures/reppy-robots.json"));
    let protego = load(include_str!("fixtures/scrapy-robots-oracle.json"));
    assert_eq!(protego["protego_version"], "0.7.0");
    let (cases, results) = (
        reppy["cases"].as_array().unwrap(),
        protego["results"].as_array().unwrap(),
    );
    assert_eq!((cases.len(), results.len()), (1620, 1620));
    for (c, r) in cases.iter().zip(results) {
        let d = evaluate(
            c["robots"].as_str().unwrap(),
            c["url"].as_str().unwrap(),
            c["agent"].as_str().unwrap(),
        );
        assert_eq!(d.allowed, r["allowed"].as_bool().unwrap(), "{c}");
        assert!(
            delay_equal(d.crawl_delay_seconds, r["crawl_delay_seconds"].as_f64()),
            "{c}"
        );
    }
}

/// name -> [cases, ECDEV allows where Protego denies, ECDEV denies where Protego allows, delay differs]
fn probe() -> (BTreeMap<String, [usize; 4]>, Vec<String>) {
    let f = load(include_str!("fixtures/scrapy-robots-probe.json"));
    let texts = f["texts"].as_object().unwrap();
    let mut by_text: BTreeMap<String, [usize; 4]> = BTreeMap::new();
    let mut detail = vec![];
    for c in f["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let url = format!("https://shop.example{}", c["path"].as_str().unwrap());
        let d = evaluate(
            texts[name].as_str().unwrap(),
            &url,
            c["agent"].as_str().unwrap(),
        );
        let row = by_text.entry(name.to_string()).or_default();
        row[0] += 1;
        let theirs = c["allowed"].as_bool().unwrap();
        let a = d.allowed != theirs;
        let t = !delay_equal(d.crawl_delay_seconds, c["crawl_delay_seconds"].as_f64());
        // ECDEV callers pass the bare product token (research and social fetches pass "ECDEV");
        // a versioned agent string is a different question and is counted apart.
        if c["agent"] != "ecdev/0.1" {
            row[1] += (a && d.allowed) as usize;
            row[2] += (a && !d.allowed) as usize;
            row[3] += t as usize;
        }
        if (a || t) && c["agent"] == "ECDEV" && detail.len() < 400 {
            detail.push(format!(
                "{name} {} {} protego allowed={} delay={} ecdev allowed={} delay={:?}",
                c["path"],
                c["agent"],
                c["allowed"],
                c["crawl_delay_seconds"],
                d.allowed,
                d.crawl_delay_seconds
            ));
        }
    }
    (by_text, detail)
}

#[test]
fn robots_edge_cases_differ_from_protego_only_where_recorded() {
    let (by_text, _) = probe();
    // text -> [ECDEV allows where Protego denies, ECDEV denies where Protego allows, delay differs]
    let want: BTreeMap<&str, [usize; 3]> = [
        // A group named "ecd" governs agent ECDEV in Protego (the 1994 substring convention);
        // ECDEV matches the whole product token (RFC 9309). Not seen in 247 live files.
        ("agent_substring_of_ua", [6, 1, 0]),
        // One pattern given both Allow and Disallow is a contradiction: ECDEV refuses, Protego
        // lets Allow win. Equally specific DIFFERENT patterns follow the RFC (least restrictive).
        ("allow_beats_disallow_on_tie", [0, 12, 0]),
        ("allow_root_then_disallow_all", [0, 36, 0]),
        // An unparseable Crawl-delay (or a negative one) voids the group in ECDEV; Protego
        // ignores the line. 0 of 247 live files had one.
        ("crawl_delay_invalid", [0, 36, 0]),
        ("crawl_delay_negative", [0, 36, 0]),
        // Protego reads "Disallow /a" and "User-agent *" without a colon; ECDEV ignores such a
        // line, which is the less cautious reading. 0 of 247 live files had one.
        ("directive_without_colon", [12, 0, 0]),
        // "Disallow: a" (no leading slash): ECDEV reads it as the prefix "/a" and refuses
        // /a and /abc; Protego reads a rule that matches nothing.
        ("disallow_without_slash", [0, 8, 0]),
        // A "$" inside a pattern ("/a$b"): ECDEV voids the group (everything refused), Protego
        // takes the "$" literally. For "/a$", Protego also refuses a URL with a literal "$" (/a$b).
        ("dollar_in_middle", [0, 34, 0]),
        ("dollar_end_anchor", [2, 0, 0]),
        // Rules written before any User-agent line: ECDEV applies them to every agent
        // (reppy's reading, the cautious one); Protego ignores them.
        ("rules_before_any_agent", [0, 12, 0]),
    ]
    .into_iter()
    .collect();
    let got: BTreeMap<&str, [usize; 3]> = by_text
        .iter()
        .filter(|(_, r)| r[1] + r[2] + r[3] > 0)
        .map(|(k, r)| (k.as_str(), [r[1], r[2], r[3]]))
        .collect();
    assert_eq!(got, want);
    assert_eq!(by_text.len(), 47);
}

#[test]
fn bare_cr_lines_and_consecutive_wildcard_agents_agree_with_protego() {
    // Both were found by this comparison (a live google.com robots.txt and a CR-only file) and
    // fixed: ECDEV used to allow what Protego denies.
    let (by_text, _) = probe();
    for k in [
        "cr_only",
        "consecutive_agents_share_rules",
        "blank_line_between_agent_and_rules",
        "group_merge_same_agent",
        "longest_match_wins",
        "wildcard_star_in_path",
    ] {
        assert_eq!(by_text[k][1] + by_text[k][2] + by_text[k][3], 0, "{k}");
    }
}
