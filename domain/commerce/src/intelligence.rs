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
        c["competition_evidence"] = json!({"status":"OBSERVED_SAMPLE_ONLY","listing_pages_observed":pages.len(),"source_pages":pages,"seller_assertions_observed":sellers.len(),"seller_assertions":sellers,"observed_offer_price_ranges":ranges,"offers":offers,"rating":c["product"]["rating"],"review_count":c["product"]["review_count"],"limitation":"These are captured listings/offers, not a total competitor count. A single manufacturer catalog does not establish competitive intensity."});
        c["demand_evidence"] = json!({"status":"PROXY","kind":"PUBLIC_CATEGORY_OR_SEARCH_LINK_PRESENCE","observed_surfaces":category_pages,"search_volume":null,"true_sales":null,"review_velocity":null,"limitation":"Link presence is a visibility proxy; no volume, sales, trend or velocity inference."});
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
