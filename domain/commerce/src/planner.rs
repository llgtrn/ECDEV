use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Intent {
    pub market: String,
    pub currency: String,
    pub capital: u64,
    pub min_price: u64,
    pub max_price: u64,
    pub max_weight_g: u64,
    pub minimum_margin_bps: u64,
    pub max_inventory_per_sku: u64,
    #[serde(default)]
    pub positive_trend: bool,
    #[serde(default)]
    pub exclude_regulated: bool,
}
impl Intent {
    pub fn validate(&self) -> Result<(), String> {
        if self.market.is_empty()
            || self.currency.len() != 3
            || !self.currency.bytes().all(|b| b.is_ascii_uppercase())
            || self.capital == 0
            || self.min_price == 0
            || self.max_price < self.min_price
            || self.max_weight_g == 0
            || self.minimum_margin_bps > 10000
            || self.max_inventory_per_sku == 0
            || self.max_inventory_per_sku > self.capital
        {
            return Err(
                "Invalid market, currency, capital, price, weight or margin constraints".into(),
            );
        }
        Ok(())
    }
}
pub const STAGES: &[&str] = &[
    "market.discover",
    "product.discover",
    "demand.validate",
    "competition.analyze",
    "supplier.discover",
    "logistics.estimate",
    "marketplace.fees",
    "ads.estimate",
    "economics.simulate",
    "risk.calculate",
    "opportunity.rank",
];
pub fn plan(i: &Intent, providers: &[Value]) -> Result<Value, String> {
    i.validate()?;
    let steps:Vec<Value>=STAGES.iter().enumerate().map(|(n,cap)|{
  let considered:Vec<Value>=providers.iter().filter(|p|p["capabilities"].as_array().is_some_and(|a|a.iter().any(|c|c==cap))).cloned().collect();
  let selected=considered.iter().find(|p|p["status"]=="AVAILABLE" && p["markets"].as_array().is_some_and(|a|a.iter().any(|m|m==&i.market)));
  json!({"step_id":format!("step-{n}"),"capability":cap,"depends_on":if n==0{vec![]}else{vec![format!("step-{}",n-1)]},"providers_considered":considered,"selected_provider":selected.map(|p|p["id"].clone()),"status":if selected.is_some(){"READY"}else if *cap=="economics.simulate"{"REQUIRES_INPUT"}else{"UNAVAILABLE"},"selection_reason":if selected.is_some(){"Configured matching capability and market"}else{"No verified available provider; no substitute observations"}})
 }).collect();
    Ok(
        json!({"intent":i,"execution_mode":"PLAN_ONLY","steps":steps,"result_status":"UNAVAILABLE","observations":[],"cost_minor":0}),
    )
}

