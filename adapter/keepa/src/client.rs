use crate::resolve_current;
use ecdev_core::{
    domain::{Evidence, ObservationMode},
    provider::{AcquireError, AcquireRequest, AcquireResult, Provider},
    service::timestamp,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Read-only native provider client. Secrets are held only in memory, never returned.
pub struct Keepa {
    key: Option<String>,
}
fn before(reason: impl Into<String>) -> AcquireError {
    let mut error = AcquireError::from(reason.into());
    error.request_count = Some(0);
    error
}
fn response_failure(receipt: &AcquireError, reason: impl Into<String>) -> AcquireError {
    let mut error = receipt.clone();
    error.reason = reason.into();
    error
}
/// Keepa's own price and sales-rank history as EXTERNAL_HISTORY metrics, as of Keepa's last
/// update of the product: provider claims, never ECDEV observations; rank is a demand proxy.
fn external_history(product: &Value, currency: &str) -> Value {
    use crate::history::{BUY_BOX_SLOT, decode, keepa_minutes_to_unix};
    use ecdev_core::external_history::{price_history_metrics, rank_history_metrics};
    let Some(as_of) = product["lastUpdate"].as_i64().map(keepa_minutes_to_unix) else {
        return json!({"state":"EXTERNAL_HISTORY","status":"NO_LAST_UPDATE_NO_AS_OF"});
    };
    let csv = &product["csv"];
    json!({"state":"EXTERNAL_HISTORY","provider":"keepa","as_of":as_of,"currency":currency,
        "amazon_price":price_history_metrics(&decode(&csv[0], 0), as_of, currency),
        "new_price":price_history_metrics(&decode(&csv[1], 1), as_of, currency),
        "buy_box_price":price_history_metrics(&decode(&csv[BUY_BOX_SLOT], BUY_BOX_SLOT), as_of, currency),
        "sales_rank":rank_history_metrics(&decode(&csv[3], 3), as_of, 30),
        "contract":"research/commerce/keepa-history-review.json"})
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
          "sales_rank":resolve_current(product,3,None).filter(|rank|*rank>0),"rating_tenths":resolve_current(product,16,product.get("reviews").and_then(|r|r.get("rating"))),"review_count":resolve_current(product,17,product.get("reviews").and_then(|r|r.get("reviewCount"))),
          "weight_g":product["packageWeight"],"dimensions_mm":{"length":product["packageLength"],"width":product["packageWidth"],"height":product["packageHeight"]},
          "external_history":external_history(product, currency),
          "source_last_update_keepa_minutes":product["lastUpdate"],"note":"Rank is an observation, not a sales estimate; rank 0 is not a rank and stays UNKNOWN. No supplier, fees or demand inferred."}),
        )
    }
    fn acquire_at(
        &self,
        r: &AcquireRequest,
        client: &reqwest::blocking::Client,
        endpoint: &str,
    ) -> Result<AcquireResult, AcquireError> {
        let (key, domain, asin) = (|| -> Result<_, &str> {
            let key = self
                .key
                .as_ref()
                .ok_or("KEEPA_UNAVAILABLE: KEEPA_API_KEY missing")?;
            if r.capability != "product.analyze" {
                return Err("UNSUPPORTED_CAPABILITY");
            }
            let domain = match r.market.as_str() {
                "AMAZON_JP" => 5,
                "AMAZON_US" => 1,
                _ => return Err("UNSUPPORTED_MARKET"),
            };
            let asin = r.query["asin"].as_str().ok_or("ASIN_REQUIRED")?;
            if asin.len() != 10
                || !asin
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
            {
                return Err("ASIN must contain 10 uppercase letters or digits");
            }
            Ok((key, domain, asin))
        })()
        .map_err(before)?;
        let response = client
            .get(endpoint)
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
        let retry = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .filter(|s| s.len() <= 4096 && !s.chars().any(char::is_control));
        let receipt = AcquireError::http(status.as_u16(), 1, retry, (timestamp() * 1000) as i64);
        if !status.is_success() {
            return Err(response_failure(
                &receipt,
                match status.as_u16() {
                    400 => "PROVIDER_AUTH_OR_PARAMETER_ERROR",
                    402 => "PROVIDER_QUOTA_EXCEEDED",
                    429 => "PROVIDER_RATE_LIMITED",
                    _ => "PROVIDER_HTTP_ERROR",
                },
            ));
        }
        let mut raw = vec![];
        use std::io::Read;
        response
            .take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut raw)
            .map_err(|_| response_failure(&receipt, "PROVIDER_BODY_ERROR"))?;
        if raw.len() > 8 * 1024 * 1024 {
            return Err(response_failure(&receipt, "PROVIDER_RESPONSE_TOO_LARGE"));
        }
        let body: Value = serde_json::from_slice(&raw)
            .map_err(|_| response_failure(&receipt, "PROVIDER_INVALID_JSON"))?;
        let normalized =
            Self::normalize(r, &body).map_err(|reason| response_failure(&receipt, reason))?;
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
        obs.validate()
            .map_err(|reason| response_failure(&receipt, reason))?;
        Ok(AcquireResult {
            observations: vec![obs],
            result: normalized,
            raw_payload: raw,
            provider_cost: json!({"provider":"keepa","request_count":1,"actual_cost_minor":null,"tokens_consumed":body["tokensConsumed"],"tokens_left":body["tokensLeft"],"freshness":"UPSTREAM_TIMESTAMP_REPORTED","network_calls":1}),
        })
    }
}
impl Provider for Keepa {
    fn id(&self) -> &str {
        "keepa"
    }
    fn metadata(&self) -> Value {
        json!({"id":"keepa","class":"PAID","type":"TYPE_B","status":if self.key.is_some(){"AVAILABLE"}else{"UNAVAILABLE"},"auth_state":if self.key.is_some(){"CONFIGURED"}else{"MISSING"},"auth_verified":false,"health":"UNVERIFIED","adapter_state":"RUST_IMPLEMENTED","capabilities":["product.analyze"],"markets":["AMAZON_JP","AMAZON_US"],"cost_minor":null,"reason":"Paid optional product inspection; disabled by default budget. Auth and live correctness unverified. No retries."})
    }
    fn acquire(&self, r: &AcquireRequest) -> Result<AcquireResult, AcquireError> {
        let client = reqwest::blocking::Client::builder()
            .https_only(true)
            .timeout(std::time::Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| before("HTTP_CLIENT_ERROR"))?;
        self.acquire_at(r, &client, "https://api.keepa.com/product")
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
        let failure = p.acquire(&r).err().unwrap();
        assert_eq!(failure.request_count, Some(0));
        assert!(failure.reason.contains("KEEPA_UNAVAILABLE"));
    }
    #[test]
    fn product_history_is_external_and_rank_is_a_demand_proxy() {
        let r = AcquireRequest {
            run_id: "t".into(),
            capability: "product.analyze".into(),
            market: "AMAZON_JP".into(),
            query: json!({"asin":"B000000001"}),
        };
        let mut csv = vec![Value::Null; 19];
        // 3000 JPY, unavailable, then 2700; rank improving from 900 to 300 over 30 days.
        csv[0] = json!([7000000, 3000, 7014400, -1, 7028800, 2700]);
        csv[3] = json!(
            (0..=30)
                .flat_map(|d| [7000000 + d * 1440, 900 - 20 * d])
                .collect::<Vec<_>>()
        );
        let product =
            json!({"asin":"B000000001","title":"Cup","lastUpdate":7000000 + 31 * 1440,"csv":csv});
        let v = Keepa::normalize(&r, &json!({"products":[product]})).unwrap();
        let h = &v["external_history"];
        assert_eq!(
            (h["state"].clone(), h["currency"].clone()),
            (json!("EXTERNAL_HISTORY"), json!("JPY"))
        );
        assert_eq!(h["amazon_price"]["discount_events"], 1);
        assert!(h["amazon_price"]["unavailable_share"].as_f64().unwrap() > 0.3);
        assert_eq!(h["sales_rank"]["evidence_class"], "DEMAND_PROXY");
        assert_eq!(h["sales_rank"]["meaning"], "SALES_RANK_NOT_SALES");
        assert_eq!(h["sales_rank"]["direction"], "IMPROVING");
        assert_eq!(h["new_price"]["status"], "NO_HISTORY");
        let no_update =
            Keepa::normalize(&r, &json!({"products":[{"asin":"B000000001","csv":[]}]})).unwrap();
        assert_eq!(
            no_update["external_history"]["status"],
            "NO_LAST_UPDATE_NO_AS_OF"
        );
    }
    #[test]
    fn donor_product_summary_oracle() {
        let oracle: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/keepa-product-summary-oracle.json"
        ))
        .unwrap();
        let cases = oracle["cases"].as_array().unwrap();
        assert!(cases.len() >= 300);
        for case in cases {
            let asin = case["product"]["asin"].as_str().unwrap();
            let r = AcquireRequest {
                run_id: "oracle".into(),
                capability: "product.analyze".into(),
                market: "AMAZON_US".into(),
                query: json!({ "asin": asin }),
            };
            let v = Keepa::normalize(&r, &json!({ "products": [case["product"]] })).unwrap();
            let want = &case["expected"];
            let id = &case["case_id"];
            assert_eq!(v["price_minor"], want["current_price_amazon_cents"], "{id}");
            assert_eq!(
                v["new_price_minor"], want["current_price_new_cents"],
                "{id}"
            );
            assert_eq!(
                v["buy_box_price_minor"], want["buy_box_price_cents"],
                "{id}"
            );
            assert_eq!(v["review_count"], want["review_count"], "{id}");
            assert_eq!(v["sales_rank"], want["sales_rank"], "{id}");
            // The donor divides the 0-50 rating by ten; ECDEV keeps the integer tenths.
            assert_eq!(
                v["rating_tenths"].as_i64().map(|t| t as f64 / 10.0),
                want["rating"].as_f64(),
                "{id}"
            );
        }
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
    #[test]
    fn http_receipts_keep_counts_status_retry_and_secret_boundaries() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
            thread,
            time::Duration,
        };
        let request = AcquireRequest {
            run_id: "mock-receipt".into(),
            capability: "product.analyze".into(),
            market: "AMAZON_JP".into(),
            query: json!({"asin":"B08N5WRWNW"}),
        };
        for (status, body, reason) in [
            (400, "mock-secret", "PROVIDER_AUTH_OR_PARAMETER_ERROR"),
            (402, "{}", "PROVIDER_QUOTA_EXCEEDED"),
            (429, "{}", "PROVIDER_RATE_LIMITED"),
            (301, "{}", "PROVIDER_HTTP_ERROR"),
            (200, "not-json", "PROVIDER_INVALID_JSON"),
            (200, "{\"products\":[]}", "PRODUCT_NOT_FOUND"),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let endpoint = format!("http://{}/product", listener.local_addr().unwrap());
            let server = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut bytes = vec![];
                while !bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                    let mut chunk = [0; 4096];
                    let n = stream.read(&mut chunk).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&chunk[..n]);
                }
                write!(stream,"HTTP/1.1 {status} Mock\r\nContent-Length: {}\r\nRetry-After: 5\r\nLocation: https://example.invalid/\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
                String::from_utf8(bytes).unwrap()
            });
            let client = reqwest::blocking::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(3))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap();
            let failure = Keepa {
                key: Some("mock-secret".into()),
            }
            .acquire_at(&request, &client, &endpoint)
            .err()
            .unwrap();
            assert_eq!(failure.http_status, Some(status));
            assert_eq!(failure.request_count, Some(1));
            assert_eq!(failure.reason, reason);
            assert_eq!(failure.retry_after_header.as_deref(), Some("5"));
            assert!(failure.retry_not_before_ms.is_some());
            assert!(
                !serde_json::to_string(&failure)
                    .unwrap()
                    .contains("mock-secret")
            );
            let wire = server.join().unwrap();
            assert!(wire.starts_with("GET /product?"));
            assert!(wire.contains("domain=5"));
            assert!(wire.contains("asin=B08N5WRWNW"));
        }
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/product", listener.local_addr().unwrap());
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .build()
            .unwrap();
        let mut invalid = request;
        invalid.market = "UNSUPPORTED".into();
        assert_eq!(
            Keepa {
                key: Some("mock-secret".into())
            }
            .acquire_at(&invalid, &client, &endpoint)
            .err()
            .unwrap()
            .request_count,
            Some(0)
        );
        assert!(listener.accept().is_err());
    }
}
