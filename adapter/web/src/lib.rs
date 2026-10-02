//! Native public-source research: bounded fetching, policy, DOM/JSON-LD and hashing.
use ecdev_core::{
    domain::{Evidence, ObservationMode},
    provider::{AcquireRequest, AcquireResult, Provider},
    service::timestamp,
};
use scraper::{Html, Selector};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    io::Read,
    net::{IpAddr, ToSocketAddrs},
    sync::Mutex,
    time::{Duration, Instant},
};
use url::Url;
use uuid::Uuid;
pub struct Web {
    gate: Mutex<Option<Instant>>,
}
impl Default for Web {
    fn default() -> Self {
        Self {
            gate: Mutex::new(None),
        }
    }
}
pub fn normalize_url(input: &str) -> Result<String, String> {
    let mut u = Url::parse(input).map_err(|_| "INVALID_URL")?;
    if !matches!(u.scheme(), "http" | "https") || !u.username().is_empty() || u.password().is_some()
    {
        return Err("PUBLIC_HTTP_URL_REQUIRED".into());
    }
    if u.query_pairs().any(|(k, _)| {
        matches!(
            k.to_ascii_lowercase().as_str(),
            "key"
                | "api_key"
                | "apikey"
                | "token"
                | "access_token"
                | "password"
                | "secret"
                | "signature"
        )
    }) {
        return Err("CREDENTIAL_QUERY_DENIED".into());
    }
    u.set_fragment(None);
    Ok(u.to_string())
}
fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(a) => {
            let denied = a.is_documentation()
                || a.is_broadcast()
                || a.is_private()
                || a.is_loopback()
                || a.is_link_local()
                || a.is_unspecified()
                || a.is_multicast()
                || a.octets()[0] == 0
                || a.octets()[0] >= 224
                || (a.octets()[0] == 100 && (64..=127).contains(&a.octets()[1]))
                || (a.octets()[0] == 198 && (18..=19).contains(&a.octets()[1]));
            !denied
        }
        IpAddr::V6(a) => {
            !a.is_loopback()
                && !a.is_unspecified()
                && !a.is_multicast()
                && (a.segments()[0] & 0xfe00) != 0xfc00
                && (a.segments()[0] & 0xffc0) != 0xfe80
                && a.to_ipv4_mapped().is_none_or(|v| public_ip(IpAddr::V4(v)))
        }
    }
}
fn money(value: &Value, currency: &str) -> Option<i64> {
    let s = if let Some(s) = value.as_str() {
        s.to_string()
    } else if value.is_number() {
        value.to_string()
    } else {
        return None;
    };
    let digits = match currency {
        "JPY" => 0,
        "USD" | "EUR" | "GBP" => 2,
        _ => return None,
    };
    let (whole, frac) = s.split_once('.').unwrap_or((&s, ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !frac.bytes().all(|b| b.is_ascii_digit())
        || frac.len() > digits
    {
        return None;
    }
    let factor = 10_i64.pow(digits as u32);
    let fraction = if frac.is_empty() {
        0
    } else {
        frac.parse::<i64>()
            .ok()?
            .checked_mul(10_i64.pow((digits - frac.len()) as u32))?
    };
    whole
        .parse::<i64>()
        .ok()?
        .checked_mul(factor)?
        .checked_add(fraction)
}
fn products(value: &Value, out: &mut Vec<Value>) {
    match value {
        Value::Array(a) => {
            for v in a {
                products(v, out)
            }
        }
        Value::Object(o) => {
            let product = o.get("@type").is_some_and(|t| {
                t == "Product"
                    || t.as_array()
                        .is_some_and(|a| a.iter().any(|v| v == "Product"))
            });
            if product {
                let offer = if value["offers"].is_array() {
                    &value["offers"][0]
                } else {
                    &value["offers"]
                };
                let currency = offer["priceCurrency"].as_str().unwrap_or("");
                let price = money(&offer["price"], currency);
                out.push(json!({"kind":"PRODUCT","title":value["name"],"sku":value["sku"],"brand":value["brand"],"currency":if currency.is_empty(){Value::Null}else{json!(currency)},"price_minor":price,"price_status":if price.is_some(){"OBSERVED"}else{"UNKNOWN"},"availability":offer["availability"],"seller":offer["seller"],"weight_g":null,"sales":null,"sales_status":"UNKNOWN","supplier_moq":null,"supplier_capacity":null,"shipping_cost":null,"search_volume":null}));
            }
            for v in o.values() {
                if v.is_array() || v.is_object() {
                    products(v, out);
                }
            }
        }
        _ => {}
    }
}
pub fn extract(html: &str, source: &str) -> Result<Value, String> {
    let base = Url::parse(&normalize_url(source)?).map_err(|_| "INVALID_URL")?;
    let doc = Html::parse_document(html);
    let mut found = vec![];
    let mut errors = vec![];
    for script in doc.select(&Selector::parse("script[type='application/ld+json']").unwrap()) {
        match serde_json::from_str::<Value>(&script.inner_html()) {
            Ok(v) => products(&v, &mut found),
            Err(_) => errors.push("INVALID_JSON_LD"),
        }
    }
    let mut links = BTreeSet::new();
    for a in doc.select(&Selector::parse("a[href]").unwrap()) {
        if let Some(h) = a.value().attr("href")
            && let Ok(u) = base.join(h)
            && u.host_str() == base.host_str()
            && let Ok(normal) = normalize_url(u.as_str())
        {
            links.insert(normal);
        }
    }
    let title = doc
        .select(&Selector::parse("title").unwrap())
        .next()
        .map(|n| n.text().collect::<String>());
    Ok(
        json!({"source":source,"title":title,"products":found,"links":links,"extraction_errors":errors,"parser":"HTML5_DOM_JSON_LD_V1","content_hash":format!("{:x}",Sha256::digest(html.as_bytes()))}),
    )
}
/// Conservative robots implementation: unsupported wildcard or ambiguous policy denies.
fn robots_allows(text: &str, path: &str) -> bool {
    type RobotsGroup = (Vec<String>, Vec<(String, String)>);
    let mut groups: Vec<RobotsGroup> = vec![];
    let mut agents = vec![];
    let mut rules = vec![];
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim().to_string();
        if key == "user-agent" {
            if !rules.is_empty() {
                groups.push((agents, rules));
                agents = vec![];
                rules = vec![];
            }
            agents.push(value.to_ascii_lowercase());
        } else if !agents.is_empty() {
            rules.push((key, value));
        }
    }
    groups.push((agents, rules));
    let specificity = groups
        .iter()
        .filter_map(|(agents, _)| {
            if agents.iter().any(|a| a == "ecdev") {
                Some(1)
            } else if agents.iter().any(|a| a == "*") {
                Some(0)
            } else {
                None
            }
        })
        .max();
    let Some(specificity) = specificity else {
        return true;
    };
    let mut allow = 0;
    let mut deny = 0;
    for (agents, rules) in groups {
        let matches = if specificity == 1 {
            agents.iter().any(|a| a == "ecdev")
        } else {
            agents.iter().any(|a| a == "*")
        };
        if !matches {
            continue;
        }
        for (key, value) in rules {
            match key.as_str() {
                "crawl-delay"
                    if value
                        .parse::<f64>()
                        .map_or(true, |v| !v.is_finite() || v > 0.0) =>
                {
                    return false;
                }
                "disallow" | "allow" if !value.is_empty() => {
                    if value.contains('*') || value.contains('$') {
                        return false;
                    }
                    if path.starts_with(&value) {
                        if key == "allow" {
                            allow = allow.max(value.len())
                        } else {
                            deny = deny.max(value.len())
                        }
                    }
                }
                _ => {}
            }
        }
    }
    deny == 0 || allow >= deny
}
impl Web {
    fn request(&self, u: &Url, conditional: &Value) -> Result<(u16, String, Value), String> {
        let host = u.host_str().ok_or("INVALID_HOST")?;
        let port = u.port_or_known_default().ok_or("INVALID_PORT")?;
        let addresses: Vec<_> = (host, port)
            .to_socket_addrs()
            .map_err(|_| "DNS_FAILED")?
            .collect();
        if addresses.is_empty() || addresses.iter().any(|a| !public_ip(a.ip())) {
            return Err("PRIVATE_OR_SPECIAL_ADDRESS_DENIED".into());
        }
        let mut gate = self.gate.lock().map_err(|_| "FETCH_GATE_FAILED")?;
        if let Some(last) = *gate {
            let elapsed = last.elapsed();
            if elapsed < Duration::from_millis(750) {
                std::thread::sleep(Duration::from_millis(750) - elapsed)
            }
        }
        *gate = Some(Instant::now());
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .resolve_to_addrs(host, &addresses)
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "HTTP_CLIENT_ERROR")?;
        let mut request = client
            .get(u.clone())
            .header("User-Agent", "ECDEV/0.1 (+read-only research)");
        for (field, header) in [
            ("etag", "If-None-Match"),
            ("last_modified", "If-Modified-Since"),
        ] {
            if let Some(v) = conditional[field].as_str() {
                request = request.header(header, v)
            }
        }
        let response = request.send().map_err(|_| "FETCH_NETWORK_ERROR")?;
        let status = response.status().as_u16();
        let headers = json!({"etag":response.headers().get("etag").and_then(|v|v.to_str().ok()),"last_modified":response.headers().get("last-modified").and_then(|v|v.to_str().ok()),"content_type":response.headers().get("content-type").and_then(|v|v.to_str().ok()),"location":response.headers().get("location").and_then(|v|v.to_str().ok())});
        let mut body = vec![];
        response
            .take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut body)
            .map_err(|_| "BODY_READ_FAILED")?;
        if body.len() > 4 * 1024 * 1024 {
            return Err("BODY_LIMIT_EXCEEDED".into());
        }
        Ok((
            status,
            String::from_utf8(body).map_err(|_| "NON_UTF8_DOCUMENT")?,
            headers,
        ))
    }
}
impl Provider for Web {
    fn id(&self) -> &str {
        "native-web"
    }
    fn normalize_query(&self, query: &Value) -> Result<Value, String> {
        Ok(json!({"url":normalize_url(query["url"].as_str().ok_or("URL_REQUIRED")?)?}))
    }
    fn metadata(&self) -> Value {
        json!({"id":self.id(),"class":"PUBLIC","status":"AVAILABLE","auth_state":"NOT_REQUIRED","capabilities":["fetch.http","extract.product","research.market"],"markets":["AMAZON_JP","AMAZON_US","PUBLIC_WEB"],"quota_remaining":null,"rate_limit":{"concurrency":1,"minimum_interval_ms":750},"estimated_cost_minor":0,"latency_estimate_ms":null,"freshness":null,"confidence_characteristics":"Source assertions, not verified commercial truth","cacheable":true,"cache_ttl_seconds":3600,"failure_state":null,"fallback_providers":[],"adapter_state":"RUST_IMPLEMENTED","reason":"Public HTTP and supplied fixtures; private addresses denied; robots enforced; no browser/CAPTCHA bypass"})
    }
    fn acquire(&self, r: &AcquireRequest) -> Result<AcquireResult, String> {
        let source = r.query["url"].as_str().ok_or("URL_REQUIRED")?;
        let normalized = normalize_url(source)?;
        let start = Instant::now();
        let (html, mode, headers, requests) = if let Some(html) = r.query["fixture_html"].as_str() {
            if html.len() > 4 * 1024 * 1024 {
                return Err("FIXTURE_BODY_LIMIT".into());
            }
            (html.to_string(), ObservationMode::Fixture, json!({}), 0)
        } else {
            let mut url = Url::parse(&normalized).map_err(|_| "INVALID_URL")?;
            let mut requests = 0;
            let mut visited = BTreeSet::new();
            let mut body = None;
            for _ in 0..6 {
                if !visited.insert(url.to_string()) {
                    return Err("REDIRECT_LOOP".into());
                }
                let mut robot = url.clone();
                robot.set_path("/robots.txt");
                robot.set_query(None);
                let (rs, policy, _) = self.request(&robot, &json!({}))?;
                requests += 1;
                if rs != 404 && (rs != 200 || !robots_allows(&policy, url.path())) {
                    return Err("ROBOTS_DENIED_OR_UNKNOWN".into());
                }
                let (status, content, headers) = self.request(&url, &r.query["conditional"])?;
                requests += 1;
                if status == 304 {
                    return Ok(AcquireResult {
                        observations: vec![],
                        result: json!({"not_modified":true,"headers":headers}),
                        raw_payload: vec![],
                        provider_cost: json!({"request_count":requests,"actual_cost_minor":0,"latency_ms":start.elapsed().as_millis()}),
                    });
                }
                if (300..400).contains(&status) {
                    url = url
                        .join(
                            headers["location"]
                                .as_str()
                                .ok_or("REDIRECT_WITHOUT_LOCATION")?,
                        )
                        .map_err(|_| "INVALID_REDIRECT")?;
                    continue;
                }
                if status != 200 {
                    return Err(format!("HTTP_STATUS_{status}"));
                }
                if !headers["content_type"]
                    .as_str()
                    .is_some_and(|c| c.contains("text/html") || c.contains("application/xhtml+xml"))
                {
                    return Err("UNSUPPORTED_CONTENT_TYPE".into());
                }
                body = Some((content, headers));
                break;
            }
            let (content, headers) = body.ok_or("REDIRECT_LIMIT")?;
            (content, ObservationMode::Live, headers, requests)
        };
        let result = extract(&html, &normalized)?;
        let raw = html.into_bytes();
        let hash = format!("{:x}", Sha256::digest(&raw));
        let now = (timestamp() * 1000).to_string();
        let evidence = Evidence {
            id: Uuid::new_v4().to_string(),
            mode,
            source_type: "PUBLIC_HTML".into(),
            provider: self.id().into(),
            external_source: normalized,
            market: r.market.clone(),
            query: json!({"url":source}),
            timestamp: now.clone(),
            retrieved_at: now,
            raw_hash: hash,
            normalized_value: result.clone(),
            unit: "MIXED_SOURCE_FIELDS".into(),
            currency: None,
            confidence: None,
            freshness_seconds: None,
            cost_minor: Some(0),
            run_id: r.run_id.clone(),
        };
        Ok(AcquireResult {
            observations: vec![evidence],
            result,
            raw_payload: raw,
            provider_cost: json!({"provider":self.id(),"request_count":requests,"actual_cost_minor":0,"latency_ms":start.elapsed().as_millis(),"headers":headers,"quota_before":null,"quota_after":null,"cache_hit":false}),
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extraction_keeps_unknowns_and_minor_units() {
        let h = r#"<title>Shop</title><script type="application/ld+json">{"@graph":[{"@type":"Product","name":"Storage box","offers":{"price":"39.95","priceCurrency":"USD","availability":"https://schema.org/InStock"}}]}</script><a href="/next#x">next</a>"#;
        let v = extract(h, "https://example.com/shop").unwrap();
        assert_eq!(v["products"][0]["price_minor"], 3995);
        assert!(v["products"][0]["sales"].is_null());
        assert_eq!(v["links"][0], "https://example.com/next");
    }
    #[test]
    fn conservative_policy_and_url_validation() {
        assert!(!robots_allows(
            "User-agent: *\nDisallow: /private",
            "/private/a"
        ));
        assert!(robots_allows("User-agent: *\nDisallow: /private", "/shop"));
        assert!(!public_ip("127.0.0.1".parse().unwrap()));
        assert!(!public_ip("10.0.0.1".parse().unwrap()));
        assert!(normalize_url("file:///etc/passwd").is_err());
        assert!(money(&json!("12.001"), "USD").is_none());
        assert!(!robots_allows(
            "User-agent: ECDEV\nDisallow: /\nUser-agent: *\nAllow: /",
            "/shop"
        ));
        assert!(!robots_allows("User-agent: *\nCrawl-delay: 2", "/shop"));
    }
}
