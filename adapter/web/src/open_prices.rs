//! Open Prices (prices.openfoodfacts.org): an open, crowdsourced database of store shelf and
//! receipt prices, each tied to a product barcode, a mapped store and a proof (a price-tag or
//! receipt photo), read through its public REST API without an account. These are observations
//! contributed by people in physical stores: never online listings, never a marketplace price,
//! never demand. Data licence ODbL-1.0 (attribution; share-alike for a publicly used derived
//! database); the server code is AGPL and none of it is used. Robots are consulted on every
//! request, as for social sources; the host serves no robots.txt rules today.
//!
//! Open Food Facts' own product API (world.openfoodfacts.org/api) is disallowed for every agent
//! in its robots.txt and is not read.

use crate::{Web, robots};
use ecdev_core::sha256::Sha256;
use ecdev_core::{
    provider::{AcquireError, AcquireRequest, AcquireResult, Provider},
    service::timestamp,
};
use serde_json::{Value, json};
use std::time::Duration;
use url::Url;

pub const BASE: &str = "https://prices.openfoodfacts.org/api/v1/";
pub const MAX_PAGE: u64 = 50;
pub const LICENCE: &str = "ODbL-1.0";
pub const ATTRIBUTION: &str =
    "Open Prices (https://prices.openfoodfacts.org), Open Food Facts contributors, ODbL-1.0";

#[derive(Default)]
pub struct OpenPrices {
    web: Web,
}

impl OpenPrices {
    pub fn from_env() -> Self {
        Self {
            web: Web::from_env(),
        }
    }
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

fn text(v: &Value) -> Option<String> {
    v.as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// One page: products whose name holds `name` and that carry at least one price, most priced
/// first; or the newest prices of one barcode that the source has not marked as duplicates.
pub fn request_url(q: &Value) -> Result<Url, String> {
    let size = q["size"].as_u64().unwrap_or(20);
    if !(1..=MAX_PAGE).contains(&size) {
        return Err("INVALID_PRICE_PAGE".into());
    }
    let size = size.to_string();
    match (
        q["endpoint"].as_str(),
        q["name"].as_str(),
        q["code"].as_str(),
    ) {
        (Some("PRODUCTS"), Some(name), None) => {
            let name = name.trim();
            if name.is_empty() || name.chars().count() > 200 {
                return Err("INVALID_PRODUCT_NAME".into());
            }
            let mut u = Url::parse(&format!("{BASE}products")).unwrap();
            u.query_pairs_mut()
                .append_pair("product_name__like", name)
                .append_pair("price_count__gte", "1")
                .append_pair("order_by", "-price_count")
                .append_pair("size", &size);
            Ok(u)
        }
        (Some("PRICES"), None, Some(code)) => {
            if !(8..=14).contains(&code.len()) || !code.bytes().all(|b| b.is_ascii_digit()) {
                return Err("INVALID_PRODUCT_CODE".into());
            }
            let mut u = Url::parse(&format!("{BASE}prices")).unwrap();
            u.query_pairs_mut()
                .append_pair("product_code", code)
                .append_pair("duplicate_of__isnull", "true")
                .append_pair("order_by", "-date")
                .append_pair("size", &size);
            Ok(u)
        }
        _ => Err("EXACTLY_ONE_OF_PRODUCTS_NAME_OR_PRICES_CODE_REQUIRED".into()),
    }
}

fn page(data: &Value) -> Result<&Vec<Value>, String> {
    let items = data["items"]
        .as_array()
        .ok_or("PRICE_ITEMS_ARRAY_REQUIRED")?;
    if items.len() as u64 > MAX_PAGE {
        return Err("PRICE_PAGE_LIMIT_EXCEEDED".into());
    }
    Ok(items)
}

/// Products as the source describes them. A barcode is an identifier only when its checksum
/// holds; the source's counts are its own.
pub fn products(data: &Value) -> Result<Value, String> {
    let rows: Vec<Value> = page(data)?
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let code = text(&p["code"]);
            let gtin = code.as_ref().and_then(|c| ecdev_core::resolution::gtin(&json!(c)));
            json!({"locator":format!("/items/{i}"),"code":code,"gtin14":gtin,"code_state":match (&code, &gtin) {(None, _) => "NOT_STATED", (Some(_), None) => "STATED_INVALID_CHECKSUM", _ => "STATED_CHECKSUM_VALID"},
                "name":text(&p["product_name"]),"brands":text(&p["brands"]),"quantity":text(&p["quantity"]),
                "source_price_count":p["price_count"].as_u64(),"source_location_count":p["location_count"].as_u64(),"source_contributor_count":p["user_count"].as_u64(),"source_country_count":p["location_type_osm_country_count"].as_u64()})
        })
        .collect();
    Ok(
        json!({"products":rows,"total":{"value":data["total"].as_u64(),"exactness":"REPORTED_BY_SOURCE"}}),
    )
}

