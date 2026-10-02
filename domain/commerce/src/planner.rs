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
