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
    db.execute_batch("CREATE TABLE IF NOT EXISTS watches(id TEXT PRIMARY KEY,args TEXT NOT NULL,interval_secs INTEGER NOT NULL,next_due INTEGER NOT NULL,lease_until INTEGER NOT NULL DEFAULT 0,token TEXT,baseline TEXT,last_error TEXT); CREATE TABLE IF NOT EXISTS watch_snapshots(id INTEGER PRIMARY KEY,watch_id TEXT NOT NULL REFERENCES watches(id),at INTEGER NOT NULL,payload TEXT NOT NULL); CREATE TABLE IF NOT EXISTS watch_changes(id INTEGER PRIMARY KEY,watch_id TEXT NOT NULL REFERENCES watches(id),at INTEGER NOT NULL,payload TEXT NOT NULL); PRAGMA user_version=2;").map_err(err)?;
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
            || input.query.len() > 2000
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
            watches.push(json!({"watch_id":id,"status":if enabled {"ACTIVE"} else {"DISABLED"},"schedule":{"interval_seconds":interval,"next_due":next},"lease_until":lease,"request":serde_json::from_str::<Value>(&args).map_err(err)?,"baseline":baseline.map(|s|serde_json::from_str::<Value>(&s)).transpose().map_err(err)?,"last_error":error,"snapshots":history("watch_snapshots")?,"changes":history("watch_changes")?,"notifications":"NONE"}));
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
                let mut baseline = old;
                for (url, source) in current.as_object().ok_or("INVALID_WATCH_SNAPSHOT")? {
                    if source["status"] != "UNKNOWN" {
                        baseline[url] = source.clone();
                    }
                }
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
        if before["products"] != after["products"] {
            out.push(json!({"kind":"PRODUCT_DATA_CHANGED","url":url,"before":before["products"],"after":after["products"]}));
        }
        for (key, p) in after["products"].as_object().into_iter().flatten() {
            let prev = &before["products"][key];
            for (field, kind) in [
                ("price_minor", "PRICE_CHANGED"),
                ("availability", "AVAILABILITY_CHANGED"),
            ] {
                if !prev[field].is_null() && !p[field].is_null() && prev[field] != p[field] {
                    out.push(json!({"kind":kind,"url":url,"product":key,"before":prev[field],"after":p[field],"raw_capture_sha256":after["raw_capture_sha256"]}));
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
        let disabled=e.monitor_create(json!({"watch_id":id,"enabled":false,"market":"PUBLIC_WEB","query":"Watch cup","targets":["https://shop.example/product"],"interval_seconds":60})).unwrap();
        assert_eq!(disabled["status"], "DISABLED");
        assert!(e.claim_watch(1000).unwrap().is_none());
    }
}
