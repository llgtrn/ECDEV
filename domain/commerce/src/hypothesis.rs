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
pub fn hypothesize(snapshot: &Value, candidates: &[Value]) -> Value {
    let query = snapshot["query"].as_str().unwrap_or("");
    let topic = terms(query);
    let mut groups: Vec<(BTreeSet<String>, Value, Value, Value)> = Vec::new();
    for c in snapshot["clusters"].as_array().into_iter().flatten() {
        let t: BTreeSet<String> = c["terms"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_lowercase)
            .collect();
        groups.push((
            t,
            c["evidence_ids"].clone(),
            c["platforms"].clone(),
            c["candidate_phrases"].clone(),
        ));
    }
    if groups.is_empty() && snapshot["mention_count"].as_u64().unwrap_or(0) > 0 {
        groups.push((
            topic.clone(),
            snapshot["evidence_ids"].clone(),
            snapshot["platforms"].clone(),
            Value::Null,
        ));
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
        groups.push((topic.clone(), json!([]), json!([]), Value::Null));
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
    for (cluster_terms, evidence, platforms, phrases) in groups {
        let refinements: Vec<String> = cluster_terms.difference(&topic).take(5).cloned().collect();
        let mut queries = vec![query.trim().to_string()];
        queries.extend(refinements.iter().map(|r| format!("{} {r}", query.trim())));
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
        for q in &phrase_queries {
            let q = q["query"].as_str().unwrap_or_default().to_string();
            if !queries.contains(&q) {
                queries.push(q);
            }
        }
        let id = format!(
            "hyp-{}",
            &format!(
                "{:x}",
                Sha256::digest(format!("{query}|{}", refinements.join(",")).as_bytes())
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
    json!({"snapshot_query":query,"capture_mode":snapshot["capture_mode"],"hypotheses":hypotheses,"invariants":["trend != demand","mention count != sales","cross-platform mention != independent market validation","a hypothesis is a reason to research, never a shortlist","population completeness qualifies a signal and never raises its mention counts","an attention counter is attention, never a mention, a sale or demand"],"network_calls":0})
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
        assert_eq!(h.len(), 2);
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
        let first = &h[0];
        assert_eq!(first["refinement_terms"], json!(["bamboo", "chasen"]));
        assert_eq!(first["product_queries"][1], "matcha whisk bamboo");
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
}
