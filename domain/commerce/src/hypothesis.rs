//! Social evidence → product hypotheses → research actions. A trend is a reason to look, never
//! evidence of demand: mention counts are not sales, and cross-platform recurrence is not
//! independent market validation. Hypotheses can only lead to research; no hypothesis is
//! shortlist-eligible on social evidence alone.
use crate::social::terms;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

/// Commerce evidence a social signal can never supply; each must come from other sources.
pub const BLOCKERS: &[&str] = &[
    "SOCIAL_SIGNAL_IS_NOT_DEMAND",
    "NO_LISTING_EVIDENCE",
    "NO_PRICE_EVIDENCE",
    "NO_SECOND_LISTING_ORIGIN",
    "NO_SUPPLIER_EVIDENCE",
    "NO_ECONOMICS",
];

fn title_terms(candidate: &Value) -> BTreeSet<String> {
    candidate["product"]["title"]
        .as_str()
        .map(terms)
        .unwrap_or_default()
}

/// The hypotheses one frozen trend snapshot supports, with links to captured candidates whose
/// titles contain every topic term (unverified) and research actions ordered by how many open
/// blockers each could resolve.
/// One hypothesis source: its terms, evidence ids, platforms, candidate phrases and refinements.
type Group = (BTreeSet<String>, Value, Value, Value, Vec<String>);

fn union(into: &mut Value, from: &Value, same: impl Fn(&Value, &Value) -> bool) {
    let Some(into) = into.as_array_mut() else {
        return;
    };
    for v in from.as_array().into_iter().flatten() {
        if !into.iter().any(|x| same(x, v)) {
            into.push(v.clone());
        }
    }
}

/// Lexical clusters split one product across several clusters (recall is low by design, see
/// research/commerce/cluster-rule-benchmark.json): hypotheses that lead with the same product
/// query are one hypothesis, with their evidence, phrases and refinements joined. Live, four
/// clusters each led with "matcha latte".
fn merge_same_lead(hypotheses: Vec<Value>) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for h in hypotheses {
        let lead = h["product_queries"][0].clone();
        let refines = lead.as_str() != h["topic"].as_str().map(str::trim);
        match out
            .iter_mut()
            .find(|o| refines && o["product_queries"][0] == lead)
        {
            Some(o) => {
                union(&mut o["evidence_ids"], &h["evidence_ids"], |a, b| a == b);
                union(&mut o["platforms"], &h["platforms"], |a, b| a == b);
                union(
                    &mut o["refinement_terms"],
                    &h["refinement_terms"],
                    |a, b| a == b,
                );
                union(&mut o["product_queries"], &h["product_queries"], |a, b| {
                    a == b
                });
                union(&mut o["phrase_queries"], &h["phrase_queries"], |a, b| {
                    a["phrase"] == b["phrase"]
                });
                union(
                    &mut o["linked_candidates"],
                    &h["linked_candidates"],
                    |a, b| a["candidate_id"] == b["candidate_id"],
                );
                let queries = o["product_queries"].clone();
                for a in o["research_actions"].as_array_mut().into_iter().flatten() {
                    if a.get("alternative_queries").is_some() {
                        a["alternative_queries"] = queries.clone();
                    }
                }
                o["merged_clusters"] = json!(o["merged_clusters"].as_u64().unwrap_or(1) + 1);
            }
            None => out.push(h),
        }
    }
    out
}

fn post_keys(snapshot: &Value, evidence: &Value) -> BTreeSet<String> {
    let by_evidence: std::collections::BTreeMap<&str, &str> = snapshot["evidence_ids"]
        .as_array()
        .into_iter()
        .flatten()
        .zip(snapshot["post_keys"].as_array().into_iter().flatten())
        .filter_map(|(e, k)| Some((e.as_str()?, k.as_str()?)))
        .collect();
    evidence
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| by_evidence.get(e.as_str()?).map(|k| k.to_string()))
        .collect()
}

