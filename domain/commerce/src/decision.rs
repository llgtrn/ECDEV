//! Evidence-based research selection; unknown commercial inputs stay unknown.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub currency: Option<String>,
    pub max_weight_g: Option<u64>,
    #[serde(default)]
    pub excluded_categories: Vec<String>,
}

pub fn assess(candidates: &mut [Value], policy: &Policy, budget_exhausted: bool) {
    for c in candidates {
        let mut rejected: Vec<Value> = c["rejection_reasons"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let p = &c["product"];
        if let Some(max) = policy.max_weight_g
            && p["fields"]["weight_g"]["status"] == "OBSERVED"
            && p["fields"]["weight_g"]["value"]
                .as_u64()
                .is_some_and(|v| v > max)
        {
            rejected.push(json!("ABOVE_MAX_OBSERVED_WEIGHT"));
        }
        if p["fields"]["category"]["status"] == "OBSERVED"
            && p["fields"]["category"]["value"]
                .as_str()
                .is_some_and(|category| {
                    policy
                        .excluded_categories
                        .iter()
                        .any(|v| v.eq_ignore_ascii_case(category.trim()))
                })
        {
            rejected.push(json!("EXCLUDED_OBSERVED_CATEGORY"));
        }
        if c["economics_uncertainty"]["profit_max"]
            .as_i64()
            .is_some_and(|max| max <= 0)
        {
            rejected.push(json!("NON_POSITIVE_MATHEMATICAL_PROFIT_UPPER_BOUND"));
        }
        let rows = c["resolution"]["observations"]
            .as_array()
            .cloned()
            .unwrap_or_else(|| vec![c.clone()]);
        let origins: BTreeSet<_> = rows
            .iter()
            .filter_map(|r| url::Url::parse(r["source"].as_str()?).ok())
            .map(|u| u.origin().ascii_serialization())
            .collect();
        let mut missing = vec![];
        if policy.max_weight_g.is_some()
            && (p["fields"]["weight_g"]["status"] != "OBSERVED"
                || p["fields"]["weight_g"]["value"].as_u64().is_none())
        {
            missing.push("OBSERVED_WEIGHT_FOR_CONSTRAINT");
        }
        if !policy.excluded_categories.is_empty()
            && (p["fields"]["category"]["status"] != "OBSERVED"
                || p["fields"]["category"]["value"].as_str().is_none())
        {
            missing.push("OBSERVED_CATEGORY_FOR_CONSTRAINT");
        }
        if c["resolution"]["basis"] != "CHECKSUM_VALID_GTIN_AND_VARIANT"
            && c["resolution"]["basis"] != "ASIN_AND_VARIANT"
        {
            missing.push("CROSS_SOURCE_PRODUCT_IDENTIFIER");
        }
        if origins.len() < 2 {
            missing.push("SECOND_LISTING_ORIGIN");
        }
        if p["price_minor"].as_i64().is_none_or(|v| v <= 0)
            || !matches!(
                p["fields"]["price_minor"]["status"].as_str(),
                Some("OBSERVED" | "DERIVED")
            )
        {
            missing.push("CONSISTENT_POSITIVE_PRICE");
        }
        if p["currency"].as_str().is_none()
            || policy
                .currency
                .as_ref()
                .is_some_and(|currency| p["currency"] != *currency)
        {
            missing.push("TARGET_CURRENCY");
        }
        let title = rows.iter().any(|r| {
            r["product"]["title"]
                .as_str()
                .is_some_and(|s| !s.trim().is_empty())
        });
        if !title {
            missing.push("SOURCE_PRODUCT_TITLE");
        }
        let in_stock = rows
            .iter()
            .flat_map(|r| {
                r["product"]["observed_offers"]
                    .as_array()
                    .into_iter()
                    .flatten()
            })
            .any(|o| {
                matches!(
                    o["availability"].as_str(),
                    Some("InStock" | "https://schema.org/InStock" | "http://schema.org/InStock")
                )
            });
        if !in_stock {
            missing.push("OBSERVED_AVAILABLE_OFFER");
        }
        if c["evidence_ids"].as_array().is_none_or(|a| a.is_empty()) {
            missing.push("CAPTURE_EVIDENCE");
        }
        let shortlisted = rejected.is_empty() && missing.is_empty();
        c["state"] = json!(if !rejected.is_empty() {
            "REJECTED"
        } else if shortlisted {
            "SHORTLISTED"
        } else if budget_exhausted {
            "INSUFFICIENT_EVIDENCE"
        } else if c["state"] == "SCREENED" {
            "SCREENED"
        } else {
            "VALIDATING"
        });
        c["rejection_reasons"] = json!(rejected);
        c["decision"] = json!({"policy":"PUBLIC_RESEARCH_SHORTLIST_V1","purpose":"FURTHER_RESEARCH","criteria":{"cross_source_identifier":"CHECKSUM_VALID_GTIN_OR_ASIN_WITH_VARIANT","minimum_listing_origins":2,"consistent_positive_price":true,"available_offer":true,"currency":policy.currency,"max_weight_g":policy.max_weight_g,"excluded_categories":policy.excluded_categories},"listing_origins":origins,"publisher_independence":"UNVERIFIED","missing_selection_evidence":missing,"budget_exhausted":budget_exhausted,"commercial_validation":"INCOMPLETE_WHILE_SUPPLIER_DEMAND_LOGISTICS_AND_RISK_REMAIN_UNKNOWN","profit_forecast":null,"reason":if shortlisted {"Matched source identifier, consistent price and available offer across captured origins; prioritize further commercial validation"} else if !rejected.is_empty() {"Failed explicit evidence constraints"} else {"Selection evidence incomplete"}});
        c["survival_reason"] = c["decision"]["reason"].clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_keeps_unknown_profit_and_distinguishes_exhaustion_from_rejection() {
        let mut c = json!({"product":{"currency":"JPY","price_minor":1980,"fields":{"price_minor":{"status":"OBSERVED"}}},"resolution":{"basis":"CHECKSUM_VALID_GTIN_AND_VARIANT","observations":[{"source":"https://one.example/p","product":{"title":"Cup","observed_offers":[{"availability":"InStock"}]}},{"source":"https://two.example/p","product":{"title":"Mug"}}]},"evidence_ids":["one","two"],"rejection_reasons":[],"economics_uncertainty":{"profit_expected":null}});
        assess(std::slice::from_mut(&mut c), &Policy::default(), true);
        assert_eq!(c["state"], "SHORTLISTED");
        assert!(c["economics_uncertainty"]["profit_expected"].is_null());
        c["product"]["price_minor"] = Value::Null;
        assess(std::slice::from_mut(&mut c), &Policy::default(), true);
        assert_eq!(c["state"], "INSUFFICIENT_EVIDENCE");
        assert!(c["rejection_reasons"].as_array().unwrap().is_empty());
        c["product"]["fields"]["category"] = json!({"status":"OBSERVED","value":"Medical"});
        assess(
            std::slice::from_mut(&mut c),
            &Policy {
                excluded_categories: vec!["medical".into()],
                ..Policy::default()
            },
            true,
        );
        assert_eq!(c["state"], "REJECTED");
        assert_eq!(c["rejection_reasons"][0], "EXCLUDED_OBSERVED_CATEGORY");
    }
}
