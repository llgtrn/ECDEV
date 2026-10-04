//! Official marketplace protocol boundary with explicit operator-gated HTTP and isolated fixtures.
pub mod transport;
use ecdev_core::{
    domain::{Evidence, ObservationMode},
    provider::{AcquireError, AcquireRequest, AcquireResult, Provider},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const MODEL_COMMIT: &str = "3677bb9d96f4450e6843f1f8207005e925d5c867";
pub const LWA_ENDPOINT: &str = "https://api.amazon.com/auth/o2/token";
pub const MAX_BODY: usize = 4 * 1024 * 1024;

pub fn market(market: &str) -> Result<(&'static str, &'static str, &'static str), String> {
    match market {
        "AMAZON_US" => Ok((
            "ATVPDKIKX0DER",
            "https://sellingpartnerapi-na.amazon.com",
            "USD",
        )),
        "AMAZON_JP" => Ok((
            "A1VC38T7YXB528",
            "https://sellingpartnerapi-fe.amazon.com",
            "JPY",
        )),
        _ => Err("UNSUPPORTED_OFFICIAL_MARKET".into()),
    }
}

pub fn lwa_refresh_form<'a>(
    client_id: &'a str,
    client_secret: &'a str,
    refresh_token: &'a str,
) -> Result<[(&'static str, &'a str); 4], String> {
    if [client_id, client_secret, refresh_token]
        .iter()
        .any(|s| s.is_empty() || s.len() > 2048)
    {
        return Err("LWA_CREDENTIALS_REQUIRED_OR_OVERSIZED".into());
    }
    Ok([
        ("grant_type", "refresh_token"),
        ("client_id", client_id),
        ("client_secret", client_secret),
        ("refresh_token", refresh_token),
    ])
}

// Never Debug/Serialize: access tokens are ephemeral secrets, not evidence captures.
pub struct LwaToken {
    token: String,
    expires_at_ms: u64,
}
impl LwaToken {
    pub fn from_response(response: &Value, received_at_ms: u64) -> Result<Self, String> {
        let token = response["access_token"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 2048 && !s.chars().any(char::is_control))
            .ok_or("INVALID_LWA_ACCESS_TOKEN")?;
        let ttl = response["expires_in"]
            .as_u64()
            .filter(|n| (1..=3600).contains(n))
            .ok_or("INVALID_LWA_TOKEN_EXPIRY")?;
        if !response["token_type"]
            .as_str()
            .is_some_and(|s| s.eq_ignore_ascii_case("bearer"))
        {
            return Err("INVALID_LWA_TOKEN_TYPE".into());
        }
        Ok(Self {
            token: token.into(),
            expires_at_ms: received_at_ms
                .checked_add(ttl * 1000)
                .ok_or("LWA_EXPIRY_OVERFLOW")?,
        })
    }
    pub fn access_token_at(&self, now_ms: u64) -> Option<&str> {
        // Renew before expiry; no expired token can be used for a request.
        (now_ms.checked_add(30_000)? < self.expires_at_ms).then_some(self.token.as_str())
    }
}

fn money(value: &Value, currency: &str) -> Result<Value, String> {
    let amount = value["amount"]
        .as_str()
        .ok_or("FEE_PRICE_REQUIRES_EXACT_DECIMAL_STRING")?;
    let mut parts = amount.split('.');
    let whole = parts.next().unwrap_or("");
    let fraction = parts.next();
    if amount.len() > 80
        || whole.is_empty()
        || !whole.bytes().all(|c| c.is_ascii_digit())
        || fraction.is_some_and(|f| f.is_empty() || !f.bytes().all(|c| c.is_ascii_digit()))
        || parts.next().is_some()
        || value["currency"] != currency
    {
        return Err("INVALID_FEE_PRICE_OR_MARKET_CURRENCY".into());
    }
    let number: Value = serde_json::from_str(amount).map_err(|_| "INVALID_FEE_PRICE")?;
    if !number.is_number() {
        return Err("INVALID_FEE_PRICE".into());
    }
    Ok(json!({"Amount":number,"CurrencyCode":currency}))
}

