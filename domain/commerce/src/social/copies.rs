//! Copied text among captured posts. A post repeated by one account, or a headline pasted by
//! several, is one text however many posts carry it: copies are grouped and counted beside the
//! mentions, never removed from them, and never proof that the remaining texts are independent.
//!
//! Live (Bluesky "matcha", 259 posts, 2026-10-07): 11 exact copies and 35 near-copy pairs, 24 of
//! them from one account; one account's "vibe with coffee matcha nottingham" posts formed a
//! six-post cluster that became a hypothesis of its own.

use super::SocialPost;
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// Character 5-gram Jaccard at or above which two texts are the same text.
pub const COPY_JACCARD: f64 = 0.8;
/// Share of the shorter text's 5-grams found in the longer at or above which the shorter is a
/// cut copy (a headline pasted without its tail), for texts of at least `CONTAINED_MIN_CHARS`.
pub const COPY_CONTAINMENT: f64 = 0.9;
pub const CONTAINED_MIN_CHARS: usize = 40;
/// Texts shorter than this after normalisation are too short to call copies of anything.
pub const MIN_CHARS: usize = 12;
pub const METHOD: &str =
    "NORMALIZED_TEXT_CHAR5_JACCARD_0.8_OR_CONTAINMENT_0.9_MIN_40_CHARS_LINKS_REMOVED";

