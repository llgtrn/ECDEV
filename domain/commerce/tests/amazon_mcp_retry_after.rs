//! coaxon/amazon-mcp reads Retry-After with float(); ECDEV reads the RFC 9110 grammar.
//! The fixture freezes what the donor's own handler did with 20 header values.
use ecdev_core::provider::retry_after_not_before;
use serde_json::Value;

const RECEIVED: i64 = 1_000_000;

fn oracle() -> Value {
    serde_json::from_str(include_str!("fixtures/amazon-mcp-retry-after-oracle.json")).unwrap()
}

#[test]
fn the_donor_handler_is_pinned_to_the_commit_that_was_run() {
    let o = oracle();
    assert_eq!(o["commit"].as_str().unwrap().len(), 40);
    assert_eq!(o["cases"].as_array().unwrap().len(), 20);
}

#[test]
fn ecdev_waits_only_on_a_valid_header_and_never_on_a_value_the_donor_mis_reads() {
    let mut table = Vec::new();
    for c in oracle()["cases"].as_array().unwrap() {
        let header = c["header"].as_str().unwrap();
        let ecdev = retry_after_not_before(header, RECEIVED);
        let donor = match c["outcome"].as_str().unwrap() {
            "WAIT" => format!("WAIT {}", c["seconds"].as_str().unwrap()),
            "EXCEPTION" => format!("EXCEPTION {}", c["type"].as_str().unwrap()),
            o => o.to_string(),
        };
        table.push((
            header.to_string(),
            donor,
            ecdev.map(|t| (t - RECEIVED) / 1000),
        ));
    }
    let by = |h: &str| table.iter().find(|r| r.0 == h).unwrap();
    // Agreement on the plain delta-seconds form.
    for (h, s) in [("0", 0), ("1", 1), ("7", 7), ("120", 120), ("86400", 86400)] {
        assert_eq!(by(h).2, Some(s), "{h}");
        assert_eq!(by(h).1, format!("WAIT {s}.0"), "{h}");
    }
    // Surrounding whitespace is allowed by the grammar and by both.
    assert_eq!(by(" 7 ").2, Some(7));
    // Values outside the grammar: the donor honours them, ECDEV refuses to guess.
    for h in ["+7", "-1", "1.5", "1e3", "inf", "nan"] {
        assert_eq!(by(h).2, None, "{h}");
        assert!(by(h).1.starts_with("WAIT "), "{h}");
    }
    assert_eq!(by("-1").1, "WAIT -1.0");
    assert_eq!(by("inf").1, "WAIT inf");
    // An empty header: the donor waits 1 s; ECDEV has no instruction.
    assert_eq!(by("").2, None);
    assert_eq!(by("").1, "WAIT 1.0");
    // The donor's int() overflow becomes 1e+20 seconds; ECDEV saturates, never wraps.
    assert_eq!(by("99999999999999999999").1, "WAIT 1e+20");
    assert!(by("99999999999999999999").2.unwrap() > 86_400 * 365);
    // Garbage: the donor's handler raises ValueError instead of throttling.
    for h in ["abc", "7s", "0x10"] {
        assert_eq!(by(h).1, "EXCEPTION ValueError", "{h}");
        assert_eq!(by(h).2, None, "{h}");
    }
    // The three HTTP-date forms are valid and make the donor raise.
    for h in [
        "Wed, 21 Oct 2015 07:28:00 GMT",
        "Wednesday, 21-Oct-15 07:28:00 GMT",
        "Wed Oct 21 07:28:00 2015",
    ] {
        assert_eq!(by(h).1, "EXCEPTION ValueError", "{h}");
        assert!(
            retry_after_not_before(h, 1_800_000_000_000).is_some(),
            "{h}"
        );
    }
}