pub fn protocol(request: &AcquireRequest) -> Result<Value, String> {
    let (marketplace, endpoint, currency) = market(&request.market)?;
    let asin = request.query["asin"]
        .as_str()
        .filter(|s| {
            s.len() == 10
                && s.bytes()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        })
        .ok_or("INVALID_ASIN")?;
    let includes = request.query["include"]
        .as_array()
        .cloned()
        .unwrap_or_else(|| vec![json!("CATALOG"), json!("OFFERS")]);
    if includes.is_empty()
        || includes.len() > 3
        || includes
            .iter()
            .any(|v| !matches!(v.as_str(), Some("CATALOG" | "OFFERS" | "FEE_ESTIMATE")))
        || includes
            .iter()
            .filter_map(Value::as_str)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != includes.len()
    {
        return Err("INVALID_OFFICIAL_PRODUCT_INCLUDE".into());
    }
    let mut operations = Vec::new();
    for included in includes {
        let (key, method, path, query, body, rate, burst) = match included.as_str().unwrap() {
            "CATALOG" => (
                "catalog",
                "GET",
                format!("/catalog/2022-04-01/items/{asin}"),
                json!({"marketplaceIds":marketplace,"includedData":"attributes,identifiers,images,summaries"}),
                Value::Null,
                json!(2),
                2,
            ),
            "OFFERS" => (
                "offers",
                "GET",
                format!("/products/pricing/v0/items/{asin}/offers"),
                json!({"MarketplaceId":marketplace,"ItemCondition":"New","CustomerType":"Consumer"}),
                Value::Null,
                json!(0.5),
                1,
            ),
            "FEE_ESTIMATE" => {
                let fee = &request.query["fee_estimate"];
                let fba = fee["is_amazon_fulfilled"]
                    .as_bool()
                    .ok_or("FEE_FULFILLMENT_ASSUMPTION_REQUIRED")?;
                let identifier = fee["identifier"].as_str().unwrap_or(&request.run_id);
                if identifier.is_empty() || identifier.len() > 128 {
                    return Err("INVALID_FEE_IDENTIFIER".into());
                }
                let mut price = json!({"ListingPrice":money(&fee["listing_price"], currency)?});
                if !fee["shipping"].is_null() {
                    price["Shipping"] = money(&fee["shipping"], currency)?;
                }
                if !fee["points"].is_null() {
                    let count = fee["points"]["count"]
                        .as_u64()
                        .ok_or("INVALID_FEE_POINTS")?;
                    price["Points"] = json!({"PointsNumber":count,"PointsMonetaryValue":money(&fee["points"]["value"], currency)?});
                }
                (
                    "fees",
                    "POST",
                    format!("/products/fees/v0/items/{asin}/feesEstimate"),
                    json!({}),
                    json!({"FeesEstimateRequest":{"MarketplaceId":marketplace,"IsAmazonFulfilled":fba,"PriceToEstimateFees":price,"Identifier":identifier}}),
                    json!(1),
                    2,
                )
            }
            _ => unreachable!(),
        };
        operations.push(json!({"operation":key,"method":method,"requested_url":format!("{endpoint}{path}"),"query":query,"body":body,"default_rate_per_second":rate,"default_burst":burst,"rate_scope":"DEFAULT_OPERATION_PLAN_NOT_GUARANTEED_ACCOUNT_QUOTA","auth":"LWA_SELLER_REFRESH_TOKEN_NON_RESTRICTED","rdt":"NOT_USED_FOR_THESE_NON_PII_OPERATIONS","aws_sigv4_required":false}));
    }
    Ok(
        json!({"marketplace_id":marketplace,"region_endpoint":endpoint,"currency":currency,"asin":asin,"operations":operations,"model_commit":MODEL_COMMIT}),
    )
}

