//! High-level official product intent; no provider substitution or implicit live permission.
use crate::{
    provider::AcquireRequest,
    service::{Engine, timestamp},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

impl Engine {
    /// Official boundary readiness: zero network, never persisted, never secret-bearing.
    pub(crate) fn provider_doctor(&self, args: Value) -> Result<Value, String> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Input {
            provider: Option<String>,
        }
        let input: Input = serde_json::from_value(if args.is_null() { json!({}) } else { args })
            .map_err(|e| e.to_string())?;
        let id = input.provider.unwrap_or_else(|| "amazon-sp-api".into());
        if id != "amazon-sp-api" {
            return Err("UNSUPPORTED_DOCTOR_PROVIDER".into());
        }
        let Some(mut report) = self
            .providers
            .iter()
            .find(|p| p.id() == id)
            .and_then(|p| p.doctor())
        else {
            return Ok(
                json!({"provider":id,"source_layer":"OFFICIAL_SP_API","status":"OFFICIAL_ADAPTER_UNAVAILABLE","network":"NOT_PROBED","live_calls_made_by_doctor":0}),
            );
        };
        let budget = crate::provider::BudgetPolicy::from_env();
        report["core_monetary_ceiling"] = json!({"state":if budget.permits(0,0,0,0){"CONFIGURED_PERMITS_ONE_REQUEST"}else{"ZERO_OR_INCOMPLETE_BLOCKS_ENGINE_LIVE_ROUTE"},"applies_to":"ecdev.product.analyze evidence_layer=OFFICIAL_SP_API","actual_cost_minor":null});
        Ok(report)
    }
    pub(crate) fn official_product(&self, args: Value) -> Result<Value, String> {
        self.official_product_with_budget(args, &crate::provider::BudgetPolicy::from_env())
    }
    fn official_product_with_budget(
        &self,
        args: Value,
        budget: &crate::provider::BudgetPolicy,
    ) -> Result<Value, String> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Intent {
            market: String,
            asin: String,
            evidence_layer: String,
            include: Option<Vec<String>>,
            fee_estimate: Option<Value>,
            fixture_responses: Option<Value>,
        }
        let intent: Intent = serde_json::from_value(args).map_err(|e| e.to_string())?;
        if intent.evidence_layer != "OFFICIAL_SP_API"
            || !matches!(intent.market.as_str(), "AMAZON_JP" | "AMAZON_US")
            || intent.asin.len() != 10
            || !intent
                .asin
                .bytes()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        {
            return Err("INVALID_OFFICIAL_PRODUCT_INTENT".into());
        }
        let fixture = intent.fixture_responses.is_some();
        let query = json!({"asin":intent.asin,"include":intent.include,"fee_estimate":intent.fee_estimate,"fixture_responses":intent.fixture_responses});
        self.official_acquire(
            "product.analyze.official",
            "OFFICIAL_PRODUCT",
            intent.market,
            query,
            fixture,
            budget,
        )
    }
    /// Seller-scoped official reads (listings item, FBA inventory summaries, marketplace
    /// participations, product type search and definition). Orders, PII, restricted data
    /// tokens and writes are refused before any IO; live reads need the same operator gate,
    /// credentials and monetary ceilings as every official read.
    pub(crate) fn seller_read(&self, args: Value) -> Result<Value, String> {
        self.seller_read_with_budget(args, &crate::provider::BudgetPolicy::from_env())
    }
    fn seller_read_with_budget(
        &self,
        args: Value,
        budget: &crate::provider::BudgetPolicy,
    ) -> Result<Value, String> {
        seller_read_policy(&args)?;
        let Value::Object(mut query) = args else {
            return Err("INVALID_SELLER_READ_INTENT".into());
        };
        let market = query
            .remove("market")
            .and_then(|m| m.as_str().map(str::to_string))
            .ok_or("INVALID_SELLER_READ_MARKET")?;
        query.remove("evidence_layer");
        let fixture = query.get("fixture_responses").is_some_and(|f| !f.is_null());
        self.official_acquire(
            "seller.read.official",
            "OFFICIAL_SELLER_READ",
            market,
            Value::Object(query),
            fixture,
            budget,
        )
    }
    fn official_acquire(
        &self,
        capability: &str,
        run_kind: &str,
        market: String,
        query: Value,
        fixture: bool,
        budget: &crate::provider::BudgetPolicy,
    ) -> Result<Value, String> {
        let id = Uuid::new_v4().to_string();
        let request = AcquireRequest {
            run_id: id.clone(),
            capability: capability.into(),
            market,
            query,
        };
        let Some(provider) = self.providers.iter().find(|p| p.id() == "amazon-sp-api") else {
            return self.persist(json!({"acquisition_run_id":id,"mode":"PLAN_ONLY","source_layer":"OFFICIAL_SP_API","status":"UNAVAILABLE","reason":"OFFICIAL_ADAPTER_UNAVAILABLE","observations":[],"network_calls":0,"cost_minor":0,"fallback_providers":[]}));
        };
        if !fixture {
            let profile = provider.metadata();
            if profile["live_authorized"] != true
                || profile["live_supported"] != true
                || profile["status"] != "AVAILABLE"
            {
                return self.persist(json!({"acquisition_run_id":id,"mode":"PLAN_ONLY","source_layer":"OFFICIAL_SP_API","status":"UNAVAILABLE","reason":profile["auth_state"],"observations":[],"network_calls":0,"cost_minor":0,"new_live_acquisition":false,"fallback_providers":[]}));
            }
            if let Err(reason) = self.reserve_paid(&id, "amazon-sp-api", capability, budget) {
                return self.persist(json!({"acquisition_run_id":id,"mode":"PLAN_ONLY","source_layer":"OFFICIAL_SP_API","status":"DENIED_BUDGET","reason":reason,"observations":[],"network_calls":0,"cost_minor":0,"new_live_acquisition":false,"fallback_providers":[]}));
            }
        }
        match provider.acquire(&request) {
            Ok(mut result) => {
                if fixture && (result.result["mode"] != "FIXTURE" || result.provider_cost["request_count"] != 0) {
                    return Err("OFFICIAL_LIVE_ACQUISITION_NOT_AUTHORIZED".into());
                }
                if !fixture && (result.result["mode"] == "FIXTURE" || (result.result["mode"] == "LIVE" && result.provider_cost["known_request_count"].as_u64().unwrap_or(0)==0)) { return Err("OFFICIAL_LIVE_HTTP_WITNESS_REQUIRED".into()); }
                let directory = self.root.join(".ecdev-data/raw");
                std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
                let records = result.result["records"].as_array_mut().ok_or("OFFICIAL_CAPTURE_RECORDS_REQUIRED")?;
                for observation in &result.observations {
                    observation.validate()?;
                    let record = records.iter().find(|r| r["evidence_id"] == observation.id).ok_or("OFFICIAL_RAW_RECORD_MISSING")?;
                    let raw = record["raw_body"].as_str().ok_or("OFFICIAL_RAW_BODY_MISSING")?;
                    if format!("{:x}", Sha256::digest(raw.as_bytes())) != observation.raw_hash { return Err("OFFICIAL_RAW_HASH_MISMATCH".into()); }
                    std::fs::write(directory.join(format!("{}.json", observation.raw_hash)), raw.as_bytes()).map_err(|e| e.to_string())?;
                }
                for record in records { if let Some(object) = record.as_object_mut() { object.remove("raw_body"); } }
                self.persist(json!({"acquisition_run_id":id,"official_run":true,"mode":result.result["mode"],"run_kind":run_kind,"source_layer":"OFFICIAL_SP_API","status":result.result["status"],"observations":result.observations,"provider_cost":result.provider_cost,"provider_calls":[{"id":Uuid::new_v4().to_string(),"provider":"amazon-sp-api","capability":capability,"request_count":result.provider_cost["request_count"],"actual_cost_minor":result.provider_cost["actual_cost_minor"],"estimated_cost_minor":if fixture{json!(0)}else{json!(budget.request_ceiling_minor)},"cache_hit":false,"status":result.result["status"],"completed_at":timestamp()*1000}],"network_calls":result.provider_cost["request_count"],"known_network_calls":result.provider_cost["known_request_count"],"cost_minor":result.provider_cost["actual_cost_minor"],"new_live_acquisition":result.result["new_live_acquisition"],"fallback_providers":[],"commercial_validation":"INCOMPLETE_OFFICIAL_SOURCE_EVIDENCE_NOT_SUFFICIENT_FOR_PROFIT","result":result.result}))
            }
            Err(failure) => self.persist(json!({"acquisition_run_id":id,"mode":if fixture{"FIXTURE"}else if failure.request_count==Some(0){"PLAN_ONLY"}else{"INFERRED"},"run_kind":run_kind,"source_layer":"OFFICIAL_SP_API","status":"UNAVAILABLE","observations":[],"errors":[failure.reason],"acquisition_failure":failure,"network_calls":if fixture{json!(0)}else{json!(failure.request_count)},"cost_minor":if fixture || failure.request_count==Some(0){json!(0)}else{Value::Null},"new_live_acquisition":false,"fallback_providers":[]})),
        }
    }
}