/// Lowercased words without links, punctuation or spacing differences.
pub fn normalized(text: &str) -> String {
    text.split_whitespace()
        .filter(|w| {
            let w = w.to_lowercase();
            !(w.starts_with("http") || w.starts_with("www.") || w.contains('/'))
        })
        .flat_map(|w| {
            w.chars()
                .map(|c| if c.is_alphanumeric() { c } else { ' ' })
                .chain([' '])
                .collect::<Vec<_>>()
        })
        .collect::<String>()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn grams(text: &str) -> BTreeSet<String> {
    let chars: Vec<char> = text.chars().collect();
    chars.windows(5).map(|w| w.iter().collect()).collect()
}

pub fn author(p: &SocialPost) -> String {
    p.author_id.clone().unwrap_or_else(|| p.source_url.clone())
}

/// Groups of post indexes carrying one text (singletons included), in first-seen order.
pub fn groups(posts: &[&SocialPost]) -> Vec<Vec<usize>> {
    let texts: Vec<String> = posts.iter().map(|p| normalized(&p.text)).collect();
    let g: Vec<BTreeSet<String>> = texts.iter().map(|t| grams(t)).collect();
    let mut parent: Vec<usize> = (0..posts.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for i in 0..posts.len() {
        for j in i + 1..posts.len() {
            let (a, b) = (&texts[i], &texts[j]);
            if a.chars().count() < MIN_CHARS || b.chars().count() < MIN_CHARS {
                continue;
            }
            let same = a == b || {
                let shared = g[i].intersection(&g[j]).count() as f64;
                let union = g[i].union(&g[j]).count() as f64;
                let shorter = g[i].len().min(g[j].len()) as f64;
                (union > 0. && shared / union >= COPY_JACCARD)
                    || (a.chars().count().min(b.chars().count()) >= CONTAINED_MIN_CHARS
                        && shorter > 0.
                        && shared / shorter >= COPY_CONTAINMENT)
            };
            if same {
                let (ri, rj) = (root(&mut parent, i), root(&mut parent, j));
                parent[ri.max(rj)] = ri.min(rj);
            }
        }
    }
    let mut out: Vec<Vec<usize>> = vec![];
    let mut at: Vec<Option<usize>> = vec![None; posts.len()];
    for i in 0..posts.len() {
        let r = root(&mut parent, i);
        match at[r] {
            Some(k) => out[k].push(i),
            None => {
                at[r] = Some(out.len());
                out.push(vec![i]);
            }
        }
    }
    out
}

pub fn distinct_authors(posts: &[&SocialPost]) -> usize {
    posts
        .iter()
        .map(|p| author(p))
        .collect::<BTreeSet<_>>()
        .len()
}

/// The snapshot's copy summary: distinct texts beside the mention count, and each copy group.
pub fn summary(posts: &[&SocialPost]) -> Value {
    let all = groups(posts);
    let copied: Vec<Value> = all
        .iter()
        .filter(|g| g.len() > 1)
        .map(|g| {
            let members: Vec<&SocialPost> = g.iter().map(|&i| posts[i]).collect();
            let authors = distinct_authors(&members);
            json!({"posts":g.len(),"distinct_authors":authors,"kind":if authors == 1 {"ONE_ACCOUNT_REPEATING"} else {"TEXT_SHARED_ACROSS_ACCOUNTS"},"evidence_ids":members.iter().map(|p| p.evidence_id.clone()).collect::<Vec<_>>()})
        })
        .collect();
    json!({"method":METHOD,"posts":posts.len(),"distinct_texts":all.len(),"posts_in_copy_groups":copied.iter().filter_map(|g| g["posts"].as_u64()).sum::<u64>(),"copy_groups":copied,"state":"DERIVED","independence":"NOT_VERIFIED_DISTINCT_TEXTS_MAY_STILL_SHARE_AN_ORIGIN"})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post(id: &str, author: &str, text: &str) -> SocialPost {
        serde_json::from_value(json!({"platform":"BLUESKY","provider":"test","source_url":format!("https://example.org/{id}"),"native_id":id,"thread_id":null,"author_id":author,"publisher":null,"published_at":9000,"captured_at":10000,"text":text,"language":"en","media":[],"hashtags":[],"mentions":[],"entities":[],"propagation":"ORIGINAL","parent_id":null,"engagement":{"views":null,"likes":null,"comments":null,"reposts":null,"favorites":null,"followers":null},"raw_hash":"a".repeat(64),"raw_locator":"/0","extraction_method":"FIXTURE","state":"OBSERVED","capture_mode":"FIXTURE","evidence_id":format!("e{id}"),"freshness_seconds":1000,"origin_evidence_id":null})).unwrap()
    }

    #[test]
    fn copies_are_grouped_and_counted_never_removed() {
        let posts = [
            post(
                "1",
                "bot",
                "vibe with coffee matcha nottingham https://x.example/a",
            ),
            post("2", "bot", "Matcha vibe with coffee — matcha Nottingham!"),
            post(
                "3",
                "ann",
                "gratis matcha beelden van eendenkroos in gentse wateren halen bbc www.vrt.be/vrtnws/nl/2026",
            ),
            post(
                "4",
                "bob",
                "Gratis matcha: beelden van eendenkroos in Gentse wateren halen (BBC)",
            ),
            post("5", "cy", "you know it's the matcha cry"),
            post("6", "cy", "pain you know it's the matcha argue"),
            post("7", "dee", "an iced matcha latte with oat milk, finally"),
        ];
        let refs: Vec<&SocialPost> = posts.iter().collect();
        let g = groups(&refs);
        assert_eq!(g, vec![vec![0, 1], vec![2, 3], vec![4], vec![5], vec![6]]);
        let s = summary(&refs);
        assert_eq!(s["posts"], 7);
        assert_eq!(s["distinct_texts"], 5);
        assert_eq!(s["posts_in_copy_groups"], 4);
        assert_eq!(s["copy_groups"][0]["kind"], "ONE_ACCOUNT_REPEATING");
        assert_eq!(s["copy_groups"][1]["kind"], "TEXT_SHARED_ACROSS_ACCOUNTS");
        // Short texts are never called copies, even when equal.
        let short = [post("8", "a", "matcha!"), post("9", "b", "Matcha")];
        let r: Vec<&SocialPost> = short.iter().collect();
        assert_eq!(groups(&r).len(), 2);
    }
}
