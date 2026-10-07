//! Official marketplace listing search (Yahoo! Shopping Japan item search). Results are the
//! provider's listings, kept with their raw capture hash: OFFICIAL_MARKETPLACE_API claims, not
//! ECDEV page observations. Offers are grouped by checksum-valid JAN so the same product's
//! sellers and prices sit together; sellers inside one marketplace are never independent sites.
use crate::{Engine, provider::AcquireRequest, service::timestamp};
use rusqlite::params;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs};
use uuid::Uuid;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub fn initialize(db: &rusqlite::Connection) -> Result<(), String> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS listing_searches(id TEXT PRIMARY KEY,mode TEXT NOT NULL,captured INTEGER NOT NULL,payload TEXT NOT NULL);")
        .map_err(err)
}

pub fn tool_definition() -> Value {
    json!({"name":"ecdev.listing.search","description":"Official marketplace item search (Yahoo! Shopping Japan, operator Client ID in YAHOO_SHOPPING_APP_ID): provider-reported listings with price, seller, stock and JAN, grouped by checksum-valid JAN; raw capture hashed; not page observations, not demand, sellers are not independent sites; fixture_raw runs without the Client ID","inputSchema":{"type":"object","properties":{"market":{"enum":["YAHOO_SHOPPING_JP"]},"query":{"type":"string","minLength":1,"maxLength":500},"jan":{"type":"string","pattern":"^[0-9]{8,14}$"},"results":{"type":"integer","minimum":1,"maximum":50,"default":20},"start":{"type":"integer","minimum":1,"maximum":1000,"default":1},"in_stock":{"type":"boolean"},"fixture_raw":{"type":"string","maxLength":4194304},"fixture_now":{"type":"integer","minimum":0}},"additionalProperties":false}})
}

fn median(v: &mut [i64]) -> Option<i64> {
    v.sort_unstable();
    (!v.is_empty()).then(|| v[(v.len() - 1) / 2])
}

/// Offers grouped by GTIN-14: listings, distinct sellers and the price range of in-stock
/// offers with a stated price. Listings without a valid JAN are counted, not grouped.
pub fn by_product(listings: &[Value]) -> Value {
    let mut groups: BTreeMap<String, Vec<&Value>> = BTreeMap::new();
    let mut without = 0;
    for l in listings {
        match l["gtin14"].as_str() {
            Some(g) => groups.entry(g.to_string()).or_default().push(l),
            None => without += 1,
        }
    }
    let products: Vec<Value> = groups
        .into_iter()
        .map(|(gtin, rows)| {
            let sellers: std::collections::BTreeSet<&str> =
                rows.iter().filter_map(|r| r["seller_id"].as_str()).collect();
            let mut prices: Vec<i64> = rows
                .iter()
                .filter(|r| r["in_stock"] == true)
                .filter_map(|r| r["price_minor"].as_i64())
                .collect();
            // The same code on Amazon Japan, through the official catalog: an item answers only when
            // its own identifiers list the JAN.
            // The code as the listing states it (JAN-13 or short JAN-8), checksum already valid.
            let jan = rows.iter().find_map(|r| r["jan"].as_str()).unwrap_or(&gtin);
            let next = json!({"tool":"ecdev.seller.read","input_template":{"market":"AMAZON_JP","operation":"CATALOG_SEARCH_BY_IDENTIFIER","identifiers_type":"JAN","identifiers":[jan]},"fills":["CROSS_SOURCE_PRODUCT_IDENTIFIER","SECOND_LISTING_ORIGIN"],"requires":"SP-API credentials and the operator gate","paid":false});
            json!({"next_action":next,"gtin14":gtin,"listings":rows.len(),"distinct_sellers":sellers.len(),"in_stock_priced_offers":prices.len(),
                "price_min_minor":prices.iter().min(),"price_median_minor":median(&mut prices),"price_max_minor":prices.iter().max(),"currency":"JPY",
                "titles":rows.iter().filter_map(|r| r["title"].as_str()).take(3).collect::<Vec<_>>()})
        })
        .collect();
    json!({"products":products,"listings_without_valid_jan":without,"seller_scope":"SELLERS_WITHIN_ONE_MARKETPLACE_NOT_INDEPENDENT_SITES"})
}

