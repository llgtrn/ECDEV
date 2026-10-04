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
    pub(crate) fn official_product(&self, args: Value) -> Result<Value, String> {
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
        let id = Uuid::new_v4().to_string();
        if !fixture {
            return self.persist(json!({"acquisition_run_id":id,"mode":"PLAN_ONLY","source_layer":"OFFICIAL_SP_API","status":"UNAVAILABLE","reason":"OFFICIAL_LIVE_AUTH_TRANSPORT_UNAVAILABLE_NO_REQUESTS","observations":[],"network_calls":0,"cost_minor":0,"new_live_acquisition":false,"fallback_providers":[]}));
        }
        let request = AcquireRequest {
            run_id: id.clone(),
            capability: "product.analyze.official".into(),
            market: intent.market,
            query: json!({"asin":intent.asin,"include":intent.include,"fee_estimate":intent.fee_estimate,"fixture_responses":intent.fixture_responses}),
        };
        let Some(provider) = self.providers.iter().find(|p| p.id() == "amazon-sp-api") else {
            return self.persist(json!({"acquisition_run_id":id,"mode":"PLAN_ONLY","source_layer":"OFFICIAL_SP_API","status":"UNAVAILABLE","reason":"OFFICIAL_ADAPTER_UNAVAILABLE","observations":[],"network_calls":0,"cost_minor":0,"fallback_providers":[]}));
        };
        match provider.acquire(&request) {
            Ok(mut result) => {
                if !fixture || result.result["mode"] != "FIXTURE" || result.provider_cost["request_count"] != 0 {
                    return Err("OFFICIAL_LIVE_ACQUISITION_NOT_AUTHORIZED".into());
                }
                let directory = self.root.join(".ynventa/materialized/raw");
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
                self.persist(json!({"acquisition_run_id":id,"official_run":true,"mode":"FIXTURE","run_kind":"OFFICIAL_PRODUCT","source_layer":"OFFICIAL_SP_API","status":result.result["status"],"result":result.result,"observations":result.observations,"provider_cost":result.provider_cost,"provider_calls":[{"id":Uuid::new_v4().to_string(),"provider":"amazon-sp-api","capability":"product.analyze.official","request_count":0,"actual_cost_minor":0,"cache_hit":false,"status":"FIXTURE","completed_at":timestamp()*1000}],"network_calls":0,"cost_minor":0,"new_live_acquisition":false,"fallback_providers":[],"commercial_validation":"INCOMPLETE_SUPPLIED_FIXTURE_NOT_AUTHENTICATED"}))
            }
            Err(failure) => self.persist(json!({"acquisition_run_id":id,"mode":if fixture{"FIXTURE"}else{"PLAN_ONLY"},"run_kind":"OFFICIAL_PRODUCT","source_layer":"OFFICIAL_SP_API","status":"UNAVAILABLE","observations":[],"errors":[failure.reason],"network_calls":0,"cost_minor":0,"new_live_acquisition":false,"fallback_providers":[]})),
        }
    }
}
