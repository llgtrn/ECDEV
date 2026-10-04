//! Durable observations, snapshots and fenced watches, owned by commerce.
use super::{ForecastScenario, SocialPost, snapshot, terms};
use crate::{
    Engine, planner::allocate_information_actions, provider::AcquireRequest, service::timestamp,
};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs};
use uuid::Uuid;
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
pub fn initialize(db: &Connection) -> Result<(), String> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS social_posts(mode TEXT NOT NULL,key TEXT NOT NULL,payload TEXT NOT NULL, PRIMARY KEY(mode,key));
 CREATE TABLE IF NOT EXISTS trend_snapshots(id TEXT PRIMARY KEY,query TEXT NOT NULL,mode TEXT NOT NULL,window INTEGER NOT NULL,captured INTEGER NOT NULL,payload TEXT NOT NULL);
 CREATE TABLE IF NOT EXISTS social_cache(key TEXT PRIMARY KEY,created INTEGER NOT NULL,payload TEXT NOT NULL);
 CREATE TABLE IF NOT EXISTS trend_watches(id TEXT PRIMARY KEY,enabled INTEGER NOT NULL,next_due INTEGER NOT NULL,lease_until INTEGER NOT NULL DEFAULT 0,lease_token TEXT,payload TEXT NOT NULL);
 CREATE TABLE IF NOT EXISTS social_scenarios(id TEXT PRIMARY KEY,payload TEXT NOT NULL);").map_err(err)
}
fn required<'a>(v: &'a Value, key: &str) -> Result<&'a str, String> {
    v[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("Required {key}"))
}
const TRIGGERS: &[&str] = &[
    "TOPIC_MENTION_GROWTH",
    "ENTITY_GROWTH",
    "CROSS_PLATFORM_APPEARANCE",
    "VELOCITY_THRESHOLD",
    "ACCELERATION_THRESHOLD",
    "NEW_SOURCE_APPEARANCE",
    "SENTIMENT_REGIME_CHANGE",
    "ENGAGEMENT_SPIKE",
    "TREND_DISAPPEARANCE",
];
pub fn tool_definitions() -> Vec<Value> {
    let source = json!({"type":"object","properties":{"platform":{"enum":["HACKER_NEWS","BLUESKY","JSON_FEED"]},"url":{"type":"string","maxLength":4096},"fixture_raw":{"type":"string","maxLength":4194304}},"required":["platform"],"additionalProperties":false});
    let discover = json!({"type":"object","properties":{"query":{"type":"string","minLength":1,"maxLength":500},"sources":{"type":"array","items":source,"minItems":1,"maxItems":5},"window_seconds":{"type":"integer","minimum":60,"maximum":2592000},"request_budget":{"type":"integer","minimum":0,"maximum":20},"cache_only":{"type":"boolean"},"fixture_now":{"type":"integer","minimum":0}},"required":["query","sources"],"additionalProperties":false});
    let inspect = json!({"type":"object","properties":{"snapshot_id":{"type":"string"}},"additionalProperties":false});
    vec![
        json!({"name":"ecdev.trend.discover","description":"Bounded public social acquisition and evidence-linked sampled trends; zero paid; supplied raw data FIXTURE, cache_only makes no requests; missing counters/timestamps UNKNOWN, no social-only shortlist","inputSchema":discover}),
        json!({"name":"ecdev.trend.inspect","description":"Inspect frozen trend snapshots, source evidence, watches and simulation boundary without network; omit snapshot_id to list","inputSchema":inspect}),
        json!({"name":"ecdev.trend.explain","description":"Explain explicit heuristic components, provenance, unknowns and conflicts for a frozen snapshot","inputSchema":inspect}),
        json!({"name":"ecdev.trend.compare","description":"Compare two compatible snapshots with actual timestamps; fixture/live/simulation scopes cannot mix","inputSchema":{"type":"object","properties":{"before_snapshot_id":{"type":"string"},"after_snapshot_id":{"type":"string"}},"required":["before_snapshot_id","after_snapshot_id"],"additionalProperties":false}}),
        json!({"name":"ecdev.trend.watch","description":"Create/update persisted public trend watch or action=list; nine trigger kinds, fenced leases, local events, minimum sample and complete-source policy; enabled=false disables","inputSchema":{"type":"object","properties":{"action":{"enum":["create","list"]},"watch_id":{"type":"string"},"query":{"type":"string","maxLength":500},"research":discover,"triggers":{"type":"array","items":{"enum":TRIGGERS},"minItems":1},"interval_seconds":{"type":"integer","minimum":60,"maximum":604800},"minimum_mentions":{"type":"integer","minimum":2},"threshold":{"type":"number","minimum":0},"enabled":{"type":"boolean"}},"additionalProperties":false}}),
    ]
}