pub fn normalize(operation: &str, body: &Value, plan: &Value) -> Result<Value, String> {
    if body["errors"].as_array().is_some_and(|a| !a.is_empty()) {
        return Err("OFFICIAL_RESPONSE_ERRORS".into());
    }
    let asin = &plan["asin"];
    let marketplace = &plan["marketplace_id"];
    match operation {
        "catalog" => {
            if body["asin"] != *asin {
                return Err("CATALOG_ASIN_MISMATCH".into());
            }
            let entity = ecdev_core::marketplace::amazon_catalog(
                body,
                marketplace.as_str().ok_or("INVALID_MARKET")?,
            )?;
            Ok(json!({"catalog_item":entity,"sales":null,"inventory":null,"observed_price":null}))
        }
        "offers" => {
            let p = &body["payload"];
            if p["marketplaceId"] != *marketplace
                || p["Identifier"]["MarketplaceId"] != *marketplace
                || p["Identifier"]["ASIN"] != *asin
                || (!p["ASIN"].is_null() && p["ASIN"] != *asin)
            {
                return Err("OFFERS_IDENTITY_OR_MARKET_MISMATCH".into());
            }
            if p["status"] != "Success"
                || p["ItemCondition"] != "New"
                || p["Identifier"]["ItemCondition"] != "New"
                || !p["Offers"].is_array()
                || !p["Summary"].is_object()
            {
                return Err("INVALID_OFFERS_SUCCESS_CONTRACT".into());
            }
            for offer in p["Offers"].as_array().unwrap() {
                if !offer["IsFulfilledByAmazon"].is_boolean()
                    || !offer["ListingPrice"].is_object()
                    || !offer["Shipping"].is_object()
                    || !offer["ShippingTime"].is_object()
                    || !offer["SubCondition"].is_string()
                {
                    return Err("INVALID_OFFER_REQUIRED_FIELDS".into());
                }
            }
            Ok(
                json!({"offers":p["Offers"],"summary":p["Summary"],"sales":null,"demand":null,"price_selection":"NONE_RAW_OFFICIAL_MONEY_RETAINED"}),
            )
        }
        "fees" => {
            let p = &body["payload"]["FeesEstimateResult"];
            let id = &p["FeesEstimateIdentifier"];
            let operation = plan["operations"]
                .as_array()
                .and_then(|a| a.iter().find(|o| o["operation"] == "fees"))
                .ok_or("FEE_REQUEST_UNAVAILABLE")?;
            let request = &operation["body"]["FeesEstimateRequest"];
            if id["MarketplaceId"] != *marketplace
                || id["IdType"] != "ASIN"
                || id["IdValue"] != *asin
                || id["SellerInputIdentifier"] != request["Identifier"]
                || id["IsAmazonFulfilled"] != request["IsAmazonFulfilled"]
                || id["PriceToEstimateFees"] != request["PriceToEstimateFees"]
            {
                return Err("FEE_ESTIMATE_CORRELATION_MISMATCH".into());
            }
            if p["Status"] != "Success"
                || !p["FeesEstimate"].is_object()
                || p["Error"]["Code"].as_str().is_some_and(|s| !s.is_empty())
            {
                return Err("FEE_ESTIMATE_FAILED_OR_MISSING".into());
            }
            Ok(
                json!({"state":"ESTIMATED","estimate":p["FeesEstimate"],"assumptions":request,"actual_fees":null,"expected_profit":null,"limitation":"Official estimate, not actual fulfillment cost; capture time does not replace TimeOfFeesEstimation"}),
            )
        }
        _ => Err("UNSUPPORTED_OFFICIAL_OPERATION".into()),
    }
}

