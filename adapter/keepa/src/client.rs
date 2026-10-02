use crate::resolve_current;
use ecdev_core::{
    domain::{Evidence, ObservationMode},
    provider::{AcquireRequest, AcquireResult, Provider},
    service::timestamp,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Read-only native provider client. Secrets are held only in memory, never returned.
pub struct Keepa {
    key: Option<String>,
}
impl Keepa {
    pub fn from_env() -> Self {
        Self {
            key: std::env::var("KEEPA_API_KEY")
                .ok()
                .filter(|k| !k.is_empty()),
        }
    }
    fn normalize(request: &AcquireRequest, body: &Value) -> Result<Value, String> {
        let product = body["products"]
            .as_array()
            .and_then(|a| a.first())
            .ok_or("PRODUCT_NOT_FOUND")?;
        let expected = request.query["asin"].as_str().ok_or("ASIN_REQUIRED")?;
        if product["asin"].as_str() != Some(expected) {
            return Err("UPSTREAM_PRODUCT_ID_MISMATCH".into());
        }
        let currency = if request.market == "AMAZON_JP" {
            "JPY"
        } else {
            "USD"
        };
        Ok(
            json!({"id":format!("{}:{expected}",request.market),"kind":"PRODUCT","asin":expected,"market":request.market,
          "title":product["title"],"brand":product["brand"],"category":product["categoryTree"],"currency":currency,
          "price_minor":resolve_current(product,0,None),"new_price_minor":resolve_current(product,1,None),"buy_box_price_minor":resolve_current(product,18,None),
          "sales_rank":resolve_current(product,3,None),"rating_tenths":resolve_current(product,16,None),"review_count":resolve_current(product,17,None),
          "weight_g":product["packageWeight"],"dimensions_mm":{"length":product["packageLength"],"width":product["packageWidth"],"height":product["packageHeight"]},
          "source_last_update_keepa_minutes":product["lastUpdate"],"note":"Rank is an observation, not a sales estimate. No supplier, fees or demand inferred."}),
        )
    }
}
impl Provider for Keepa {
    fn id(&self) -> &str {
        "keepa"
    }
    fn metadata(&self) -> Value {
        json!({"id":"keepa","class":"PAID","type":"TYPE_B","status":if self.key.is_some(){"AVAILABLE"}else{"UNAVAILABLE"},"auth_state":if self.key.is_some(){"CONFIGURED"}else{"MISSING"},"auth_verified":false,"health":"UNVERIFIED","adapter_state":"RUST_IMPLEMENTED","capabilities":["product.analyze"],"markets":["AMAZON_JP","AMAZON_US"],"cost_minor":null,"reason":"Paid optional product inspection; disabled by default budget. Auth and live correctness unverified. No retries."})
    }
    fn acquire(&self, r: &AcquireRequest) -> Result<AcquireResult, String> {
        let key = self
            .key
            .as_ref()
            .ok_or("KEEPA_UNAVAILABLE: KEEPA_API_KEY missing")?;
        if r.capability != "product.analyze" {
            return Err("UNSUPPORTED_CAPABILITY".into());
        }
        let domain = match r.market.as_str() {
            "AMAZON_JP" => 5,
            "AMAZON_US" => 1,
            _ => return Err("UNSUPPORTED_MARKET".into()),
        };
        let asin = r.query["asin"].as_str().ok_or("ASIN_REQUIRED")?;
        if asin.len() != 10
            || !asin
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        {
            return Err("ASIN must contain 10 uppercase letters or digits".into());
        }
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "HTTP_CLIENT_ERROR")?;
        let response = client
            .get("https://api.keepa.com/product")
            .query(&[
                ("key", key.as_str()),
                ("domain", &domain.to_string()),
                ("asin", asin),
                ("stats", "90"),
                ("history", "1"),
                ("rating", "1"),
                ("buybox", "1"),
            ])
            .header("Accept", "application/json")
            .send()
            .map_err(|e| {
                if e.is_timeout() {
                    "PROVIDER_TIMEOUT"
                } else {
                    "PROVIDER_NETWORK_ERROR"
                }
            })?;
        let status = response.status();
        if !status.is_success() {
            return Err(match status.as_u16() {
                400 => "PROVIDER_AUTH_OR_PARAMETER_ERROR",
                402 => "PROVIDER_QUOTA_EXCEEDED",
                429 => "PROVIDER_RATE_LIMITED",
                _ => "PROVIDER_HTTP_ERROR",
            }
            .into());
        }
        let mut raw = vec![];
        use std::io::Read;
        response
            .take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut raw)
            .map_err(|_| "PROVIDER_BODY_ERROR")?;
        if raw.len() > 8 * 1024 * 1024 {
            return Err("PROVIDER_RESPONSE_TOO_LARGE".into());
        }
        let body: Value = serde_json::from_slice(&raw).map_err(|_| "PROVIDER_INVALID_JSON")?;
        let normalized = Self::normalize(r, &body)?;
        let hash = format!("{:x}", Sha256::digest(&raw));
        let obs = Evidence {
            id: Uuid::new_v4().to_string(),
            mode: ObservationMode::Live,
            source_type: "API".into(),
            provider: "keepa".into(),
            external_source: "https://api.keepa.com/product".into(),
            market: r.market.clone(),
            query: r.query.clone(),
            timestamp: body["timestamp"]
                .as_i64()
                .map(|t| t.to_string())
                .unwrap_or_else(|| (timestamp() * 1000).to_string()),
            retrieved_at: (timestamp() * 1000).to_string(),
            raw_hash: hash,
            normalized_value: normalized.clone(),
            unit: "PROVIDER_REPORTED".into(),
            currency: Some(normalized["currency"].as_str().unwrap().into()),
            confidence: None,
            freshness_seconds: None,
            cost_minor: None,
            run_id: r.run_id.clone(),
        };
        obs.validate()?;
        Ok(AcquireResult {
            observations: vec![obs],
            result: normalized,
            raw_payload: raw,
            provider_cost: json!({"provider":"keepa","request_count":1,"actual_cost_minor":null,"tokens_consumed":body["tokensConsumed"],"tokens_left":body["tokensLeft"],"freshness":"UPSTREAM_TIMESTAMP_REPORTED","network_calls":1}),
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_key_performs_no_network() {
        let p = Keepa { key: None };
        let r = AcquireRequest {
            run_id: "fixture-run".into(),
            capability: "product.analyze".into(),
            market: "AMAZON_JP".into(),
            query: json!({"asin":"B08N5WRWNW"}),
        };
        assert!(
            p.acquire(&r)
                .unwrap_err_text()
                .contains("KEEPA_UNAVAILABLE")
        );
    }
    #[test]
    fn currency_does_not_follow_donor_usd_formatter() {
        let r = AcquireRequest {
            run_id: "fixture-run".into(),
            capability: "product.analyze".into(),
            market: "AMAZON_JP".into(),
            query: json!({"asin":"B08N5WRWNW"}),
        };
        let v = Keepa::normalize(
            &r,
            &json!({"products":[{"asin":"B08N5WRWNW","stats":{"current":[4000]}}]}),
        )
        .unwrap();
        assert_eq!(v["currency"], "JPY");
        assert_eq!(v["price_minor"], 4000);
        assert!(v["review_count"].is_null());
    }
    trait ErrorText {
        fn unwrap_err_text(self) -> String;
    }
    impl ErrorText for Result<AcquireResult, String> {
        fn unwrap_err_text(self) -> String {
            match self {
                Err(e) => e,
                Ok(_) => panic!("Expected unavailable"),
            }
        }
    }
}
