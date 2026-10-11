//! The native URL parser against the web-platform-tests URL conformance data (BSD-3-Clause,
//! https://github.com/web-platform-tests/wpt, url/resources/urltestdata.json, fetched 2026-10).
//!
//! Every case must either match the expected components exactly or be one of two documented
//! exceptions: the `file` scheme, which is refused outright, and labels the standard's newest
//! revision treats as lenient Punycode (IgnoreInvalidPunycode) where ECDEV, like the url crate it
//! replaces, rejects the host.
use ecdev_core::json::Value;
use ecdev_core::web_address::{ParseError, Url};

/// Hosts the newest standard accepts as invalid Punycode but ECDEV rejects.
const LENIENT_PUNYCODE: &[&str] = &[
    "http://a.b.c.xn--pokxncvks",
    "http://10.0.0.xn--pokxncvks",
    "http://a.b.c.XN--pokxncvks",
    "http://a.b.c.Xn--pokxncvks",
    "http://10.0.0.XN--pokxncvks",
    "http://10.0.0.xN--pokxncvks",
    "https://xn--/",
];

fn components(u: &Url) -> [String; 9] {
    let host = match (u.host_str(), u.port()) {
        (Some(h), Some(p)) => format!("{h}:{p}"),
        (Some(h), None) => h.to_string(),
        _ => String::new(),
    };
    [
        u.as_str().to_string(),
        format!("{}:", u.scheme()),
        u.username().to_string(),
        u.password().unwrap_or("").to_string(),
        host,
        u.host_str().unwrap_or("").to_string(),
        u.port().map(|p| p.to_string()).unwrap_or_default(),
        u.path().to_string(),
        u.query()
            .filter(|q| !q.is_empty())
            .map(|q| format!("?{q}"))
            .unwrap_or_default(),
    ]
}

#[test]
fn native_urls_match_the_web_platform_tests_except_the_documented_refusals() {
    let data: Value =
        ecdev_core::json::from_str(include_str!("fixtures/wpt-urltestdata.json")).unwrap();
    let (mut ok, mut refused, mut lenient) = (0, 0, vec![]);
    for case in data.as_array().unwrap().iter().filter(|c| c.is_object()) {
        let input = case["input"].as_str().unwrap();
        let base = case["base"].as_str();
        let result = match base {
            Some(b) => Url::parse(b).and_then(|b| b.join(input)),
            None => Url::parse(input),
        };
        let failure = case["failure"].as_bool().unwrap_or(false);
        match (&result, failure) {
            (Err(ParseError::UnsupportedScheme), _) => {
                assert!(
                    input.to_ascii_lowercase().contains("file")
                        || base.is_some_and(|b| b.starts_with("file"))
                        || !input.contains(':'),
                    "only file URLs may be refused: {input:?} base {base:?}"
                );
                refused += 1;
            }
            (Err(_), true) => ok += 1,
            (Ok(u), false) => {
                let got = components(u);
                let want = [
                    case["href"].as_str().unwrap(),
                    case["protocol"].as_str().unwrap(),
                    case["username"].as_str().unwrap(),
                    case["password"].as_str().unwrap(),
                    case["host"].as_str().unwrap(),
                    case["hostname"].as_str().unwrap(),
                    case["port"].as_str().unwrap(),
                    case["pathname"].as_str().unwrap(),
                    case["search"].as_str().unwrap(),
                ];
                assert!(
                    got.iter().map(String::as_str).eq(want.iter().copied()),
                    "{input:?} base {base:?}: got {got:?} want {want:?}"
                );
                ok += 1;
            }
            (Ok(u), true) => {
                panic!("{input:?} base {base:?}: accepted {u} but the standard rejects it")
            }
            (Err(e), false) => {
                assert!(
                    LENIENT_PUNYCODE.contains(&input),
                    "{input:?} base {base:?}: rejected {e:?}"
                );
                lenient.push(input.to_string());
            }
        }
    }
    assert_eq!(lenient.len(), LENIENT_PUNYCODE.len());
    assert!(ok >= 748 && refused >= 135, "ok {ok} refused {refused}");
}
