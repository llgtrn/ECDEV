//! Observed sample projections; these do not estimate the size or demand of a market.
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub fn enrich(candidates: &mut [Value], snapshots: &[Value]) -> Result<Value, String> {
    let mut brands: BTreeMap<String, usize> = BTreeMap::new();
    let origins: BTreeSet<_> = snapshots
        .iter()
        .filter_map(|s| s["source"].as_str())
        .filter_map(|s| url::Url::parse(s).ok())
        .map(|u| u.origin().ascii_serialization())
        .collect();
    for c in candidates.iter() {
        let b = &c["product"]["brand"];
        let name = if b.is_object() {
            b["name"].as_str()
        } else {
            b.as_str()
        };
        if let Some(name) = name {
            *brands.entry(name.to_string()).or_default() += 1;
        }
    }
    for c in candidates.iter_mut() {
        let rows = c["resolution"]["observations"]
            .as_array()
            .cloned()
            .unwrap_or_else(|| vec![c.clone()]);
        let pages: BTreeSet<_> = rows
            .iter()
            .filter_map(|r| r["source"].as_str())
            .map(str::to_string)
            .collect();
        let listing_origins: BTreeSet<_> = pages
            .iter()
            .filter_map(|page| url::Url::parse(page).ok())
            .map(|u| u.origin().ascii_serialization())
            .collect();
        let mut sellers = BTreeSet::new();
        let mut prices: BTreeMap<String, BTreeSet<i64>> = BTreeMap::new();
        let mut offers = vec![];
        for row in &rows {
            for offer in row["product"]["observed_offers"]
                .as_array()
                .into_iter()
                .flatten()
            {
                if !offer["seller"].is_null() {
                    sellers.insert(offer["seller"].to_string());
                }
                if let (Some(currency), Some(price)) =
                    (offer["currency"].as_str(), offer["price_minor"].as_i64())
                {
                    prices
                        .entry(currency.to_string())
                        .or_default()
                        .insert(price);
                }
                offers.push(
                    json!({"offer":offer,"page":row["source"],"evidence_ids":row["evidence_ids"]}),
                );
            }
        }
        let ranges:Vec<_>=prices.iter().map(|(currency,values)|json!({"currency":currency,"min_minor":values.first(),"max_minor":values.last(),"status":"OBSERVED_IN_CAPTURED_OFFERS"})).collect();
        let category_pages:Vec<_>=snapshots.iter().filter(|s|matches!(s["page_metadata"]["classification"]["role"].as_str(),Some("CATEGORY"|"PAGINATION"|"SEARCH_RESULT"))).filter(|s|s["links"].as_array().is_some_and(|links|links.iter().any(|l|l.as_str().is_some_and(|u|pages.contains(u))))).map(|s|json!({"page":s["source"],"role":s["page_metadata"]["classification"]["role"],"raw_capture_sha256":s["content_hash"]})).collect();
        c["competition_evidence"] = json!({"status":"OBSERVED_SAMPLE_ONLY","listing_pages_observed":pages.len(),"source_pages":pages,"listing_origins_observed":listing_origins,"publisher_independence":"UNVERIFIED","seller_assertions_observed":sellers.len(),"seller_assertions":sellers,"observed_offer_price_ranges":ranges,"offers":offers,"rating":c["product"]["rating"],"review_count":c["product"]["review_count"],"limitation":"These are captured listings/offers, not a total competitor count. A single manufacturer catalog does not establish competitive intensity."});
        c["demand_evidence"] = json!({"status":"PROXY","kind":"PUBLIC_LINK_AND_MATCHED_PRODUCT_LISTING_PRESENCE","observed_surfaces":category_pages,"observed_listing_origins":listing_origins,"listing_origin_evidence":rows.iter().map(|r|json!({"page":r["source"],"evidence_ids":r["evidence_ids"]})).collect::<Vec<_>>(),"publisher_independence":"UNVERIFIED","search_volume":null,"true_sales":null,"review_velocity":null,"limitation":"Link presence is a visibility proxy; no volume, sales, trend or velocity inference."});
        c["economics_uncertainty"] = crate::uncertainty::observed_candidate(&c["product"])?;
    }
    Ok(
        json!({"scope":"RESOLVED_CANDIDATES_IN_THIS_CAPTURED_RUN","candidate_count":candidates.len(),"origins_observed":origins,"brand_counts":brands,"brand_count_denominator":candidates.len(),"status":"OBSERVED_SAMPLE_ONLY","independent_market_coverage":"UNVERIFIED","total_market_competitors":null,"total_market_demand":null}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn captured_presence_and_prices_never_become_sales_or_profit_forecasts() {
        let mut candidates = vec![
            json!({"product":{"currency":"JPY","price_minor":4000,"brand":"Hario","provenance":{"page":"https://shop.example/product/a"}},"resolution":{"observations":[{"source":"https://shop.example/product/a","product":{"observed_offers":[{"currency":"JPY","price_minor":4000,"seller":"Shop"}]},"evidence_ids":["one"]}]}}),
        ];
        let snapshots = vec![
            json!({"source":"https://shop.example/catalog","content_hash":"hash","page_metadata":{"classification":{"role":"CATEGORY"}},"links":["https://shop.example/product/a"]}),
        ];
        let summary = enrich(&mut candidates, &snapshots).unwrap();
        assert_eq!(
            candidates[0]["competition_evidence"]["listing_pages_observed"],
            1
        );
        assert_eq!(
            candidates[0]["demand_evidence"]["observed_surfaces"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(candidates[0]["demand_evidence"]["search_volume"].is_null());
        assert!(candidates[0]["economics_uncertainty"]["profit_expected"].is_null());
        assert!(summary["total_market_competitors"].is_null());
    }
}

/// Per-run observations are kept separate from executable-capability coverage.
pub fn completeness(
    candidates: &[Value],
    snapshots: &[Value],
    fixture: bool,
    network_calls: u64,
    supplier_count: usize,
) -> Value {
    let required = [
        "title",
        "brand",
        "sku",
        "gtin",
        "ean",
        "upc",
        "mpn",
        "price_minor",
        "currency",
        "original_price",
        "discount",
        "availability",
        "seller",
        "shipping_text",
        "rating",
        "review_count",
        "images",
        "variants",
        "breadcrumbs",
        "category",
        "description",
        "specifications",
        "canonical_url",
        "weight_g",
    ];
    let mut counts: BTreeMap<&str, usize> = [
        ("OBSERVED", 0),
        ("DERIVED", 0),
        ("ESTIMATED", 0),
        ("UNKNOWN", 0),
        ("CONFLICT", 0),
    ]
    .into();
    let mut with_evidence = 0;
    let mut known = 0;
    for candidate in candidates {
        for key in required {
            let field = &candidate["product"]["fields"][key];
            let status = field["status"]
                .as_str()
                .filter(|s| counts.contains_key(*s))
                .unwrap_or("UNKNOWN");
            *counts.get_mut(status).unwrap() += 1;
            if matches!(status, "OBSERVED" | "DERIVED" | "ESTIMATED") && !field["value"].is_null() {
                known += 1;
                if field["evidence"].as_array().is_some_and(|a| {
                    a.iter()
                        .any(|e| e["page"].is_string() && e["raw_capture_sha256"].is_string())
                }) {
                    with_evidence += 1;
                }
            }
        }
    }
    let denominator = candidates.len() * required.len();
    let ratio = |n: usize, d: usize| (n * 10000).checked_div(d).map_or(Value::Null, |v| json!(v));
    let stages: Vec<Value> = crate::research::ZERO_COST_STAGES
        .iter()
        .map(|(name, _)| {
            let observed = match *name {
                "discovery_from_seed_links" => snapshots
                    .iter()
                    .any(|s| s["links"].as_array().is_some_and(|a| !a.is_empty())),
                "crawl" | "extraction" | "evidence" => !snapshots.is_empty(),
                "candidate_creation" | "ranking_and_filtering" => !candidates.is_empty(),
                "competitor_price_comparison" => candidates.iter().any(|c| {
                    c["competition_evidence"]["observed_offer_price_ranges"]
                        .as_array()
                        .is_some_and(|a| !a.is_empty())
                }),
                "economics_from_assumptions" => {
                    candidates.iter().any(|c| !c["economics"].is_null())
                }
                "report" => true,
                "supplier_discovery" => supplier_count > 0,
                _ => false,
            };
            json!({"stage":name,"observed_in_run":observed})
        })
        .collect();
    let observed_stages = stages
        .iter()
        .filter(|s| s["observed_in_run"] == true)
        .count();
    let live = !fixture && network_calls > 0;
    json!({"scope":"THIS_RUN_ONLY","evidence_mode":if fixture{"FIXTURE"}else if live{"LIVE_NETWORK"}else{"CACHED_NO_NEW_NETWORK"},"live_zero_paid_research_coverage_bps":if live{ratio(observed_stages,stages.len())}else{Value::Null},"fixture_zero_paid_research_coverage_bps":if fixture{ratio(observed_stages,stages.len())}else{Value::Null},"stage_denominator":stages,"live_data_completeness_bps":if live{ratio(known,denominator)}else{Value::Null},"candidate_field_observability_bps":ratio(known,denominator),"evidence_completeness_bps":ratio(with_evidence,known),"evidence_supported_known_fields":with_evidence,"known_fields":known,"field_count_denominator":denominator,"required_fields":required,"field_evidence_states":counts,"definitions":"Field observability counts known nonconflicting required product fields. Evidence completeness counts known fields with page and raw-capture hash. Stage coverage records operations evidenced in this run, not accuracy or total market coverage. Empty denominators remain unknown; live coverage requires new network IO. Cached and fixture observations never increment live coverage."})
}
#[cfg(test)]
mod completeness_tests {
    use super::*;
    #[test]
    fn fixture_and_empty_runs_cannot_claim_live_completeness() {
        let rows = vec![
            json!({"product":{"fields":{"title":{"status":"OBSERVED","value":"Cup","evidence":[{"page":"https://example.org/p","raw_capture_sha256":"fixture"}]}}}}),
        ];
        let fixture = completeness(&rows, &[], true, 0, 0);
        assert!(fixture["live_zero_paid_research_coverage_bps"].is_null());
        assert_eq!(fixture["known_fields"], 1);
        assert_eq!(fixture["field_count_denominator"], 24);
        assert_eq!(fixture["evidence_completeness_bps"], 10000);
        let empty = completeness(&[], &[], false, 1, 0);
        assert!(empty["live_data_completeness_bps"].is_null());
        assert!(completeness(&rows, &[], false, 0, 0)["live_data_completeness_bps"].is_null());
        assert_eq!(
            completeness(&rows, &[], false, 2, 0)["evidence_mode"],
            "LIVE_NETWORK"
        );
    }
}