/// A transparent heuristic allocation, not measured entropy or a promise of a quotation.
/// Monetary permission and crawl capacity are hard constraints, independent of utility.
pub fn information_gain(
    candidates: &[Value],
    suppliers: &[Value],
    paid_budget: u64,
    request_budget: usize,
) -> Value {
    use std::collections::BTreeSet;
    let mut actions = vec![];
    let mut skipped = vec![];
    let mut seen = BTreeSet::new();
    for candidate in candidates {
        if candidate["state"] == "REJECTED" {
            skipped.push(
                json!({"candidate_id":candidate["id"],"reason":"ALREADY_REJECTED_BY_EVIDENCE"}),
            );
            continue;
        }
        let brand = &candidate["product"]["brand"];
        let brand = brand
            .as_str()
            .or_else(|| brand["name"].as_str())
            .unwrap_or("")
            .to_lowercase();
        let unknown = candidate["economics_uncertainty"]["unknown_costs"].as_array();
        if !unknown.is_some_and(|items| items.iter().any(|v| v == "product_cost")) {
            continue;
        }
        for lead in suppliers {
            let label = lead["fields"]["name"]["value"]
                .as_str()
                .unwrap_or("")
                .to_lowercase();
            let brand_match = brand.chars().count() >= 3 && label.contains(&brand);
            let same_origin = candidate["source"]
                .as_str()
                .and_then(|s| url::Url::parse(s).ok())
                .zip(
                    lead["source"]
                        .as_str()
                        .and_then(|s| url::Url::parse(s).ok()),
                )
                .is_some_and(|(a, b)| a.origin() == b.origin());
            if !brand_match && !same_origin {
                continue;
            }
            for target in lead["follow_up_urls"].as_array().into_iter().flatten() {
                let Some(url) = target.as_str() else {
                    continue;
                };
                if url == lead["source"].as_str().unwrap_or("") {
                    continue;
                }
                let key = (candidate["id"].to_string(), url.to_string());
                if !seen.insert(key) {
                    continue;
                }
                actions.push(json!({"candidate_id":candidate["id"],"action":"PUBLIC_SUPPLIER_FOLLOW_UP","provider":"native-web","url":url,"target_unknown":"product_cost","uncertainty_reduction_points":if url.contains("contact") {100}else{50},"reduction_basis":"HEURISTIC_POTENTIAL: unbounded product-cost uncertainty; related public supplier lead may expose terms or a route to request them; association is explicitly unverified","association_status":if brand_match{"DERIVED_BRAND_LABEL_MATCH_NOT_VERIFIED_SUPPLY_RELATION"}else{"DERIVED_SHARED_PUBLISHER_ORIGIN_NOT_VERIFIED_SUPPLY_RELATION"},"evidence":{"supplier_source":lead["source"],"supplier_capture_sha256":lead["raw_capture_sha256"],"brand":brand,"page_label":label},"expected_cost_minor":0,"expected_requests":1,"expected_latency_ms":1000,"estimate_status":"HEURISTIC_NOT_MEASURED"}));
            }
        }
        // Paid providers are represented independently, never silently selected by this public executor.
        skipped.push(json!({"candidate_id":candidate["id"],"action":"PAID_ENRICHMENT","reason":if paid_budget==0{"PAID_BUDGET_ZERO"}else{"NO_VERIFIED_ACTION_COST_OR_PERMISSION"}}));
    }
    actions.sort_by(|a, b| {
        let score = |v: &Value| {
            v["uncertainty_reduction_points"].as_f64().unwrap_or(0.0)
                / (v["expected_requests"].as_f64().unwrap_or(1.0)
                    + v["expected_latency_ms"].as_f64().unwrap_or(0.0) / 1000.0
                    + v["expected_cost_minor"].as_f64().unwrap_or(0.0))
        };
        score(b)
            .total_cmp(&score(a))
            .then_with(|| a["url"].as_str().cmp(&b["url"].as_str()))
            .then_with(|| a["candidate_id"].as_str().cmp(&b["candidate_id"].as_str()))
    });
    let mut selected = vec![];
    let mut selected_urls = BTreeSet::new();
    for action in &actions {
        if selected_urls.len() >= request_budget {
            break;
        }
        if selected_urls.insert(action["url"].as_str().unwrap_or("")) {
            selected.push(action.clone());
        }
    }
    json!({"status":if selected.is_empty(){"NO_EXECUTABLE_INFORMATION_GAIN_ACTION"}else{"READY_PUBLIC_FOLLOW_UP"},"method":"HEURISTIC_EXPECTED_UNCERTAINTY_REDUCTION_PER_RESOURCE_COST","score_denominator":"expected requests + expected latency seconds + expected monetary minor units; explicit heuristic weights, not currency conversion","uncertainty_policy":"Unbounded product cost gives supplier contact routes 100 and general organization routes 50 heuristic utility points; no claimed probability or measured reduction","paid_budget_minor":paid_budget,"request_budget":request_budget,"actions":actions,"selected_actions":selected,"skipped":skipped,"execution":"research.run follow_up_run_id executes selected public URLs under robots/frontier policy; contact forms are never submitted","limitations":"May discover supplier information but does not obtain negotiated quotations automatically. No verified paid actions are currently configured."})
}
#[cfg(test)]
mod information_tests {
    use super::*;
    #[test]
    fn allocates_public_budget_to_unresolved_candidate_not_rejected_candidate() {
        let eligible = json!({"id":"A","state":"VALIDATING","source":"https://example.org/product/cup","product":{"brand":"Tea Collection"},"economics_uncertainty":{"unknown_costs":["product_cost"]}});
        let rejected = json!({"id":"B","state":"REJECTED","product":{"brand":"Example"},"economics_uncertainty":{"unknown_costs":["product_cost"]}});
        let lead = json!({"source":"https://example.org/partners","fields":{"name":{"value":"Example Wholesale"}},"follow_up_urls":["https://example.org/contact","https://example.org/company"],"raw_capture_sha256":"fixture"});
        let plan = information_gain(&[rejected, eligible], &[lead], 0, 1);
        assert_eq!(plan["selected_actions"].as_array().unwrap().len(), 1);
        assert_eq!(plan["selected_actions"][0]["candidate_id"], "A");
        assert_eq!(plan["selected_actions"][0]["expected_cost_minor"], 0);
        assert_eq!(
            plan["selected_actions"][0]["association_status"],
            "DERIVED_SHARED_PUBLISHER_ORIGIN_NOT_VERIFIED_SUPPLY_RELATION"
        );
        assert!(
            plan["skipped"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["reason"] == "ALREADY_REJECTED_BY_EVIDENCE")
        );
        assert!(
            information_gain(&[], &[], 0, 1)["selected_actions"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}