/// Price observations. The contributor's account name is returned only for the caller to key
/// into a pseudonym before anything is stored; prices without an amount or currency are
/// counted, not kept.
pub fn prices(data: &Value) -> Result<Value, String> {
    let mut rows = vec![];
    let mut unpriced = 0;
    for (i, p) in page(data)?.iter().enumerate() {
        let (Some(price), Some(currency)) = (p["price"].as_f64(), text(&p["currency"])) else {
            unpriced += 1;
            continue;
        };
        let l = &p["location"];
        rows.push(json!({"locator":format!("/items/{i}"),"price_id":p["id"],"product_code":text(&p["product_code"]),"date":text(&p["date"]),
            "price":price,"currency":currency,"price_per":text(&p["price_per"]),"discounted":p["price_is_discounted"].as_bool(),"price_without_discount":p["price_without_discount"].as_f64(),
            "receipt_quantity":p["receipt_quantity"].as_f64(),"store_name":text(&l["osm_name"]),"store_brand":text(&l["osm_brand"]),"store_kind":text(&l["osm_tag_value"]),
            "store_osm":l["osm_id"].as_u64().map(|id| format!("{}/{id}", l["osm_type"].as_str().unwrap_or("").to_lowercase())),
            "location_type":text(&l["type"]),"store_url":text(&l["website_url"]),"city":text(&l["osm_address_city"]),"country_code":text(&l["osm_address_country_code"]),
            "proof_type":text(&p["proof"]["type"]),"proof_image_md5":text(&p["proof"]["image_md5_hash"]),"contributor_handle":text(&p["owner"])}));
    }
    Ok(
        json!({"prices":rows,"unpriced_items":unpriced,"total":{"value":data["total"].as_u64(),"exactness":"REPORTED_BY_SOURCE"}}),
    )
}

