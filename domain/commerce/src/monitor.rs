//! Persisted bounded watches. Leases fence concurrent schedulers and survive restart.
use crate::Engine;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
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
            watches.push(json!({"watch_id":id,"facts":facts,"status":if enabled {"ACTIVE"} else {"DISABLED"},"schedule":{"interval_seconds":interval,"next_due":next},"lease_until":lease,"request":serde_json::from_str::<Value>(&args).map_err(err)?,"baseline":baseline.map(|s|serde_json::from_str::<Value>(&s)).transpose().map_err(err)?,"last_error":error,"snapshots":history("watch_snapshots")?,"changes":history("watch_changes")?,"notifications":"NONE"}));
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
            for field in ["price_minor", "availability"] {
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
                if !prev[field].is_null() && !p[field].is_null() && prev[field] != p[field] {
                    out.push(json!({"kind":kind,"url":url,"product":key,"before":prev[field],"after":p[field],"across_gap":gap,"raw_capture_sha256":after["raw_capture_sha256"]}));
                }
            }
        }
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
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
        let availability = facts.iter().find(|f| f["field"] == "availability").unwrap();
        assert_eq!(availability["last_observed_at"].as_u64(), Some(701));
    }
}
