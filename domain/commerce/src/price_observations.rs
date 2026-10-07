//! Store shelf prices contributed to Open Prices: for products found by name (or one barcode),
//! the newest prices each was seen at, by store, date and proof. A reference retail price seen
//! in physical stores by volunteers, kept apart from online listings and marketplace offers;
//! never demand. Contributors are told apart by keyed pseudonym only (several prices from one
//! person are one witness, not several). ODbL-1.0 obligations travel with every result.
use crate::{Engine, provider::AcquireRequest, service::timestamp, social::pseudonym};
use rusqlite::params;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
};
use uuid::Uuid;

pub const MAX_PRODUCTS: u64 = 5;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub fn initialize(db: &rusqlite::Connection) -> Result<(), String> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS price_observation_runs(id TEXT PRIMARY KEY,mode TEXT NOT NULL,captured INTEGER NOT NULL,payload TEXT NOT NULL);")
        .map_err(err)
}

pub fn tool_definition() -> Value {
    json!({"name":"ecdev.price.observations","description":"Store shelf and receipt prices from Open Prices (open crowdsourced database, ODbL-1.0, no account): products found by name (or one barcode), each with its newest prices by store, date and proof type, summarised per currency and price unit with distinct stores, countries and pseudonymous contributors. Reference retail prices seen in physical stores, mostly packaged food and grocery: not online listings, not marketplace offers, not demand. Raw pages are hashed; contributor names are never stored.","inputSchema":{"type":"object","properties":{"query":{"type":"string","minLength":1,"maxLength":200},"code":{"type":"string","pattern":"^[0-9]{8,14}$"},"max_products":{"type":"integer","minimum":1,"maximum":MAX_PRODUCTS,"default":3},"fixture_products":{"type":"string","maxLength":4194304},"fixture_prices":{"type":"object","additionalProperties":{"type":"string","maxLength":4194304}},"fixture_now":{"type":"integer","minimum":0}},"additionalProperties":false}})
}

fn median(v: &mut [f64]) -> Option<f64> {
    v.sort_by(f64::total_cmp);
    (!v.is_empty()).then(|| v[(v.len() - 1) / 2])
}

/// Per currency and unit (an item price and a per-kilogram price are never mixed): how many
/// prices, their range and median, the dates they span and how many stores, countries and
/// contributors stand behind them.
pub fn summarize(prices: &[Value]) -> Vec<Value> {
    let mut groups: BTreeMap<(String, String), Vec<&Value>> = BTreeMap::new();
    for p in prices {
        let unit = p["price_per"]
            .as_str()
            .unwrap_or("ITEM_OR_UNSTATED")
            .to_string();
        groups
            .entry((p["currency"].as_str().unwrap_or("").to_string(), unit))
            .or_default()
            .push(p);
    }
    groups
        .into_iter()
        .map(|((currency, unit), rows)| {
            let distinct = |k: &str| rows.iter().filter_map(|r| r[k].as_str()).collect::<BTreeSet<_>>().len();
            let mut amounts: Vec<f64> = rows.iter().filter_map(|r| r["price"].as_f64()).collect();
            let dates: BTreeSet<&str> = rows.iter().filter_map(|r| r["date"].as_str()).collect();
            json!({"currency":currency,"price_unit":unit,"prices":rows.len(),"discounted":rows.iter().filter(|r| r["discounted"] == true).count(),
                "min":amounts.iter().copied().reduce(f64::min),"median":median(&mut amounts),"max":amounts.iter().copied().reduce(f64::max),
                "earliest_date":dates.first(),"latest_date":dates.last(),"distinct_stores":distinct("store_key"),"distinct_countries":distinct("country_code"),"distinct_contributors":distinct("contributor")})
        })
        .collect()
}

fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// The source matches names by substring ("oat milk" finds "Goat Milk"): a product is taken
/// only when its name holds the query's words as words, in order.
pub fn name_holds_query(name: &str, query: &str) -> bool {
    // Unspaced CJK has no word boundaries to respect: the query must appear as written.
    if query.chars().any(crate::social::is_cjk) {
        let compact = |t: &str| t.split_whitespace().collect::<String>().to_lowercase();
        return !query.trim().is_empty() && compact(name).contains(&compact(query));
    }
    let (n, q) = (words(name), words(query));
    !q.is_empty() && n.windows(q.len()).any(|w| w == q.as_slice())
}

/// The official catalog check a barcode allows: a 12-digit code is a UPC, a 13-digit code with
/// Japan's 45 or 49 prefix a JAN, any other an EAN.
fn catalog_action(code: &str) -> Value {
    // A 13-digit code with a leading zero is a UPC-A written as an EAN-13.
    let code = match code.strip_prefix('0') {
        Some(upc) if code.len() == 13 => upc,
        _ => code,
    };
    let (kind, market) = match (code.len(), &code[..2.min(code.len())]) {
        (12, _) => ("UPC", "AMAZON_US"),
        (13, "45" | "49") => ("JAN", "AMAZON_JP"),
        _ => ("EAN", "AMAZON_US"),
    };
    json!({"tool":"ecdev.seller.read","input_template":{"market":market,"operation":"CATALOG_SEARCH_BY_IDENTIFIER","identifiers_type":kind,"identifiers":[code]},"fills":["CROSS_SOURCE_PRODUCT_IDENTIFIER"],"requires":"SP-API credentials and the operator gate","paid":false})
}

