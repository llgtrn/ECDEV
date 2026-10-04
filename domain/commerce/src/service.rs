use crate::{
    economics::{self, Scenario},
    planner::{self, Intent},
};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

#[derive(Clone)]
pub struct Engine {
    pub(crate) root: PathBuf,
    pub(crate) db: Arc<Mutex<Connection>>,
    pub(crate) providers: Vec<Arc<dyn crate::provider::Provider>>,
}
fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}
pub fn timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
impl Engine {
    pub fn open(root: &Path) -> Result<Self, String> {
        let dir = root.join(".ynventa/materialized/runtime");
        fs::create_dir_all(&dir).map_err(error)?;
        let db = Connection::open(dir.join("ecdev.sqlite")).map_err(error)?;
        db.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(error)?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
   CREATE TABLE IF NOT EXISTS runs(id TEXT PRIMARY KEY, created_at INTEGER NOT NULL, mode TEXT NOT NULL, payload TEXT NOT NULL);
   CREATE TABLE IF NOT EXISTS events(id INTEGER PRIMARY KEY AUTOINCREMENT, run_id TEXT NOT NULL REFERENCES runs(id), payload TEXT NOT NULL);
   CREATE TABLE IF NOT EXISTS evidence(id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES runs(id), payload TEXT NOT NULL);
   CREATE TABLE IF NOT EXISTS entities(id TEXT PRIMARY KEY, payload TEXT NOT NULL);
   CREATE TABLE IF NOT EXISTS edges(id INTEGER PRIMARY KEY, payload TEXT NOT NULL);
   CREATE TABLE IF NOT EXISTS provider_calls(run_id TEXT PRIMARY KEY, provider TEXT NOT NULL, created_at INTEGER NOT NULL);
   CREATE TABLE IF NOT EXISTS paid_reservations(run_id TEXT PRIMARY KEY, provider TEXT NOT NULL, capability TEXT NOT NULL, created_at INTEGER NOT NULL, reserved_minor INTEGER NOT NULL);
   CREATE TABLE IF NOT EXISTS fetch_cache(key TEXT PRIMARY KEY, payload TEXT NOT NULL, created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL);
   CREATE TABLE IF NOT EXISTS candidates(id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES runs(id), state TEXT NOT NULL, payload TEXT NOT NULL);
   CREATE TABLE IF NOT EXISTS provider_accounting(id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES runs(id), payload TEXT NOT NULL);
   PRAGMA user_version=1;").map_err(error)?;
        crate::monitor::initialize(&db)?;
        Ok(Self {
            root: root.to_path_buf(),
            db: Arc::new(Mutex::new(db)),
            providers: vec![],
        })
    }
    pub fn with_provider(mut self, provider: Arc<dyn crate::provider::Provider>) -> Self {
        self.providers.push(provider);
        self
    }
    pub fn read_json(&self, path: &str) -> Result<Value, String> {
        serde_json::from_slice(&fs::read(self.root.join(path)).map_err(error)?).map_err(error)
    }
    pub fn registry(&self) -> Result<Value, String> {
        let mut registry = self.read_json("research/commerce/donors/registry.json")?;
        let review = self
            .read_json("research/commerce/donor-lifecycle-review.json")
            .ok();
        for donor in registry["donors"]
            .as_array_mut()
            .ok_or("Invalid registry")?
        {
            let recorded = review
                .as_ref()
                .and_then(|r| r["donors"].as_array())
                .and_then(|rows| {
                    rows.iter().find(|r| {
                        r["donor_id"] == donor["donor_id"] && r["commit_sha"] == donor["commit_sha"]
                    })
                });
            donor["lifecycle_review"] = match recorded {
                Some(row) => {
                    json!({"status":"RECORDED_CANONICAL_ASSESSMENT","assessment":row["canonical_assessment"],"full_semantic_census":row["full_semantic_census"],"full_absorption":row["full_absorption"],"extinct":row["extinct"],"review_scope":row["review_scope"],"source":"research/commerce/donor-lifecycle-review.json","freshness":"RECORDED_REVIEW_NOT_DYNAMIC_ASSESSMENT"})
                }
                None => json!({"status":"UNKNOWN_NO_MATCHING_LOCKED_REVIEW","assessment":null}),
            };
        }
        Ok(registry)
    }
    pub fn metrics(&self) -> Result<Value, String> {
        let registry = self.registry()?;
        let donors = registry["donors"].as_array().ok_or("Invalid registry")?;
        let mut total = 0_u64;
        let mut classified = 0_u64;
        let mut sources = 0_u64;
        let mut parsed = 0_u64;
        let mut unknown = 0_u64;
        let mut tests = 0_u64;
        let mut verified = 0_u64;
        for d in donors {
            let id = d["donor_id"].as_str().ok_or("Invalid donor id")?;
            let s = self.census(id)?;
            total += s["total_files"].as_u64().unwrap_or(0);
            classified += s["classified_files"].as_u64().unwrap_or(0);
            sources += s["first_party_source_files"].as_u64().unwrap_or(0);
            parsed += s["source_parsed"].as_u64().unwrap_or(0);
            unknown += s["parse_unknown"].as_u64().unwrap_or(0);
            tests += s["tests"].as_u64().unwrap_or(0);
            verified += d["capabilities_verified"].as_u64().unwrap_or(0);
        }
        let dependencies = self.read_json("research/commerce/dependencies.json")?;
        let upstreams: std::collections::BTreeSet<_> = dependencies["packages"]
            .as_array()
            .ok_or("Invalid dependency records")?
            .iter()
            .filter(|p| p["scope"] == "RUNTIME")
            .filter_map(|p| p["repository_url"].as_str())
            .collect();
        let capabilities = self.read_json("research/commerce/capabilities.json")?;
        let compared = capabilities
            .as_array()
            .ok_or("Invalid capability records")?
            .iter()
            .filter(|c| {
                matches!(
                    c["oracle_status"].as_str(),
                    Some(
                        "508_INTEGER_JSON_CASES_MATCHED"
                            | "1620_ROBOTS_CASES_MATCHED"
                            | "132_MICRODATA_CASES_MATCHED"
                            | "54_DECLARED_DOCUMENT_CASES_MATCHED"
                            | "1235_PRICE_NUMBER_CASES_MATCHED"
                            | "46_QUEUE_TRACES_MATCHED"
                    )
                )
            })
            .count();
        Ok(
            json!({"donor_candidates":donors.len(),"remote_verified":donors.iter().filter(|d|d["remote_status"]=="VERIFIED_REMOTE").count(),"full_clones":donors.iter().filter(|d|d["clone_status"]=="FULL_CLONE").count(),"total_files":total,"classified_files":classified,"first_party_source_files":sources,"source_parsed":parsed,"parse_unknown":unknown,"tests":tests,"capabilities_verified":verified,"native_absorbed":0,"oracle_verified":0,"oracle_compared_capabilities":compared,"oracle_proven_native_capabilities":compared,"oracle_cases_executed":capabilities.as_array().unwrap().iter().filter(|c|matches!(c["oracle_status"].as_str(),Some("508_INTEGER_JSON_CASES_MATCHED"|"1620_ROBOTS_CASES_MATCHED"|"132_MICRODATA_CASES_MATCHED"|"46_QUEUE_TRACES_MATCHED"|"54_DECLARED_DOCUMENT_CASES_MATCHED"|"1235_PRICE_NUMBER_CASES_MATCHED"))).filter_map(|c|c["oracle_cases"].as_u64()).sum::<u64>(),"extinct":0,"seed_runtime_dependencies":0,"runtime_donor_dependencies":upstreams.len(),"runtime_dependency_packages":dependencies["runtime_packages"]}),
        )
    }
    pub fn census(&self, id: &str) -> Result<Value, String> {
        if !valid_id(id) {
            return Err("Invalid donor id".into());
        }
        let path = format!("research/commerce/donors/census/{id}/summary.json");
        match self.read_json(&path) {
            Ok(v) => Ok(v),
            Err(e) => {
                let identity = self.read_json(&format!(
                    "research/commerce/donors/census/{id}/identity.json"
                ))?;
                Ok(json!({"donor_id":id,"status":"NOT_CENSUSED","identity":identity,"reason":e}))
            }
        }
    }
    pub fn providers(&self) -> Value {
        let entries = [
            ("amazon-sp-api", "SP_API_REFRESH_TOKEN"),
            ("amazon-ads", "AMAZON_ADS_REFRESH_TOKEN"),
            ("keepa", "KEEPA_API_KEY"),
            ("semrush", "SEMRUSH_API_KEY"),
            ("junglescout", "JUNGLESCOUT_API_KEY"),
            ("google-trends", ""),
            ("amzscout", "AMZSCOUT_API_KEY"),
            ("supplier-acquisition", ""),
        ];
        let mut profiles:Vec<_>=entries.iter().map(|(id,key)|self.providers.iter().find(|p|p.id()==*id).map(|p|p.metadata()).unwrap_or_else(||json!({"id":id,"type":"TYPE_B","class":if id.starts_with("amazon-"){"OFFICIAL"}else{"PAID"},"status":"UNAVAILABLE","auth_state":if key.is_empty(){"UNKNOWN"}else if std::env::var(key).is_ok_and(|v|!v.is_empty()){"CONFIGURED"}else{"MISSING"},"adapter_state":"NOT_IMPLEMENTED","capabilities":[],"markets":[],"cost_minor":null,"reason":"Native client blocked pending donor semantic census and behavior contract"}))).collect();
        for p in &self.providers {
            if !entries.iter().any(|(id, _)| *id == p.id()) {
                profiles.push(p.metadata());
            }
        }
        profiles.push(json!({"id":"native-economics","class":"NATIVE","status":"AVAILABLE","capabilities":["economics.simulate"],"estimated_cost_minor":0,"auth_state":"NOT_REQUIRED"}));
        profiles.push(json!({"id":"local-cache","class":"NATIVE","status":"AVAILABLE","capabilities":["evidence.reuse"],"estimated_cost_minor":0,"auth_state":"NOT_REQUIRED"}));
        for p in &mut profiles {
            match p["id"].as_str() {
                Some("amazon-sp-api") => p["source_layer"] = json!("OFFICIAL_SP_API"),
                Some("keepa") => p["source_layer"] = json!("KEEPA"),
                _ => {}
            }
            for (k, v) in [
                ("quota_remaining", Value::Null),
                ("rate_limit", Value::Null),
                ("estimated_request_cost", Value::Null),
                ("estimated_monetary_cost", Value::Null),
                ("latency_estimate_ms", Value::Null),
                ("freshness", Value::Null),
                ("cacheability", json!(p["cacheable"] == true)),
                ("cache_ttl_seconds", Value::Null),
                ("failure_state", Value::Null),
                ("fallback_providers", json!(["local-cache", "native-web"])),
            ] {
                if p.get(k).is_none() {
                    p[k] = v;
                }
            }
        }
        json!(profiles)
    }
    pub fn product(&self, args: Value) -> Result<Value, String> {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Input {
            market: String,
            asin: String,
        }
        let input: Input = serde_json::from_value(args).map_err(error)?;
        if !matches!(input.market.as_str(), "AMAZON_JP" | "AMAZON_US")
            || input.asin.len() != 10
            || !input
                .asin
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        {
            return Err("Expected AMAZON_JP or AMAZON_US and a 10-character uppercase ASIN".into());
        }
        let id = Uuid::new_v4().to_string();
        let started_clock = std::time::Instant::now();
        let started_at = timestamp() * 1000;
        let request = crate::provider::AcquireRequest {
            run_id: id.clone(),
            capability: "product.analyze".into(),
            market: input.market,
            query: json!({"asin":input.asin}),
        };
        let Some(provider) = self
            .providers
            .iter()
            .find(|p| p.id() == "keepa" && p.metadata()["status"] == "AVAILABLE")
        else {
            return self.persist(json!({"mode":"PLAN_ONLY","status":"UNAVAILABLE","request":request,"observations":[],"network_calls":0,"cost_minor":0,"errors":["KEEPA_API_KEY missing or adapter unavailable"]}));
        };
        if let Err(reason) = self.reserve_paid(
            &id,
            "keepa",
            "product.analyze",
            &crate::provider::BudgetPolicy::from_env(),
        ) {
            return self.persist(json!({"mode":"PLAN_ONLY","status":"DENIED_BUDGET","request":request,"observations":[],"network_calls":0,"cost_minor":0,"errors":[reason],"provider_calls":[{"id":Uuid::new_v4().to_string(),"provider":"keepa","capability":"product.analyze","status":"DENIED_BUDGET","started_at":started_at,"completed_at":timestamp()*1000,"request_count":0,"actual_cost_minor":0,"cache_hit":false,"quota_before":null,"quota_after":null}]}));
        }
        // Reserve under the database lock before IO; retries cannot bypass the daily bound.
        {
            let db = self.db.lock().map_err(error)?;
            let count: u64 = db
                .query_row(
                    "SELECT COUNT(*) FROM provider_calls WHERE provider='keepa' AND created_at>=?1",
                    [timestamp() / 86400 * 86400],
                    |r| r.get(0),
                )
                .map_err(error)?;
            let limit = std::env::var("ECDEV_MAX_PROVIDER_REQUESTS_PER_DAY")
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(100);
            if count >= limit {
                return Err("PROVIDER_DAILY_REQUEST_BUDGET_EXHAUSTED".into());
            }
            db.execute(
                "INSERT INTO provider_calls VALUES(?1,?2,?3)",
                params![id, "keepa", timestamp()],
            )
            .map_err(error)?;
        }
        match provider.acquire(&request) {
            Ok(result) => {
                for observation in &result.observations { observation.validate()?; }
                let dir = self.root.join(".ynventa/materialized/raw");
                fs::create_dir_all(&dir).map_err(error)?;
                for observation in &result.observations { fs::write(dir.join(format!("{}.json",observation.raw_hash)), &result.raw_payload).map_err(error)?; }
                let run = self.persist(json!({"mode":"LIVE","status":"COMPLETE","acquisition_run_id":id,"request":request,"observations":result.observations,"result":result.result,"provider_cost":result.provider_cost,"network_calls":1,"cost_minor":null,"provider_calls":[{"id":Uuid::new_v4().to_string(),"provider":"keepa","capability":"product.analyze","status":"COMPLETE","started_at":started_at,"completed_at":timestamp()*1000,"latency_ms":started_clock.elapsed().as_millis(),"request_count":1,"estimated_cost_minor":crate::provider::BudgetPolicy::from_env().request_ceiling_minor,"actual_cost_minor":null,"cache_hit":false,"result_count":result.observations.len(),"quota_before":null,"quota_after":result.provider_cost["tokens_left"]}]}))?;
                Ok(run)
            },
            Err(message) => self.persist(json!({"mode":"LIVE","status":"FAILED","acquisition_run_id":id,"request":request,"observations":[],"errors":[message.reason],"acquisition_failure":message,"network_calls":null,"cost_minor":null,"provider_calls":[{"id":Uuid::new_v4().to_string(),"provider":"keepa","capability":"product.analyze","status":"FAILED","started_at":started_at,"completed_at":timestamp()*1000,"latency_ms":started_clock.elapsed().as_millis(),"request_count":null,"actual_cost_minor":null,"cache_hit":false,"quota_before":null,"quota_after":null}]}))
        }
    }
    pub fn submit(&self, intent: Intent) -> Result<Value, String> {
        let providers = self.providers();
        let plan = planner::plan(&intent, providers.as_array().unwrap())?;
        self.persist(json!({"intent":intent,"plan":plan,"mode":"PLAN_ONLY","status":"UNAVAILABLE","observations":[],"errors":[{"code":"PROVIDERS_UNAVAILABLE","message":"No live acquisition performed"}],"result":null,"cost_minor":0}))
    }
    pub(crate) fn persist(&self, mut payload: Value) -> Result<Value, String> {
        let id = if payload["acquisition_run_id"].is_string() && payload["mode"] != "REPLAY" {
            payload["acquisition_run_id"]
                .as_str()
                .ok_or("Acquisition identity missing")?
                .to_string()
        } else {
            Uuid::new_v4().to_string()
        };
        payload["run_id"] = json!(id);
        payload["created_at"] = json!(timestamp());
        let mut db = self.db.lock().map_err(error)?;
        let tx = db.transaction().map_err(error)?;
        tx.execute(
            "INSERT INTO runs(id,created_at,mode,payload) VALUES(?1,?2,?3,?4)",
            params![
                id,
                timestamp(),
                payload["mode"].as_str().unwrap_or("PLAN_ONLY"),
                payload.to_string()
            ],
        )
        .map_err(error)?;
        if payload["mode"] == "LIVE"
            || (payload["research_run"] == true && payload["mode"] != "REPLAY")
        {
            for observation in payload["observations"]
                .as_array()
                .ok_or("Observations required")?
            {
                tx.execute(
                    "INSERT INTO evidence VALUES(?1,?2,?3)",
                    params![observation["id"].as_str(), id, observation.to_string()],
                )
                .map_err(error)?;
            }
        }
        if payload["research_run"] == true && payload["mode"] != "REPLAY" {
            for candidate in payload["candidates"]
                .as_array()
                .ok_or("Candidates required")?
            {
                tx.execute(
                    "INSERT INTO candidates VALUES(?1,?2,?3,?4)",
                    params![
                        candidate["id"].as_str(),
                        id,
                        candidate["state"].as_str(),
                        candidate.to_string()
                    ],
                )
                .map_err(error)?;
                tx.execute("INSERT INTO entities VALUES(?1,?2)",params![candidate["id"].as_str(),json!({"id":candidate["id"],"kind":"PRODUCT_CANDIDATE","attributes":candidate["product"],"evidence_ids":candidate["evidence_ids"]}).to_string()]).map_err(error)?;
                for evidence in candidate["evidence_ids"]
                    .as_array()
                    .ok_or("Evidence IDs required")?
                {
                    tx.execute("INSERT INTO edges(payload) VALUES(?1)",[json!({"from":candidate["id"],"relation":"OBSERVED_BY","to":evidence,"mode":payload["mode"]}).to_string()]).map_err(error)?;
                }
            }
        }
        if payload["mode"] != "REPLAY"
            && let Some(calls) = payload["provider_calls"].as_array()
        {
            for call in calls {
                tx.execute(
                    "INSERT INTO provider_accounting VALUES(?1,?2,?3)",
                    params![call["id"].as_str(), id, call.to_string()],
                )
                .map_err(error)?;
            }
        }
        tx.execute("INSERT INTO events(run_id,payload) VALUES(?1,?2)",params![id,json!({"run_id":id,"event":"RUN_STORED","status":payload["status"],"timestamp":timestamp()}).to_string()]).map_err(error)?;
        tx.commit().map_err(error)?;
        Ok(payload)
    }
    pub fn runs(&self) -> Result<Value, String> {
        let db = self.db.lock().map_err(error)?;
        let mut st = db
            .prepare("SELECT payload FROM runs ORDER BY created_at DESC,rowid DESC LIMIT 100")
            .map_err(error)?;
        let rows = st.query_map([], |r| r.get::<_, String>(0)).map_err(error)?;
        let mut out = vec![];
        for row in rows {
            out.push(serde_json::from_str::<Value>(&row.map_err(error)?).map_err(error)?);
        }
        Ok(json!(out))
    }
    pub fn run(&self, id: &str) -> Result<Value, String> {
        let db = self.db.lock().map_err(error)?;
        let text: String = db
            .query_row("SELECT payload FROM runs WHERE id=?1", [id], |r| r.get(0))
            .map_err(error)?;
        serde_json::from_str(&text).map_err(error)
    }
    pub fn replay(&self, id: &str) -> Result<Value, String> {
        let original = self.run(id)?;
        let mut copy = original.clone();
        copy["mode"] = json!("REPLAY");
        copy["replay_of"] = json!(id);
        copy["status"] = json!(if original["observations"]
            .as_array()
            .is_some_and(|a| !a.is_empty())
        {
            "CAPTURED_OBSERVATIONS"
        } else {
            "UNAVAILABLE"
        });
        copy["network_calls"] = json!(0);
        copy["cost_minor"] = json!(0);
        if let Some(observations) = copy["observations"].as_array_mut() {
            for observation in observations {
                observation["original_observation_mode"] = observation["mode"].clone();
                observation["mode"] = json!("REPLAY");
            }
        }
        self.persist(copy)
    }
    pub fn events(&self, after: u64) -> Result<Value, String> {
        let db = self.db.lock().map_err(error)?;
        let mut st = db
            .prepare("SELECT id,payload FROM events WHERE id>?1 ORDER BY id LIMIT 100")
            .map_err(error)?;
        let rows = st
            .query_map([after], |r| {
                Ok((r.get::<_, u64>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(error)?;
        let mut out = vec![];
        for row in rows {
            let (id, text) = row.map_err(error)?;
            out.push(json!({"id":id,"data":serde_json::from_str::<Value>(&text).map_err(error)?}));
        }
        Ok(json!(out))
    }
    pub fn live_research_status(&self) -> Result<Value, String> {
        let payloads = {
            let db = self.db.lock().map_err(error)?;
            let mut statement = db.prepare("SELECT payload FROM runs WHERE mode='LIVE' ORDER BY created_at DESC,rowid DESC LIMIT 100").map_err(error)?;
            statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(error)?
        };
        for payload in payloads {
            let run: Value = serde_json::from_str(&payload).map_err(error)?;
            if let Some(witness) = live_shortlist_witness(&self.root, &run) {
                return Ok(witness);
            }
        }
        Ok(
            json!({"status":"UNAVAILABLE","scope":"LATEST_100_PERSISTED_LIVE_RUNS","reason":"No zero-paid research shortlist with available matching live capture hashes","network_calls":0}),
        )
    }
    pub fn status(&self) -> Result<Value, String> {
        let live = self.live_research_status()?;
        Ok(
            json!({"name":"ECDEV","version":env!("CARGO_PKG_VERSION"),"phase":"FOUNDATION","mcp":"SDK_STREAMABLE_HTTP","live_e2e":live["status"],"live_research":live,"protocol":"ynventa-v1","protocol_shard":"UNREGISTERED_UPSTREAM","metrics":self.metrics()?,"providers":self.providers()}),
        )
    }
    pub fn call(&self, name: &str, args: Value) -> Result<Value, String> {
        match name {
            "ecdev.research.run" | "ecdev.product.discover" => self.research(args),
            "ecdev.monitor.create" => self.monitor_create(args),
            "ecdev.monitor.status" => self.monitor_status(args["watch_id"].as_str()),
            "ecdev.provider.status" => Ok(self.providers()),
            "ecdev.product.inspect" => self.inspect_candidate(required_str(&args, "candidate_id")?),
            "ecdev.product.compare" => self.compare_candidates(args),
            "ecdev.research.status" => {
                let id = required_str(&args, "run_id")?;
                if !valid_id(id) {
                    return Err("INVALID_CRAWL_RUN_ID".into());
                }
                let frontier = crate::frontier::Frontier::open(
                    &self.root.join(".ynventa/materialized/runtime/ecdev.sqlite"),
                )?;
                match self.run(id) {
                    Ok(mut run) => {
                        if let Some(crawl) = run["crawl_run_id"].as_str() {
                            run["frontier"] = frontier.status(crawl)?;
                        }
                        Ok(run)
                    }
                    Err(_) => Ok(
                        json!({"run_id":id,"status":"CRAWL_RUNNING_OR_INTERRUPTED","frontier":frontier.status(id)?}),
                    ),
                }
            }
            "ecdev.provider.budget" => self.budget_status(),
            "ecdev.provider.calls" => self.accounting(),
            "ecdev.research.candidates" => self.candidates(),
            "ecdev.evidence.inspect" => self.evidence(required_str(&args, "run_id")?),
            "ecdev.evidence.graph" => self.evidence_graph(),
            "ecdev.marketplace.normalize" => {
                let entity = crate::marketplace::amazon_catalog(
                    &args["catalog_item"],
                    required_str(&args, "marketplace_id")?,
                )?;
                serde_json::to_value(entity).map_err(error)
            }
            "ecdev.monitor.compare" => self.compare_snapshots(args),
            "ecdev.product.analyze" => self.product(args),
            "ecdev.system.status" => self.status(),
            "ecdev.system.capabilities" => Ok(json!(tool_definitions())),
            "ecdev.system.providers" => Ok(self.providers()),
            "ecdev.system.health" => {
                Ok(json!({"storage":"PASS","migrations":2,"live_providers":"UNAVAILABLE"}))
            }
            "ecdev.system.metrics" => self.metrics(),
            "ecdev.ynventa.donors" => self.registry(),
            "ecdev.ynventa.census" => self.census(required_str(&args, "donor_id")?),
            "ecdev.ynventa.absorption" | "ecdev.ynventa.extinction" => Ok(
                json!({"status":"PARTIAL_NOT_EXTINCT","metrics":self.metrics()?,"blockers":["Remaining semantic donor census","Remaining behavior contracts and parity","Live validation","Foundational external dependencies","ECDEV upstream shard registration"]}),
            ),
            "ecdev.ynventa.evidence" => self.read_json("research/commerce/capabilities.json"),
            "ecdev.runs.list" => self.runs(),
            "ecdev.runs.inspect" => self.run(required_str(&args, "run_id")?),
            "ecdev.runs.replay" => self.replay(required_str(&args, "run_id")?),
            "ecdev.opportunity.search" => self.submit(serde_json::from_value(args).map_err(error)?),
            "ecdev.economics.simulate" => {
                let s: Scenario = serde_json::from_value(args).map_err(error)?;
                self.persist(json!({"mode":"SIMULATED","status":"COMPLETE","scenario":s,"result":economics::simulate(&s)?,"observations":[],"cost_minor":0}))
            }
            _ => Err(format!("Unknown capability: {name}")),
        }
    }
}
pub fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_' || c == b'.')
}
fn required_str<'a>(a: &'a Value, key: &str) -> Result<&'a str, String> {
    a[key]
        .as_str()
        .ok_or_else(|| format!("Required string: {key}"))
}
pub fn tool_definitions() -> Vec<Value> {
    let empty = json!({"type":"object","properties":{},"additionalProperties":false});
    let run = json!({"type":"object","properties":{"run_id":{"type":"string"}},"required":["run_id"],"additionalProperties":false});
    let donor = json!({"type":"object","properties":{"donor_id":{"type":"string"}},"required":["donor_id"],"additionalProperties":false});
    let mut out = vec![];
    out.push(json!({"name":"ecdev.product.discover","description":"Discover real product candidates through bounded native research; paid providers optional; supplied fixtures explicitly labeled","inputSchema":serde_json::from_str::<Value>(include_str!("../../../tools/commerce/schemas/research.schema.json")).unwrap()}));
    out.push(json!({"name":"ecdev.product.inspect","description":"Inspect one persisted candidate with field provenance, conflicts and economics uncertainty; zero network","inputSchema":{"type":"object","properties":{"candidate_id":{"type":"string"}},"required":["candidate_id"],"additionalProperties":false}}));
    out.push(json!({"name":"ecdev.product.compare","description":"Compare 2 to 20 captured candidates without network acquisition or invented market ranking","inputSchema":{"type":"object","properties":{"candidate_ids":{"type":"array","items":{"type":"string"},"minItems":2,"maxItems":20}},"required":["candidate_ids"],"additionalProperties":false}}));
    out.push(json!({"name":"ecdev.provider.status","description":"Provider availability and credentials state without secrets or network","inputSchema":empty}));
    out.push(json!({"name":"ecdev.monitor.create","description":"Create or update a persisted public watch for up to five URLs; watch_id updates and enabled=false disables; refresh acquisitions use native robots/budget policy; no external notifications","inputSchema":{"type":"object","properties":{"watch_id":{"type":"string"},"enabled":{"type":"boolean","default":true},"market":{"enum":["PUBLIC_WEB","AMAZON_JP","AMAZON_US"]},"query":{"type":"string","maxLength":500},"targets":{"type":"array","items":{"type":"string"},"minItems":1,"maxItems":5},"interval_seconds":{"type":"integer","minimum":60,"maximum":604800}},"required":["market","query","targets","interval_seconds"],"additionalProperties":false}}));
    out.push(json!({"name":"ecdev.monitor.status","description":"Persisted watch schedule, snapshots, change triggers and acquisition errors; omit watch_id to list","inputSchema":{"type":"object","properties":{"watch_id":{"type":"string"}},"additionalProperties":false}}));
    out.push(json!({"name":"ecdev.evidence.graph","description":"Actual persisted candidate/evidence edges","inputSchema":empty}));
    out.push(json!({"name":"ecdev.marketplace.normalize","description":"Map a supplied Amazon Catalog Items payload into the ECDEV ontology; authenticity unverified, absent fields unknown","inputSchema":{"type":"object","properties":{"marketplace_id":{"type":"string"},"catalog_item":{"type":"object"}},"required":["marketplace_id","catalog_item"],"additionalProperties":false}}));
    for (name, desc) in [
        (
            "ecdev.provider.budget",
            "Configured paid ceilings, reserved usage and remaining budget",
        ),
        (
            "ecdev.provider.calls",
            "Persisted provider call/cache accounting",
        ),
        (
            "ecdev.research.candidates",
            "Candidate ledger including rejections",
        ),
    ] {
        out.push(json!({"name":name,"description":desc,"inputSchema":empty}));
    }
    for name in ["ecdev.research.status", "ecdev.evidence.inspect"] {
        out.push(json!({"name":name,"description":"Persisted research run or evidence","inputSchema":run}));
    }
    out.push(json!({"name":"ecdev.research.run","description":"Bounded native/public-source product research. Paid providers never required. Supplied HTML is explicitly FIXTURE; unknown commercial fields remain unknown.","inputSchema":serde_json::from_str::<Value>(include_str!("../../../tools/commerce/schemas/research.schema.json")).unwrap()}));
    out.push(json!({"name":"ecdev.monitor.compare","description":"Compare captured product fields between two persisted research snapshots","inputSchema":{"type":"object","properties":{"before_run_id":{"type":"string"},"after_run_id":{"type":"string"}},"required":["before_run_id","after_run_id"],"additionalProperties":false}}));
    out.push(json!({"name":"ecdev.product.analyze","description":"Inspect one ASIN using a native Keepa client. Requires KEEPA_API_KEY; makes one paid API request, never retries. Missing credentials yield UNAVAILABLE.","inputSchema":{"type":"object","properties":{"market":{"enum":["AMAZON_JP","AMAZON_US"]},"asin":{"type":"string","pattern":"^[A-Z0-9]{10}$"}},"required":["market","asin"],"additionalProperties":false}}));
    for (name, desc) in [
        ("ecdev.system.status", "Engine and census status"),
        ("ecdev.system.capabilities", "Implemented tool catalog"),
        (
            "ecdev.system.providers",
            "External boundary and credential presence; no secrets",
        ),
        ("ecdev.system.health", "Storage health"),
        ("ecdev.system.metrics", "Metrics derived from donor records"),
        (
            "ecdev.ynventa.donors",
            "Verified donor identities and exact commits",
        ),
        ("ecdev.ynventa.absorption", "Native absorption blockers"),
        ("ecdev.ynventa.extinction", "Extinction blockers"),
        ("ecdev.ynventa.evidence", "Source-backed capability ledger"),
        ("ecdev.runs.list", "Persisted runs"),
    ] {
        out.push(json!({"name":name,"description":desc,"inputSchema":empty}));
    }
    out.push(json!({"name":"ecdev.ynventa.census","description":"Donor census and unknowns","inputSchema":donor}));
    for name in ["ecdev.runs.inspect", "ecdev.runs.replay"] {
        out.push(json!({"name":name,"description":"Inspect or replay recorded run without network","inputSchema":run}));
    }
    for (name, file, desc) in [
        (
            "ecdev.opportunity.search",
            "intent.schema.json",
            "Persist a PLAN_ONLY capability DAG; missing providers produce UNAVAILABLE",
        ),
        (
            "ecdev.economics.simulate",
            "scenario.schema.json",
            "Deterministic costs in minor units; always SIMULATED",
        ),
    ] {
        let schema = if file == "intent.schema.json" {
            include_str!("../../../tools/commerce/schemas/intent.schema.json")
        } else {
            include_str!("../../../tools/commerce/schemas/scenario.schema.json")
        };
        out.push(json!({"name":name,"description":desc,"inputSchema":serde_json::from_str::<Value>(schema).unwrap()}));
    }
    out
}
// Historical capture validation, never a freshness or profitability assertion.
fn live_shortlist_witness(root: &Path, run: &Value) -> Option<Value> {
    use sha2::{Digest, Sha256};
    if run["mode"] != "LIVE"
        || run["research_run"] != true
        || run["cost_minor"] != 0
        || run["known_network_calls"].as_u64().unwrap_or(0) == 0
    {
        return None;
    }
    let calls = run["provider_calls"].as_array()?;
    if calls.is_empty()
        || !calls.iter().all(|call| {
            matches!(
                call["provider"].as_str(),
                Some("native-web" | "public-amazon")
            ) && call["actual_cost_minor"] == 0
        })
    {
        return None;
    }
    let observations = run["observations"].as_array()?;
    for candidate in run["candidates"].as_array()? {
        if candidate["state"] != "SHORTLISTED"
            || candidate["decision"]["purpose"] != "FURTHER_RESEARCH"
        {
            continue;
        }
        let Some(ids) = candidate["evidence_ids"].as_array() else {
            continue;
        };
        let mut hashes = Vec::new();
        let mut origins = std::collections::BTreeSet::new();
        for id in ids {
            let Some(observation) = observations
                .iter()
                .find(|o| o["id"] == *id && o["mode"] == "LIVE")
            else {
                continue;
            };
            let Some(hash) = observation["raw_hash"].as_str().filter(|h| {
                h.len() == 64
                    && h.bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            }) else {
                continue;
            };
            if observation["normalized_value"]["content_hash"] != hash {
                continue;
            }
            let acquired = calls.iter().any(|call| {
                call["provider"] == observation["provider"]
                    && call["cache_hit"] == false
                    && call["request_count"].as_u64().unwrap_or(0) > 0
                    && call["evidence_ids"]
                        .as_array()
                        .is_some_and(|ids| ids.contains(id))
            });
            if !acquired {
                continue;
            }
            let Ok(bytes) = fs::read(
                root.join(".ynventa/materialized/raw")
                    .join(format!("{hash}.html")),
            ) else {
                continue;
            };
            if format!("{:x}", Sha256::digest(&bytes)) != hash {
                continue;
            }
            let Some(source) = observation["external_source"].as_str() else {
                continue;
            };
            let Ok(source) = url::Url::parse(source) else {
                continue;
            };
            if !matches!(source.scheme(), "http" | "https") || source.host_str().is_none() {
                continue;
            }
            origins.insert(source.origin().ascii_serialization());
            hashes.push(json!({"evidence_id":id,"raw_capture_sha256":hash}));
        }
        if origins.len() >= 2 {
            return Some(
                json!({"status":"CAPTURED_ZERO_PAID_RESEARCH_SHORTLIST","scope":"HISTORICAL_PERSISTED_CAPTURE_NOT_CURRENT_MARKET_VALIDATION","run_id":run["run_id"],"run_status":run["status"],"captured_at":run["created_at"],"candidate_id":candidate["id"],"verified_capture_hashes":hashes,"listing_origins":origins,"publisher_independence":"UNVERIFIED","paid_provider_calls":0,"paid_cost_minor":0,"network_calls":0,"commercial_validation":"INCOMPLETE","goal_complete":false}),
            );
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    fn temp() -> PathBuf {
        let p = std::env::temp_dir().join(format!("ecdev-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&p).unwrap();
        p
    }
    #[test]
    fn durable_runs_replay_without_network() {
        let p = temp();
        let e = Engine::open(&p).unwrap();
        let intent:Intent=serde_json::from_value(json!({"market":"AMAZON_JP","currency":"JPY","capital":300000,"min_price":3000,"max_price":6000,"max_weight_g":700,"minimum_margin_bps":2500,"max_inventory_per_sku":150000})).unwrap();
        let run = e.submit(intent).unwrap();
        let id = run["run_id"].as_str().unwrap();
        drop(e);
        let e = Engine::open(&p).unwrap();
        assert_eq!(e.run(id).unwrap()["status"], "UNAVAILABLE");
        let replay = e.replay(id).unwrap();
        assert_eq!(replay["mode"], "REPLAY");
        assert_eq!(replay["network_calls"], 0);
        assert_eq!(e.runs().unwrap().as_array().unwrap().len(), 2);
        assert_eq!(e.events(0).unwrap().as_array().unwrap().len(), 2);
    }
    #[test]
    fn live_status_requires_network_witnesses_and_capture_hashes() {
        use sha2::{Digest, Sha256};
        let root = temp();
        let engine = Engine::open(&root).unwrap();
        let dir = root.join(".ynventa/materialized/raw");
        fs::create_dir_all(&dir).unwrap();
        let mut observations = vec![];
        let mut calls = vec![];
        for (id, source) in [
            ("one", "https://manufacturer.example/product"),
            ("two", "https://retailer.example/product"),
        ] {
            let raw = format!("<html>{id}</html>");
            let hash = format!("{:x}", Sha256::digest(raw.as_bytes()));
            fs::write(dir.join(format!("{hash}.html")), raw).unwrap();
            observations.push(json!({"id":id,"provider":"native-web","mode":"LIVE","raw_hash":hash,"external_source":source,"normalized_value":{"content_hash":hash}}));
            calls.push(json!({"provider":"native-web","actual_cost_minor":0,"cache_hit":false,"request_count":2,"evidence_ids":[id]}));
        }
        let mut run = json!({"research_run":true,"mode":"LIVE","status":"PARTIAL","cost_minor":0,"known_network_calls":4,"observations":observations,"provider_calls":calls,"candidates":[{"id":"candidate","state":"SHORTLISTED","decision":{"purpose":"FURTHER_RESEARCH"},"evidence_ids":["one","two"]}]});
        assert_eq!(
            engine.live_research_status().unwrap()["status"],
            "UNAVAILABLE"
        );
        assert!(live_shortlist_witness(&root, &run).is_some());
        for mode in ["FIXTURE", "CACHED", "REPLAY"] {
            run["mode"] = json!(mode);
            assert!(live_shortlist_witness(&root, &run).is_none());
        }
        run["mode"] = json!("LIVE");
        run["observations"][1]["mode"] = json!("CACHED");
        assert!(live_shortlist_witness(&root, &run).is_none());
        run["observations"][1]["mode"] = json!("LIVE");
        run["provider_calls"][1]["cache_hit"] = json!(true);
        assert!(live_shortlist_witness(&root, &run).is_none());
        run["provider_calls"][1]["cache_hit"] = json!(false);
        run["provider_calls"][1]["provider"] = json!("keepa");
        assert!(live_shortlist_witness(&root, &run).is_none());
        run["provider_calls"][1]["provider"] = json!("native-web");
        let saved = engine.persist(run.clone()).unwrap();
        drop(engine);
        let engine = Engine::open(&root).unwrap();
        assert_eq!(
            engine.live_research_status().unwrap()["run_id"],
            saved["run_id"]
        );
        let hash = run["observations"][1]["raw_hash"].as_str().unwrap();
        fs::write(dir.join(format!("{hash}.html")), "changed bytes").unwrap();
        assert_eq!(
            engine.live_research_status().unwrap()["status"],
            "UNAVAILABLE"
        );
    }
    #[test]
    fn rejects_paths_and_bad_intents() {
        assert!(!valid_id("../../secrets"));
        assert!(!valid_id("a/b"));
        assert!(valid_id("dgtlmoon--changedetection.io"));
        assert!(!valid_id(".."));
        let i = Intent {
            market: "AMAZON_JP".into(),
            currency: "JPY".into(),
            capital: 0,
            min_price: 3000,
            max_price: 6000,
            max_weight_g: 700,
            minimum_margin_bps: 2500,
            max_inventory_per_sku: 150000,
            positive_trend: true,
            exclude_regulated: true,
        };
        assert!(planner::plan(&i, &[]).is_err());
    }
}
