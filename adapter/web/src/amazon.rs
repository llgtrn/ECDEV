//! Public Amazon documents share native HTTP policy, never SP-API or Keepa data.
use crate::Web;
use ecdev_core::provider::{AcquireRequest, AcquireResult, Provider};
use serde_json::{Value, json};

pub fn public_host(url: &str) -> bool {
    url::Url::parse(url).ok().is_some_and(|u| {
        matches!(
            u.host_str(),
            Some("amazon.co.jp" | "www.amazon.co.jp" | "amazon.com" | "www.amazon.com")
        )
    })
}

#[derive(Default)]
pub struct PublicAmazon {
    web: Web,
}

impl Provider for PublicAmazon {
    fn id(&self) -> &str {
        "public-amazon"
    }
    fn metadata(&self) -> Value {
        json!({"id":self.id(),"class":"PUBLIC","source_layer":"PUBLIC_AMAZON","status":"AVAILABLE","adapter_state":"NATIVE_PUBLIC_DOCUMENT_BOUNDARY","auth_state":"NOT_REQUIRED","capabilities":["fetch.http","extract.product"],"markets":["AMAZON_JP","AMAZON_US","PUBLIC_WEB"],"estimated_cost_minor":0,"cacheable":true,"cache_ttl_seconds":3600,"fallback_providers":["native-web"],"reason":"Robots-enforced public documents only; individual routes may be blocked. No search API, authentication, CAPTCHA bypass, SP-API or Keepa substitution."})
    }
    fn normalize_query(&self, query: &Value) -> Result<Value, String> {
        let result = self.web.normalize_query(query)?;
        if !public_host(result["url"].as_str().unwrap_or("")) {
            return Err("PUBLIC_AMAZON_HOST_REQUIRED".into());
        }
        Ok(result)
    }
    fn acquire(&self, request: &AcquireRequest) -> Result<AcquireResult, String> {
        let source = self.normalize_query(&request.query)?["url"]
            .as_str()
            .unwrap()
            .to_string();
        let host = url::Url::parse(&source).map_err(|_| "INVALID_URL")?;
        if (request.market == "AMAZON_JP"
            && !host.host_str().unwrap_or("").ends_with("amazon.co.jp"))
            || (request.market == "AMAZON_US"
                && !host.host_str().unwrap_or("").ends_with("amazon.com"))
        {
            return Err("PUBLIC_AMAZON_MARKET_HOST_MISMATCH".into());
        }
        let mut query = request.query.clone();
        query["source_layer"] = json!("PUBLIC_AMAZON");
        let scoped = AcquireRequest {
            run_id: request.run_id.clone(),
            capability: request.capability.clone(),
            market: request.market.clone(),
            query,
        };
        let mut acquired = self.web.acquire(&scoped)?;
        acquired.provider_cost["provider"] = json!(self.id());
        if acquired.result["not_modified"] == true {
            return Ok(acquired);
        }
        acquired.result["source_layer"] = json!("PUBLIC_AMAZON");
        acquired.result["source_authenticity"] =
            json!("PUBLIC_DOCUMENT_ASSERTIONS_NOT_OFFICIAL_API");
        acquired.result["public_source_url"] = json!(source);
        let challenge =
            scraper::Html::parse_document(std::str::from_utf8(&acquired.raw_payload).unwrap_or(""))
                .select(&scraper::Selector::parse("form[action]").unwrap())
                .any(|form| {
                    form.value()
                        .attr("action")
                        .is_some_and(|action| action.contains("/errors/validateCaptcha"))
                });
        if challenge {
            acquired.result["source_status"] = json!("SOURCE_BLOCKED");
            acquired.result["blocked_reason"] = json!("PUBLIC_AMAZON_CAPTCHA_FORM");
            acquired.result["products"] = json!([]);
        }
        let final_url = acquired.result["final_url"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let parsed = url::Url::parse(&final_url).map_err(|_| "PUBLIC_AMAZON_FINAL_URL_REQUIRED")?;
        let marketplace = if matches!(parsed.host_str(), Some("amazon.co.jp" | "www.amazon.co.jp"))
        {
            "AMAZON_JP"
        } else {
            "AMAZON_US"
        };
        let parts: Vec<_> = parsed.path_segments().into_iter().flatten().collect();
        let asin = parts
            .windows(2)
            .enumerate()
            .find(|(i, pair)| {
                pair[0] == "dp" || (pair[0] == "product" && *i > 0 && parts[*i - 1] == "gp")
            })
            .map(|(_, pair)| pair[1])
            .filter(|id| {
                id.len() == 10
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
            });
        if let Some(products) = acquired.result["products"].as_array_mut()
            && products.len() == 1
            && let Some(asin) = asin
        {
            let product = &mut products[0];
            product["asin"] = json!(asin);
            product["marketplace"] = json!(marketplace);
            product["fields"]["asin"] = json!({"value":asin,"status":"DERIVED","evidence":[{"source":"PUBLIC_DOCUMENT_URL","page":final_url,"selector":"URL path /dp/ASIN or /gp/product/ASIN","raw_capture_sha256":product["provenance"]["raw_capture_sha256"]}],"confidence":"URL_ASSERTION_NOT_OFFICIAL_CATALOG_VALIDATION"});
            product["marketplace_projection"] = json!({"schema":"ECDEV_PUBLIC_DOCUMENT_V1","source_layer":"PUBLIC_AMAZON","marketplace":marketplace,"asin":asin,"product_title":product["title"],"brand":product["brand"],"offers":product["observed_offers"],"price_minor":product["price_minor"],"currency":product["currency"],"seller":product["seller"],"availability":product["availability"],"variations":product["variants"],"category":product["category"],"reviews_summary":{"rating":product["rating"],"review_count":product["review_count"],"velocity":null},"sales":null,"official_validation":"UNAVAILABLE"});
        }
        for observation in &mut acquired.observations {
            observation.provider = self.id().into();
            observation.normalized_value = acquired.result.clone();
        }
        Ok(acquired)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_documents_have_their_own_provider_and_never_impersonate_official_data() {
        let provider = PublicAmazon::default();
        assert!(
            provider
                .normalize_query(&json!({"url":"https://amazon.co.jp.evil.example/dp/B012345678"}))
                .is_err()
        );
        let result = provider.acquire(&AcquireRequest { run_id:"fixture".into(), capability:"fetch.http".into(), market:"AMAZON_JP".into(), query:json!({"url":"https://www.amazon.co.jp/dp/B012345678","fixture_html":"<script type='application/ld+json'>{\"@type\":\"Product\",\"name\":\"Fixture cup\",\"offers\":{\"price\":2980,\"priceCurrency\":\"JPY\"}}</script>"}) }).unwrap();
        assert_eq!(result.observations[0].provider, "public-amazon");
        assert_eq!(result.result["source_layer"], "PUBLIC_AMAZON");
        assert_eq!(
            result.result["products"][0]["fields"]["asin"]["status"],
            "DERIVED"
        );
        assert_eq!(result.result["products"][0]["asin"], "B012345678");
        assert!(result.result["products"][0]["marketplace_projection"]["reviews_summary"]["review_count"].is_null());
        assert_eq!(result.provider_cost["request_count"], 0);
        assert_eq!(
            result.observations[0].mode,
            ecdev_core::domain::ObservationMode::Fixture
        );
    }
}