impl Engine {
    pub fn listing_search(&self, args: Value) -> Result<Value, String> {
        if args.get("market").is_some_and(|m| m != "YAHOO_SHOPPING_JP") {
            return Err("UNSUPPORTED_LISTING_MARKET".into());
        }
        let fixture = args.get("fixture_raw").is_some();
        if !fixture && args.get("fixture_now").is_some() {
            return Err("LIVE_CLOCK_OVERRIDE_DENIED".into());
        }
        let now = if fixture {
            args["fixture_now"].as_u64().unwrap_or(timestamp())
        } else {
            timestamp()
        };
        let provider = self
            .providers
            .iter()
            .find(|p| p.id() == "yahoo-shopping-jp")
            .ok_or("OFFICIAL_LISTING_PROVIDER_NOT_CONFIGURED")?;
        let mut query = args.clone();
        if let Some(o) = query.as_object_mut() {
            o.remove("market");
            o.remove("fixture_now");
        }
        query["captured_at"] = json!(now);
        let acquired = match provider.acquire(&AcquireRequest {
            run_id: Uuid::new_v4().to_string(),
            capability: "listing.search".into(),
            market: "YAHOO_SHOPPING_JP".into(),
            query,
        }) {
            Ok(a) => a,
            Err(e) => {
                let state = match (e.reason.as_str(), e.http_status) {
                    (r, _) if r.starts_with("AUTH_REQUIRED") => "AUTH_REQUIRED",
                    (_, Some(429)) => "RATE_LIMITED",
                    (_, Some(401 | 403)) => "SOURCE_BLOCKED",
                    _ => "SOURCE_UNAVAILABLE",
                };
                return Ok(
                    json!({"state":state,"reason":e.reason,"http_status":e.http_status,"request_count":e.request_count,"listings":[],"paid":false}),
                );
            }
        };
        let count = acquired.provider_cost["request_count"].as_u64();
        let mode = if fixture { "FIXTURE" } else { "LIVE" };
        let hash = format!("{:x}", Sha256::digest(&acquired.raw_payload));
        if (fixture && count != Some(0))
            || (!fixture && count.is_none_or(|n| n == 0))
            || acquired.result["raw_hash"] != json!(hash)
            || acquired.result["capture_mode"] != json!(mode)
        {
            return Err("LISTING_CAPTURE_WITNESS_MISSING_OR_MODE_MISMATCH".into());
        }
        let dir = self.root.join(".ecdev-data/runtime/listing-captures");
        fs::create_dir_all(&dir).map_err(err)?;
        fs::write(dir.join(format!("{hash}.raw")), &acquired.raw_payload).map_err(err)?;
        let mut out = acquired.result;
        let listings = out["listings"].as_array().cloned().unwrap_or_default();
        out["by_product"] = by_product(&listings);
        out["search_id"] = json!(Uuid::new_v4().to_string());
        out["market"] = json!("YAHOO_SHOPPING_JP");
        out["state"] = json!("PROVIDER_REPORTED_LISTINGS");
        out["request_count"] = json!(count);
        out["egress"] = provider.metadata()["egress"].clone();
        out["invariants"] = json!([
            "provider-reported listings are not ECDEV page observations",
            "sellers inside one marketplace are not independent sites",
            "a listing is not demand",
            "the total is the provider's count and only the first 1000 are reachable"
        ]);
        self.db
            .lock()
            .map_err(err)?
            .execute(
                "INSERT INTO listing_searches VALUES(?1,?2,?3,?4)",
                params![out["search_id"].as_str(), mode, now, out.to_string()],
            )
            .map_err(err)?;
        Ok(out)
    }
}