/// Each hypothesis against the previous snapshot of its query: whether the same lead (the same
/// id) was there, and how many of its posts that snapshot had not captured. Only those are new
/// evidence; posts captured before are the same evidence seen again.
pub fn mark_recurrence(
    out: &mut Value,
    snapshot: &Value,
    prior: Option<&Value>,
    candidates: &[Value],
) {
    let Some(prior) = prior else {
        for h in out["hypotheses"].as_array_mut().into_iter().flatten() {
            h["recurrence"] = json!({"state":"NO_PRIOR_SNAPSHOT"});
        }
        return;
    };
    let before = hypothesize(prior, candidates);
    let prior_ids: BTreeSet<&str> = before["hypotheses"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|h| h["hypothesis_id"].as_str())
        .collect();
    let prior_posts: BTreeSet<&str> = prior["post_keys"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    for h in out["hypotheses"].as_array_mut().into_iter().flatten() {
        let keys = post_keys(snapshot, &h["evidence_ids"]);
        let new = keys
            .iter()
            .filter(|k| !prior_posts.contains(k.as_str()))
            .count();
        let seen = h["hypothesis_id"]
            .as_str()
            .is_some_and(|i| prior_ids.contains(i));
        let state = match (seen, new) {
            (false, _) => "NEW_LEAD_SINCE_PRIOR_SNAPSHOT",
            (true, 0) => "RECURRING_SAME_POSTS_ONLY",
            (true, _) => "RECURRING_WITH_POSTS_NOT_IN_PRIOR_SNAPSHOT",
        };
        h["recurrence"] = json!({"state":state,"prior_snapshot_id":prior["snapshot_id"],"prior_captured_at":prior["captured_at"],"posts":keys.len(),"posts_not_in_prior_snapshot":new,"basis":"SAME_LEAD_QUERY_ID_AND_POST_KEYS","note":"a post the prior snapshot did not capture may be newly published or newly reached"});
    }
}

pub fn hypothesize(snapshot: &Value, candidates: &[Value]) -> Value {
    let query = snapshot["query"].as_str().unwrap_or("");
    let topic = terms(query);
    // One hypothesis per cluster of two or more posts; a single post is no pattern. Refinements
    // are the cluster's salient terms (shared by two or more of its posts, informative), most
    // shared first; snapshots from before salient terms existed fall back to their terms.
    let mut groups: Vec<Group> = Vec::new();
    // One account repeating itself, or one text pasted by several, is no pattern either: such
    // clusters are reported as excluded, with why. Snapshots from before these counts pass.
    let mut excluded: Vec<Value> = Vec::new();
    for c in snapshot["clusters"].as_array().into_iter().flatten() {
        if c["evidence_ids"].as_array().is_none_or(|e| e.len() < 2) {
            continue;
        }
        let reason = if c["distinct_authors"].as_u64() == Some(1) {
            Some("ONE_ACCOUNT_REPEATING")
        } else if c["distinct_texts"].as_u64() == Some(1) {
            Some("ONE_TEXT_COPIED_ACROSS_ACCOUNTS")
        } else {
            None
        };
        if let Some(reason) = reason {
            excluded.push(json!({"cluster_id":c["id"],"posts":c["evidence_ids"].as_array().map_or(0, Vec::len),"distinct_authors":c["distinct_authors"],"distinct_texts":c["distinct_texts"],"reason":reason,"evidence_ids":c["evidence_ids"]}));
            continue;
        }
        let t: BTreeSet<String> = c["terms"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_lowercase)
            .collect();
        let salient: Vec<String> = match c["salient_terms"].as_array() {
            Some(rows) => rows
                .iter()
                .filter_map(|r| r["term"].as_str())
                .filter(|term| !topic.contains(*term))
                .take(5)
                .map(str::to_string)
                .collect(),
            None => t.difference(&topic).take(5).cloned().collect(),
        };
        // A cluster that refines nothing (no salient term, no phrase) repeats the topic.
        if salient.is_empty()
            && c["candidate_phrases"]
                .as_array()
                .is_none_or(|p| p.is_empty())
        {
            continue;
        }
        groups.push((
            t,
            c["evidence_ids"].clone(),
            c["platforms"].clone(),
            c["candidate_phrases"].clone(),
            salient,
        ));
    }
    // The topic itself is always a hypothesis over every mention, first; clusters refine it.
    // Without it, mentions outside multi-post clusters would be represented by nothing.
    if snapshot["mention_count"].as_u64().unwrap_or(0) > 0 {
        groups.insert(
            0,
            (
                topic.clone(),
                snapshot["evidence_ids"].clone(),
                snapshot["platforms"].clone(),
                Value::Null,
                vec![],
            ),
        );
    }
    // Source-reported attention counters (a Mastodon tag's daily uses): kept beside the post
    // signal, never added to its mentions. A rising counter alone is a reason to research.
    let attention: Vec<Value> = snapshot["source_counters"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| json!({"platform":c["platform"],"tag":c["tag"],"scope":c["scope"],"growth_state":c["growth"]["state"],"account_days":{"before":c["growth"]["account_days"]["before"],"after":c["growth"]["account_days"]["after"]},"raw_hash":c["raw_hash"],"evidence_class":"ATTENTION_SIGNAL_NOT_DEMAND"}))
        .collect();
    let attention_rising = attention.iter().any(|a| a["growth_state"] == "RISING");
    if groups.is_empty() && attention_rising {
        groups.push((topic.clone(), json!([]), json!([]), Value::Null, vec![]));
    }
    let signal = snapshot["state"].as_str().unwrap_or("UNKNOWN");
    // How complete the population behind the signal is: a separate dimension, never a weight on
    // the mention metrics. Snapshots from before coverage was recorded say so.
    let evidence_completeness = if snapshot["population_evidence"].is_object() {
        json!({"grade":snapshot["population_evidence"]["grade"],"sources":snapshot["population_evidence"]["sources"],"min_temporal_span_coverage":snapshot["population_evidence"]["min_temporal_span_coverage"],"items_observed":snapshot["population_evidence"]["items_observed"]})
    } else {
        json!({"grade":"NOT_RECORDED_FOR_THIS_SNAPSHOT"})
    };
    let mut hypotheses = Vec::new();
    for (cluster_terms, evidence, platforms, phrases, refinements) in groups {
        // Recurring phrases are product-name candidates: a phrase holding every topic term is
        // a query by itself, any other is appended to the topic. Unverified until listings match.
        let phrase_queries: Vec<Value> = phrases
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| {
                let phrase = p["phrase"].as_str()?;
                let q = if topic.is_subset(&terms(phrase)) { phrase.to_string() } else { format!("{} {phrase}", query.trim()) };
                Some(json!({"query":q,"phrase":phrase,"posts":p["posts"],"distinct_authors":p["distinct_authors"],"evidence_ids":p["evidence_ids"],"state":"CANDIDATE_PRODUCT_PHRASE_UNVERIFIED"}))
            })
            .collect();
        // Most specific first, so an action's own query is the hypothesis's, not the topic's:
        // recurring phrases, then refinements no phrase already holds, then the topic alone.
        // More authors, then more posts, then a phrase naming the topic itself leads: among
        // equals, "matcha latte" says more about matcha than "ingrown nail" from the same posts.
        let count = |p: &Value, k: &str| std::cmp::Reverse(p[k].as_u64().unwrap_or(0));
        let names_topic = |p: &Value| {
            p["phrase"]
                .as_str()
                .is_some_and(|x| !topic.is_disjoint(&terms(x)))
        };
        let mut phrase_queries = phrase_queries;
        phrase_queries.sort_by_key(|p| {
            (
                count(p, "distinct_authors"),
                count(p, "posts"),
                !names_topic(p),
            )
        });
        let phrase_terms: BTreeSet<String> = phrase_queries
            .iter()
            .filter_map(|p| p["phrase"].as_str())
            .flat_map(terms)
            .collect();
        let mut queries: Vec<String> = vec![];
        let refined = refinements
            .iter()
            .filter(|r| !phrase_terms.contains(r.as_str()))
            .map(|r| format!("{} {r}", query.trim()));
        for q in phrase_queries
            .iter()
            .filter_map(|p| p["query"].as_str().map(str::to_string))
            .chain(refined)
            .chain([query.trim().to_string()])
        {
            if !queries.contains(&q) {
                queries.push(q);
            }
        }
        // Identity follows the hypothesis's own lead query, not this run's refinement terms, so
        // the same lead found again in a later snapshot is the same hypothesis.
        let lead = queries.first().map(String::as_str).unwrap_or_default();
        let id = format!(
            "hyp-{}",
            &format!(
                "{:x}",
                Sha256::digest(
                    format!("{query}|{}", if lead == query.trim() { "" } else { lead }).as_bytes()
                )
            )[..16]
        );
        let linked: Vec<Value> = candidates
            .iter()
            .filter(|c| !topic.is_empty() && topic.is_subset(&title_terms(c)))
            .map(|c| {
                let shared: Vec<String> = title_terms(c).intersection(&cluster_terms).cloned().collect();
                json!({"candidate_id":c["id"],"title":c["product"]["title"],"link_state":"UNVERIFIED_TOPIC_LINK","basis":"ALL_TOPIC_TERMS_IN_CAPTURED_TITLE","shared_cluster_terms":shared,"candidate_state":c["state"],"source":c["source"],"asin":c["product"]["asin"]})
            })
            .collect();
        let mut blockers: Vec<&str> = BLOCKERS.to_vec();
        if !linked.is_empty() {
            blockers.retain(|b| *b != "NO_LISTING_EVIDENCE");
        }
        let gain = |fills: &[&str]| fills.iter().filter(|f| blockers.contains(f)).count();
        let mut actions = vec![
            json!({"kind":"PUBLIC_LISTING_SEARCH","tool":"ecdev.research.run","input_template":{"query":queries[0],"market":"PUBLIC_WEB","seed_urls":"OPERATOR_SUPPLIED_PUBLIC_CATALOG_OR_SEARCH_URLS_REQUIRED"},"alternative_queries":queries,"fills":["NO_LISTING_EVIDENCE","NO_PRICE_EVIDENCE","NO_SECOND_LISTING_ORIGIN"],"paid":false}),
            json!({"kind":"OFFICIAL_LISTING_SEARCH","tool":"ecdev.listing.search","input_template":{"market":"YAHOO_SHOPPING_JP","query":queries[0]},"alternative_queries":queries,"fills":["NO_LISTING_EVIDENCE","NO_PRICE_EVIDENCE"],"paid":false,"requires":"YAHOO_SHOPPING_APP_ID (operator Client ID); Japan market; provider-reported listings, sellers are not independent sites"}),
            json!({"kind":"PUBLIC_STORE_PRICES","tool":"ecdev.price.observations","input_template":{"query":queries[0]},"alternative_queries":queries,"fills":[],"improves":["REFERENCE_STORE_PRICE","CROSS_SOURCE_PRODUCT_IDENTIFIER"],"scope":"crowdsourced shelf prices, mostly packaged food and grocery; not online listings","paid":false}),
            json!({"kind":"SUPPLIER_SEARCH","tool":"ecdev.research.run","input_template":{"query":format!("{} wholesale supplier",queries[0]),"market":"PUBLIC_WEB","seed_urls":"OPERATOR_SUPPLIED_SUPPLIER_DIRECTORY_URLS_REQUIRED"},"fills":["NO_SUPPLIER_EVIDENCE"],"paid":false}),
            json!({"kind":"TREND_PERSISTENCE_WATCH","tool":"ecdev.trend.watch","input_template":{"query":query,"interval_seconds":86400},"fills":[],"improves":["persistence","velocity","acceleration"],"paid":false}),
        ];
        for l in &linked {
            if let Some(asin) = l["asin"].as_str() {
                actions.push(json!({"kind":"OFFICIAL_CATALOG_AND_FEES","tool":"ecdev.product.analyze","input_template":{"asin":asin,"market":"AMAZON_JP_OR_AMAZON_US","evidence_layer":"OFFICIAL_SP_API","include":["CATALOG","OFFERS","FEE_ESTIMATE"]},"fills":["NO_PRICE_EVIDENCE","NO_ECONOMICS"],"paid":false,"requires":"SP-API credentials and operator gate"}));
            }
        }
        for a in &mut actions {
            let fills: Vec<&str> = a["fills"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            a["expected_information_gain"] = json!({"open_blockers_addressed":gain(&fills),"method":"COUNT_OF_OPEN_BLOCKERS_THE_ACTION_CAN_RESOLVE"});
        }
        actions.sort_by_key(|a| {
            std::cmp::Reverse(
                a["expected_information_gain"]["open_blockers_addressed"]
                    .as_u64()
                    .unwrap_or(0),
            )
        });
        let basis = if evidence.as_array().is_some_and(|e| !e.is_empty()) {
            "SOCIAL_POSTS"
        } else {
            "ATTENTION_COUNTER_ONLY"
        };
        hypotheses.push(json!({"hypothesis_id":id,"topic":query,"topic_terms":topic,"refinement_terms":refinements,"product_queries":queries,"state":"HYPOTHESIS","social_signal_state":signal,"evidence_ids":evidence,"platforms":platforms,"linked_candidates":linked,"phrase_queries":phrase_queries,"shortlist_eligible":false,"blocked_by":blockers,"research_actions":actions,"evidence_completeness":evidence_completeness.clone(),"attention_counters":attention,"basis":basis}));
    }
    let hypotheses = merge_same_lead(hypotheses);
    json!({"snapshot_query":query,"capture_mode":snapshot["capture_mode"],"hypotheses":hypotheses,"excluded_clusters":excluded,"invariants":["trend != demand","mention count != sales","cross-platform mention != independent market validation","a hypothesis is a reason to research, never a shortlist","population completeness qualifies a signal and never raises its mention counts","an attention counter is attention, never a mention, a sale or demand","one account repeating itself, or one text copied, is one voice, not a pattern"],"network_calls":0})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> Value {
        json!({"query":"matcha whisk","capture_mode":"FIXTURE","state":"EMERGING","mention_count":3,"evidence_ids":["e1","e2","e3"],"platforms":["BLUESKY","HACKER_NEWS"],
            "clusters":[{"terms":["matcha","whisk","bamboo","chasen"],"evidence_ids":["e1","e2"],"platforms":["BLUESKY"]},{"terms":["matcha","whisk","electric"],"evidence_ids":["e3"],"platforms":["HACKER_NEWS"]}]})
    }

    #[test]
    fn social_trends_become_research_hypotheses_never_shortlists() {
        let candidates = vec![
            json!({"id":"c1","state":"VALIDATING","source":"https://shop.example/a","product":{"title":"Bamboo Matcha Whisk (Chasen)","asin":"B000000001"}}),
            json!({"id":"c2","state":"VALIDATING","source":"https://shop.example/b","product":{"title":"Matcha Bowl"}}),
        ];
        let out = hypothesize(&snapshot(), &candidates);
        let h = out["hypotheses"].as_array().unwrap();
        // The topic over every mention first, then the two-post cluster; the one-post cluster
        // (e3) is no pattern and yields no hypothesis of its own.
        assert_eq!(h.len(), 2);
        assert_eq!(h[0]["evidence_ids"], json!(["e1", "e2", "e3"]));
        assert_eq!(h[0]["refinement_terms"], json!([]));
        for x in h {
            assert_eq!(x["state"], "HYPOTHESIS");
            assert_eq!(x["shortlist_eligible"], false);
            assert!(
                x["blocked_by"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("SOCIAL_SIGNAL_IS_NOT_DEMAND"))
            );
            assert!(!x["research_actions"].as_array().unwrap().is_empty());
        }
        let first = &h[1];
        assert_eq!(first["refinement_terms"], json!(["bamboo", "chasen"]));
        assert_eq!(
            first["product_queries"],
            json!(["matcha whisk bamboo", "matcha whisk chasen", "matcha whisk"])
        );
        assert_eq!(
            first["research_actions"][0]["input_template"]["query"],
            "matcha whisk bamboo"
        );
        // Only the candidate containing every topic term links, and only as unverified.
        let linked = first["linked_candidates"].as_array().unwrap();
        assert_eq!(linked.len(), 1);
        assert_eq!(linked[0]["candidate_id"], "c1");
        assert_eq!(linked[0]["link_state"], "UNVERIFIED_TOPIC_LINK");
        assert!(
            !first["blocked_by"]
                .as_array()
                .unwrap()
                .contains(&json!("NO_LISTING_EVIDENCE"))
        );
        // An ASIN from a captured listing adds an official check; actions are ordered by gain.
        let actions = first["research_actions"].as_array().unwrap();
        assert!(
            actions
                .iter()
                .any(|a| a["kind"] == "OFFICIAL_CATALOG_AND_FEES")
        );
        let gains: Vec<u64> = actions
            .iter()
            .map(|a| {
                a["expected_information_gain"]["open_blockers_addressed"]
                    .as_u64()
                    .unwrap()
            })
            .collect();
        assert!(gains.windows(2).all(|w| w[0] >= w[1]), "{gains:?}");
        assert_eq!(out["network_calls"], 0);
    }

    #[test]
    fn the_most_supported_phrase_leads_and_ties_go_to_the_one_naming_the_topic() {
        let phrase = |p: &str, posts: u64, authors: u64| json!({"phrase":p,"posts":posts,"distinct_authors":authors,"evidence_ids":["e1","e2"]});
        let snap = json!({"query":"matcha","mention_count":2,"evidence_ids":["e1","e2"],"platforms":["BLUESKY"],
            "clusters":[{"terms":["matcha","latte","ingrown","nail"],"evidence_ids":["e1","e2"],"platforms":["BLUESKY"],
                "candidate_phrases":[phrase("ingrown nail",2,2),phrase("matcha latte",2,2),phrase("oat milk",3,3)]}]});
        let out = hypothesize(&snap, &[]);
        let h = out["hypotheses"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| !x["phrase_queries"].as_array().unwrap().is_empty())
            .unwrap();
        let order: Vec<&str> = h["phrase_queries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["phrase"].as_str().unwrap())
            .collect();
        assert_eq!(order, ["oat milk", "matcha latte", "ingrown nail"]);
        assert_eq!(
            h["research_actions"][0]["input_template"]["query"],
            "matcha oat milk"
        );
    }

    #[test]
    fn one_account_or_one_copied_text_is_no_pattern() {
        let cluster = |id: &str, authors: u64, texts: u64| json!({"id":id,"terms":["matcha","nottingham","vibe"],"salient_terms":[{"term":"nottingham"},{"term":"vibe"}],"evidence_ids":["e1","e2","e3"],"platforms":["BLUESKY"],"distinct_authors":authors,"distinct_texts":texts});
        let snap = json!({"query":"matcha","mention_count":9,"evidence_ids":["e1","e2","e3"],"platforms":["BLUESKY"],
            "clusters":[cluster("bot", 1, 2), cluster("pasted", 3, 1), cluster("real", 3, 3)]});
        let out = hypothesize(&snap, &[]);
        // The topic, then only the cluster with several voices and texts.
        assert_eq!(out["hypotheses"].as_array().unwrap().len(), 2);
        let ex = out["excluded_clusters"].as_array().unwrap();
        assert_eq!(
            ex.iter()
                .map(|e| (
                    e["cluster_id"].as_str().unwrap(),
                    e["reason"].as_str().unwrap()
                ))
                .collect::<Vec<_>>(),
            [
                ("bot", "ONE_ACCOUNT_REPEATING"),
                ("pasted", "ONE_TEXT_COPIED_ACROSS_ACCOUNTS")
            ]
        );
    }

    #[test]
    fn clusters_leading_with_one_product_are_one_hypothesis_with_a_stable_id() {
        let phrase = |p: &str, ids: Value| json!({"phrase":p,"posts":2,"distinct_authors":2,"evidence_ids":ids});
        let cluster = |id: &str, ids: Value, salient: &[&str]| {
            json!({"id":id,"terms":["matcha","latte"],"salient_terms":salient.iter().map(|t| json!({"term":t})).collect::<Vec<_>>(),"evidence_ids":ids.clone(),"platforms":["BLUESKY"],
            "candidate_phrases":[phrase("matcha latte", ids)]})
        };
        let snap = |clusters: Value| json!({"query":"matcha","mention_count":6,"evidence_ids":["e1","e2","e3","e4","e5","e6"],"platforms":["BLUESKY"],"clusters":clusters});
        let two = hypothesize(
            &snap(json!([
                cluster("a", json!(["e1", "e2"]), &["latte", "oat"]),
                cluster("b", json!(["e3", "e4"]), &["latte", "banana"])
            ])),
            &[],
        );
        let h = two["hypotheses"].as_array().unwrap();
        assert_eq!(h.len(), 2, "the topic and one matcha latte hypothesis");
        assert_eq!(h[1]["merged_clusters"], 2);
        assert_eq!(h[1]["evidence_ids"], json!(["e1", "e2", "e3", "e4"]));
        assert_eq!(h[1]["refinement_terms"], json!(["latte", "oat", "banana"]));
        // Another snapshot with other refinements finds the same lead: the same identity.
        let later = hypothesize(
            &snap(json!([cluster(
                "c",
                json!(["e5", "e6"]),
                &["latte", "maple"]
            )])),
            &[],
        );
        assert_eq!(
            later["hypotheses"][1]["hypothesis_id"],
            h[1]["hypothesis_id"]
        );
        assert_eq!(
            later["hypotheses"][0]["hypothesis_id"],
            h[0]["hypothesis_id"]
        );
    }

    #[test]
    fn recurrence_counts_only_posts_the_prior_snapshot_lacked() {
        let cluster = |ids: Value| json!({"id":"c","terms":["matcha","latte"],"salient_terms":[{"term":"latte"},{"term":"oat"}],"evidence_ids":ids,"platforms":["BLUESKY"]});
        let prior = json!({"snapshot_id":"s1","captured_at":100,"query":"matcha","mention_count":2,"evidence_ids":["a1","a2"],"post_keys":["B:1","B:2"],"platforms":["BLUESKY"],"clusters":[cluster(json!(["a1","a2"]))]});
        let now = json!({"snapshot_id":"s2","captured_at":200,"query":"matcha","mention_count":3,"evidence_ids":["b1","b2","b3"],"post_keys":["B:1","B:2","B:3"],"platforms":["BLUESKY"],"clusters":[cluster(json!(["b1","b2","b3"]))]});
        let mut out = hypothesize(&now, &[]);
        mark_recurrence(&mut out, &now, Some(&prior), &[]);
        let r = &out["hypotheses"][1]["recurrence"];
        assert_eq!(r["state"], "RECURRING_WITH_POSTS_NOT_IN_PRIOR_SNAPSHOT");
        assert_eq!(
            (r["posts"].clone(), r["posts_not_in_prior_snapshot"].clone()),
            (json!(3), json!(1))
        );
        // The same capture again is the same evidence, not continued attention.
        let mut again = hypothesize(&prior, &[]);
        mark_recurrence(&mut again, &prior, Some(&prior), &[]);
        assert_eq!(
            again["hypotheses"][1]["recurrence"]["state"],
            "RECURRING_SAME_POSTS_ONLY"
        );
        let mut first = hypothesize(&prior, &[]);
        mark_recurrence(&mut first, &prior, None, &[]);
        assert_eq!(
            first["hypotheses"][0]["recurrence"]["state"],
            "NO_PRIOR_SNAPSHOT"
        );
    }

    #[test]
    fn no_social_evidence_yields_no_hypothesis() {
        let empty = json!({"query":"matcha whisk","mention_count":0,"clusters":[]});
        assert!(
            hypothesize(&empty, &[])["hypotheses"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_rising_attention_counter_is_a_reason_to_research_not_a_mention() {
        let counter = |state: &str| json!({"query":"matcha whisk","mention_count":0,"clusters":[],"source_counters":[{"platform":"MASTODON_TAG","tag":"matchawhisk","scope":"INSTANCE_FEDERATED_VIEW","raw_hash":"ab","growth":{"state":state,"account_days":{"before":3,"after":40}}}]});
        let out = hypothesize(&counter("RISING"), &[]);
        let h = &out["hypotheses"][0];
        assert_eq!(h["basis"], "ATTENTION_COUNTER_ONLY");
        assert_eq!(h["evidence_ids"], json!([]));
        assert_eq!(
            h["attention_counters"][0]["evidence_class"],
            "ATTENTION_SIGNAL_NOT_DEMAND"
        );
        assert_eq!(h["attention_counters"][0]["account_days"]["after"], 40);
        assert_eq!(h["shortlist_eligible"], false);
        assert!(
            h["blocked_by"]
                .as_array()
                .unwrap()
                .contains(&json!("SOCIAL_SIGNAL_IS_NOT_DEMAND"))
        );
        // A flat or unknown counter alone is no reason.
        for state in [
            "NO_DETECTABLE_CHANGE",
            "INSUFFICIENT_COMPLETE_DAYS",
            "FALLING",
        ] {
            assert!(
                hypothesize(&counter(state), &[])["hypotheses"]
                    .as_array()
                    .unwrap()
                    .is_empty(),
                "{state}"
            );
        }
        // Beside posts, the counter qualifies and the posts stay the basis.
        let mut both = snapshot();
        both["source_counters"] = counter("RISING")["source_counters"].clone();
        let h = &hypothesize(&both, &[])["hypotheses"][0];
        assert_eq!(h["basis"], "SOCIAL_POSTS");
        assert_eq!(h["attention_counters"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn refinements_are_shared_informative_terms_not_alphabetical_noise() {
        // As a live run produced them: link hashes, image sizes and function words in a
        // cluster's terms, with the informative ones shared by several posts.
        let snap = json!({"query":"matcha","mention_count":3,"evidence_ids":["a","b","c"],"clusters":[
            {"terms":["0a1b2c3d4e","1920x1080","answer","matcha","whisk","bamboo"],"evidence_ids":["a","b","c"],
             "salient_terms":[{"term":"whisk","posts":3},{"term":"bamboo","posts":2}]},
            {"terms":["matcha","solo"],"evidence_ids":["d"]}]});
        let h = hypothesize(&snap, &[]);
        let h = h["hypotheses"].as_array().unwrap();
        assert_eq!(h.len(), 2);
        assert_eq!(h[1]["refinement_terms"], json!(["whisk", "bamboo"]));
        assert_ne!(h[0]["hypothesis_id"], h[1]["hypothesis_id"]);
        // Every cluster a single post: one topic-level hypothesis, not one per post.
        let lone = json!({"query":"matcha","mention_count":2,"evidence_ids":["a","b"],"platforms":["BLUESKY"],"clusters":[
            {"terms":["matcha","x"],"evidence_ids":["a"]},{"terms":["matcha","y"],"evidence_ids":["b"]}]});
        let h = hypothesize(&lone, &[]);
        assert_eq!(h["hypotheses"].as_array().unwrap().len(), 1);
        assert_eq!(h["hypotheses"][0]["evidence_ids"], json!(["a", "b"]));
    }
}