/// The seller-read refusals that need no IO, in the order they are reported: intent shape,
/// market, evidence layer, then the restricted, write and unsupported operations.
pub(crate) fn seller_read_policy(args: &Value) -> Result<(), String> {
    let Value::Object(query) = args else {
        return Err("INVALID_SELLER_READ_INTENT".into());
    };
    query
        .get("market")
        .and_then(|m| m.as_str().map(str::to_string))
        .filter(|m| matches!(m.as_str(), "AMAZON_JP" | "AMAZON_US"))
        .ok_or("INVALID_SELLER_READ_MARKET")?;
    if query
        .get("evidence_layer")
        .is_some_and(|l| l != "OFFICIAL_SP_API")
    {
        return Err("SELLER_READ_IS_OFFICIAL_SP_API_ONLY".into());
    }
    match query.get("operation").and_then(Value::as_str) {
        Some(
            "LISTINGS_ITEM"
            | "INVENTORY_SUMMARIES"
            | "MARKETPLACE_PARTICIPATIONS"
            | "CATALOG_SEARCH_BY_IDENTIFIER"
            | "PRODUCT_TYPE_SEARCH"
            | "PRODUCT_TYPE_DEFINITION",
        ) => {}
        Some(other)
            if other.contains("ORDER") || other.contains("RDT") || other.contains("RESTRICTED") =>
        {
            return Err("RESTRICTED_DOMAIN_DISABLED".into());
        }
        _ => return Err("UNSUPPORTED_SELLER_READ_OPERATION".into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{AcquireError, AcquireResult, BudgetPolicy, Provider};
    use std::sync::Arc;
    struct Authorized;
    impl Provider for Authorized {
        fn id(&self) -> &str {
            "amazon-sp-api"
        }
        fn metadata(&self) -> Value {
            json!({"status":"AVAILABLE","live_supported":true,"live_authorized":true})
        }
        fn acquire(&self, _: &AcquireRequest) -> Result<AcquireResult, AcquireError> {
            panic!("zero monetary budget must prevent provider IO")
        }
    }
    #[test]
    fn official_authorized_transport_still_requires_core_budget_before_io() {
        let root = std::env::temp_dir().join(format!("ecdev-official-budget-{}", Uuid::new_v4()));
        {
            let engine = Engine::open(&root)
                .unwrap()
                .with_provider(Arc::new(Authorized));
            let result=engine.official_product_with_budget(json!({"market":"AMAZON_US","asin":"B00V5DG6IQ","evidence_layer":"OFFICIAL_SP_API"}),&BudgetPolicy::default()).unwrap();
            assert_eq!(result["status"], "DENIED_BUDGET");
            assert_eq!(result["network_calls"], 0);
            assert_eq!(result["new_live_acquisition"], false);
            let reservations: u64 = engine
                .db
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM paid_reservations", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(reservations, 0);
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn authorized_seller_read_still_requires_core_budget_before_io() {
        let root = std::env::temp_dir().join(format!("ecdev-seller-budget-{}", Uuid::new_v4()));
        {
            let engine = Engine::open(&root)
                .unwrap()
                .with_provider(Arc::new(Authorized));
            let result = engine
                .seller_read_with_budget(
                    json!({"market":"AMAZON_JP","operation":"MARKETPLACE_PARTICIPATIONS"}),
                    &BudgetPolicy::default(),
                )
                .unwrap();
            assert_eq!(result["status"], "DENIED_BUDGET");
            assert_eq!(result["run_kind"], Value::Null);
            assert_eq!(result["network_calls"], 0);
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
