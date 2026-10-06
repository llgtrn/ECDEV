//! Persisted bounded watches. Leases fence concurrent schedulers and survive restart.
use crate::Engine;
use crate::availability::{availability_term, derived_availability};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
pub fn initialize(db: &Connection) -> Result<(), String> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS watches(id TEXT PRIMARY KEY,args TEXT NOT NULL,interval_secs INTEGER NOT NULL,next_due INTEGER NOT NULL,lease_until INTEGER NOT NULL DEFAULT 0,token TEXT,baseline TEXT,last_error TEXT); CREATE TABLE IF NOT EXISTS watch_snapshots(id INTEGER PRIMARY KEY,watch_id TEXT NOT NULL REFERENCES watches(id),at INTEGER NOT NULL,payload TEXT NOT NULL); CREATE TABLE IF NOT EXISTS watch_changes(id INTEGER PRIMARY KEY,watch_id TEXT NOT NULL REFERENCES watches(id),at INTEGER NOT NULL,payload TEXT NOT NULL); CREATE TABLE IF NOT EXISTS watch_facts(id INTEGER PRIMARY KEY,watch_id TEXT NOT NULL REFERENCES watches(id),url TEXT NOT NULL,product TEXT NOT NULL,field TEXT NOT NULL,value TEXT NOT NULL,valid_at INTEGER NOT NULL,invalid_at INTEGER,run_id TEXT,raw_capture_sha256 TEXT); PRAGMA user_version=2;").map_err(err)?;
    let enabled: bool = db
        .prepare("PRAGMA table_info(watches)")
        .map_err(err)?
        .query_map([], |r| r.get::<_, String>(1))
        .map_err(err)?
        .filter_map(Result::ok)
        .any(|name| name == "enabled");
    if !enabled {
        db.execute_batch("ALTER TABLE watches ADD COLUMN enabled INTEGER NOT NULL DEFAULT 1;")
            .map_err(err)?;
    }
    let observed: bool = db
        .prepare("PRAGMA table_info(watch_facts)")
        .map_err(err)?
        .query_map([], |r| r.get::<_, String>(1))
        .map_err(err)?
        .filter_map(Result::ok)
        .any(|name| name == "last_observed_at");
    if !observed {
        // Facts recorded before this column existed were last observed when they began.
        db.execute_batch("ALTER TABLE watch_facts ADD COLUMN last_observed_at INTEGER; UPDATE watch_facts SET last_observed_at=valid_at WHERE last_observed_at IS NULL;")
            .map_err(err)?;
    }
    Ok(())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WatchInput {
    #[serde(default)]
    watch_id: Option<String>,
    #[serde(default = "default_enabled")]
    enabled: bool,
    market: String,
    query: String,
    targets: Vec<String>,
    interval_seconds: u64,
}
fn default_enabled() -> bool {
    true
}
impl Engine {
    pub fn monitor_create(&self, args: Value) -> Result<Value, String> {
        let input: WatchInput = serde_json::from_value(args).map_err(err)?;
        if !(60..=604800).contains(&input.interval_seconds)
            || input.targets.is_empty()
            || input.targets.len() > 5
            || input.query.is_empty()
            || input.query.len() > 500
            || !["PUBLIC_WEB", "AMAZON_JP", "AMAZON_US"].contains(&input.market.as_str())
        {
            return Err("INVALID_WATCH_LIMITS".into());
        }
        let mut sources = vec![];
        for target in input.targets {
            let identity = crate::frontier::canonicalize(
                &target,
                None,
                &crate::frontier::UrlPolicy::default(),
                0,
            )?;
            sources.push(json!({"url":identity.canonical_url}));
        }
        let request = json!({"market":input.market,"query":input.query,"sources":sources,"max_pages":sources.len(),"max_depth":0,"max_urls":sources.len(),"deadline_seconds":120,"force_refresh":true});
        let id = input
            .watch_id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        {
            let db = self.db.lock().map_err(err)?;
            if input.watch_id.is_some() {
                let changed = db.execute("UPDATE watches SET args=?2,interval_secs=?3,next_due=?4,enabled=?5,token=NULL,lease_until=0 WHERE id=?1", params![id,request.to_string(),input.interval_seconds,crate::service::timestamp(),input.enabled]).map_err(err)?;
                if changed == 0 {
                    return Err("WATCH_NOT_FOUND".into());
                }
            } else {
                db.execute("INSERT INTO watches(id,args,interval_secs,next_due,enabled) VALUES(?1,?2,?3,?4,?5)",params![id,request.to_string(),input.interval_seconds,crate::service::timestamp(),input.enabled]).map_err(err)?;
            }
        }
        self.monitor_status(Some(&id))
    }
    pub fn monitor_status(&self, id: Option<&str>) -> Result<Value, String> {
        let db = self.db.lock().map_err(err)?;
        let mut stmt=db.prepare("SELECT id,args,interval_secs,next_due,lease_until,baseline,last_error,enabled FROM watches WHERE (?1 IS NULL OR id=?1) ORDER BY rowid DESC LIMIT 100").map_err(err)?;
        let rows = stmt
            .query_map([id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, u64>(2)?,
                    r.get::<_, u64>(3)?,
                    r.get::<_, u64>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, bool>(7)?,
                ))
            })
            .map_err(err)?;
        let mut watches = vec![];
        for row in rows {
            let (id, args, interval, next, lease, baseline, error, enabled) = row.map_err(err)?;
            let history = |table: &str| -> Result<Vec<Value>, String> {
                let mut s = db
                    .prepare(&format!(
                        "SELECT payload FROM {table} WHERE watch_id=?1 ORDER BY id DESC LIMIT 100"
                    ))
                    .map_err(err)?;
                let rows = s.query_map([&id], |r| r.get::<_, String>(0)).map_err(err)?;
                rows.map(|r| serde_json::from_str(&r.map_err(err)?).map_err(err))
                    .collect()
            };
            let facts = {
                let mut s = db
                    .prepare("SELECT url,product,field,value,valid_at,invalid_at,run_id,raw_capture_sha256,last_observed_at FROM watch_facts WHERE watch_id=?1 ORDER BY id DESC LIMIT 200")
                    .map_err(err)?;
                let rows = s
                    .query_map([&id], |r| {
                        Ok(json!({"url":r.get::<_,String>(0)?,"product":r.get::<_,String>(1)?,"field":r.get::<_,String>(2)?,"value":serde_json::from_str::<Value>(&r.get::<_,String>(3)?).unwrap_or(Value::Null),"valid_at":r.get::<_,u64>(4)?,"invalid_at":r.get::<_,Option<u64>>(5)?,"run_id":r.get::<_,Option<String>>(6)?,"raw_capture_sha256":r.get::<_,Option<String>>(7)?,"last_observed_at":r.get::<_,Option<u64>>(8)?,"state":"OBSERVED"}))
                    })
                    .map_err(err)?;
                rows.collect::<Result<Vec<_>, _>>().map_err(err)?
            };
            let price_windows = {
                let mut s = db
                    .prepare("SELECT url,product,field,value,valid_at,invalid_at,last_observed_at FROM watch_facts WHERE watch_id=?1 AND field IN ('price_minor','currency') ORDER BY id")
                    .map_err(err)?;
                let rows = s
                    .query_map([&id], |r| {
                        Ok(FactWindow {
                            url: r.get(0)?,
                            product: r.get(1)?,
                            field: r.get(2)?,
                            value: r.get(3)?,
                            valid_at: r.get(4)?,
                            invalid_at: r.get(5)?,
                            last_observed_at: r.get(6)?,
                        })
                    })
                    .map_err(err)?;
                rows.collect::<Result<Vec<_>, _>>().map_err(err)?
            };
            let price_statistics = price_statistics(&price_windows);
            watches.push(json!({"watch_id":id,"facts":facts,"price_statistics":price_statistics,"status":if enabled {"ACTIVE"} else {"DISABLED"},"schedule":{"interval_seconds":interval,"next_due":next},"lease_until":lease,"request":serde_json::from_str::<Value>(&args).map_err(err)?,"baseline":baseline.map(|s|serde_json::from_str::<Value>(&s)).transpose().map_err(err)?,"last_error":error,"snapshots":history("watch_snapshots")?,"changes":history("watch_changes")?,"notifications":"NONE"}));
        }
        if id.is_some() {
            watches.into_iter().next().ok_or("WATCH_NOT_FOUND".into())
        } else {
            Ok(json!(watches))
        }
    }
    fn claim_watch(&self, now: u64) -> Result<Option<Value>, String> {
        let mut db = self.db.lock().map_err(err)?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        let row:Option<(String,String)>=tx.query_row("SELECT id,args FROM watches WHERE enabled=1 AND next_due<=?1 AND lease_until<=?1 ORDER BY next_due,rowid LIMIT 1",[now],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(err)?;
        let Some((id, args)) = row else {
            return Ok(None);
        };
        let token = Uuid::new_v4().to_string();
        tx.execute(
            "UPDATE watches SET lease_until=?2,token=?3 WHERE id=?1",
            params![id, now + 180, token],
        )
        .map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(Some(
            json!({"watch_id":id,"token":token,"request":serde_json::from_str::<Value>(&args).map_err(err)?}),
        ))
    }
    pub fn monitor_tick(&self, now: u64) -> Result<bool, String> {
        let Some(lease) = self.claim_watch(now)? else {
            return Ok(false);
        };
        let result = self.research(lease["request"].clone());
        self.finish_watch(&lease, crate::service::timestamp(), result)?;
        Ok(true)
    }
    fn finish_watch(
        &self,
        lease: &Value,
        now: u64,
        result: Result<Value, String>,
    ) -> Result<(), String> {
        let mut db = self.db.lock().map_err(err)?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        let prior: Option<Option<String>> = tx
            .query_row(
                "SELECT baseline FROM watches WHERE id=?1 AND token=?2 AND lease_until>?3",
                params![lease["watch_id"].as_str(), lease["token"].as_str(), now],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        let Some(prior) = prior else {
            return Err("WATCH_LEASE_LOST".into());
        };
        match result {
            Ok(run) => {
                let old = prior
                    .map(|s| serde_json::from_str::<Value>(&s))
                    .transpose()
                    .map_err(err)?
                    .unwrap_or(json!({}));
                let current = snapshot(&run, &lease["request"]);
                let changes = changes(&old, &current);
                let mut baseline = old.clone();
                for (url, source) in current.as_object().ok_or("INVALID_WATCH_SNAPSHOT")? {
                    if source["status"] != "UNKNOWN" {
                        baseline[url] = carry_unobserved(&old[url], source, now);
                    }
                }
                record_facts(
                    &tx,
                    lease["watch_id"].as_str().unwrap_or(""),
                    now,
                    &run,
                    &current,
                )?;
                tx.execute("INSERT INTO watch_snapshots(watch_id,at,payload) VALUES(?1,?2,?3)",params![lease["watch_id"].as_str(),now,json!({"at":now,"run_id":run["run_id"],"mode":run["mode"],"sources":current}).to_string()]).map_err(err)?;
                for change in changes {
                    tx.execute(
                        "INSERT INTO watch_changes(watch_id,at,payload) VALUES(?1,?2,?3)",
                        params![
                            lease["watch_id"].as_str(),
                            now,
                            json!({"at":now,"run_id":run["run_id"],"trigger":change}).to_string()
                        ],
                    )
                    .map_err(err)?;
                }
                tx.execute("UPDATE watches SET baseline=?2,last_error=?3,next_due=?4+interval_secs,lease_until=0,token=NULL WHERE id=?1",params![lease["watch_id"].as_str(),baseline.to_string(),if run["errors"].as_array().is_some_and(|e|!e.is_empty()){Some(run["errors"].to_string())}else{None},now]).map_err(err)?;
            }
            Err(reason) => {
                tx.execute("UPDATE watches SET last_error=?2,next_due=?3+interval_secs,lease_until=0,token=NULL WHERE id=?1",params![lease["watch_id"].as_str(),reason,now]).map_err(err)?;
            }
        }
        tx.commit().map_err(err)
    }
}

/// Observed price and availability become facts with validity windows: a newly observed value
/// starts a window at the observation time and closes every open window it supersedes
/// (memory::superseded). Windows open at the same instant stay open side by side: a conflict,
/// never a silent choice. Unknown observations change nothing. Re-observing an open value moves
/// its `last_observed_at`; a value missing from a capture closes nothing, so a window kept open
/// through a gap shows when it was last actually seen.
fn record_facts(
    tx: &rusqlite::Transaction,
    watch: &str,
    now: u64,
    run: &Value,
    current: &Value,
) -> Result<(), String> {
    for (url, source) in current.as_object().into_iter().flatten() {
        if source["status"] != "AVAILABLE" {
            continue;
        }
        for (product, fields) in source["products"].as_object().into_iter().flatten() {
            for field in ["price_minor", "currency", "availability"] {
                let value = &fields[field];
                if value.is_null() {
                    continue;
                }
                let mut stmt = tx
                    .prepare("SELECT id,value,valid_at FROM watch_facts WHERE watch_id=?1 AND url=?2 AND product=?3 AND field=?4 AND invalid_at IS NULL ORDER BY id")
                    .map_err(err)?;
                let open: Vec<(i64, String, u64)> = stmt
                    .query_map(params![watch, url, product, field], |r| {
                        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                    })
                    .map_err(err)?
                    .collect::<Result<_, _>>()
                    .map_err(err)?;
                let text = value.to_string();
                if let Some((id, _, _)) = open.iter().find(|(_, v, _)| *v == text) {
                    tx.execute(
                        "UPDATE watch_facts SET last_observed_at=?2 WHERE id=?1",
                        params![id, now],
                    )
                    .map_err(err)?;
                    continue;
                }
                let windows: Vec<crate::memory::Validity<u64>> = open
                    .iter()
                    .map(|(_, _, at)| crate::memory::Validity {
                        valid_at: Some(*at),
                        invalid_at: None,
                    })
                    .collect();
                let new = crate::memory::Validity {
                    valid_at: Some(now),
                    invalid_at: None,
                };
                for (i, end) in crate::memory::superseded(&new, &windows) {
                    tx.execute(
                        "UPDATE watch_facts SET invalid_at=?2 WHERE id=?1",
                        params![open[i].0, end],
                    )
                    .map_err(err)?;
                }
                tx.execute("INSERT INTO watch_facts(watch_id,url,product,field,value,valid_at,invalid_at,run_id,raw_capture_sha256,last_observed_at) VALUES(?1,?2,?3,?4,?5,?6,NULL,?7,?8,?6)",params![watch,url,product,field,text,now,run["run_id"].as_str(),source["raw_capture_sha256"].as_str()]).map_err(err)?;
            }
        }
    }
    Ok(())
}

fn snapshot(run: &Value, request: &Value) -> Value {
    let mut sources = serde_json::Map::new();
    for source in request["sources"].as_array().into_iter().flatten() {
        let Some(url) = source["url"].as_str() else {
            continue;
        };
        let page = run["snapshots"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|s| s["source"] == url);
        if let Some(page) = page {
            let mut products: BTreeMap<String, Value> = BTreeMap::new();
            for p in page["products"].as_array().into_iter().flatten() {
                let key = if p["sku"].is_string() {
                    p["sku"].to_string()
                } else {
                    p["title"].to_string()
                };
                let fields: serde_json::Map<String, Value> = [
                    "title",
                    "sku",
                    "brand",
                    "mpn",
                    "model",
                    "ean",
                    "upc",
                    "price_minor",
                    "currency",
                    "availability",
                    "seller",
                    "rating",
                    "review_count",
                    "images",
                    "description",
                    "specifications",
                    "observed_offers",
                ]
                .iter()
                .map(|k| (k.to_string(), p[*k].clone()))
                .collect();
                products.insert(key, Value::Object(fields));
            }
            sources.insert(url.into(),json!({"status":"AVAILABLE","products":products,"raw_capture_sha256":page["content_hash"]}));
        } else {
            let gone = run["errors"].as_array().into_iter().flatten().any(|e| {
                e["source"] == url
                    && matches!(
                        e["reason"].as_str(),
                        Some("HTTP_STATUS_404" | "HTTP_STATUS_410")
                    )
            });
            sources.insert(
                url.into(),
                json!({"status":if gone{"DISAPPEARED"}else{"UNKNOWN"},"products":{}}),
            );
        }
    }
    Value::Object(sources)
}
impl Engine {
    /// What a watch knew about one product field at time `at`, from its stored fact windows.
    pub fn monitor_fact_as_of(
        &self,
        watch_id: &str,
        url: &str,
        product: &str,
        field: &str,
        at: u64,
    ) -> Result<Value, String> {
        let db = self.db.lock().map_err(err)?;
        let mut s = db
            .prepare("SELECT url,product,field,value,valid_at,invalid_at,last_observed_at FROM watch_facts WHERE watch_id=?1 AND url=?2 AND product=?3 AND field=?4 ORDER BY id")
            .map_err(err)?;
        let windows = s
            .query_map(params![watch_id, url, product, field], |r| {
                Ok(FactWindow {
                    url: r.get(0)?,
                    product: r.get(1)?,
                    field: r.get(2)?,
                    value: r.get(3)?,
                    valid_at: r.get(4)?,
                    invalid_at: r.get(5)?,
                    last_observed_at: r.get(6)?,
                })
            })
            .map_err(err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err)?;
        Ok(fact_as_of(&windows, at))
    }
}

/// The answer one product field's fact windows give for time `at`. A value is OBSERVED only
/// between its first and last observation; after the last observation it is the last observed
/// value with its age (open window) or a change at an unknown time between the last observation
/// and the superseding one (closed window). Before the first observation, and for a field never
/// observed, the answer is UNKNOWN; windows that disagree at `at` are a CONFLICT.
pub fn fact_as_of(windows: &[FactWindow], at: u64) -> Value {
    let parse = |v: &str| serde_json::from_str::<Value>(v).unwrap_or(Value::Null);
    let covering: Vec<&FactWindow> = windows
        .iter()
        .filter(|w| w.valid_at <= at && w.invalid_at.is_none_or(|i| at < i))
        .collect();
    let values: BTreeSet<&str> = covering.iter().map(|w| w.value.as_str()).collect();
    if values.len() > 1 {
        return json!({"at":at,"state":"CONFLICT","value":null,"values":values.iter().map(|v|parse(v)).collect::<Vec<_>>()});
    }
    let Some(w) = covering.first() else {
        let first = windows.iter().map(|w| w.valid_at).min();
        return json!({"at":at,"state":"UNKNOWN","value":null,"reason":if first.is_some(){"BEFORE_FIRST_OBSERVATION"}else{"NEVER_OBSERVED"},"first_observed_at":first});
    };
    let last = covering
        .iter()
        .map(|w| w.last_observed_at.unwrap_or(w.valid_at))
        .max()
        .unwrap_or(w.valid_at);
    let value = parse(&w.value);
    if at <= last {
        return json!({"at":at,"state":"OBSERVED","value":value,"observed_from":w.valid_at,"observed_until":last});
    }
    match w.invalid_at {
        None => {
            json!({"at":at,"state":"NOT_OBSERVED_SINCE","value":null,"last_observed_value":value,"last_observed_at":last,"age_seconds":at-last})
        }
        Some(end) => {
            let next = windows
                .iter()
                .find(|n| n.valid_at == end && n.value != w.value)
                .map(|n| parse(&n.value));
            json!({"at":at,"state":"CHANGE_TIME_UNKNOWN","value":null,"before":value,"after":next,"last_observed_at":last,"next_observed_at":end})
        }
    }
}

/// One stored fact window, as `price_statistics` reads it.
#[derive(Clone, Debug)]
pub struct FactWindow {
    pub url: String,
    pub product: String,
    pub field: String,
    pub value: String,
    pub valid_at: u64,
    pub invalid_at: Option<u64>,
    pub last_observed_at: Option<u64>,
}

/// Time-weighted price statistics per watched product, over observed spans only. A price counts
/// for the time it was actually seen, from its first to its last observation; the time between
/// the last sighting of one price and the first of the next is unobserved and reported, never
/// attributed. Low, quartiles, median and high are observed prices (no arithmetic on money).
/// Overlapping windows with different prices, or more than one currency, give a CONFLICT with
/// no statistics; no observed duration gives INSUFFICIENT_DURATION.
pub fn price_statistics(windows: &[FactWindow]) -> Vec<Value> {
    let mut by_product: BTreeMap<(String, String), Vec<&FactWindow>> = BTreeMap::new();
    for w in windows {
        by_product
            .entry((w.url.clone(), w.product.clone()))
            .or_default()
            .push(w);
    }
    let mut out = vec![];
    for ((url, product), facts) in by_product {
        let currencies: BTreeSet<&str> = facts
            .iter()
            .filter(|w| w.field == "currency")
            .map(|w| w.value.as_str())
            .collect();
        let mut prices: Vec<(i64, u64, u64)> = facts
            .iter()
            .filter(|w| w.field == "price_minor")
            .filter_map(|w| {
                let value = w.value.parse::<i64>().ok()?;
                let seen = w.last_observed_at.unwrap_or(w.valid_at).max(w.valid_at);
                Some((value, w.valid_at, seen))
            })
            .collect();
        if prices.is_empty() {
            continue;
        }
        prices.sort_by_key(|(_, start, _)| *start);
        let base = json!({"url":url,"product":product,"method":"TIME_WEIGHTED_OBSERVED_SPANS","windows":prices.len(),"currency":if currencies.len()==1 {json!(currencies.iter().next())} else {Value::Null}});
        let overlap = prices
            .windows(2)
            .any(|p| p[1].1 < p[0].2 && p[0].0 != p[1].0);
        if currencies.len() > 1 || overlap {
            let mut v = base;
            v["state"] = json!("CONFLICT");
            v["reason"] = json!(if overlap {
                "OVERLAPPING_PRICE_WINDOWS"
            } else {
                "MORE_THAN_ONE_CURRENCY"
            });
            out.push(v);
            continue;
        }
        let observed: u64 = prices.iter().map(|(_, a, b)| b - a).sum();
        let span = prices.iter().map(|p| p.2).max().unwrap_or(0) - prices[0].1;
        let mut v = base;
        v["observed_seconds"] = json!(observed);
        v["unobserved_seconds"] = json!(span.saturating_sub(observed));
        v["currency_state"] = json!(if currencies.len() == 1 {
            "OBSERVED"
        } else {
            "UNKNOWN"
        });
        if observed == 0 {
            v["state"] = json!("INSUFFICIENT_DURATION");
            v["observed_prices"] = json!(prices.iter().map(|p| p.0).collect::<Vec<_>>());
            out.push(v);
            continue;
        }
        let mut weighted: Vec<(i64, u64)> = prices.iter().map(|(p, a, b)| (*p, b - a)).collect();
        weighted.sort();
        // The observed price at which the cumulative observed time first reaches q of the total.
        let quantile = |num: u64, den: u64| -> i64 {
            let mut cumulative = 0u64;
            for (price, seconds) in &weighted {
                cumulative += seconds;
                if cumulative * den >= observed * num && *seconds > 0 {
                    return *price;
                }
            }
            weighted.last().map(|w| w.0).unwrap_or_default()
        };
        let positive: Vec<i64> = weighted.iter().filter(|w| w.1 > 0).map(|w| w.0).collect();
        let current = prices.last().map(|p| p.0).unwrap_or_default();
        let below: u64 = weighted.iter().filter(|w| w.0 < current).map(|w| w.1).sum();
        v["state"] = json!("DERIVED");
        v["low_minor"] = json!(positive.first());
        v["p25_minor"] = json!(quantile(1, 4));
        v["median_minor"] = json!(quantile(1, 2));
        v["p75_minor"] = json!(quantile(3, 4));
        v["high_minor"] = json!(positive.last());
        v["latest_minor"] = json!(current);
        v["share_of_observed_time_below_latest_bps"] = json!(below * 10_000 / observed);
        out.push(v);
    }
    out
}

/// Fields a capture may stop carrying; a gap is reported, never read as "unchanged".
const TRACKED: [(&str, &str); 2] = [
    ("price_minor", "PRICE_CHANGED"),
    ("availability", "AVAILABILITY_CHANGED"),
];

/// The products of a baseline source as last observed in its latest capture: products and
/// fields carried through a gap are left out, so a gap is reported once, not on every tick.
fn observed_view(products: &Value) -> Value {
    let mut out = serde_json::Map::new();
    for (key, p) in products.as_object().into_iter().flatten() {
        if !p["_unobserved_since"].is_null() {
            continue;
        }
        let mut fields = serde_json::Map::new();
        for (k, v) in p.as_object().into_iter().flatten() {
            if k.starts_with('_') {
                continue;
            }
            let gap = !p["_unobserved_fields"][k].is_null();
            fields.insert(k.clone(), if gap { Value::Null } else { v.clone() });
        }
        out.insert(key.clone(), Value::Object(fields));
    }
    Value::Object(out)
}

/// The next baseline of a source: the new capture, with every product and tracked field it no
/// longer carries kept at its last observed value and marked with when it stopped being seen.
/// A later observation is compared against that last observed value.
fn carry_unobserved(before: &Value, after: &Value, now: u64) -> Value {
    let mut next = after.clone();
    if after["status"] != "AVAILABLE" || before["status"] != "AVAILABLE" {
        return next;
    }
    let Some(products) = next["products"].as_object_mut() else {
        return next;
    };
    for (key, prev) in before["products"].as_object().into_iter().flatten() {
        match products.get_mut(key) {
            None => {
                let mut carried = prev.clone();
                if carried["_unobserved_since"].is_null() {
                    carried["_unobserved_since"] = json!(now);
                }
                products.insert(key.clone(), carried);
            }
            Some(p) => {
                for (field, _) in TRACKED {
                    if p[field].is_null() && !prev[field].is_null() {
                        p[field] = prev[field].clone();
                        let since = prev["_unobserved_fields"][field].as_u64().unwrap_or(now);
                        p["_unobserved_fields"][field] = json!(since);
                    }
                }
            }
        }
    }
    next
}

fn changes(old: &Value, new: &Value) -> Vec<Value> {
    let mut out = vec![];
    for (url, after) in new.as_object().into_iter().flatten() {
        let before = &old[url];
        if before.is_null() || after["status"] == "UNKNOWN" {
            continue;
        }
        if after["status"] == "DISAPPEARED" && before["status"] == "AVAILABLE" {
            out.push(json!({"kind":"PAGE_DISAPPEARED","url":url,"evidence":"HTTP_404_OR_410"}));
            continue;
        }
        if after["status"] != "AVAILABLE" {
            continue;
        }
        if observed_view(&before["products"]) != after["products"] {
            out.push(json!({"kind":"PRODUCT_DATA_CHANGED","url":url,"before":observed_view(&before["products"]),"after":after["products"]}));
        }
        for (key, prev) in before["products"].as_object().into_iter().flatten() {
            if after["products"][key].is_null() && prev["_unobserved_since"].is_null() {
                out.push(json!({"kind":"PRODUCT_NOT_OBSERVED","url":url,"product":key,"last_observed":observed_view(&Value::Object([(key.clone(),prev.clone())].into_iter().collect()))[key.as_str()],"raw_capture_sha256":after["raw_capture_sha256"],"evidence":"CAPTURED_PAGE_WITHOUT_THE_PRODUCT"}));
            }
        }
        for (key, p) in after["products"].as_object().into_iter().flatten() {
            let prev = &before["products"][key];
            for (field, kind) in TRACKED {
                let gap = !prev["_unobserved_since"].is_null()
                    || !prev["_unobserved_fields"][field].is_null();
                if !prev[field].is_null() && p[field].is_null() && !gap {
                    out.push(json!({"kind":"FIELD_NOT_OBSERVED","url":url,"product":key,"field":field,"last_observed":prev[field],"raw_capture_sha256":after["raw_capture_sha256"],"evidence":"CAPTURED_PRODUCT_WITHOUT_THE_FIELD"}));
                }
                let differs = if field == "availability" {
                    // Two spellings of one schema.org term are not a stock change.
                    match (
                        availability_term(&prev[field]),
                        availability_term(&p[field]),
                    ) {
                        (Some(a), Some(b)) => a != b,
                        _ => prev[field] != p[field],
                    }
                } else {
                    prev[field] != p[field]
                };
                if !prev[field].is_null() && !p[field].is_null() && differs {
                    let mut event = json!({"kind":kind,"url":url,"product":key,"before":prev[field],"after":p[field],"across_gap":gap,"raw_capture_sha256":after["raw_capture_sha256"]});
                    if field == "availability" {
                        event["before_derived"] = derived_availability(&prev[field]);
                        event["after_derived"] = derived_availability(&p[field]);
                    }
                    out.push(event);
                }
            }
        }
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    /// LongMemEval's knowledge-update, temporal-reasoning and abstention question types,
    /// restated over a watch's own observation history.
    #[test]
    fn memory_questions_over_observed_history() {
        let root = std::env::temp_dir().join(format!("ecdev-asof-{}", Uuid::new_v4()));
        let e = Engine::open(&root).unwrap();
        let watch=e.monitor_create(json!({"market":"PUBLIC_WEB","query":"Watch cup","targets":["https://shop.example/product"],"interval_seconds":60})).unwrap();
        let id = watch["watch_id"].as_str().unwrap().to_string();
        let url = "https://shop.example/product";
        let run = |price: u64| json!({"run_id":"fixture","mode":"FIXTURE","errors":[],"snapshots":[{"source":url,"content_hash":"fixture","products":[{"sku":"Cup","title":"Cup","price_minor":price,"currency":"JPY"}]}]});
        let tick = |at: u64, outcome: Value| {
            e.db.lock()
                .unwrap()
                .execute("UPDATE watches SET next_due=?1", [at])
                .unwrap();
            let lease = e.claim_watch(at).unwrap().unwrap();
            e.finish_watch(&lease, at, Ok(outcome)).unwrap();
        };
        tick(100, run(3000));
        tick(200, run(3000));
        tick(
            300,
            json!({"run_id":"gap","mode":"LIVE","errors":[{"source":url,"reason":"ROBOTS_DENIED_OR_UNKNOWN"}],"snapshots":[]}),
        );
        tick(400, run(4000));
        let product = e.monitor_status(Some(&id)).unwrap()["facts"][0]["product"]
            .as_str()
            .unwrap()
            .to_string();
        let ask =
            |field: &str, at: u64| e.monitor_fact_as_of(&id, url, &product, field, at).unwrap();
        // Knowledge update: the latest answer is the superseding value.
        let now = ask("price_minor", 400);
        assert_eq!(
            (now["state"].clone(), now["value"].clone()),
            (json!("OBSERVED"), json!(4000))
        );
        // Temporal reasoning: an earlier time gets the value observed then ...
        assert_eq!(ask("price_minor", 150)["value"], 3000);
        // ... and a time inside the gap gets no value: the change happened somewhere in it.
        let gap = ask("price_minor", 300);
        assert_eq!(gap["state"], "CHANGE_TIME_UNKNOWN");
        assert!(gap["value"].is_null());
        assert_eq!(
            (gap["before"].clone(), gap["after"].clone()),
            (json!(3000), json!(4000))
        );
        assert_eq!(
            (
                gap["last_observed_at"].clone(),
                gap["next_observed_at"].clone()
            ),
            (json!(200), json!(400))
        );
        // Abstention: before the first observation, a field never observed, and after the last.
        let early = ask("price_minor", 50);
        assert_eq!(
            (early["state"].clone(), early["reason"].clone()),
            (json!("UNKNOWN"), json!("BEFORE_FIRST_OBSERVATION"))
        );
        assert_eq!(ask("availability", 400)["reason"], "NEVER_OBSERVED");
        let later = ask("price_minor", 1000);
        assert_eq!(
            (
                later["state"].clone(),
                later["value"].clone(),
                later["age_seconds"].clone()
            ),
            (json!("NOT_OBSERVED_SINCE"), Value::Null, json!(600))
        );
    }

    #[test]
    fn disagreeing_windows_are_a_conflict() {
        let w = |value: &str| FactWindow {
            url: "u".into(),
            product: "p".into(),
            field: "price_minor".into(),
            value: value.into(),
            valid_at: 10,
            invalid_at: None,
            last_observed_at: Some(10),
        };
        let a = fact_as_of(&[w("1"), w("2")], 10);
        assert_eq!(a["state"], "CONFLICT");
        assert!(a["value"].is_null());
    }

    #[test]
    fn availability_spelling_is_not_a_stock_change() {
        let page =
            |a: &str| json!({"u":{"status":"AVAILABLE","products":{"cup":{"availability":a}}}});
        let kinds = |a: &str, b: &str| {
            changes(&page(a), &page(b))
                .into_iter()
                .filter(|c| c["kind"] == "AVAILABILITY_CHANGED")
                .collect::<Vec<_>>()
        };
        assert!(kinds("InStock", "https://schema.org/InStock").is_empty());
        let real = kinds("schema:InStock", "https://schema.org/OutOfStock");
        assert_eq!(real.len(), 1);
        assert_eq!(real[0]["before"], "schema:InStock");
        assert_eq!(real[0]["after_derived"]["class"], "NOT_AVAILABLE");
        // Unrecognised values still compare as observed strings.
        assert_eq!(kinds("In stock", "in stock").len(), 1);
    }
    #[test]
    fn watch_restart_fencing_and_changes_persist() {
        let root = std::env::temp_dir().join(format!("ecdev-watch-{}", Uuid::new_v4()));
        let e = Engine::open(&root).unwrap();
        let watch=e.monitor_create(json!({"market":"PUBLIC_WEB","query":"Watch cup","targets":["https://shop.example/product"],"interval_seconds":60})).unwrap();
        let id = watch["watch_id"].as_str().unwrap();
        e.db.lock()
            .unwrap()
            .execute("UPDATE watches SET next_due=100", [])
            .unwrap();
        let first = e.claim_watch(100).unwrap().unwrap();
        assert!(e.claim_watch(101).unwrap().is_none());
        drop(e);
        let e = Engine::open(&root).unwrap();
        let recovered = e.claim_watch(281).unwrap().unwrap();
        assert_ne!(first["token"], recovered["token"]);
        assert!(
            e.finish_watch(&first, 282, Err("old worker".into()))
                .is_err()
        );
        let run = |price, availability| json!({"run_id":"fixture","mode":"FIXTURE","errors":[],"snapshots":[{"source":"https://shop.example/product","content_hash":"fixture","products":[{"sku":"Cup","title":"Cup","price_minor":price,"availability":availability}]}]});
        e.finish_watch(&recovered, 282, Ok(run(3000, "InStock")))
            .unwrap();
        let next = e.claim_watch(342).unwrap().unwrap();
        e.finish_watch(&next, 343, Ok(run(4000, "OutOfStock")))
            .unwrap();
        let status = e.monitor_status(Some(id)).unwrap();
        assert_eq!(status["snapshots"].as_array().unwrap().len(), 2);
        let kinds: Vec<_> = status["changes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["trigger"]["kind"].as_str().unwrap())
            .collect();
        assert!(kinds.contains(&"PRICE_CHANGED"));
        assert!(kinds.contains(&"AVAILABILITY_CHANGED"));
        assert!(kinds.contains(&"PRODUCT_DATA_CHANGED"));
        let lease = e.claim_watch(403).unwrap().unwrap();
        e.finish_watch(&lease,404,Ok(json!({"run_id":"unavailable","mode":"LIVE","errors":[{"source":"https://shop.example/product","reason":"ROBOTS_DENIED_OR_UNKNOWN"}],"snapshots":[]}))).unwrap();
        assert_eq!(
            e.monitor_status(Some(id)).unwrap()["changes"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        let lease = e.claim_watch(464).unwrap().unwrap();
        e.finish_watch(&lease,465,Ok(json!({"run_id":"gone","mode":"LIVE","errors":[{"source":"https://shop.example/product","reason":"HTTP_STATUS_404"}],"snapshots":[]}))).unwrap();
        assert_eq!(
            e.monitor_status(Some(id)).unwrap()["changes"][0]["trigger"]["kind"],
            "PAGE_DISAPPEARED"
        );
        // Observed values became facts: the first price window closed when the second began.
        let facts = e.monitor_status(Some(id)).unwrap()["facts"].clone();
        let price: Vec<&Value> = facts
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["field"] == "price_minor")
            .collect();
        assert_eq!(price.len(), 2, "{facts}");
        let old = price.iter().find(|f| f["value"] == 3000).unwrap();
        let new = price.iter().find(|f| f["value"] == 4000).unwrap();
        assert_eq!(
            (old["valid_at"].as_u64(), old["invalid_at"].as_u64()),
            (Some(282), Some(343))
        );
        assert_eq!(
            (new["valid_at"].as_u64(), new["invalid_at"].as_u64()),
            (Some(343), None)
        );
        // Unknown and disappeared observations closed nothing.
        assert!(
            facts
                .as_array()
                .unwrap()
                .iter()
                .all(|f| f["invalid_at"].is_null() || f["invalid_at"] == 343)
        );
        let disabled=e.monitor_create(json!({"watch_id":id,"enabled":false,"market":"PUBLIC_WEB","query":"Watch cup","targets":["https://shop.example/product"],"interval_seconds":60})).unwrap();
        assert_eq!(disabled["status"], "DISABLED");
        assert!(e.claim_watch(1000).unwrap().is_none());
    }

    fn window(
        field: &str,
        value: &str,
        valid_at: u64,
        invalid_at: Option<u64>,
        seen: u64,
    ) -> FactWindow {
        FactWindow {
            url: "https://shop.example/p".into(),
            product: "\"Cup\"".into(),
            field: field.into(),
            value: value.into(),
            valid_at,
            invalid_at,
            last_observed_at: Some(seen),
        }
    }

    #[test]
    fn price_statistics_weigh_observed_time_not_change_events() {
        const DAY: u64 = 86_400;
        // 1000 held for 30 observed days, 900 and 1200 for one day each.
        let w = [
            window("price_minor", "1000", 0, Some(30 * DAY), 30 * DAY),
            window("price_minor", "900", 30 * DAY, Some(31 * DAY), 31 * DAY),
            window("price_minor", "1200", 31 * DAY, None, 32 * DAY),
            window("currency", "\"USD\"", 0, None, 32 * DAY),
        ];
        let s = &price_statistics(&w)[0];
        assert_eq!(s["state"], "DERIVED");
        assert_eq!(
            (
                s["low_minor"].as_i64(),
                s["median_minor"].as_i64(),
                s["high_minor"].as_i64()
            ),
            (Some(900), Some(1000), Some(1200))
        );
        assert_eq!(
            (s["p25_minor"].as_i64(), s["p75_minor"].as_i64()),
            (Some(1000), Some(1000))
        );
        assert_eq!(s["currency"], "\"USD\"");
        // 31 of 32 observed days were below the latest 1200.
        assert_eq!(
            s["share_of_observed_time_below_latest_bps"].as_u64(),
            Some(9687)
        );
        assert_eq!(s["unobserved_seconds"].as_u64(), Some(0));
    }

    #[test]
    fn price_statistics_report_gaps_and_refuse_conflicts() {
        // Seen 0..100 at 1000, then nothing until 1200 is seen 200..300: 100 s unobserved.
        let gap = price_statistics(&[
            window("price_minor", "1000", 0, Some(200), 100),
            window("price_minor", "1200", 200, None, 300),
        ]);
        assert_eq!(
            (
                gap[0]["observed_seconds"].as_u64(),
                gap[0]["unobserved_seconds"].as_u64()
            ),
            (Some(200), Some(100))
        );
        assert_eq!(gap[0]["currency_state"], "UNKNOWN");
        let currencies = price_statistics(&[
            window("price_minor", "1000", 0, None, 100),
            window("currency", "\"USD\"", 0, Some(50), 50),
            window("currency", "\"JPY\"", 50, None, 100),
        ]);
        assert_eq!(
            (
                currencies[0]["state"].as_str(),
                currencies[0]["reason"].as_str()
            ),
            (Some("CONFLICT"), Some("MORE_THAN_ONE_CURRENCY"))
        );
        assert!(currencies[0]["median_minor"].is_null());
        let overlap = price_statistics(&[
            window("price_minor", "1000", 0, None, 100),
            window("price_minor", "1100", 50, None, 100),
        ]);
        assert_eq!(overlap[0]["reason"], "OVERLAPPING_PRICE_WINDOWS");
        let once = price_statistics(&[window("price_minor", "1000", 7, None, 7)]);
        assert_eq!(once[0]["state"], "INSUFFICIENT_DURATION");
    }
    #[test]
    fn a_missing_product_or_field_is_a_gap_never_unchanged() {
        let root = std::env::temp_dir().join(format!("ecdev-watch-gap-{}", Uuid::new_v4()));
        let e = Engine::open(&root).unwrap();
        let watch=e.monitor_create(json!({"market":"PUBLIC_WEB","query":"Gap cup","targets":["https://shop.example/p"],"interval_seconds":60})).unwrap();
        let id = watch["watch_id"].as_str().unwrap().to_string();
        let mut at = 100;
        let mut tick = |products: Value| {
            e.db.lock()
                .unwrap()
                .execute("UPDATE watches SET next_due=?1", [at])
                .unwrap();
            let lease = e.claim_watch(at).unwrap().unwrap();
            e.finish_watch(&lease, at + 1, Ok(json!({"run_id":format!("r{at}"),"mode":"FIXTURE","errors":[],"snapshots":[{"source":"https://shop.example/p","content_hash":format!("h{at}"),"products":products}]}))).unwrap();
            at += 100;
        };
        let cup = |price: Value, availability: Value| json!({"sku":"Cup","title":"Cup","price_minor":price,"availability":availability});
        tick(json!([cup(json!(3000), json!("InStock"))]));
        // The price is missing from a captured page: one FIELD_NOT_OBSERVED, not silence.
        tick(json!([cup(Value::Null, json!("InStock"))]));
        tick(json!([cup(Value::Null, json!("InStock"))]));
        // The price returns changed: detected against the last observed value, across the gap.
        tick(json!([cup(json!(2500), json!("InStock"))]));
        // The product is missing from a captured page: one PRODUCT_NOT_OBSERVED.
        tick(json!([]));
        tick(json!([]));
        tick(json!([cup(json!(2500), json!("InStock"))]));
        let status = e.monitor_status(Some(&id)).unwrap();
        let triggers: Vec<Value> = status["changes"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .map(|c| c["trigger"].clone())
            .collect();
        let kinds: Vec<&str> = triggers
            .iter()
            .map(|t| t["kind"].as_str().unwrap())
            .collect();
        assert_eq!(
            kinds.iter().filter(|k| **k == "FIELD_NOT_OBSERVED").count(),
            1,
            "{kinds:?}"
        );
        assert_eq!(
            kinds
                .iter()
                .filter(|k| **k == "PRODUCT_NOT_OBSERVED")
                .count(),
            1,
            "{kinds:?}"
        );
        let price = triggers
            .iter()
            .find(|t| t["kind"] == "PRICE_CHANGED")
            .unwrap();
        assert_eq!(
            (
                price["before"].as_i64(),
                price["after"].as_i64(),
                price["across_gap"].as_bool()
            ),
            (Some(3000), Some(2500), Some(true))
        );
        let missing = triggers
            .iter()
            .find(|t| t["kind"] == "FIELD_NOT_OBSERVED")
            .unwrap();
        assert_eq!(
            (missing["field"].as_str(), missing["last_observed"].as_i64()),
            (Some("price_minor"), Some(3000))
        );
        // Reappearing unchanged after a product gap raises no price change.
        assert_eq!(
            kinds.iter().filter(|k| **k == "PRICE_CHANGED").count(),
            1,
            "{kinds:?}"
        );
        // The 3000 window closed only when 2500 was observed; its last observation precedes the gap.
        let facts = status["facts"].as_array().unwrap();
        let old = facts
            .iter()
            .find(|f| f["field"] == "price_minor" && f["value"] == 3000)
            .unwrap();
        assert_eq!(
            (
                old["valid_at"].as_u64(),
                old["last_observed_at"].as_u64(),
                old["invalid_at"].as_u64()
            ),
            (Some(101), Some(101), Some(401))
        );
        // Statistics read every stored price window: 3000 seen at 101 only, 2500 seen 401..701.
        let stats = &status["price_statistics"][0];
        assert_eq!(
            (
                stats["state"].as_str(),
                stats["median_minor"].as_i64(),
                stats["observed_seconds"].as_u64()
            ),
            (Some("DERIVED"), Some(2500), Some(300))
        );
        let availability = facts.iter().find(|f| f["field"] == "availability").unwrap();
        assert_eq!(availability["last_observed_at"].as_u64(), Some(701));
    }
}
