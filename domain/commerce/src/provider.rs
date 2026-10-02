//! Canonical acquisition boundary. Provider models remain inside adapters.
use crate::domain::Evidence;
use serde_json::Value;
#[derive(
    Clone, Copy, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq, PartialOrd, Ord,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProviderClass {
    Native,
    Public,
    Official,
    Paid,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct BudgetPolicy {
    pub currency: String,
    pub per_run_minor: u64,
    pub per_day_minor: u64,
    pub per_month_minor: u64,
    pub per_provider_minor: u64,
    pub per_capability_minor: u64,
    pub request_ceiling_minor: u64,
}
impl Default for BudgetPolicy {
    fn default() -> Self {
        Self {
            currency: "USD".into(),
            per_run_minor: 0,
            per_day_minor: 0,
            per_month_minor: 0,
            per_provider_minor: 0,
            per_capability_minor: 0,
            request_ceiling_minor: 0,
        }
    }
}
impl BudgetPolicy {
    pub fn from_env() -> Self {
        fn get(k: &str) -> u64 {
            std::env::var(k)
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0)
        }
        Self {
            currency: std::env::var("ECDEV_RESEARCH_BUDGET_CURRENCY").unwrap_or("USD".into()),
            per_run_minor: get("ECDEV_PAID_PER_RUN_MINOR"),
            per_day_minor: get("ECDEV_PAID_PER_DAY_MINOR"),
            per_month_minor: get("ECDEV_PAID_PER_MONTH_MINOR"),
            per_provider_minor: get("ECDEV_PAID_PER_PROVIDER_MINOR"),
            per_capability_minor: get("ECDEV_PAID_PER_CAPABILITY_MINOR"),
            request_ceiling_minor: get("ECDEV_PAID_REQUEST_CEILING_MINOR"),
        }
    }
    pub fn permits(&self, day: u64, month: u64, provider: u64, capability: u64) -> bool {
        let c = self.request_ceiling_minor;
        self.currency == "USD"
            && c <= i64::MAX as u64
            && c > 0
            && c <= self.per_run_minor
            && day.checked_add(c).is_some_and(|v| v <= self.per_day_minor)
            && month
                .checked_add(c)
                .is_some_and(|v| v <= self.per_month_minor)
            && provider
                .checked_add(c)
                .is_some_and(|v| v <= self.per_provider_minor)
            && capability
                .checked_add(c)
                .is_some_and(|v| v <= self.per_capability_minor)
    }
}

/// Route selection records exclusions; uncertainty never becomes a made-up estimate.
pub fn routes(capability: &str, providers: &[Value], paid_budget: u64, candidates: usize) -> Value {
    let mut eligible = vec![];
    let mut skipped = vec![];
    for p in providers {
        if !p["capabilities"]
            .as_array()
            .is_some_and(|a| a.iter().any(|c| c == capability))
        {
            continue;
        }
        let class = p["class"].as_str().unwrap_or("PAID");
        let reason = if p["status"] != "AVAILABLE" {
            Some("UNAVAILABLE")
        } else if class == "PAID" && paid_budget == 0 {
            Some("BUDGET_ZERO")
        } else if class == "PAID" && candidates > 10 {
            Some("REDUCE_CANDIDATES_FIRST")
        } else {
            None
        };
        if let Some(reason) = reason {
            skipped.push(serde_json::json!({"provider":p["id"],"reason":reason}));
        } else {
            eligible.push(p.clone());
        }
    }
    eligible.sort_by_key(|p| match p["class"].as_str() {
        Some("NATIVE") => 0,
        Some("CACHE") => 1,
        Some("PUBLIC") => 2,
        Some("OFFICIAL") => 3,
        _ => 4,
    });
    serde_json::json!({"capability":capability,"routes":eligible,"skipped":skipped,"expected_information_gain":null,"confidence_required":"SOURCE_BACKED","freshness_required_seconds":null,"policy":"NATIVE_CACHE_PUBLIC_OFFICIAL_PAID_FINALISTS"})
}
#[cfg(test)]
mod budget_tests {
    use super::*;
    #[test]
    fn zero_budget_and_unknown_cost_deny_paid_calls() {
        assert!(!BudgetPolicy::default().permits(0, 0, 0, 0));
        let b = BudgetPolicy {
            per_run_minor: 10,
            per_day_minor: 20,
            per_month_minor: 30,
            per_provider_minor: 30,
            per_capability_minor: 30,
            request_ceiling_minor: 10,
            ..Default::default()
        };
        assert!(b.permits(10, 20, 0, 0));
        assert!(!b.permits(11, 0, 0, 0));
        assert!(!b.permits(0, u64::MAX, 0, 0));
    }
    #[test]
    fn fallback_does_not_require_paid_providers() {
        let p = vec![
            serde_json::json!({"id":"semrush","class":"PAID","status":"AVAILABLE","capabilities":["research.market"]}),
            serde_json::json!({"id":"web","class":"PUBLIC","status":"AVAILABLE","capabilities":["research.market"]}),
        ];
        let r = routes("research.market", &p, 0, 500);
        assert_eq!(r["routes"][0]["id"], "web");
        assert_eq!(r["skipped"][0]["reason"], "BUDGET_ZERO");
    }
}

#[derive(serde::Serialize)]
pub struct AcquireRequest {
    pub run_id: String,
    pub capability: String,
    pub market: String,
    pub query: Value,
}
pub struct AcquireResult {
    pub observations: Vec<Evidence>,
    pub result: Value,
    pub raw_payload: Vec<u8>,
    pub provider_cost: Value,
}
pub trait Provider: Send + Sync {
    fn id(&self) -> &str;
    fn metadata(&self) -> Value;
    fn normalize_query(&self, query: &Value) -> Result<Value, String> {
        Ok(query.clone())
    }
    /// Blocking IO; application transports execute this on a blocking worker.
    fn acquire(&self, request: &AcquireRequest) -> Result<AcquireResult, String>;
}
