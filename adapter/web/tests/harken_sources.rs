//! ECDEV's native Hacker News and Bluesky normalisation against frozen outputs of the pinned
//! harken (MIT, VladUZH/harken at d0710a42). Compared on what both report: canonical URL, author,
//! score and text. Deliberate differences are counted, never hidden: harken drops items lacking
//! the keyword (ECDEV keeps every returned item as an observation of the query), stamps the
//! current time on items without a timestamp (ECDEV leaves it UNKNOWN), and prefers Bluesky's
//! server indexedAt over the author's createdAt (ECDEV keeps createdAt).
use serde_json::Value;

fn oracle() -> Value {
    serde_json::from_str(include_str!("fixtures/harken-sources-oracle.json")).unwrap()
}

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn epoch(iso: &str) -> u64 {
    let v = Value::from(iso);
    ecdev_web::social::published(&v).unwrap()
}

#[test]
fn harken_hn_and_bluesky_normalisation_oracle() {
    let (mut compared, mut fabricated_now, mut indexed_at, mut keyword_dropped) = (0, 0, 0, 0);
    for case in oracle()["cases"].as_array().unwrap() {
        let platform = case["platform"].as_str().unwrap();
        let raw = serde_json::to_vec(&case["raw"]).unwrap();
        let posts = ecdev_web::social::normalize(
            &case["raw"],
            platform,
            "https://fixture.example/",
            &raw,
            1_900_000_000,
            "FIXTURE",
        )
        .unwrap();
        let items = case["raw"][if platform == "HACKER_NEWS" {
            "hits"
        } else {
            "posts"
        }]
        .as_array()
        .unwrap();
        let mentions = case["mentions"].as_array().unwrap();
        keyword_dropped += items.len() - mentions.len();
        for m in mentions {
            let url = m["url"].as_str().unwrap();
            let post = posts
                .iter()
                .find(|p| p.source_url == url)
                .unwrap_or_else(|| panic!("{}: {url}", case["case_id"]));
            assert_eq!(
                post.engagement
                    .likes
                    .map(Value::from)
                    .unwrap_or(Value::Null),
                m["score"],
                "{}",
                case["case_id"]
            );
            let item = items
                .iter()
                .find(|i| {
                    if platform == "HACKER_NEWS" {
                        url.ends_with(&format!("id={}", i["objectID"].as_str().unwrap()))
                    } else {
                        url.ends_with(i["uri"].as_str().unwrap().rsplit('/').next().unwrap())
                    }
                })
                .unwrap();
            if platform == "HACKER_NEWS" {
                assert_eq!(
                    post.author_id.as_deref(),
                    m["author"].as_str(),
                    "{}",
                    case["case_id"]
                );
                let title = m["title"].as_str().unwrap_or("");
                assert_eq!(
                    squash(&post.text),
                    squash(&format!("{title} {}", m["text"].as_str().unwrap())),
                    "{}",
                    case["case_id"]
                );
                match item["created_at_i"].as_u64() {
                    Some(ts) => assert_eq!(post.published_at, Some(ts), "{}", case["case_id"]),
                    None => {
                        assert_eq!(post.published_at, None);
                        fabricated_now += 1;
                    }
                }
            } else {
                assert!(
                    url.contains(m["author"].as_str().unwrap()),
                    "{}",
                    case["case_id"]
                );
                assert_eq!(
                    post.text,
                    m["text"].as_str().unwrap(),
                    "{}",
                    case["case_id"]
                );
                let created = item["record"]["createdAt"].as_str();
                if item["indexedAt"].is_string() {
                    indexed_at += 1;
                    assert_eq!(post.published_at, created.map(epoch));
                } else if let Some(created) = created {
                    assert_eq!(
                        post.published_at,
                        Some(epoch(created)),
                        "{}",
                        case["case_id"]
                    );
                    assert_eq!(epoch(m["created_at"].as_str().unwrap()), epoch(created));
                } else {
                    assert_eq!(post.published_at, None);
                    fabricated_now += 1;
                }
            }
            compared += 1;
        }
    }
    // Pinned divergence counts for the frozen corpus.
    assert_eq!(
        (compared, fabricated_now, indexed_at, keyword_dropped),
        (526, 63, 185, 236)
    );
}
