//! Native public-source research: bounded fetching, policy, DOM/JSON-LD and hashing.
pub mod amazon;
pub mod commerce;
pub mod document;
pub mod microdata;
pub mod page_product;
pub mod price;
pub mod robots;
pub mod sitemap;
pub mod social;
pub mod supplier;
pub mod supplier_terms;
use ecdev_core::{
    domain::{Evidence, ObservationMode},
    provider::{AcquireError, AcquireRequest, AcquireResult, Provider},
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
    robots: Mutex<robots::RobotsCache>,
}
impl Default for Web {
    fn default() -> Self {
        Self {
            gate: Mutex::new(None),
            robots: Mutex::new(robots::RobotsCache::default()),
        }
    }
}
/// XML media types, read only as sitemaps.
pub fn is_xml(content_type: &str) -> bool {
    let media = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    media == "application/xml" || media == "text/xml"
}

/// A fetched sitemap as a research result: no products are asserted, its entries are links.
pub fn sitemap_result(raw: &[u8], source: &str) -> Result<Value, String> {
    let parsed = sitemap::parse(raw)?;
    Ok(
        json!({"source":source,"document_kind":"SITEMAP","products":[],"links":parsed.entries.iter().map(|(u,_)|u.as_str()).collect::<Vec<_>>(),"lastmod":parsed.entries.iter().map(|(_,l)|l).collect::<Vec<_>>(),"sitemap":parsed.evidence(),"raw_capture_sha256":format!("{:x}", Sha256::digest(raw))}),
    )
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
/// Where a response sends the fetch loop next. `Ok(None)`: the response is final (only 301, 302,
/// 303, 307 and 308 with a `Location` redirect, as in httpx). `Ok(Some(url))`: the next URL,
/// resolved against `current` and held to the same public-URL policy as the requested one
/// (`normalize_url`: http(s) only, no credentials, no credential query, no fragment). A Location
/// that clients resolve differently is refused rather than guessed: non-ASCII bytes, a backslash,
/// or a scheme without an authority (`http:/x`, `https:x`).
pub fn redirect_target(
    current: &Url,
    status: u16,
    location: Option<&str>,
) -> Result<Option<String>, String> {
    if !matches!(status, 301 | 302 | 303 | 307 | 308) {
        return Ok(None);
    }
    let Some(location) = location else {
        return Ok(None);
    };
    let location = location.trim_matches([' ', '\t']);
    let scheme_without_authority = location.split_once(':').is_some_and(|(scheme, rest)| {
        !scheme.is_empty()
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
            && !rest.starts_with("//")
            && !scheme.contains(['/', '?', '#'])
    });
    if !location.is_ascii() || location.contains('\\') || scheme_without_authority {
        return Err("INVALID_REDIRECT".into());
    }
    let next = current.join(location).map_err(|_| "INVALID_REDIRECT")?;
    normalize_url(next.as_str()).map(Some)
}
/// The cache validators (`etag`, `last_modified`) to send to `url`: only to the URL whose response
/// produced them (`conditional.url`, the previous capture's final URL). A validator sent anywhere
/// else could be answered 304 by a different resource, and the cached payload would be reused
/// for content it never captured. Validators recorded without their URL are sent only to the
/// requested URL, never across a redirect.
pub fn validators_for(conditional: &Value, url: &Url, first_hop: bool) -> Value {
    let applies = match conditional["url"].as_str() {
        Some(origin) => Url::parse(origin).is_ok_and(|o| o == *url),
        None => first_hop,
    };
    if applies && conditional.is_object() {
        json!({"etag": conditional["etag"], "last_modified": conditional["last_modified"]})
    } else {
        Value::Null
    }
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
    let raw = if let Some(s) = value.as_str() {
        s.to_string()
    } else if value.is_number() {
        value.to_string()
    } else {
        return None;
    };
    // JSON numeric lexemes retain decimal precision; exponent expansion is bounded.
    let s = if value.is_number() {
        price::parse_number(&raw, Some('.'))?
    } else {
        raw
    };
    let digits = match currency {
        "JPY" => 0,
        "USD" | "EUR" | "GBP" => 2,
        _ => return None,
    };
    let (whole, frac) = s.split_once('.').unwrap_or((&s, ""));
    let frac = frac.trim_end_matches('0');
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
fn products(value: &Value, out: &mut Vec<Value>, pointer: &str) {
    match value {
        Value::Array(a) => {
            for (i, v) in a.iter().enumerate() {
                products(v, out, &format!("{pointer}/{i}"))
            }
        }
        Value::Object(o) => {
            let product = o.get("@type").is_some_and(|t| {
                t == "Product"
                    || t.as_array()
                        .is_some_and(|a| a.iter().any(|v| v == "Product"))
            });
            if product {
                let raw_offers: Vec<&Value> = match value["offers"].as_array() {
                    Some(a) => a.iter().collect(),
                    None if value["offers"].is_object() => vec![&value["offers"]],
                    _ => vec![],
                };
                let observed_offers:Vec<Value>=raw_offers.iter().enumerate().map(|(i,offer)|{
                    let currency=offer["priceCurrency"].as_str().unwrap_or("");
                    let interpretation=offer["price"].as_str().map(|raw|json!({"state":if raw.len()<=4096{"DERIVED"}else{"UNKNOWN"},"result":price::parse_price(Some(raw),offer["priceCurrency"].as_str(),None,None),"method":"FIRST_PRICE_TEXT_CURRENCY_TOKEN_AND_SEPARATOR_HEURISTICS","used_for_observed_price":false,"currency_token_is_iso_identity":false,"limitations":"First number may precede the actual price; signs may be discarded; free substring may become zero. Requires source review before commercial use."}));
                    json!({"price_minor":money(&offer["price"],currency),"raw_price":offer["price"],"price_text_interpretation":interpretation,"currency":offer["priceCurrency"],"availability":offer["availability"],"seller":offer["seller"],"sku":offer["sku"],"url":offer["url"],"provenance":{"source":"JSON_LD","field_path":format!("Product.offers[{i}]")}})
                }).collect();
                let currencies: BTreeSet<_> = observed_offers
                    .iter()
                    .filter_map(|v| v["currency"].as_str())
                    .collect();
                let currency = if currencies.len() == 1 {
                    *currencies.first().unwrap()
                } else {
                    ""
                };
                let prices: BTreeSet<_> = observed_offers
                    .iter()
                    .filter_map(|v| v["price_minor"].as_i64())
                    .collect();
                let price = if !observed_offers.is_empty()
                    && currencies.len() == 1
                    && prices.len() == 1
                    && observed_offers.iter().all(|o| o["price_minor"].is_i64())
                {
                    prices.first().copied()
                } else {
                    None
                };
                let offer = raw_offers.first().copied().unwrap_or(&Value::Null);
                out.push(json!({"raw_json_ld":value,"json_pointer":pointer,"kind":"PRODUCT","title":value["name"],"sku":value["sku"],"brand":value["brand"],"currency":if currency.is_empty(){Value::Null}else{json!(currency)},"price_minor":price,"price_status":if price.is_some(){"OBSERVED"}else if prices.len()>1{"CONFLICT"}else{"UNKNOWN"},"observed_offers":observed_offers,"price_range":if currency.is_empty(){Value::Null}else{json!({"min_minor":prices.first(),"max_minor":prices.last(),"currency":currency,"status":"OBSERVED_OFFER_RANGE"})},"availability":offer["availability"],"seller":offer["seller"],"weight_g":null,"sales":null,"sales_status":"UNKNOWN","supplier_moq":null,"supplier_capacity":null,"shipping_cost":null,"search_volume":null}));
            }
            for (key, v) in o {
                if v.is_array() || v.is_object() {
                    products(
                        v,
                        out,
                        &format!("{pointer}/{}", key.replace('~', "~0").replace('/', "~1")),
                    );
                }
            }
        }
        _ => {}
    }
}
pub fn extract(html: &str, source: &str) -> Result<Value, String> {
    extract_with_hash(
        html,
        source,
        &format!("{:x}", Sha256::digest(html.as_bytes())),
    )
}
fn extract_with_hash(html: &str, source: &str, hash: &str) -> Result<Value, String> {
    let base = Url::parse(&normalize_url(source)?).map_err(|_| "INVALID_URL")?;
    let doc = Html::parse_document(html);
    let mut structured_data = vec![];
    let mut found = vec![];
    let mut errors = vec![];
    for (script_number, script) in doc
        .select(&Selector::parse("script[type='application/ld+json']").unwrap())
        .enumerate()
    {
        // Raw script text: re-serialising the element (inner_html) escapes `&` and `>` in
        // documents html5ever does not treat as raw text, which corrupts string values.
        match serde_json::from_str::<Value>(&script.text().collect::<String>()) {
            Ok(v) => {
                structured_data.push(json!({"script_index":script_number,"value":v}));
                let start = found.len();
                products(&v, &mut found, "");
                for product in &mut found[start..] {
                    commerce::annotate(product, source, script_number, hash);
                    product["provenance"] = json!({"source":"JSON_LD","page":source,"script_index":script_number,"raw_capture_sha256":hash,"identity_assertions":{"title":product["title"],"sku":product["sku"]},"price_derivation":"Exact minor-unit conversion; conflicting offers retained, no selected price"});
                }
            }
            Err(_) => errors.push("INVALID_JSON_LD"),
        }
    }
    let microdata = microdata::extract(&doc, Some(&base), false);
    let (normalized_microdata, microdata_paths) = microdata::normalize(&microdata);
    let start = found.len();
    products(&normalized_microdata, &mut found, "");
    for product in &mut found[start..] {
        commerce::annotate(product, source, 0, hash);
        fn relabel(value: &mut Value, paths: &std::collections::BTreeMap<String, String>) {
            match value {
                Value::Object(object) => {
                    if object.get("source").is_some_and(|v| v == "JSON_LD") {
                        object.insert("source".into(), json!("MICRODATA"));
                        object.remove("script_index");
                        object.insert("data_container".into(), json!("microdata"));
                        if let Some(raw) = object
                            .get("json_pointer")
                            .and_then(Value::as_str)
                            .and_then(|path| paths.get(path))
                            .cloned()
                        {
                            object.insert("json_pointer".into(), json!(raw));
                        }
                        object.insert("normalization".into(),json!("Microdata type/properties projected to schema fields; exact raw graph pointer retained"));
                    }
                    for child in object.values_mut() {
                        relabel(child, paths);
                    }
                }
                Value::Array(a) => {
                    for child in a {
                        relabel(child, paths);
                    }
                }
                _ => {}
            }
        }
        relabel(product, &microdata_paths);
        let normalized_pointer = product["json_pointer"].as_str().unwrap_or("");
        let raw_pointer = microdata_paths
            .get(normalized_pointer)
            .cloned()
            .unwrap_or_else(|| normalized_pointer.into());
        product["raw_microdata"] = microdata
            .pointer(&raw_pointer)
            .cloned()
            .unwrap_or(Value::Null);
        product["json_pointer"] = json!(raw_pointer);
        product.as_object_mut().unwrap().remove("raw_json_ld");
        product["provenance"] = json!({"source":"MICRODATA","page":source,"data_container":"microdata","json_pointer":raw_pointer,"raw_capture_sha256":hash,"identity_assertions":{"title":product["title"],"sku":product["sku"]},"price_derivation":"Exact minor-unit conversion; conflicting offers retained; no selected price"});
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
    let page_metadata = commerce::page_metadata(&doc, &base, &found, hash);
    page_product::enrich(&mut found, &page_metadata, &structured_data, source, hash);
    let mut supplier_leads = supplier::extract(&doc, &base, &structured_data, hash);
    supplier_leads.extend(supplier_terms::extract(&base, &structured_data, hash));
    supplier_leads.extend(supplier_terms::microdata(
        &base,
        &normalized_microdata,
        &microdata_paths,
        &microdata,
        hash,
    ));
    Ok(
        json!({"supplier_leads":supplier_leads,"page_metadata":page_metadata,"structured_data":structured_data,"microdata":microdata,"source":source,"title":title,"products":found,"links":links,"extraction_errors":errors,"parser":"HTML5_DOM_COMMERCE_V3","content_hash":hash}),
    )
}
impl Web {
    fn request(
        &self,
        u: &Url,
        conditional: &Value,
        minimum_interval: Duration,
    ) -> Result<(u16, Vec<u8>, Value), String> {
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
            if elapsed < minimum_interval {
                std::thread::sleep(minimum_interval - elapsed)
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
        let headers = json!({"etag":response.headers().get("etag").and_then(|v|v.to_str().ok()),"last_modified":response.headers().get("last-modified").and_then(|v|v.to_str().ok()),"content_type":response.headers().get("content-type").and_then(|v|v.to_str().ok()),"location":response.headers().get("location").and_then(|v|v.to_str().ok()),"retry_after":response.headers().get("retry-after").and_then(|v|v.to_str().ok()),"cache_control":response.headers().get("cache-control").and_then(|v|v.to_str().ok()),"expires":response.headers().get("expires").and_then(|v|v.to_str().ok()),"date":response.headers().get("date").and_then(|v|v.to_str().ok()),"age":response.headers().get("age").and_then(|v|v.to_str().ok()),"received_at_ms":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis().min(i64::MAX as u128) as i64});
        if status != 200 {
            // Error, redirect and validator responses are control evidence; an unused
            // body read must not discard their status or Retry-After header.
            return Ok((status, vec![], headers));
        }
        let mut body = vec![];
        response
            .take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut body)
            .map_err(|_| "BODY_READ_FAILED")?;
        if body.len() > 4 * 1024 * 1024 {
            return Err("BODY_LIMIT_EXCEEDED".into());
        }
        Ok((status, body, headers))
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
    fn reextract(
        &self,
        raw: &[u8],
        source_type: &str,
        recorded: &Value,
    ) -> Option<Result<Value, String>> {
        let source = recorded["final_url"]
            .as_str()
            .or(recorded["source"].as_str())
            .unwrap_or("");
        Some(match source_type {
            "PUBLIC_SITEMAP_XML" => sitemap_result(raw, source),
            "PUBLIC_HTML" => {
                // The charset the capture was decoded with came from its HTTP header only when
                // the recorded recipe says so; BOM and meta declarations are in the bytes.
                let decoding = &recorded["document_decoding"];
                let content_type = match decoding["declared_label"].as_str() {
                    Some(label) if decoding["source"] == "HTTP_CONTENT_TYPE" => {
                        format!("text/html; charset={label}")
                    }
                    _ => "text/html".to_string(),
                };
                document::extract(raw, &content_type, source)
            }
            _ => return None,
        })
    }
    fn acquire(&self, r: &AcquireRequest) -> Result<AcquireResult, AcquireError> {
        let source = r.query["url"].as_str().ok_or("URL_REQUIRED")?;
        let normalized = normalize_url(source)?;
        let start = Instant::now();
        let mut declared_sitemaps: Vec<String> = vec![];
        let (raw, mode, headers, requests, robots_cache_hits, final_url) = if let Some(html) =
            r.query["fixture_html"].as_str()
        {
            if html.len() > 4 * 1024 * 1024 {
                return Err("FIXTURE_BODY_LIMIT".into());
            }
            (
                html.as_bytes().to_vec(),
                ObservationMode::Fixture,
                json!({}),
                0,
                0,
                normalized.clone(),
            )
        } else {
            let mut url = Url::parse(&normalized).map_err(|_| "INVALID_URL")?;
            let mut requests = 0;
            let mut robots_cache_hits = 0;
            let mut visited = BTreeSet::new();
            let mut body = None;
            for hop in 0..6 {
                if r.query["source_layer"] == "PUBLIC_AMAZON" && !amazon::public_host(url.as_str())
                {
                    return Err("PUBLIC_AMAZON_REDIRECT_SCOPE_DENIED".into());
                }
                if !visited.insert(url.to_string()) {
                    return Err("REDIRECT_LOOP".into());
                }
                let robot = robots::robots_url(&url);
                let kept = self
                    .robots
                    .lock()
                    .map_err(|_| "ROBOTS_CACHE_POISONED")?
                    .get(&robot, timestamp());
                let (rs, policy, robot_headers) = match kept {
                    Some(e) => {
                        robots_cache_hits += 1;
                        (e.status, e.body, json!({}))
                    }
                    None => {
                        let fetched =
                            self.request(&robot, &json!({}), Duration::from_millis(750))?;
                        requests += 1;
                        self.robots
                            .lock()
                            .map_err(|_| "ROBOTS_CACHE_POISONED")?
                            .put(&robot, fetched.0, &fetched.1, &fetched.2, timestamp());
                        fetched
                    }
                };
                let policy = if rs == 200 {
                    std::str::from_utf8(&policy)
                        .map_err(|_| "ROBOTS_ENCODING_UNKNOWN")?
                        .trim_start_matches('\u{feff}')
                } else {
                    ""
                };
                let decision = robots::evaluate(policy, url.as_str(), "ECDEV");
                if hop == 0 && rs == 200 {
                    declared_sitemaps = robots::sitemaps(policy, &robot)
                        .iter()
                        .map(Url::to_string)
                        .collect();
                }
                if rs == 429 || (500..600).contains(&rs) {
                    return Err(AcquireError::http(
                        rs,
                        requests,
                        robot_headers["retry_after"].as_str(),
                        robot_headers["received_at_ms"]
                            .as_i64()
                            .unwrap_or((timestamp() * 1000) as i64),
                    ));
                }
                if rs != 404 && (rs != 200 || !decision.allowed) {
                    return Err("ROBOTS_DENIED_OR_UNKNOWN".into());
                }
                let delay = if rs == 404 {
                    0.0
                } else {
                    decision.crawl_delay_seconds.unwrap_or(0.0)
                };
                if delay > 20.0 {
                    return Err("ROBOTS_DELAY_EXCEEDS_FETCH_BUDGET".into());
                }
                let interval = Duration::from_secs_f64(delay).max(Duration::from_millis(750));
                let validators = validators_for(&r.query["conditional"], &url, hop == 0);
                let (status, content, headers) = self.request(&url, &validators, interval)?;
                requests += 1;
                if status == 304 {
                    if validators.is_null() {
                        // Nothing conditional was asked of this URL: a 304 has no payload to stand for.
                        return Err("UNSOLICITED_NOT_MODIFIED".into());
                    }
                    return Ok(AcquireResult {
                        observations: vec![],
                        result: json!({"not_modified":true,"headers":headers,"validated_url":url.as_str()}),
                        raw_payload: vec![],
                        provider_cost: json!({"request_count":requests,"robots_cache_hits":robots_cache_hits,"actual_cost_minor":0,"latency_ms":start.elapsed().as_millis()}),
                    });
                }
                if let Some(next) = redirect_target(&url, status, headers["location"].as_str())? {
                    url = Url::parse(&next).map_err(|_| "INVALID_REDIRECT")?;
                    continue;
                }
                if status != 200 {
                    return Err(AcquireError::http(
                        status,
                        requests,
                        headers["retry_after"].as_str(),
                        headers["received_at_ms"]
                            .as_i64()
                            .unwrap_or((timestamp() * 1000) as i64),
                    ));
                }
                if !headers["content_type"].as_str().is_some_and(|c| {
                    c.contains("text/html") || c.contains("application/xhtml+xml") || is_xml(c)
                }) {
                    return Err("UNSUPPORTED_CONTENT_TYPE".into());
                }
                body = Some((content, headers, url.to_string()));
                break;
            }
            let (content, headers, final_url) = body.ok_or("REDIRECT_LIMIT")?;
            (
                content,
                ObservationMode::Live,
                headers,
                requests,
                robots_cache_hits,
                final_url,
            )
        };
        let sitemap = is_xml(headers["content_type"].as_str().unwrap_or(""));
        let mut result = if sitemap {
            sitemap_result(&raw, &final_url)?
        } else {
            document::extract(
                &raw,
                headers["content_type"].as_str().unwrap_or(""),
                &final_url,
            )?
        };
        result["robots_sitemaps"] = json!(declared_sitemaps);
        result["requested_url"] = json!(normalized);
        result["final_url"] = json!(final_url);
        let hash = format!("{:x}", Sha256::digest(&raw));
        let now = (timestamp() * 1000).to_string();
        let evidence = Evidence {
            id: Uuid::new_v4().to_string(),
            mode,
            source_type: if sitemap {
                "PUBLIC_SITEMAP_XML"
            } else {
                "PUBLIC_HTML"
            }
            .into(),
            provider: self.id().into(),
            external_source: final_url,
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
            provider_cost: json!({"provider":self.id(),"request_count":requests,"robots_cache_hits":robots_cache_hits,"actual_cost_minor":0,"latency_ms":start.elapsed().as_millis(),"headers":headers,"quota_before":null,"quota_after":null,"cache_hit":false}),
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn xml_responses_are_read_only_as_sitemaps() {
        assert!(is_xml("application/xml; charset=UTF-8") && is_xml("TEXT/XML"));
        assert!(!is_xml("application/rss+xml") && !is_xml("text/html"));
        let r = sitemap_result(
            b"<sitemapindex><sitemap><loc>https://shop.example/sitemap_products_1.xml?from=1&amp;to=9</loc></sitemap></sitemapindex>",
            "https://shop.example/sitemap.xml",
        )
        .unwrap();
        assert_eq!(r["document_kind"], "SITEMAP");
        assert_eq!(r["products"], json!([]));
        assert_eq!(r["sitemap"]["kind"], "SITEMAP_INDEX");
        assert_eq!(
            r["links"][0],
            "https://shop.example/sitemap_products_1.xml?from=1&to=9"
        );
        assert_eq!(
            sitemap_result(b"<rss></rss>", "https://shop.example/f").unwrap_err(),
            "SITEMAP_ROOT_UNKNOWN"
        );
    }
    #[test]
    fn validators_go_only_to_the_url_that_produced_them() {
        let page = Url::parse("https://shop.example/p/1").unwrap();
        let moved = Url::parse("https://shop.example/p/2").unwrap();
        let c = json!({"etag":"\"v1\"","last_modified":null,"url":"https://shop.example/p/1"});
        assert_eq!(validators_for(&c, &page, false)["etag"], "\"v1\"");
        // A redirect to another resource never receives them, first hop or not.
        assert!(validators_for(&c, &moved, true).is_null());
        // Validators recorded without their URL: the requested URL only, never across a redirect.
        let legacy = json!({"etag":"\"v1\""});
        assert_eq!(validators_for(&legacy, &page, true)["etag"], "\"v1\"");
        assert!(validators_for(&legacy, &moved, false).is_null());
        assert!(validators_for(&Value::Null, &page, true).is_null());
    }
    #[test]
    fn extraction_keeps_unknowns_and_minor_units() {
        let h = r#"<title>Shop</title><script type="application/ld+json">{"@graph":[{"@type":"Product","name":"Storage box","offers":{"price":"39.95","priceCurrency":"USD","availability":"https://schema.org/InStock"}}]}</script><a href="/next#x">next</a>"#;
        let v = extract(h, "https://example.com/shop").unwrap();
        assert_eq!(v["products"][0]["price_minor"], 3995);
        assert!(v["products"][0]["sales"].is_null());
        assert_eq!(v["links"][0], "https://example.com/next");
        assert_eq!(money(&json!(550.0), "JPY"), Some(550));
        assert_eq!(money(&json!("550.00"), "JPY"), Some(550));
        assert_eq!(money(&json!("550.50"), "JPY"), None);
    }
    #[test]
    fn json_numeric_prices_preserve_source_precision() {
        for (literal, currency, expected) in [
            ("2980.0000000000001", "JPY", None),
            ("0.29000000000000001", "USD", None),
            ("9007199254740993.0", "JPY", Some(9007199254740993_i64)),
            ("2.98e3", "JPY", Some(2980)),
            ("2.9e-1", "USD", Some(29)),
            ("1e999999", "JPY", None),
        ] {
            let html = format!(
                r#"<script type="application/ld+json">{{"@type":"Product","name":"Exact decimal","offers":{{"price":{literal},"priceCurrency":"{currency}"}}}}</script>"#
            );
            let data = extract(&html, "https://shop.example/product").unwrap();
            let product = &data["products"][0];
            assert_eq!(
                product["price_minor"],
                json!(expected),
                "{literal} {currency}"
            );
            let retained = product["observed_offers"][0]["raw_price"].to_string();
            if !literal.contains('e') {
                assert_eq!(retained, literal);
            } else {
                assert_eq!(
                    price::parse_number(&retained, Some('.')),
                    price::parse_number(literal, Some('.'))
                );
            }
        }
    }
    #[test]
    fn formatted_price_hints_preserve_unknown_observed_money_and_raw_source() {
        let html = r#"<script type="application/ld+json">{"@type":"Product","name":"Formatted source","offers":[{"price":"$12.99","priceCurrency":"USD"},{"price":"free shipping"},{"price":1e3,"priceCurrency":"USD"}]}</script>"#;
        let value = extract(html, "https://example.org/product").unwrap();
        let product = &value["products"][0];
        let offers = &product["observed_offers"];
        assert_eq!(offers[0]["raw_price"], "$12.99");
        assert!(offers[0]["price_minor"].is_null());
        assert_eq!(offers[0]["price_text_interpretation"]["state"], "DERIVED");
        assert_eq!(
            offers[0]["price_text_interpretation"]["result"]["amount"],
            "12.99"
        );
        assert_eq!(
            offers[0]["price_text_interpretation"]["result"]["currency"],
            "$"
        );
        assert_eq!(
            offers[0]["price_text_interpretation"]["used_for_observed_price"],
            false
        );
        assert_eq!(
            offers[1]["price_text_interpretation"]["result"]["amount"],
            "0"
        );
        assert!(offers[1]["price_minor"].is_null());
        assert!(offers[1]["currency"].is_null());
        assert_eq!(offers[2]["price_minor"], 100000);
        assert!(offers[2]["price_text_interpretation"].is_null());
        assert!(product["price_minor"].is_null());
    }
    #[test]
    fn conservative_policy_and_url_validation() {
        let robots_allows = |text: &str, path: &str| {
            robots::evaluate(text, &format!("https://shop.example{path}"), "ECDEV").allowed
        };
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
        assert!(robots_allows("User-agent: *\nCrawl-delay: 2", "/shop"));
    }
    #[test]
    fn conflicting_offers_never_become_precise_price() {
        let html = r#"<script type="application/ld+json">{"@type":"Product","name":"Real offer conflict shape","offers":[{"price":3300.0,"priceCurrency":"JPY"},{"price":4400.0,"priceCurrency":"JPY"}]}</script>"#;
        let data = extract(html, "https://shop.example/products/one").unwrap();
        let p = &data["products"][0];
        assert!(p["price_minor"].is_null());
        assert_eq!(p["price_status"], "CONFLICT");
        assert_eq!(p["observed_offers"].as_array().unwrap().len(), 2);
        assert_eq!(p["price_range"]["min_minor"], 3300);
        assert_eq!(p["price_range"]["max_minor"], 4400);
        assert_eq!(
            p["provenance"]["raw_capture_sha256"],
            format!("{:x}", Sha256::digest(html.as_bytes()))
        );
    }
    #[test]
    fn json_ld_string_values_keep_raw_ampersands_and_angles() {
        let html = concat!(
            r#"<!DOCTYPE html PUBLIC "-//W3C//DTD XHTML 1.0 Strict//EN" "http://www.w3.org/TR/xhtml1/DTD/xhtml1-strict.dtd">"#,
            r#"<html xmlns="http://www.w3.org/1999/xhtml"><head><script type="application/ld+json">"#,
            r#"{"@type":"Product","name":"Tom & Jerry > Cup","offers":{"price":"1","priceCurrency":"USD"}}"#,
            "</script></head><body></body></html>"
        );
        let out = extract(html, "https://shop.example/cup").unwrap();
        assert_eq!(
            out["structured_data"][0]["value"]["name"],
            "Tom & Jerry > Cup"
        );
        assert_eq!(out["products"][0]["title"], "Tom & Jerry > Cup");
    }
}