impl Engine {
    fn price_capture(
        &self,
        query: Value,
        now: u64,
        fixture: bool,
        captures: &mut Vec<Value>,
    ) -> Result<Value, Value> {
        let provider = self
            .providers
            .iter()
            .find(|p| p.id() == "open-prices")
            .ok_or_else(|| json!({"reason":"OPEN_PRICES_PROVIDER_NOT_CONFIGURED"}))?;
        let mut query = query;
        query["captured_at"] = json!(now);
        let endpoint = query["endpoint"].clone();
        let acquired = provider
            .acquire(&AcquireRequest {
                run_id: Uuid::new_v4().to_string(),
                capability: "price.observations".into(),
                market: "OPEN_PRICES".into(),
                query,
            })
            .map_err(|e| json!({"endpoint":endpoint,"reason":e.reason,"http_status":e.http_status,"request_count":e.request_count}))?;
        let count = acquired.provider_cost["request_count"].as_u64();
        let mode = if fixture { "FIXTURE" } else { "LIVE" };
        let hash = format!("{:x}", Sha256::digest(&acquired.raw_payload));
        if (fixture && count != Some(0))
            || (!fixture && count.is_none_or(|n| n == 0))
            || acquired.result["raw_hash"] != json!(hash)
            || acquired.result["capture_mode"] != json!(mode)
        {
            return Err(
                json!({"endpoint":endpoint,"reason":"PRICE_CAPTURE_WITNESS_MISSING_OR_MODE_MISMATCH"}),
            );
        }
        let dir = self.root.join(".ecdev-data/runtime/price-captures");
        fs::create_dir_all(&dir)
            .and_then(|_| fs::write(dir.join(format!("{hash}.raw")), &acquired.raw_payload))
            .map_err(|e| json!({"endpoint":endpoint,"reason":err(e)}))?;
        captures.push(json!({"endpoint":endpoint,"source_url":acquired.result["source_url"],"raw_hash":hash,"capture_mode":mode,"captured_at":acquired.result["captured_at"],"request_count":count}));
        Ok(acquired.result)
    }