impl Provider for OpenPrices {
    fn id(&self) -> &str {
        "open-prices"
    }
    fn metadata(&self) -> Value {
        json!({"id":self.id(),"class":"PUBLIC","source_layer":"OPEN_CROWDSOURCED_STORE_PRICES","status":"AVAILABLE","auth_state":"NO_ACCOUNT_REQUIRED_FOR_READS","capabilities":["price.observations"],"cost_minor":0,"rate_limit":{"minimum_interval_ms":1000,"basis":"ECDEV politeness interval; robots consulted per request"},"fixture_supported":true,"licence":LICENCE,"attribution":ATTRIBUTION,"egress":self.web.egress_disclosure()})
    }
    fn normalize_query(&self, q: &Value) -> Result<Value, String> {
        request_url(q)?;
        Ok(q.clone())
    }
    fn acquire(&self, r: &AcquireRequest) -> Result<AcquireResult, AcquireError> {
        let u = request_url(&r.query).map_err(|e| failure(e, None, 0))?;
        let fixture = r.query.get("fixture_raw");
        let mut requests = 0;
        let raw = if let Some(v) = fixture {
            v.as_str()
                .ok_or_else(|| failure("fixture_raw must be text", None, 0))?
                .as_bytes()
                .to_vec()
        } else {
            let policy = robots::robots_url(&u);
            let (status, body, _) = self
                .web
                .request(&policy, &Value::Null, Duration::from_secs(1))
                .map_err(|e| {
                    let mut error = failure(e, None, 0);
                    error.request_count = None;
                    error
                })?;
            requests += 1;
            let policy_text = String::from_utf8_lossy(&body);
            if status == 200 && robots::ai_agent_refusal(&policy_text, u.as_str()).is_some() {
                return Err(failure("ROBOTS_REFUSES_AI_AGENTS", Some(status), requests));
            }
            let robot = robots::evaluate(&policy_text, u.as_str(), "ECDEV");
            if status != 404 && (status != 200 || !robot.allowed) {
                return Err(failure(
                    "SOURCE_BLOCKED_ROBOTS_OR_POLICY",
                    Some(status),
                    requests,
                ));
            }
            let delay = robot.crawl_delay_seconds.unwrap_or(0.).max(1.);
            if delay > 20. {
                return Err(failure(
                    "ROBOTS_DELAY_EXCEEDS_BUDGET",
                    Some(status),
                    requests,
                ));
            }
            let (status, body, headers) = self
                .web
                .request(&u, &Value::Null, Duration::from_secs_f64(delay))
                .map_err(|e| {
                    let mut error = failure(e, None, requests);
                    error.request_count = None;
                    error
                })?;
            requests += 1;
            if status != 200 {
                let mut e = failure("OPEN_PRICES_HTTP_FAILURE", Some(status), requests);
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
            .map_err(|_| failure("MALFORMED_PRICE_JSON", None, requests))?;
        let mut result = match r.query["endpoint"].as_str() {
            Some("PRODUCTS") => products(&data),
            _ => prices(&data),
        }
        .map_err(|e| failure(e, None, requests))?;
        let mode = if fixture.is_some() { "FIXTURE" } else { "LIVE" };
        result["source_url"] = json!(u.as_str());
        result["raw_hash"] = json!(format!("{:x}", Sha256::digest(&raw)));
        result["capture_mode"] = json!(mode);
        result["captured_at"] = json!(if fixture.is_some() {
            r.query["captured_at"].as_u64().unwrap_or(timestamp())
        } else {
            timestamp()
        });
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
    fn pages_are_bounded_and_ask_for_one_thing() {
        let p =
            request_url(&json!({"endpoint":"PRODUCTS","name":"matcha latte","size":50})).unwrap();
        assert!(
            p.as_str()
                .starts_with("https://prices.openfoodfacts.org/api/v1/products?")
        );
        assert!(
            p.as_str().contains("product_name__like=matcha+latte")
                && p.as_str().contains("price_count__gte=1")
        );
        let c = request_url(&json!({"endpoint":"PRICES","code":"4901305410982"})).unwrap();
        assert!(
            c.as_str().contains("product_code=4901305410982")
                && c.as_str().contains("duplicate_of__isnull=true")
        );
        for bad in [
            json!({"endpoint":"PRODUCTS","name":"x","size":51}),
            json!({"endpoint":"PRODUCTS","name":"  "}),
            json!({"endpoint":"PRICES","code":"49013054A0982"}),
            json!({"endpoint":"PRICES","code":"1234567"}),
            json!({"endpoint":"PRICES","name":"x","code":"4901305410982"}),
            json!({"endpoint":"LOCATIONS","name":"x"}),
        ] {
            assert!(request_url(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn rows_keep_what_the_source_states_and_count_what_it_lacks() {
        let products_page = json!({"items":[{"code":"4901305410982","product_name":"MATCHA LATTE","brands":"辻利","price_count":1,"location_count":1,"user_count":1,"location_type_osm_country_count":0},{"code":"4901305410983","product_name":"bad code"}],"total":2});
        let out = products(&products_page).unwrap();
        assert_eq!(out["products"][0]["gtin14"], "04901305410982");
        assert_eq!(out["products"][1]["code_state"], "STATED_INVALID_CHECKSUM");
        let prices_page = json!({"items":[{"id":7,"product_code":"0892859002898","date":"2026-07-03","price":12.49,"currency":"USD","price_is_discounted":false,"owner":"someone","location":{"type":"OSM","osm_id":3208512953_u64,"osm_type":"NODE","osm_name":"Sakura Market","osm_address_country_code":"US"},"proof":{"type":"PRICE_TAG"}},{"id":8,"price":null,"currency":"USD"}],"total":2});
        let out = prices(&prices_page).unwrap();
        assert_eq!(out["prices"].as_array().unwrap().len(), 1);
        assert_eq!(out["unpriced_items"], 1);
        assert_eq!(out["prices"][0]["store_osm"], "node/3208512953");
        assert_eq!(out["prices"][0]["proof_type"], "PRICE_TAG");
        assert!(prices(&json!({})).is_err());
    }
}
