//! ECDEV's native redirect decision against frozen decisions of the pinned httpx (BSD-3-Clause,
//! encode/httpx at b5addb64): which responses redirect, and where. Compared on the target with
//! the fragment removed (ECDEV URL identity carries no fragment; httpx re-attaches the previous
//! one, RFC 7231 7.1.2). Deliberate differences are counted, never hidden: ECDEV holds every
//! redirect target to its public-URL policy (httpx follows credentials, credential queries and
//! non-http schemes), and refuses a Location that clients resolve differently where httpx
//! guesses (empty authority, scheme without authority, backslashes, invalid host or port).
use ecdev_web::redirect_target;
use serde_json::Value;
use url::Url;

fn oracle() -> Value {
    serde_json::from_str(include_str!("fixtures/httpx-redirect-oracle.json")).unwrap()
}

/// httpx's target without its fragment, in WHATWG serialization (an empty path is `/`; the
/// resource is the same).
fn without_fragment(u: &str) -> String {
    let bare = u.split('#').next().unwrap();
    Url::parse(bare).map_or(bare.to_string(), |u| u.to_string())
}

#[test]
fn httpx_redirect_decision_oracle() {
    let o = oracle();
    assert_eq!(o["commit_sha"], "b5addb64f0161ff6bfe94c124ef76f6a1fba5254");
    let (mut final_both, mut same_target, mut both_refuse) = (0, 0, 0);
    let (mut policy_refused, mut ambiguous_refused) = (0, 0);
    let mut policy = std::collections::BTreeSet::new();
    let mut ambiguous = std::collections::BTreeSet::new();
    let mut unbuildable = std::collections::BTreeSet::new();
    for c in o["cases"].as_array().unwrap() {
        let base = Url::parse(c["base"].as_str().unwrap()).unwrap();
        let status = c["status"].as_u64().unwrap() as u16;
        let location = c["location"].as_str();
        let ours = redirect_target(&base, status, location);
        let id = format!("{} {status} {location:?}", c["base"]);
        match (c["follows"].as_bool(), c["target"].as_str()) {
            (Some(false), _) => {
                assert_eq!(ours, Ok(None), "{id}: httpx does not redirect");
                final_both += 1;
            }
            (Some(true), Some(target)) => match ours {
                Ok(Some(next)) => {
                    assert_eq!(next, without_fragment(target), "{id}");
                    same_target += 1;
                }
                Ok(None) => panic!("{id}: httpx redirects to {target}, ECDEV stops"),
                Err(e) => {
                    let t = Url::parse(target).ok();
                    let by_policy = t.as_ref().is_some_and(|t| {
                        !matches!(t.scheme(), "http" | "https")
                            || !t.username().is_empty()
                            || t.query_pairs().any(|(k, _)| k == "token")
                    });
                    if by_policy {
                        assert!(
                            e == "PUBLIC_HTTP_URL_REQUIRED" || e == "CREDENTIAL_QUERY_DENIED",
                            "{id}: {e}"
                        );
                        policy.insert(location.unwrap().to_string());
                        policy_refused += 1;
                    } else {
                        assert_eq!(e, "INVALID_REDIRECT", "{id}");
                        ambiguous.insert(location.unwrap().to_string());
                        ambiguous_refused += 1;
                    }
                }
            },
            // httpx could not build a target (or a response): ECDEV never follows one either.
            _ => {
                assert!(ours.is_err() || ours == Ok(None), "{id}: {ours:?}");
                unbuildable.insert(location.unwrap_or("<none>").to_string());
                both_refuse += 1;
            }
        }
    }
    assert_eq!(
        (
            final_both,
            same_target,
            both_refuse,
            policy_refused,
            ambiguous_refused
        ),
        (975, 650, 150, 75, 150)
    );
    let v = |s: std::collections::BTreeSet<String>| s.into_iter().collect::<Vec<_>>();
    assert_eq!(
        v(policy),
        vec![
            "ftp://example.com/f",
            "https://example.com/?token=abc",
            "https://user:pw@example.com/"
        ],
        "refused by the public-URL policy"
    );
    assert_eq!(
        v(ambiguous),
        vec![
            "//",
            "///triple",
            "\\\\backslash\\\\path",
            "http://exa mple.com/",
            "http:/nohost",
            "https://example.com:99999/"
        ],
        "refused instead of guessed"
    );
    assert_eq!(
        v(unbuildable),
        vec![
            "/ü/path",
            "https://bücher.example/p",
            "https:path",
            "javascript:alert(1)"
        ],
        "httpx builds no target (non-ASCII Location, scheme without authority)"
    );
}

#[test]
fn redirect_targets_obey_the_requested_url_policy() {
    let base = Url::parse("https://www.example.com/a?x=1").unwrap();
    let t = |s: u16, l: &str| redirect_target(&base, s, Some(l));
    assert_eq!(
        t(301, "/b#frag"),
        Ok(Some("https://www.example.com/b".into()))
    );
    assert_eq!(t(300, "/b"), Ok(None), "300 is not a redirect");
    assert_eq!(
        t(302, "https://u:p@example.com/").unwrap_err(),
        "PUBLIC_HTTP_URL_REQUIRED"
    );
    assert_eq!(
        t(307, "/x?api_key=1").unwrap_err(),
        "CREDENTIAL_QUERY_DENIED"
    );
    assert_eq!(
        t(308, "file:///etc/passwd").unwrap_err(),
        "PUBLIC_HTTP_URL_REQUIRED"
    );
    assert_eq!(
        t(308, "javascript:alert(1)").unwrap_err(),
        "INVALID_REDIRECT"
    );
    assert_eq!(
        t(302, "\\\\evil.example\\x").unwrap_err(),
        "INVALID_REDIRECT"
    );
    assert_eq!(t(302, "http:/elsewhere").unwrap_err(), "INVALID_REDIRECT");
    assert_eq!(redirect_target(&base, 302, None), Ok(None));
}