    pub fn price_observations(&self, args: Value) -> Result<Value, String> {
        let fixture =
            args.get("fixture_products").is_some() || args.get("fixture_prices").is_some();
        if !fixture && args.get("fixture_now").is_some() {
            return Err("LIVE_CLOCK_OVERRIDE_DENIED".into());
        }
        let now = if fixture {
            args["fixture_now"].as_u64().unwrap_or(timestamp())
        } else {
            timestamp()
        };
        let max = args["max_products"]
            .as_u64()
            .unwrap_or(3)
            .clamp(1, MAX_PRODUCTS) as usize;
        let mut captures = vec![];
        let mut failures = vec![];
        let mut chosen: Vec<Value> = vec![];
        let mut search_total = Value::Null;
        let mut substring_only = 0;
        match (args["query"].as_str(), args["code"].as_str()) {
            (Some(q), None) => {
                let mut query = json!({"endpoint":"PRODUCTS","name":q,"size":20});
                if fixture {
                    match args["fixture_products"].as_str() {
                        Some(raw) => query["fixture_raw"] = json!(raw),
                        None => return Err("FIXTURE_PRODUCTS_REQUIRED_WITH_FIXTURE_PRICES".into()),
                    }
                }
                match self.price_capture(query, now, fixture, &mut captures) {
                    Ok(found) => {
                        search_total = found["total"].clone();
                        // Only products with a checksum-valid barcode whose name holds the
                        // query as words, most priced first.
                        let rows = found["products"].as_array().cloned().unwrap_or_default();
                        let named =
                            |p: &Value| p["name"].as_str().is_some_and(|n| name_holds_query(n, q));
                        substring_only = rows.iter().filter(|p| !named(p)).count();
                        chosen = rows
                            .into_iter()
                            .filter(|p| p["code_state"] == "STATED_CHECKSUM_VALID" && named(p))
                            .take(max)
                            .collect();
                    }
                    Err(f) => failures.push(f),
                }
            }
            (None, Some(code)) => {
                crate::resolution::gtin(&json!(code)).ok_or("INVALID_PRODUCT_CODE_CHECKSUM")?;
                chosen.push(json!({"code":code,"code_state":"STATED_CHECKSUM_VALID","gtin14":crate::resolution::gtin(&json!(code))}));
            }
            _ => return Err("EXACTLY_ONE_OF_QUERY_OR_CODE_REQUIRED".into()),
        }
        let key = pseudonym::installation_key(&self.root)?;
        let mut products = vec![];
        for mut product in chosen {
            let code = product["code"].as_str().unwrap_or_default().to_string();
            let mut query = json!({"endpoint":"PRICES","code":code,"size":50});
            if fixture {
                match args["fixture_prices"][&code].as_str() {
                    Some(raw) => query["fixture_raw"] = json!(raw),
                    None => {
                        failures.push(json!({"endpoint":"PRICES","code":code,"reason":"FIXTURE_PRICES_MISSING_FOR_CODE","request_count":0}));
                        continue;
                    }
                }
            }
            let page = match self.price_capture(query, now, fixture, &mut captures) {
                Ok(p) => p,
                Err(mut f) => {
                    f["code"] = json!(code);
                    failures.push(f);
                    continue;
                }
            };
            let prices: Vec<Value> = page["prices"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|p| {
                    let mut p = p.clone();
                    let who = p
                        .as_object_mut()
                        .and_then(|o| o.remove("contributor_handle"));
                    p["contributor"] = json!(
                        who.as_ref()
                            .and_then(Value::as_str)
                            .map(|h| pseudonym::pseudonym(&key, "OPEN_PRICES", h))
                    );
                    p["store_key"] = p["store_osm"].clone();
                    if p["store_key"].is_null() {
                        p["store_key"] = json!(
                            p["store_name"]
                                .as_str()
                                .map(|n| format!("name:{}", n.to_lowercase()))
                        );
                    }
                    p["evidence_class"] =
                        json!("CROWDSOURCED_STORE_SHELF_PRICE_NOT_ONLINE_LISTING");
                    p
                })
                .collect();
            product["summary"] = json!(summarize(&prices));
            product["prices"] = json!(prices);
            product["unpriced_items"] = page["unpriced_items"].clone();
            product["source_price_total"] = page["total"].clone();
            product["next_action"] = catalog_action(&code);
            products.push(product);
        }
        let mode = if fixture { "FIXTURE" } else { "LIVE" };
        let requests: u64 = captures
            .iter()
            .filter_map(|c| c["request_count"].as_u64())
            .sum::<u64>()
            + failures
                .iter()
                .filter_map(|f| f["request_count"].as_u64())
                .sum::<u64>();
        let state = match (products.is_empty(), failures.is_empty()) {
            (false, true) => "OBSERVED_STORE_PRICES",
            (false, false) => "PARTIAL_SOURCE_FAILURES",
            (true, true) => "NO_PRICED_PRODUCT_FOUND",
            (true, false) => "SOURCE_UNAVAILABLE",
        };
        let out = json!({"run_id":Uuid::new_v4().to_string(),"state":state,"capture_mode":mode,"captured_at":now,"query":args["query"],"code":args["code"],
            "search_total":search_total,"excluded_substring_only_matches":substring_only,"products":products,"failures":failures,"captures":captures,"request_count":requests,"paid":false,
            "source":"OPEN_PRICES","evidence_layer":"OPEN_CROWDSOURCED_STORE_PRICES",
            "licence":{"data":"ODbL-1.0","attribution":"Open Prices (https://prices.openfoodfacts.org), Open Food Facts contributors","obligations":["attribute the source","share-alike applies to a publicly used database derived from this data"],"legal_assessment":"NOT_MADE"},
            "invariants":["a store shelf price is not an online listing or marketplace offer","prices from one contributor are one witness","a price is not demand","item and per-unit prices are never mixed","the source's totals are its own count","a name that holds the query only inside another word is not a match"]});
        self.db
            .lock()
            .map_err(err)?
            .execute(
                "INSERT INTO price_observation_runs VALUES(?1,?2,?3,?4)",
                params![out["run_id"].as_str(), mode, now, out.to_string()],
            )
            .map_err(err)?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_by_words_and_codes_by_their_kind() {
        assert!(name_holds_query("Oatly Oat Milk, Barista", "oat milk"));
        assert!(!name_holds_query("Goat Milk", "oat milk"));
        assert!(!name_holds_query("Milk Oat", "oat milk"));
        assert!(name_holds_query("MATCHA-LATTE mix", "matcha latte"));
        assert!(name_holds_query(
            "明治 エッセルスーパーカップ 抹茶",
            "スーパーカップ 抹茶"
        ));
        assert!(!name_holds_query(
            "明治 エッセルスーパーカップ バニラ",
            "抹茶"
        ));
        let kind = |c: &str| catalog_action(c)["input_template"]["identifiers_type"].clone();
        assert_eq!(kind("0892859002898"), "UPC");
        assert_eq!(
            catalog_action("0892859002898")["input_template"]["identifiers"][0],
            "892859002898"
        );
        assert_eq!(kind("4901305410982"), "JAN");
        assert_eq!(kind("4002971197709"), "EAN");
    }
}