impl Engine {
    fn social_rows(&self, mode: &str) -> Result<Vec<SocialPost>, String> {
        let db = self.db.lock().map_err(err)?;
        let mut stmt = db
            .prepare("SELECT payload FROM social_posts WHERE mode=?1 ORDER BY key LIMIT 5000")
            .map_err(err)?;
        let rows = stmt
            .query_map([mode], |r| r.get::<_, String>(0))
            .map_err(err)?;
        rows.map(|r| serde_json::from_str(&r.map_err(err)?).map_err(err))
            .collect()
    }
    pub fn trend_inspect(&self, args: Value) -> Result<Value, String> {
        if let Some(id) = args["snapshot_id"].as_str() {
            let db = self.db.lock().map_err(err)?;
            let row: String = db
                .query_row(
                    "SELECT payload FROM trend_snapshots WHERE id=?1",
                    [id],
                    |r| r.get(0),
                )
                .map_err(err)?;
            let mut value: Value = serde_json::from_str(&row).map_err(err)?;
            let mode = value["capture_mode"]
                .as_str()
                .unwrap_or("FIXTURE")
                .to_owned();
            drop(db);
            let keys: BTreeSet<_> = value["post_keys"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
            value["observed_source_evidence"] = json!(
                self.social_rows(&mode)?
                    .into_iter()
                    .filter(|p| keys.contains(&p.key()))
                    .collect::<Vec<_>>()
            );
            // Frozen captured observations are retained on each snapshot; current rows must not rewrite history.
            if value["captured_posts"].is_array() {
                value["observed_source_evidence"] = value["captured_posts"].clone();
            }
            return Ok(value);
        }
        let db = self.db.lock().map_err(err)?;
        let mut stmt = db
            .prepare(
                "SELECT payload FROM trend_snapshots ORDER BY captured DESC,rowid DESC LIMIT 100",
            )
            .map_err(err)?;
        let rows: Result<Vec<Value>, String> = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(err)?
            .map(|s| serde_json::from_str(&s.map_err(err)?).map_err(err))
            .collect();
        let mut watches = db
            .prepare("SELECT payload FROM trend_watches ORDER BY id")
            .map_err(err)?;
        let watches: Result<Vec<Value>, String> = watches
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(err)?
            .map(|s| serde_json::from_str(&s.map_err(err)?).map_err(err))
            .collect();
        Ok(
            json!({"snapshots":rows?,"watches":watches?,"simulation":{"state":"SIMULATED","execution":"UNAVAILABLE_NO_SIMULATION_RUNTIME","observed_contribution":0},"platforms_declared":["HACKER_NEWS","BLUESKY","JSON_FEED"],"donor_provenance":"research/commerce/social-capability-graph.json","license_review":"research/commerce/social-license-review.json"}),
        )
    }
    pub fn trend_compare(&self, args: Value) -> Result<Value, String> {
        let before =
            self.trend_inspect(json!({"snapshot_id":required(&args,"before_snapshot_id")?}))?;
        let after =
            self.trend_inspect(json!({"snapshot_id":required(&args,"after_snapshot_id")?}))?;
        if before["capture_mode"] != after["capture_mode"]
            || before["query"] != after["query"]
            || before["window_seconds"] != after["window_seconds"]
        {
            return Err("INCOMPATIBLE_TREND_SNAPSHOT_SCOPE".into());
        }
        Ok(
            json!({"before":before,"after":after,"mention_delta":after["mention_count"].as_i64().unwrap_or(0)-before["mention_count"].as_i64().unwrap_or(0),"state":"DERIVED","method":"CAPTURED_SAMPLE_WINDOW_DIFFERENCE_NOT_PLATFORM_TOTAL","simulation_contribution":0}),
        )
    }
    pub fn trend_discover(&self, args: Value) -> Result<Value, String> {
        let query = required(&args, "query")?.trim().to_owned();
        if query.len() > 500 || terms(&query).is_empty() {
            return Err("INVALID_SOCIAL_QUERY".into());
        }
        let sources = args["sources"].as_array().ok_or("sources array required")?;
        if sources.is_empty() || sources.len() > 5 {
            return Err("ONE_TO_FIVE_SOCIAL_SOURCES_REQUIRED".into());
        }
        let window = args["window_seconds"].as_u64().unwrap_or(86400);
        if !(60..=2592000).contains(&window) {
            return Err("INVALID_SOCIAL_WINDOW".into());
        }
        let fixture = sources.iter().any(|s| s.get("fixture_raw").is_some());
        if fixture && sources.iter().any(|s| s.get("fixture_raw").is_none()) {
            return Err("MIXED_FIXTURE_LIVE_SOURCES_DENIED".into());
        }
        if !fixture && args.get("fixture_now").is_some() {
            return Err("LIVE_CLOCK_OVERRIDE_DENIED".into());
        }
        let now = if fixture {
            args["fixture_now"].as_u64().unwrap_or(timestamp())
        } else {
            timestamp()
        };
        let mode = if fixture {
            "FIXTURE"
        } else if args["cache_only"] == true {
            "CACHED"
        } else {
            "LIVE"
        };
        let request_budget = args["request_budget"].as_u64().unwrap_or(10).min(20) as usize;
        let proposals:Vec<_>=sources.iter().enumerate().map(|(i,s)|json!({"candidate_id":query,"action":"PUBLIC_SOCIAL_QUERY","provider":"native-social","class":"PUBLIC","target_unknown":"SOCIAL_TOPIC_EVIDENCE","evidence_gap":format!("{}:{}",query,s["platform"]),"source_group":s["platform"],"url":format!("source-{i}"),"expected_cost_minor":0,"expected_requests":if fixture||mode=="CACHED"{0}else{2},"expected_latency_ms":if fixture{1}else{1000},"uncertainty_reduction_points":10})).collect();
        let allocation = allocate_information_actions(&proposals, 0, request_budget);
        let selected: BTreeSet<_> = allocation["selected_actions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|a| a["url"].as_str())
            .collect();
        let mut captured = vec![];
        let mut failures = vec![];
        let mut requests = 0u64;
        let mut hits = 0u64;
        let run_id = Uuid::new_v4().to_string();
        for (i, source) in sources.iter().enumerate() {
            if !selected.contains(format!("source-{i}").as_str()) {
                continue;
            }
            if !matches!(
                source["platform"].as_str(),
                Some("HACKER_NEWS" | "BLUESKY" | "JSON_FEED")
            ) {
                failures.push(json!({"platform":source["platform"],"state":"SOURCE_UNAVAILABLE","reason":"No native allowed connector; donor platform availability is not permission"}));
                continue;
            }
            let key = format!(
                "{:x}",
                Sha256::digest(
                    json!({"query":query,"source":source})
                        .to_string()
                        .as_bytes()
                )
            );
            if mode == "CACHED" {
                let db = self.db.lock().map_err(err)?;
                let row: Result<(u64, String), _> = db.query_row(
                    "SELECT created,payload FROM social_cache WHERE key=?1",
                    [&key],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                );
                if let Ok((created, payload)) = row
                    && now.saturating_sub(created) <= 3600
                {
                    let mut posts: Vec<SocialPost> = serde_json::from_str(&payload).map_err(err)?;
                    for p in &mut posts {
                        p.capture_mode = "CACHED".into();
                        p.origin_evidence_id = Some(p.evidence_id.clone());
                        p.evidence_id = Uuid::new_v4().to_string();
                    }
                    captured.extend(posts);
                    hits += 1;
                    continue;
                }
                failures.push(json!({"platform":source["platform"],"state":"SOURCE_UNAVAILABLE","reason":"CACHE_MISS_OR_EXPIRED_NO_NETWORK"}));
                continue;
            }
            let Some(provider) = self.providers.iter().find(|p| p.id() == "native-social") else {
                failures.push(json!({"platform":source["platform"],"state":"SOURCE_UNAVAILABLE","reason":"NATIVE_SOCIAL_PROVIDER_NOT_CONFIGURED"}));
                continue;
            };
            if provider.metadata()["class"] != "PUBLIC" || provider.metadata()["cost_minor"] != 0 {
                failures.push(json!({"platform":source["platform"],"state":"SOURCE_BLOCKED","reason":"PAID_OR_UNKNOWN_PROVIDER_COST_DENIED_BEFORE_IO"}));
                continue;
            }
            let mut payload = source.clone();
            payload["query"] = json!(query);
            payload["captured_at"] = json!(now);
            let acquired = provider.acquire(&AcquireRequest {
                run_id: run_id.clone(),
                capability: "social.query".into(),
                market: "PUBLIC_SOCIAL".into(),
                query: payload,
            });
            match acquired {
                Ok(acquired) => {
                    requests += acquired.provider_cost["request_count"]
                        .as_u64()
                        .unwrap_or(0);
                    let posts: Vec<SocialPost> =
                        serde_json::from_value(acquired.result["posts"].clone()).map_err(err)?;
                    for p in &posts {
                        p.validate()?;
                        if p.capture_mode != mode {
                            return Err("PROVIDER_CAPTURE_MODE_MISMATCH".into());
                        }
                    }
                    let raw_hash = format!("{:x}", Sha256::digest(&acquired.raw_payload));
                    if posts.iter().any(|p| p.raw_hash != raw_hash) {
                        return Err("SOCIAL_CAPTURE_HASH_MISMATCH".into());
                    }
                    let rawdir = self
                        .root
                        .join(".ynventa/materialized/runtime/social-captures");
                    fs::create_dir_all(&rawdir).map_err(err)?;
                    fs::write(
                        rawdir.join(format!("{raw_hash}.raw")),
                        &acquired.raw_payload,
                    )
                    .map_err(err)?;
                    if !fixture {
                        self.db
                            .lock()
                            .map_err(err)?
                            .execute(
                                "INSERT OR REPLACE INTO social_cache VALUES(?1,?2,?3)",
                                params![key, now, serde_json::to_string(&posts).map_err(err)?],
                            )
                            .map_err(err)?;
                    }
                    captured.extend(posts);
                }
                Err(e) => {
                    requests += e.request_count.unwrap_or(0);
                    let state = match e.http_status {
                        Some(401) => "AUTH_REQUIRED",
                        Some(429) => "RATE_LIMITED",
                        Some(403) => "SOURCE_BLOCKED",
                        _ => "SOURCE_UNAVAILABLE",
                    };
                    failures.push(json!({"platform":source["platform"],"state":state,"reason":e.reason,"http_status":e.http_status,"request_count":e.request_count,"retry_not_before_ms":e.retry_not_before_ms}));
                }
            }
        }
        let mut scope: Vec<String> = sources
            .iter()
            .map(|s| json!({"platform":s["platform"],"url":s["url"]}).to_string())
            .collect();
        scope.sort();
        scope.dedup();
        let source_scope = json!(scope);
        let acquisition_complete =
            failures.is_empty() && allocation["skipped"].as_array().is_some_and(Vec::is_empty);
        let mut history = vec![];
        {
            let db = self.db.lock().map_err(err)?;
            let mut stmt=db.prepare("SELECT payload FROM trend_snapshots WHERE query=?1 AND mode=?2 AND window=?3 AND captured<?4 ORDER BY captured DESC,rowid DESC LIMIT 30").map_err(err)?;
            for row in stmt
                .query_map(params![query, mode, window, now], |r| r.get::<_, String>(0))
                .map_err(err)?
            {
                let previous = serde_json::from_str::<Value>(&row.map_err(err)?).map_err(err)?;
                if acquisition_complete
                    && previous["source_complete"] == true
                    && previous["source_scope"] == source_scope
                {
                    history.push(previous);
                }
            }
        }
        history.reverse();
        let mut posts = self.social_rows(mode)?;
        posts.extend(captured.clone());
        let mut snap = snapshot(&posts, &history, &query, now, window, mode);
        let snapshot_id = Uuid::new_v4().to_string();
        snap["source_scope"] = source_scope;
        snap["snapshot_id"] = json!(snapshot_id);
        snap["run_id"] = json!(run_id);
        snap["provider_failures"] = json!(failures);
        snap["budget_usage"] = json!({"cost_minor":0,"request_count":if failures.iter().any(|f|f.get("request_count").is_some_and(Value::is_null)){Value::Null}else{json!(requests)},"known_request_count":requests,"request_budget":request_budget,"cache_hits":hits,"paid_budget_minor":0,"paid_execution":"NOT_IMPLEMENTED_PAID_PROPOSALS_ONLY","allocation":allocation});
        snap["source_complete"] = json!(
            snap["provider_failures"]
                .as_array()
                .is_some_and(Vec::is_empty)
                && allocation["skipped"].as_array().is_some_and(Vec::is_empty)
        );
        snap["captured_posts"] = json!(
            super::deduplicate(&posts)
                .0
                .into_iter()
                .filter(|p| snap["post_keys"]
                    .as_array()
                    .is_some_and(|keys| keys.iter().any(|k| k == &p.key())))
                .collect::<Vec<_>>()
        );
        let candidates = self.candidates()?;
        let qterms = terms(&query);
        let mut links = vec![];
        for c in candidates.as_array().into_iter().flatten() {
            let product_text = ["title", "brand", "category"]
                .iter()
                .filter_map(|key| c["product"][key].as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let cterms = terms(&product_text);
            if qterms.intersection(&cterms).count() > 0 {
                links.push(json!({"candidate_id":c["id"],"identity_state":"DERIVED_WEAK_MATCH","relation":"TOPIC_CATEGORY_RESEARCH_HINT","evidence_ids":snap["evidence_ids"],"commercial_state":c["state"],"shortlist_permitted_by_social":false,"next":"Use existing commerce research and shortlist evidence policy"}));
            }
        }
        snap["commerce_links"] = json!(links);
        snap["population_complete"] = json!(false);
        let run=self.persist(json!({"acquisition_run_id":run_id,"mode":mode,"run_kind":"SOCIAL_TREND","social_run":true,"status":if snap["source_complete"]==true{"COMPLETE"}else{"PARTIAL"},"snapshot_id":snapshot_id,"observations":captured.iter().map(|p|json!({"id":p.evidence_id,"mode":mode,"source_type":"PUBLIC_SOCIAL_JSON","provider":p.provider,"external_source":p.source_url,"market":"PUBLIC_SOCIAL","query":query,"timestamp":p.published_at,"retrieved_at":p.captured_at,"raw_hash":p.raw_hash,"raw_locator":p.raw_locator,"normalized_value":p,"state":"OBSERVED","run_id":run_id})).collect::<Vec<_>>(),"cost_minor":0,"provider_failures":snap["provider_failures"],"budget_usage":snap["budget_usage"]}))?;
        snap["run_id"] = run["run_id"].clone();
        {
            let mut db = self.db.lock().map_err(err)?;
            let tx = db.transaction().map_err(err)?;
            for p in captured {
                tx.execute("INSERT INTO social_posts VALUES(?1,?2,?3) ON CONFLICT(mode,key) DO UPDATE SET payload=excluded.payload",params![mode,p.key(),serde_json::to_string(&p).map_err(err)?]).map_err(err)?;
            }
            tx.execute(
                "INSERT INTO trend_snapshots VALUES(?1,?2,?3,?4,?5,?6)",
                params![snapshot_id, query, mode, window, now, snap.to_string()],
            )
            .map_err(err)?;
            for id in snap["evidence_ids"].as_array().into_iter().flatten() {
                tx.execute("INSERT INTO edges(payload) VALUES(?1)",[json!({"from":snapshot_id,"to":id,"relation":"SOCIAL_SUPPORTED_BY","mode":mode}).to_string()]).map_err(err)?;
            }
            tx.execute("INSERT INTO entities VALUES(?1,?2)",params![snapshot_id,json!({"id":snapshot_id,"kind":"TREND_CANDIDATE","attributes":snap,"evidence_ids":snap["evidence_ids"]}).to_string()]).map_err(err)?;
            tx.commit().map_err(err)?;
        }
        Ok(snap)
    }
    /// Explicit type and separate table: no conversion from scenario to observed rows.
    pub fn store_social_scenario(&self, scenario: ForecastScenario) -> Result<(), String> {
        scenario.validate()?;
        self.trend_inspect(json!({"snapshot_id":scenario.seed_snapshot_id}))?;
        self.db
            .lock()
            .map_err(err)?
            .execute(
                "INSERT INTO social_scenarios VALUES(?1,?2)",
                params![scenario.id, serde_json::to_string(&scenario).map_err(err)?],
            )
            .map_err(err)?;
        Ok(())
    }
    pub fn trend_watch(&self, args: Value) -> Result<Value, String> {
        if args["action"] == "list" {
            return Ok(self.trend_inspect(json!({}))?["watches"].clone());
        }
        let query = required(&args, "query")?;
        if query.len() > 500 || terms(query).is_empty() {
            return Err("INVALID_WATCH_QUERY".into());
        }
        let interval = args["interval_seconds"].as_u64().unwrap_or(3600);
        if !(60..=604800).contains(&interval) {
            return Err("INVALID_WATCH_INTERVAL".into());
        }
        let triggers = args["triggers"].as_array().ok_or("triggers required")?;
        if triggers.is_empty()
            || triggers
                .iter()
                .any(|t| !TRIGGERS.contains(&t.as_str().unwrap_or("")))
        {
            return Err("INVALID_TREND_TRIGGER".into());
        }
        let research = &args["research"];
        if research["sources"]
            .as_array()
            .is_none_or(|s| s.is_empty() || s.len() > 5)
            || research["sources"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|s| s.get("fixture_raw").is_some())
        {
            return Err("WATCH_REQUIRES_PUBLIC_SOURCE_CONFIGURATION".into());
        }
        let id = args["watch_id"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let mut db = self.db.lock().map_err(err)?;
        let tx = db.transaction().map_err(err)?;
        let old: Option<String> = tx
            .query_row(
                "SELECT payload FROM trend_watches WHERE id=?1",
                [&id],
                |r| r.get(0),
            )
            .ok();
        let old: Value = old
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(json!({}));
        let same = old["query"] == query
            && old["research"] == *research
            && old["triggers"] == args["triggers"];
        let value = json!({"watch_id":id,"query":query,"research":research,"triggers":triggers,"interval_seconds":interval,"minimum_mentions":args["minimum_mentions"].as_u64().unwrap_or(3).max(2),"threshold":args["threshold"].as_f64().unwrap_or(1.).max(0.),"enabled":args["enabled"].as_bool().unwrap_or(true),"baseline":if same{old["baseline"].clone()}else{Value::Null},"last_snapshot":if same{old["last_snapshot"].clone()}else{Value::Null},"events":if same{old["events"].clone()}else{json!([])},"lease_state":"IDLE","notification_policy":"PERSIST_LOCAL_EVENTS_ONLY_MINIMUM_SAMPLE_AND_COMPLETE_ACQUISITION"});
        tx.execute("INSERT INTO trend_watches VALUES(?1,?2,?3,0,NULL,?4) ON CONFLICT(id) DO UPDATE SET enabled=excluded.enabled,next_due=excluded.next_due,lease_until=0,lease_token=NULL,payload=excluded.payload",params![id,value["enabled"].as_bool().unwrap(),timestamp()+interval,value.to_string()]).map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(value)
    }
    pub fn trend_watch_tick(&self, now: u64) -> Result<Value, String> {
        let lease = Uuid::new_v4().to_string();
        let claimed: Option<(String, Value)> = {
            let mut db = self.db.lock().map_err(err)?;
            let tx = db
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(err)?;
            let row:Option<(String,String)>=tx.query_row("SELECT id,payload FROM trend_watches WHERE enabled=1 AND next_due<=?1 AND lease_until<=?1 ORDER BY next_due LIMIT 1",[now],|r|Ok((r.get(0)?,r.get(1)?))).ok();
            if let Some((id, payload)) = row {
                tx.execute(
                    "UPDATE trend_watches SET lease_until=?1,lease_token=?2 WHERE id=?3",
                    params![now + 300, lease, id],
                )
                .map_err(err)?;
                tx.commit().map_err(err)?;
                Some((id, serde_json::from_str(&payload).map_err(err)?))
            } else {
                None
            }
        };
        let Some((id, mut watch)) = claimed else {
            return Ok(json!({"status":"NO_DUE_TREND_WATCH"}));
        };
        let mut research = watch["research"].clone();
        research["query"] = watch["query"].clone();
        let result = self.trend_discover(research);
        match result {
            Ok(snap) => {
                let events = watch_events(&watch["baseline"], &snap, &watch);
                watch["events"] = events;
                if snap["source_complete"] == true {
                    watch["baseline"] = snap.clone();
                }
                watch["last_snapshot"] = snap.clone();
                watch["run_id"] = snap["run_id"].clone();
                watch["error"] = Value::Null;
            }
            Err(e) => {
                watch["error"] = json!(e);
                watch["events"] = json!([]);
            }
        }
        watch["lease_state"] = json!("IDLE");
        let changed=self.db.lock().map_err(err)?.execute("UPDATE trend_watches SET payload=?1,next_due=?2,lease_until=0,lease_token=NULL WHERE id=?3 AND lease_token=?4",params![watch.to_string(),now+watch["interval_seconds"].as_u64().unwrap_or(3600),id,lease]).map_err(err)?;
        if changed != 1 {
            return Err("STALE_TREND_WATCH_LEASE".into());
        }
        Ok(watch)
    }
}

pub fn watch_events(before: &Value, after: &Value, policy: &Value) -> Value {
    if !before.is_object()
        || after["source_complete"] != true
        || before["capture_mode"] != after["capture_mode"]
        || before["query"] != after["query"]
        || before["window_seconds"] != after["window_seconds"]
    {
        return json!([]);
    }
    let n = after["mention_count"].as_u64().unwrap_or(0);
    let old = before["mention_count"].as_u64().unwrap_or(0);
    let min = policy["minimum_mentions"].as_u64().unwrap_or(3);
    let delta = policy["threshold"].as_f64().unwrap_or(1.);
    let mut out = vec![];
    for t in policy["triggers"].as_array().into_iter().flatten() {
        let triggered = match t.as_str().unwrap_or("") {
            "TOPIC_MENTION_GROWTH" => n >= min && n as f64 - old as f64 >= delta,
            "CROSS_PLATFORM_APPEARANCE" => {
                n >= min && after["platform_count"].as_u64() > before["platform_count"].as_u64()
            }
            "NEW_SOURCE_APPEARANCE" => {
                n >= min && after["unique_sources"].as_u64() > before["unique_sources"].as_u64()
            }
            "VELOCITY_THRESHOLD" => {
                n >= min
                    && after["velocity"]["value"]
                        .as_f64()
                        .is_some_and(|v| v >= delta)
                    && before["velocity"]["value"]
                        .as_f64()
                        .is_some_and(|v| v < delta)
            }
            "ACCELERATION_THRESHOLD" => {
                n >= min
                    && after["acceleration"]["value"]
                        .as_f64()
                        .is_some_and(|v| v >= delta)
                    && before["acceleration"]["value"]
                        .as_f64()
                        .is_some_and(|v| v < delta)
            }
            "TREND_DISAPPEARANCE" => {
                before["population_complete"] == true
                    && after["population_complete"] == true
                    && old >= min
                    && n == 0
                    && after["captured_at"]
                        .as_u64()
                        .zip(before["captured_at"].as_u64())
                        .is_some_and(|(a, b)| {
                            a.saturating_sub(b) >= after["window_seconds"].as_u64().unwrap_or(86400)
                        })
            }
            "ENTITY_GROWTH" => {
                n >= min
                    && after["entity_links"].as_array().map(Vec::len)
                        > before["entity_links"].as_array().map(Vec::len)
            }
            "SENTIMENT_REGIME_CHANGE" => {
                n >= min
                    && sentiment_net(after)
                        .zip(sentiment_net(before))
                        .is_some_and(|(a, b)| (a - b).abs() >= delta)
            }
            "ENGAGEMENT_SPIKE" => {
                n >= min && compatible_engagement_delta(before, after).is_some_and(|v| v >= delta)
            }
            _ => false,
        };
        if triggered {
            out.push(json!({"trigger":t,"state":"DERIVED","snapshot_id":after["snapshot_id"],"run_id":after["run_id"],"evidence_ids":after["evidence_ids"],"baseline_snapshot_id":before["snapshot_id"],"scope":"CAPTURED_SAMPLE_ONLY"}));
        }
    }
    json!(out)
}
fn sentiment_net(snapshot: &Value) -> Option<f64> {
    let rows = snapshot["sentiment"].as_array()?;
    let mut count = 0.;
    let mut total = 0.;
    for r in rows {
        match r["label"].as_str() {
            Some("POSITIVE") => {
                total += 1.;
                count += 1.;
            }
            Some("NEGATIVE") => {
                total -= 1.;
                count += 1.;
            }
            Some("NEUTRAL" | "MIXED") => count += 1.,
            _ => {}
        }
    }
    (count >= 3.).then(|| total / count)
}
pub(super) fn compatible_engagement_delta(before: &Value, after: &Value) -> Option<f64> {
    let b = before["engagement_observations"].as_array()?;
    let a = after["engagement_observations"].as_array()?;
    let mut max: Option<f64> = None;
    for post in a {
        if let Some(old) = b
            .iter()
            .find(|p| p["post_key"] == post["post_key"] && p["platform"] == post["platform"])
        {
            for key in ["views", "likes", "comments", "reposts", "favorites"] {
                if let Some((x, y)) = old["metrics"][key]
                    .as_f64()
                    .zip(post["metrics"][key].as_f64())
                {
                    let d = y - x;
                    max = Some(max.unwrap_or(d).max(d));
                }
            }
        }
    }
    max
}