#[derive(Default)]
pub struct Amazon;
impl Amazon {
    pub fn from_env() -> transport::ConfiguredAmazon {
        transport::ConfiguredAmazon::from_env()
    }
}
impl Provider for Amazon {
    fn id(&self) -> &str {
        "amazon-sp-api"
    }
    fn metadata(&self) -> Value {
        let configured = [
            "SP_API_CLIENT_ID",
            "SP_API_CLIENT_SECRET",
            "SP_API_REFRESH_TOKEN",
        ]
        .iter()
        .all(|k| std::env::var_os(k).is_some_and(|v| !v.is_empty()));
        json!({"id":self.id(),"class":"OFFICIAL","source_layer":"OFFICIAL_SP_API","status":"UNAVAILABLE","adapter_state":"NATIVE_FIXTURE_PROTOCOL_LIVE_AUTH_NOT_IMPLEMENTED","auth_state":if configured{"CREDENTIALS_PRESENT_LIVE_NOT_AUTHORIZED"}else{"AUTH_REQUIRED"},"capabilities":["product.analyze.official"],"markets":["AMAZON_JP","AMAZON_US"],"fixture_supported":true,"live_supported":false,"fallback_providers":[],"cacheable":false,"estimated_cost_minor":null,"model_commit":MODEL_COMMIT,"reason":"Fixture and protocol boundary available; authenticated transport and actual account quota unverified; no public HTML or Keepa substitution"})
    }
    fn acquire(&self, request: &AcquireRequest) -> Result<AcquireResult, AcquireError> {
        if request.capability != "product.analyze.official" {
            return Err("UNSUPPORTED_OFFICIAL_CAPABILITY".into());
        }
        let plan = protocol(request).map_err(AcquireError::from)?;
        let Some(fixtures) = request.query["fixture_responses"].as_object() else {
            let mut denied =
                AcquireError::from("OFFICIAL_LIVE_AUTH_TRANSPORT_UNAVAILABLE_NO_REQUESTS");
            denied.request_count = Some(0);
            return Err(denied);
        };
        if fixtures
            .keys()
            .any(|k| !matches!(k.as_str(), "catalog" | "offers" | "fees"))
        {
            return Err("INVALID_FIXTURE_OPERATION".into());
        }
        let mut observations = Vec::new();
        let mut records = Vec::new();
        let mut total_bytes = 0usize;
        for operation in plan["operations"].as_array().unwrap() {
            let key = operation["operation"].as_str().unwrap();
            let Some(capture) = fixtures.get(key) else {
                records.push(json!({"operation":key,"status":"UNAVAILABLE","reason":"FIXTURE_CAPTURE_NOT_SUPPLIED","request_count":0}));
                continue;
            };
            let raw = capture["raw_body"]
                .as_str()
                .filter(|s| s.len() <= MAX_BODY)
                .ok_or_else(|| AcquireError::from("FIXTURE_BODY_REQUIRED_OR_OVERSIZED"))?;
            let status = capture["status"]
                .as_u64()
                .filter(|s| (100..=599).contains(s))
                .ok_or_else(|| AcquireError::from("FIXTURE_HTTP_STATUS_REQUIRED"))?;
            total_bytes += raw.len();
            if total_bytes > 8 * 1024 * 1024 {
                return Err("OFFICIAL_FIXTURE_TOTAL_BODY_LIMIT".into());
            }
            for header in ["x-amzn-requestid", "x-amzn-ratelimit-limit", "retry-after"] {
                let value = &capture["headers"][header];
                if !value.is_null()
                    && !value
                        .as_str()
                        .is_some_and(|s| s.len() <= 4096 && !s.chars().any(char::is_control))
                {
                    return Err("INVALID_OFFICIAL_RESPONSE_HEADER".into());
                }
            }
            let hash = format!("{:x}", Sha256::digest(raw.as_bytes()));
            let headers = json!({"x-amzn-requestid":capture["headers"]["x-amzn-requestid"],"x-amzn-ratelimit-limit":capture["headers"]["x-amzn-ratelimit-limit"],"retry-after":capture["headers"]["retry-after"]});
            let normalized = if status == 200 {
                let value: Value = serde_json::from_str(raw)
                    .map_err(|_| AcquireError::from("MALFORMED_OFFICIAL_FIXTURE_JSON"))?;
                normalize(key, &value, &plan)
            } else {
                Err(format!("OFFICIAL_HTTP_{status}"))
            };
            let evidence_id = Uuid::new_v4().to_string();
            records.push(json!({"operation":key,"status":if normalized.is_ok(){"COMPLETE"}else{"UNAVAILABLE"},"normalized":normalized.as_ref().ok(),"reason":normalized.as_ref().err(),"http_status":status,"response_headers":headers,"raw_capture_sha256":hash,"raw_body":raw,"evidence_id":evidence_id,"requested_url":operation["requested_url"],"final_url":null,"request_count":0,"state":"FIXTURE","source_authenticity":"UNVERIFIED_SUPPLIED_FIXTURE_NOT_AUTHENTICATED","rate_scope":"HEADER_ACCOUNT_APPLICATION_PAIR_NOT_ALL_LIMITS"}));
            observations.push(Evidence { id: evidence_id, mode: ObservationMode::Fixture, source_type:"API".into(), provider:self.id().into(), external_source:operation["requested_url"].as_str().unwrap().into(), market:request.market.clone(), query:operation.clone(), timestamp:ecdev_core::service::timestamp().to_string(), retrieved_at:ecdev_core::service::timestamp().to_string(), raw_hash:hash, normalized_value:json!({"operation":key,"status":if normalized.is_ok(){"DERIVED"}else{"UNKNOWN"},"value":normalized.ok(),"source_authenticity":"UNVERIFIED_SUPPLIED_FIXTURE"}), unit:"OFFICIAL_PROTOCOL_FIXTURE".into(), currency:Some(plan["currency"].as_str().unwrap().into()), confidence:None, freshness_seconds:None, cost_minor:Some(0), run_id:request.run_id.clone() });
        }
        let complete = records.iter().all(|r| r["status"] == "COMPLETE");
        let result = json!({"source_layer":"OFFICIAL_SP_API","mode":"FIXTURE","status":if complete{"COMPLETE_FIXTURE_WITH_UNKNOWNS"}else{"PARTIAL"},"plan":plan,"records":records,"official_live_validation":"UNAVAILABLE","sales":null,"demand":null,"profit_expected":null,"new_live_acquisition":false});
        Ok(AcquireResult {
            raw_payload: serde_json::to_vec(&result)
                .map_err(|_| AcquireError::from("FIXTURE_SERIALIZATION_FAILED"))?,
            observations,
            result,
            provider_cost: json!({"mode":"FIXTURE","request_count":0,"actual_cost_minor":0,"live_account_quota":null}),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cases() -> Value {
        serde_json::from_str(include_str!("../tests/fixtures/official-responses.json")).unwrap()
    }
    fn request(case: &Value) -> AcquireRequest {
        let mut query = json!({"asin":case["asin"],"include":[match case["operation"].as_str().unwrap(){"catalog"=>"CATALOG","offers"=>"OFFERS",_=>"FEE_ESTIMATE"}],"fixture_responses":{case["operation"].as_str().unwrap():case["response"]}});
        if case["operation"] == "fees" {
            query["fee_estimate"] = json!({"listing_price":{"currency":"USD","amount":"10"},"shipping":{"currency":"USD","amount":"10"},"is_amazon_fulfilled":false,"identifier":"UmaS1","points":{"count":0,"value":{"currency":"USD","amount":"0"}}});
        }
        AcquireRequest {
            run_id: "fixture-run".into(),
            capability: "product.analyze.official".into(),
            market: "AMAZON_US".into(),
            query,
        }
    }
    #[test]
    fn locked_official_model_examples_preserve_unknowns_and_estimated_fees() {
        let fixtures = cases();
        assert_eq!(fixtures["commit"], MODEL_COMMIT);
        for case in fixtures["cases"].as_array().unwrap() {
            let request = request(case);
            let result = Amazon.acquire(&request).unwrap();
            assert_eq!(result.result["status"], "COMPLETE_FIXTURE_WITH_UNKNOWNS");
            assert_eq!(result.provider_cost["request_count"], 0);
            assert_eq!(result.observations[0].mode, ObservationMode::Fixture);
            result.observations[0].validate().unwrap();
            assert_eq!(
                result.observations[0].raw_hash,
                format!(
                    "{:x}",
                    Sha256::digest(case["response"]["raw_body"].as_str().unwrap().as_bytes())
                )
            );
            assert!(result.result["sales"].is_null());
            assert!(result.result["profit_expected"].is_null());
            if case["operation"] == "fees" {
                let fee = &result.result["records"][0]["normalized"];
                assert_eq!(fee["state"], "ESTIMATED");
                assert_eq!(
                    fee["estimate"]["TimeOfFeesEstimation"],
                    "Mon Oct 28 18:49:32 UTC 2019"
                );
                assert!(fee["actual_fees"].is_null());
            }
        }
    }
    #[test]
    fn official_region_identity_and_fee_assumptions_cannot_be_substituted() {
        let fixture = cases();
        let case = &fixture["cases"][1];
        let mut r = request(case);
        let us = protocol(&r).unwrap();
        assert_eq!(
            us["region_endpoint"],
            "https://sellingpartnerapi-na.amazon.com"
        );
        r.market = "AMAZON_JP".into();
        let jp = protocol(&r).unwrap();
        assert_eq!(jp["marketplace_id"], "A1VC38T7YXB528");
        assert_eq!(
            jp["region_endpoint"],
            "https://sellingpartnerapi-fe.amazon.com"
        );
        assert_eq!(
            Amazon.acquire(&r).unwrap().result["records"][0]["reason"],
            "OFFERS_IDENTITY_OR_MARKET_MISMATCH"
        );
        r.market = "AMAZON_EU".into();
        assert!(protocol(&r).is_err());
        r.market = "AMAZON_US".into();
        r.query["include"] = json!(["OFFERS", "OFFERS"]);
        assert!(protocol(&r).is_err());
        r.query["include"] = json!(["FEE_ESTIMATE"]);
        assert!(protocol(&r).is_err());
        let mut fees = request(&fixture["cases"][2]);
        fees.query["fee_estimate"]["listing_price"]["amount"] = json!("11");
        assert_eq!(
            Amazon.acquire(&fees).unwrap().result["records"][0]["reason"],
            "FEE_ESTIMATE_CORRELATION_MISMATCH"
        );
        fees.query["fee_estimate"]["listing_price"]["currency"] = json!("JPY");
        assert!(protocol(&fees).is_err());
    }
    #[test]
    fn lwa_refresh_and_expiry_contract_never_uses_grantless_or_expired_tokens() {
        let form = lwa_refresh_form("client", "secret", "refresh").unwrap();
        assert_eq!(form[0], ("grant_type", "refresh_token"));
        assert!(!form.iter().any(|(k, _)| *k == "scope"));
        assert!(lwa_refresh_form("client", "", "refresh").is_err());
        let body =
            json!({"access_token":"ephemeral-token","expires_in":3600,"token_type":"bearer"});
        let token = LwaToken::from_response(&body, 1000).unwrap();
        assert_eq!(token.access_token_at(1000), Some("ephemeral-token"));
        assert!(token.access_token_at(3_571_000).is_none());
        assert!(token.access_token_at(3_601_000).is_none());
        for invalid in [
            json!({"expires_in":0}),
            json!({"expires_in":3601}),
            json!({"token_type":"invalid"}),
            json!({"access_token":"bad\nheader"}),
        ] {
            let mut value = body.clone();
            for (k, v) in invalid.as_object().unwrap() {
                value[k] = v.clone();
            }
            assert!(LwaToken::from_response(&value, 1000).is_err());
        }
        assert!(LwaToken::from_response(&body, u64::MAX).is_err());
    }
    #[test]
    fn official_fixture_errors_keep_rate_headers_and_never_establish_live_io() {
        let fixture = cases();
        let mut r = request(&fixture["cases"][1]);
        r.query["fixture_responses"]["offers"]["status"] = json!(429);
        r.query["fixture_responses"]["offers"]["headers"] = json!({"retry-after":"5","x-amzn-requestid":"fixture-request","x-amzn-ratelimit-limit":"0.5","x-amz-access-token":"must-not-project"});
        let result = Amazon.acquire(&r).unwrap();
        let record = &result.result["records"][0];
        assert_eq!(record["http_status"], 429);
        assert_eq!(record["response_headers"]["retry-after"], "5");
        assert_eq!(record["response_headers"]["x-amzn-ratelimit-limit"], "0.5");
        assert!(record["response_headers"]["x-amz-access-token"].is_null());
        assert_eq!(record["reason"], "OFFICIAL_HTTP_429");
        assert!(record["normalized"].is_null());
        assert_eq!(result.result["new_live_acquisition"], false);
        r.query.as_object_mut().unwrap().remove("fixture_responses");
        let denied = Amazon.acquire(&r).err().unwrap();
        assert_eq!(denied.request_count, Some(0));
        assert_eq!(Amazon.metadata()["fallback_providers"], json!([]));
    }
}
