//! The Public Suffix List project's own test vectors (tests/tests.txt, CC0), run against
//! ECDEV's native matcher over the vendored list.
use ecdev_core::public_suffix::registrable_domain;

#[test]
fn official_public_suffix_test_vectors_pass() {
    let text = include_str!("fixtures/public-suffix-tests.txt");
    let (mut run, mut failed) = (0, vec![]);
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        let mut parts = line.split_whitespace();
        let (input, expected) = (parts.next().unwrap(), parts.next().unwrap());
        let input = if input == "null" { "" } else { input };
        let expected = if expected == "null" {
            None
        } else {
            Some(expected.to_string())
        };
        // Expected values are compared in the ASCII (punycode) form ECDEV returns.
        let got = registrable_domain(input);
        let want = expected.map(|e| {
            url::Url::parse(&format!("http://{e}/"))
                .map(|u| u.host_str().unwrap().to_string())
                .unwrap_or(e)
        });
        run += 1;
        if got != want {
            failed.push(format!("{input}: got {got:?}, want {want:?}"));
        }
    }
    assert!(run > 50, "{run}");
    assert!(
        failed.is_empty(),
        "{} of {run} failed:\n{}",
        failed.len(),
        failed.join("\n")
    );
}
