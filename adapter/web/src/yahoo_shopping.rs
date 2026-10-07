//! Yahoo! Shopping (Japan) item search through its official Web API (ShoppingWebService V3
//! itemSearch). The operator's Client ID (application ID) comes from YAHOO_SHOPPING_APP_ID, is
//! sent only to shopping.yahooapis.jp, and never appears in a returned URL, error or capture
//! record. Results are the provider's listings: OFFICIAL_MARKETPLACE_API claims, not ECDEV page
//! observations. Contract: research/commerce/yahoo-shopping-contract.json (official
//! documentation; no live call made without an operator's Client ID).

use crate::Web;
use ecdev_core::{
    provider::{AcquireError, AcquireRequest, AcquireResult, Provider},
    service::timestamp,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::Duration;
use url::Url;

pub const ENDPOINT: &str = "https://shopping.yahooapis.jp/ShoppingWebService/V3/itemSearch";
pub const MAX_RESULTS: u64 = 50;
/// start + results may not exceed this; listings past it are unreachable through the API.
pub const REACHABLE: u64 = 1000;
pub const ATTRIBUTION: &str =
    "Web Services by Yahoo! JAPAN (https://developer.yahoo.co.jp/sitemap/)";

#[derive(Default)]
pub struct YahooShopping {
    web: Web,
    app_id: Option<String>,
}

fn failure(reason: impl Into<String>, status: Option<u16>, requests: u64) -> AcquireError {
    AcquireError {
        reason: reason.into(),
        http_status: status,
        request_count: Some(requests),
        retry_after_header: None,
        retry_not_before_ms: None,
    }
}

impl YahooShopping {
    pub fn from_env() -> Self {
        Self {
            web: Web::from_env(),
            app_id: std::env::var("YAHOO_SHOPPING_APP_ID")
                .ok()
                .filter(|k| !k.trim().is_empty()),
        }
    }
}

/// The request URL without the Client ID, as every record shows it.
pub fn request_url(q: &Value) -> Result<Url, String> {
    let mut u = Url::parse(ENDPOINT).unwrap();
    let results = q["results"].as_u64().unwrap_or(20);
    let start = q["start"].as_u64().unwrap_or(1);
    if !(1..=MAX_RESULTS).contains(&results) || start == 0 || start + results - 1 > REACHABLE {
        return Err("INVALID_LISTING_PAGE".into());
    }
    {
        let mut pairs = u.query_pairs_mut();
        match (q["query"].as_str(), q["jan"].as_str()) {
            (Some(query), None) if !query.trim().is_empty() && query.len() <= 500 => {
                pairs.append_pair("query", query.trim());
            }
            (None, Some(jan)) => {
                ecdev_core::resolution::gtin(&json!(jan)).ok_or("INVALID_JAN_CHECKSUM")?;
                pairs.append_pair("jan_code", jan);
            }
            _ => return Err("EXACTLY_ONE_OF_QUERY_OR_JAN_REQUIRED".into()),
        }
        pairs
            .append_pair("results", &results.to_string())
            .append_pair("start", &start.to_string());
        if let Some(stock) = q["in_stock"].as_bool() {
            pairs.append_pair("in_stock", if stock { "true" } else { "false" });
        }
    }
    Ok(u)
}

fn text(v: &Value) -> Option<String> {
    v.as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// The listings of one response. A JAN is kept only when its checksum holds; an empty or
/// invalid one is unknown, never an identifier. Prices are JPY as the source states them, with
/// its taxable flag; a missing price stays unknown.
pub fn listings(data: &Value) -> Result<Value, String> {
    let hits = data["hits"]
        .as_array()
        .ok_or("LISTING_HITS_ARRAY_REQUIRED")?;
    if hits.len() as u64 > MAX_RESULTS {
        return Err("LISTING_LIMIT_EXCEEDED".into());
    }
    let mut rows = vec![];
    for (i, h) in hits.iter().enumerate() {
        let url = text(&h["url"]).ok_or("LISTING_URL_REQUIRED")?;
        let url = crate::normalize_url(&url)?;
        let raw_jan = text(&h["janCode"]);
        let gtin = raw_jan
            .as_ref()
            .and_then(|j| ecdev_core::resolution::gtin(&json!(j)));
        rows.push(json!({
            "locator": format!("/hits/{i}"), "item_code": text(&h["code"]), "title": text(&h["name"]), "url": url,
            "price_minor": h["price"].as_i64().filter(|p| *p > 0), "currency": "JPY", "price_taxable_flag": h["priceLabel"]["taxable"],
            "fixed_price_minor": h["priceLabel"]["fixedPrice"].as_i64(), "in_stock": h["inStock"].as_bool(), "condition": text(&h["condition"]),
            "jan": raw_jan, "gtin14": gtin, "jan_state": match (&raw_jan, &gtin) { (None, _) => "NOT_STATED", (Some(_), None) => "STATED_INVALID_CHECKSUM", _ => "STATED_CHECKSUM_VALID" },
            "brand": text(&h["brand"]["name"]), "category": text(&h["genreCategory"]["name"]),
            "seller_id": text(&h["seller"]["sellerId"]), "seller_name": text(&h["seller"]["name"]),
            "review_rate": h["review"]["rate"].as_f64(), "review_count": h["review"]["count"].as_u64(),
            "shipping_code": h["shipping"]["code"].as_u64(),
        }));
    }
    // The documentation's field table says firstResultsPosition; its sample says firstResultPosition.
    let first = data["firstResultsPosition"]
        .as_u64()
        .or(data["firstResultPosition"].as_u64());
    Ok(
        json!({"listings":rows,"total":{"value":data["totalResultsAvailable"].as_u64(),"exactness":"REPORTED_BY_SOURCE","reachable_through_api":REACHABLE},"returned":data["totalResultsReturned"].as_u64(),"first_position":first}),
    )
}

impl Provider for YahooShopping {
    fn id(&self) -> &str {
        "yahoo-shopping-jp"
    }
    fn metadata(&self) -> Value {
        json!({"id":self.id(),"class":"OFFICIAL","source_layer":"OFFICIAL_MARKETPLACE_API","status":if self.app_id.is_some() {"AVAILABLE"} else {"UNAVAILABLE"},"auth_state":if self.app_id.is_some() {"CLIENT_ID_PRESENT"} else {"AUTH_REQUIRED_YAHOO_SHOPPING_APP_ID"},"capabilities":["listing.search"],"markets":["YAHOO_SHOPPING_JP"],"cost_minor":0,"rate_limit":{"minimum_interval_ms":1000,"basis":"documented 1 query per second"},"fixture_supported":true,"attribution":ATTRIBUTION,"terms":"research/commerce/yahoo-shopping-contract.json","egress":self.web.egress_disclosure()})
    }
    fn normalize_query(&self, q: &Value) -> Result<Value, String> {
        request_url(q)?;
        Ok(q.clone())
    }
    fn acquire(&self, r: &AcquireRequest) -> Result<AcquireResult, AcquireError> {
        let shown = request_url(&r.query).map_err(|e| failure(e, None, 0))?;
        let fixture = r.query.get("fixture_raw");
        let mut requests = 0;
        let raw = if let Some(v) = fixture {
            v.as_str()
                .ok_or_else(|| failure("fixture_raw must be text", None, 0))?
                .as_bytes()
                .to_vec()
        } else {
            let id = self
                .app_id
                .as_ref()
                .ok_or_else(|| failure("AUTH_REQUIRED_YAHOO_SHOPPING_APP_ID", None, 0))?;
            let mut u = shown.clone();
            u.query_pairs_mut().append_pair("appid", id);
            let (status, body, headers) = self
                .web
                .request(&u, &Value::Null, Duration::from_secs(1))
                .map_err(|e| {
                    let mut error = failure(e, None, 0);
                    error.request_count = None;
                    error
                })?;
            requests += 1;
            if status != 200 {
                let mut e = failure("OFFICIAL_LISTING_HTTP_FAILURE", Some(status), requests);
                e.retry_after_header = text(&headers["retry_after"]);
                return Err(e);
            }
            if !headers["content_type"]
                .as_str()
                .unwrap_or("")
                .contains("json")
            {
                return Err(failure(
                    "UNSUPPORTED_CONTENT_TYPE_EXPECTED_JSON",
                    Some(status),
                    requests,
                ));
            }
            body
        };
        if raw.len() > 4 * 1024 * 1024 {
            return Err(failure("PAYLOAD_TOO_LARGE", None, requests));
        }
        let data: Value = serde_json::from_slice(&raw)
            .map_err(|_| failure("MALFORMED_LISTING_JSON", None, requests))?;
        let mut result = listings(&data).map_err(|e| failure(e, None, requests))?;
        let mode = if fixture.is_some() { "FIXTURE" } else { "LIVE" };
        result["source_url"] = json!(shown.as_str());
        result["raw_hash"] = json!(format!("{:x}", Sha256::digest(&raw)));
        result["capture_mode"] = json!(mode);
        result["captured_at"] = json!(if fixture.is_some() {
            r.query["captured_at"].as_u64().unwrap_or(timestamp())
        } else {
            timestamp()
        });
        result["evidence_layer"] = json!("OFFICIAL_MARKETPLACE_API_PROVIDER_REPORTED");
        result["attribution"] = json!(ATTRIBUTION);
        Ok(AcquireResult {
            observations: vec![],
            result,
            raw_payload: raw,
            provider_cost: json!({"cost_minor":0,"request_count":requests,"paid":false}),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_never_carry_the_client_id_and_bound_paging() {
        let u = request_url(&json!({"query":"抹茶 茶筅","results":50,"start":951})).unwrap();
        assert!(!u.as_str().contains("appid"));
        assert!(u.as_str().starts_with(ENDPOINT));
        assert!(!u.as_str().contains("jan_code") && u.as_str().contains("results=50"));
        for bad in [
            json!({"query":"x","results":51}),
            json!({"query":"x","start":952,"results":50}),
            json!({"query":"x","start":0}),
            json!({"query":"x","jan":"4901234567894"}),
            json!({}),
            json!({"jan":"4901234567890"}),
        ] {
            assert!(request_url(&bad).is_err(), "{bad}");
        }
        assert_eq!(
            request_url(&json!({"jan":"4901234567890"})).unwrap_err(),
            "INVALID_JAN_CHECKSUM"
        );
        assert!(
            request_url(&json!({"jan":"4901234567894"}))
                .unwrap()
                .as_str()
                .contains("jan_code=4901234567894")
        );
    }

    #[test]
    fn listings_follow_the_documented_shape() {
        let data: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/yahoo-itemsearch-synthetic.json"
        ))
        .unwrap();
        let out = listings(&data).unwrap();
        let l = out["listings"].as_array().unwrap();
        assert_eq!(l.len(), 4);
        assert_eq!(
            out["first_position"], 1,
            "the sample's firstResultPosition spelling is read"
        );
        assert_eq!(out["total"]["value"], 1234);
        assert_eq!(l[0]["gtin14"], "04901234567894");
        assert_eq!(l[0]["jan_state"], "STATED_CHECKSUM_VALID");
        assert_eq!(
            l[3]["jan_state"], "NOT_STATED",
            "an empty janCode is unknown"
        );
        assert_eq!(
            (l[0]["price_minor"].clone(), l[0]["seller_id"].clone()),
            (json!(2980), json!("shop-a"))
        );
        let mut bad = data.clone();
        bad["hits"][0]["janCode"] = json!("4901234567890");
        assert_eq!(
            listings(&bad).unwrap()["listings"][0]["jan_state"],
            "STATED_INVALID_CHECKSUM"
        );
        assert!(listings(&json!({})).is_err());
    }

    #[test]
    fn live_search_without_a_client_id_makes_no_request() {
        let p = YahooShopping::default();
        assert_eq!(
            p.metadata()["auth_state"],
            "AUTH_REQUIRED_YAHOO_SHOPPING_APP_ID"
        );
        let Err(e) = p.acquire(&AcquireRequest {
            run_id: "r".into(),
            capability: "listing.search".into(),
            market: "YAHOO_SHOPPING_JP".into(),
            query: json!({"query":"matcha"}),
        }) else {
            panic!("a live search without a Client ID must fail before IO")
        };
        assert_eq!(
            (e.reason.as_str(), e.request_count),
            ("AUTH_REQUIRED_YAHOO_SHOPPING_APP_ID", Some(0))
        );
    }
}
