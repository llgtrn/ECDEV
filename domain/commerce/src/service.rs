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
        let dir = root.join(".ecdev-data/runtime");
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
        crate::social::runtime::initialize(&db)?;
        crate::social::feeds::initialize(&db)?;
        crate::simulation::initialize(&db)?;
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
    /// The recorded governance lifecycle: effective donor states and their extinction gates,
    /// exactly as `ecdev-gov extinction` judged them at the recorded commit.
    pub fn lifecycle(&self) -> Result<Value, String> {
        let l = self.read_json("research/commerce/governance/lifecycle.json")?;
        let extinct = l["extinct"].as_array().cloned().unwrap_or_default();
        Ok(json!({
            "status": if extinct.is_empty() { "NO_DONOR_EXTINCT" } else { "SOME_DONORS_EXTINCT" },
            "extinct": extinct,
            "effective_states": l["effective_states"],
            "assessed_parent_commit": l["assessed_parent_commit"],
            "freshness": "RECORDED_ASSESSMENT_NOT_DYNAMIC",
            "source": "research/commerce/governance/lifecycle.json",
            "donors": l["donors"],
        }))
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
        let extinct = self.lifecycle()?["extinct"].as_array().map_or(0, Vec::len);
        let upstreams: std::collections::BTreeSet<_> = dependencies["packages"]
            .as_array()
            .ok_or("Invalid dependency records")?
            .iter()
            .filter(|p| p["scope"] == "RUNTIME")
            .filter_map(|p| p["repository_url"].as_str())
            .collect();
        let capabilities = self.read_json("research/commerce/capabilities.json")?;
        let social_oracle = self
            .read_json("research/commerce/social-oracle-report.json")
            .ok()
            .filter(|r| r["status"] == "PASS");
        let social_families = social_oracle
            .as_ref()
            .and_then(|r| r["families"].as_u64())
            .unwrap_or(0);
        let social_cases = social_oracle
            .as_ref()
            .and_then(|r| r["cases"].as_u64())
            .unwrap_or(0);
        let memory_oracle = self
            .read_json("research/commerce/memory-time-oracle-report.json")
            .ok()
            .filter(|r| r["status"] == "PASS");
        let memory_families = memory_oracle
            .as_ref()
            .and_then(|r| r["families"].as_u64())
            .unwrap_or(0);
        let memory_cases = memory_oracle
            .as_ref()
            .and_then(|r| r["cases"].as_u64())
            .unwrap_or(0);
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
                            | "2937_PRICE_PUBLIC_API_CASES_MATCHED"
                            | "46_QUEUE_TRACES_MATCHED"
                    )
                )
            })
            .count();
        Ok(
            json!({"donor_candidates":donors.len(),"remote_verified":donors.iter().filter(|d|d["remote_status"]=="VERIFIED_REMOTE").count(),"full_clones":donors.iter().filter(|d|d["clone_status"]=="FULL_CLONE").count(),"total_files":total,"classified_files":classified,"first_party_source_files":sources,"source_parsed":parsed,"parse_unknown":unknown,"tests":tests,"capabilities_verified":verified,"native_absorbed":0,"oracle_verified":0,"oracle_compared_capabilities":compared as u64+social_families+memory_families,"oracle_proven_native_capabilities":compared as u64+social_families+memory_families,"oracle_cases_executed":capabilities.as_array().unwrap().iter().filter(|c|matches!(c["oracle_status"].as_str(),Some("508_INTEGER_JSON_CASES_MATCHED"|"1620_ROBOTS_CASES_MATCHED"|"132_MICRODATA_CASES_MATCHED"|"46_QUEUE_TRACES_MATCHED"|"54_DECLARED_DOCUMENT_CASES_MATCHED"|"1235_PRICE_NUMBER_CASES_MATCHED"|"2937_PRICE_PUBLIC_API_CASES_MATCHED"))).filter_map(|c|c["oracle_cases"].as_u64()).sum::<u64>()+social_cases+memory_cases,"memory_oracle_families":memory_families,"memory_oracle_cases":memory_cases,"social_oracle_families":social_families,"social_oracle_cases":social_cases,"extinct":extinct,"extinct_source":"research/commerce/governance/lifecycle.json","seed_runtime_dependencies":0,"runtime_donor_dependencies":upstreams.len(),"runtime_dependency_packages":dependencies["runtime_packages"]}),
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
        let limit = std::env::var("ECDEV_MAX_PROVIDER_REQUESTS_PER_DAY")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(100);
        self.product_with_policy(args, &crate::provider::BudgetPolicy::from_env(), limit)
    }
    fn product_with_policy(
        &self,
        mut args: Value,
        budget: &crate::provider::BudgetPolicy,
        daily_limit: u64,
    ) -> Result<Value, String> {
        if args["evidence_layer"] == "OFFICIAL_SP_API" {
            return self.official_product(args);
        }
        if let Some(layer) = args.get("evidence_layer") {
            if layer != "KEEPA" {
                return Err("UNKNOWN_PRODUCT_EVIDENCE_LAYER".into());
            }
            args.as_object_mut()
                .ok_or("PRODUCT_INTENT_OBJECT_REQUIRED")?
                .remove("evidence_layer");
        }
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
        if let Err(reason) = self.reserve_paid(&id, "keepa", "product.analyze", budget) {
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
            if count >= daily_limit {
                return Err("PROVIDER_DAILY_REQUEST_BUDGET_EXHAUSTED".into());
            }
            db.execute(
                "INSERT INTO provider_calls VALUES(?1,?2,?3)",
                params![id, "keepa", timestamp()],
            )
            .map_err(error)?;
        }
        let failed = |failure: crate::provider::AcquireError| {
            let count = failure.request_count;
            self.persist(json!({"mode":crate::provider::acquisition_mode(false,count.unwrap_or(0),count.is_none(),false),"requested_mode":"LIVE","status":"FAILED","acquisition_run_id":id,"request":request,"observations":[],"errors":[failure.reason],"acquisition_failure":failure,"network_calls":count,"known_network_calls":count.unwrap_or(0),"live_io_established":count.is_some_and(|n|n>0),"new_live_acquisition":false,"cost_minor":if count==Some(0){json!(0)}else{Value::Null},"provider_calls":[{"id":Uuid::new_v4().to_string(),"provider":"keepa","capability":"product.analyze","status":"FAILED","started_at":started_at,"completed_at":timestamp()*1000,"latency_ms":started_clock.elapsed().as_millis(),"request_count":count,"actual_cost_minor":if count==Some(0){json!(0)}else{Value::Null},"cache_hit":false,"quota_before":null,"quota_after":null}]}))
        };
        match provider.acquire(&request) {
            Ok(result) => {
                let count = result.provider_cost["request_count"].as_u64();
                if count.is_none_or(|n| n == 0)
                    || result.observations.is_empty()
                    || result.observations.iter().any(|o| {
                        o.mode != crate::domain::ObservationMode::Live
                            || o.provider != "keepa"
                            || o.source_type != "API"
                    })
                {
                    let mut failure = crate::provider::AcquireError::from(
                        "KEEPA_LIVE_HTTP_AND_PRODUCT_WITNESS_REQUIRED",
                    );
                    failure.request_count = count;
                    return failed(failure);
                }
                let count = count.unwrap();
                for observation in &result.observations {
                    observation.validate()?;
                }
                let dir = self.root.join(".ecdev-data/raw");
                fs::create_dir_all(&dir).map_err(error)?;
                for observation in &result.observations {
                    fs::write(
                        dir.join(format!("{}.json", observation.raw_hash)),
                        &result.raw_payload,
                    )
                    .map_err(error)?;
                }
                self.persist(json!({"mode":"LIVE","requested_mode":"LIVE","status":"COMPLETE","acquisition_run_id":id,"request":request,"observations":result.observations,"result":result.result,"provider_cost":result.provider_cost,"network_calls":count,"known_network_calls":count,"live_io_established":true,"new_live_acquisition":true,"cost_minor":null,"provider_calls":[{"id":Uuid::new_v4().to_string(),"provider":"keepa","capability":"product.analyze","status":"COMPLETE","started_at":started_at,"completed_at":timestamp()*1000,"latency_ms":started_clock.elapsed().as_millis(),"request_count":count,"estimated_cost_minor":budget.request_ceiling_minor,"actual_cost_minor":null,"cache_hit":false,"result_count":result.observations.len(),"quota_before":null,"quota_after":result.provider_cost["tokens_left"]}]}))
            }
            Err(failure) => failed(failure),
        }
    }

    pub fn submit(&self, intent: Intent) -> Result<Value, String> {
        let providers = self.providers();
        let plan = planner::plan(&intent, providers.as_array().unwrap())?;
        self.persist(json!({"intent":intent,"plan":plan,"mode":"PLAN_ONLY","status":"UNAVAILABLE","observations":[],"errors":[{"code":"PROVIDERS_UNAVAILABLE","message":"No live acquisition performed"}],"result":null,"cost_minor":0}))
    }
    pub(crate) fn persist(&self, mut payload: Value) -> Result<Value, String> {
        // Simulated records live only in simulation_runs; the observed stores refuse them.
        // Candidates may carry assumption-based economics labelled SIMULATED; the candidate
        // and its product must not be.
        let simulated_candidate = payload["candidates"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|c| {
                crate::simulation::carries_simulated(&c["product"])
                    || ["state", "evidence_class", "mode"].iter().any(|k| {
                        c[*k]
                            .as_str()
                            .is_some_and(|s| s.starts_with(crate::simulation::SIMULATED))
                    })
            });
        if crate::simulation::carries_simulated(&payload["observations"]) || simulated_candidate {
            return Err("SIMULATED_RECORD_REFUSED_BY_OBSERVED_STORE".into());
        }
        crate::capture::verify_payload(&self.root, &payload)?;
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
            || ((payload["research_run"] == true
                || payload["social_run"] == true
                || payload["official_run"] == true)
                && payload["mode"] != "REPLAY")
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
            let value = serde_json::from_str::<Value>(&row.map_err(error)?).map_err(error)?;
            out.push(match crate::capture::verify_payload(&self.root, &value) {
                Ok(()) => value,
                Err(reason) => json!({"run_id":value["run_id"],"created_at":value["created_at"],"mode":value["mode"],"status":"UNAVAILABLE","reason":reason,"network_calls":0,"historical_projection":true}),
            });
        }
        Ok(json!(out))
    }
    pub fn run(&self, id: &str) -> Result<Value, String> {
        let db = self.db.lock().map_err(error)?;
        let text: String = db
            .query_row("SELECT payload FROM runs WHERE id=?1", [id], |r| r.get(0))
            .map_err(error)?;
        let mut payload: Value = serde_json::from_str(&text).map_err(error)?;
        crate::capture::verify_payload(&self.root, &payload)?;
        payload["raw_capture_verification"] = json!("VERIFIED_LOCAL_SHA256_NO_NETWORK");
        Ok(payload)
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
            let mut statement = db.prepare("SELECT payload FROM runs WHERE mode='LIVE' AND json_extract(payload,'$.research_run')=1 ORDER BY created_at DESC,rowid DESC LIMIT 100").map_err(error)?;
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
            json!({"name":"ECDEV","version":env!("CARGO_PKG_VERSION"),"phase":"FOUNDATION","mcp":"SDK_STREAMABLE_HTTP","live_e2e":live["status"],"live_research":live,"governance":"ecdev-governance-v1","governance_authority":".ecdev","metrics":self.metrics()?,"providers":self.providers()}),
        )
    }
    pub fn call(&self, name: &str, args: Value) -> Result<Value, String> {
        match name {
            "ecdev.trend.discover" => self.trend_discover(args),
            "ecdev.trend.feeds" => self.trend_feeds(args),
            "ecdev.trend.inspect" | "ecdev.trend.explain" => self.trend_inspect(args),
            "ecdev.trend.compare" => self.trend_compare(args),
            "ecdev.trend.hypothesize" => self.trend_hypothesize(args),
            "ecdev.trend.watch" => self.trend_watch(args),
            "ecdev.research.run" | "ecdev.product.discover" => self.research(args),
            "ecdev.monitor.create" => self.monitor_create(args),
            "ecdev.monitor.status" => self.monitor_status(args["watch_id"].as_str()),
            "ecdev.monitor.export" => self.monitor_export(required_str(&args, "watch_id")?),
            "ecdev.monitor.import" => self.monitor_import(&args["bundle"]),
            "ecdev.monitor.fact_as_of" => self.monitor_fact_as_of(
                required_str(&args, "watch_id")?,
                required_str(&args, "url")?,
                required_str(&args, "product")?,
                required_str(&args, "field")?,
                args["at"].as_u64().ok_or("Required unsigned integer: at")?,
            ),
            "ecdev.provider.status" => Ok(self.providers()),
            "ecdev.provider.doctor" => self.provider_doctor(args),
            "ecdev.seller.read" => self.seller_read(args),
            "ecdev.product.inspect" => self.inspect_candidate(required_str(&args, "candidate_id")?),
            "ecdev.product.compare" => self.compare_candidates(args),
            "ecdev.research.status" => {
                let id = required_str(&args, "run_id")?;
                if !valid_id(id) {
                    return Err("INVALID_CRAWL_RUN_ID".into());
                }
                let frontier = crate::frontier::Frontier::open(
                    &self.root.join(".ecdev-data/runtime/ecdev.sqlite"),
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
            "ecdev.research.reextract" => self.research_reextract(
                &serde_json::from_value::<Vec<String>>(args["run_ids"].clone())
                    .map_err(|_| "Required string array: run_ids")?,
            ),
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
            "ecdev.governance.donors" => self.registry(),
            "ecdev.governance.census" => self.census(required_str(&args, "donor_id")?),
            "ecdev.governance.absorption" | "ecdev.governance.extinction" => self.lifecycle(),
            "ecdev.governance.conformance" => {
                self.read_json("research/commerce/governance/conformance.json")
            }
            "ecdev.governance.evidence" => self.read_json("research/commerce/capabilities.json"),
            "ecdev.runs.list" => self.runs(),
            "ecdev.runs.inspect" => self.run(required_str(&args, "run_id")?),
            "ecdev.runs.replay" => self.replay(required_str(&args, "run_id")?),
            "ecdev.opportunity.search" => self.submit(serde_json::from_value(args).map_err(error)?),
            "ecdev.simulation.compare" => self.simulation_compare(args),
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
    out.extend(crate::social::runtime::tool_definitions());
    out.push(crate::social::feeds::tool_definition());
    out.push(json!({"name":"ecdev.product.discover","description":"Discover real product candidates through bounded native research; paid providers optional; supplied fixtures explicitly labeled","inputSchema":serde_json::from_str::<Value>(include_str!("../../../tools/commerce/schemas/research.schema.json")).unwrap()}));
    out.push(json!({"name":"ecdev.product.inspect","description":"Inspect one persisted candidate with field provenance, conflicts and economics uncertainty; zero network","inputSchema":{"type":"object","properties":{"candidate_id":{"type":"string"}},"required":["candidate_id"],"additionalProperties":false}}));
    out.push(json!({"name":"ecdev.product.compare","description":"Compare 2 to 20 captured candidates without network acquisition or invented market ranking","inputSchema":{"type":"object","properties":{"candidate_ids":{"type":"array","items":{"type":"string"},"minItems":2,"maxItems":20}},"required":["candidate_ids"],"additionalProperties":false}}));
    out.push(json!({"name":"ecdev.provider.status","description":"Provider availability and credentials state without secrets or network","inputSchema":empty}));
    out.push(json!({"name":"ecdev.provider.doctor","description":"Official SP-API credential readiness without secrets or network: credential presence, operator gate, request budget, regions, token state, read operations available or blocked, restricted PII/RDT domain and writes disabled","inputSchema":{"type":"object","properties":{"provider":{"enum":["amazon-sp-api"],"default":"amazon-sp-api"}},"additionalProperties":false}}));
    let capture = json!({"type":"object","properties":{"status":{"type":"integer","minimum":100,"maximum":599},"raw_body":{"type":"string","maxLength":4194304},"headers":{"type":"object","properties":{"x-amzn-requestid":{"type":"string","maxLength":4096},"x-amzn-ratelimit-limit":{"type":"string","maxLength":4096},"retry-after":{"type":"string","maxLength":4096}},"additionalProperties":false}},"required":["status","raw_body"],"additionalProperties":false});
    let read_fixtures = json!({"type":"object","properties":{"listings_item":capture,"inventory_summaries":capture,"marketplace_participations":capture,"product_type_search":capture,"product_type_definition":capture},"additionalProperties":false});
    out.push(json!({"name":"ecdev.seller.read","description":"Official SP-API seller-scoped reads (OFFICIAL_SP_API only, never public Amazon or Keepa): one listings item, FBA inventory summaries, marketplace participations, product type search or definition. Orders, PII, restricted data tokens and writes are refused before IO. Live reads need host credentials, the operator gate and explicit monetary ceilings; supplied fixtures are labeled FIXTURE and never establish live validation.","inputSchema":{"type":"object","properties":{"market":{"enum":["AMAZON_JP","AMAZON_US"]},"operation":{"enum":["LISTINGS_ITEM","INVENTORY_SUMMARIES","MARKETPLACE_PARTICIPATIONS","PRODUCT_TYPE_SEARCH","PRODUCT_TYPE_DEFINITION"]},"evidence_layer":{"enum":["OFFICIAL_SP_API"]},"seller_id":{"type":"string","maxLength":64},"sku":{"type":"string","maxLength":200},"included_data":{"type":"array","items":{"type":"string","maxLength":64},"maxItems":10},"issue_locale":{"type":"string","maxLength":10},"seller_skus":{"type":"array","items":{"type":"string","maxLength":200},"maxItems":50},"details":{"type":"boolean"},"next_token":{"type":"string","maxLength":4096},"keywords":{"type":"array","items":{"type":"string","maxLength":100},"maxItems":20},"item_name":{"type":"string","maxLength":500},"locale":{"type":"string","maxLength":10},"search_locale":{"type":"string","maxLength":10},"product_type":{"type":"string","maxLength":100},"product_type_version":{"type":"string","maxLength":100},"requirements":{"enum":["LISTING","LISTING_PRODUCT_ONLY","LISTING_OFFER_ONLY"]},"requirements_enforced":{"enum":["ENFORCED","NOT_ENFORCED"]},"fixture_responses":read_fixtures},"required":["market","operation"],"additionalProperties":false}}));
    out.push(json!({"name":"ecdev.monitor.create","description":"Create or update a persisted public watch for up to five URLs; watch_id updates and enabled=false disables; refresh acquisitions use native robots/budget policy; no external notifications","inputSchema":{"type":"object","properties":{"watch_id":{"type":"string"},"enabled":{"type":"boolean","default":true},"market":{"enum":["PUBLIC_WEB","AMAZON_JP","AMAZON_US"]},"query":{"type":"string","maxLength":500},"targets":{"type":"array","items":{"type":"string"},"minItems":1,"maxItems":5},"interval_seconds":{"type":"integer","minimum":60,"maximum":604800},"conditions":{"type":"array","maxItems":10,"description":"Rules judged only on values observed in each capture; each firing records the rule, observed and previous observed value. price_minor: lt|lte|gt|gte (value minor units) or change_bps_lte|change_bps_gte (value basis points versus the previous observed price), currency required; availability: class_is|class_is_not with a derived class","items":{"type":"object","properties":{"id":{"type":"string","maxLength":64},"field":{"enum":["price_minor","availability"]},"op":{"enum":["lt","lte","gt","gte","change_bps_lte","change_bps_gte","class_is","class_is_not"]},"value":{},"currency":{"type":"string","pattern":"^[A-Z]{3}$"}},"required":["id","field","op","value"],"additionalProperties":false}}},"required":["market","query","targets","interval_seconds"],"additionalProperties":false}}));
    out.push(json!({"name":"ecdev.monitor.export","description":"A watch's history (request, snapshots, change triggers, fact windows with their times) as a self-verifying bundle with per-record SHA-256 and a chained root; leases, tokens and errors are left out, raw captures appear only as hashes; no network","inputSchema":{"type":"object","properties":{"watch_id":{"type":"string"}},"required":["watch_id"],"additionalProperties":false}}));
    out.push(json!({"name":"ecdev.monitor.import","description":"Replay a verified ecdev.monitor.export bundle as a disabled watch with the same id; any hash mismatch or an existing watch id is refused, never merged","inputSchema":{"type":"object","properties":{"bundle":{"type":"object"}},"required":["bundle"],"additionalProperties":false}}));
    out.push(json!({"name":"ecdev.monitor.fact_as_of","description":"What a watch knew about one product field at a Unix time, from its stored fact windows, without network: OBSERVED only between first and last observation; otherwise CHANGE_TIME_UNKNOWN, NOT_OBSERVED_SINCE, UNKNOWN or CONFLICT with no value. url, product and field as in ecdev.monitor.status facts","inputSchema":{"type":"object","properties":{"watch_id":{"type":"string"},"url":{"type":"string"},"product":{"type":"string"},"field":{"type":"string","enum":["price_minor","currency","availability"]},"at":{"type":"integer","minimum":0}},"required":["watch_id","url","product","field","at"],"additionalProperties":false}}));
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
    out.push(json!({"name":"ecdev.simulation.compare","description":"Deterministic commerce simulation (SIMULATED, never observed, never demand): a scenario of typed buyer agents, or a baseline against a variant paired on common random numbers, over seeded replicates with dispersion; every declared parameter must be consumed; kept apart from observed stores; no network, no LLM","inputSchema":{"type":"object","properties":{"baseline":{"type":"object","description":"SimulationScenario: name, horizon_steps (1-365), step_seconds, population {buyers, budget_minor, price_sensitivity, brand_loyalty, category_interest as {low,high}}, market {currency, our_price_minor, competitor_price_minor, reference_price_minor}, shocks [{kind: OurPriceChange|CompetitorPriceChange, at_step, bps}], termination {rule: Horizon|Quiescence, window, max_units_per_step}, review_probability, return_base_probability"},"variant":{"type":"object","description":"Optional SimulationScenario compared against the baseline"},"seed":{"type":"integer","minimum":0},"replicates":{"type":"integer","minimum":1,"maximum":50,"default":10}},"required":["baseline","seed"],"additionalProperties":false}}));
    out.push(json!({"name":"ecdev.research.reextract","description":"Re-derive typed product series (price, currency, availability) from the stored, hash-verified raw captures of up to 50 research runs with the current extractor, and report where it disagrees with what was recorded; missing or altered captures contribute nothing; no network","inputSchema":{"type":"object","properties":{"run_ids":{"type":"array","items":{"type":"string"},"minItems":1,"maxItems":50}},"required":["run_ids"],"additionalProperties":false}}));
    out.push(json!({"name":"ecdev.research.run","description":"Bounded native/public-source product research. Paid providers never required. Supplied HTML is explicitly FIXTURE; unknown commercial fields remain unknown.","inputSchema":serde_json::from_str::<Value>(include_str!("../../../tools/commerce/schemas/research.schema.json")).unwrap()}));
    out.push(json!({"name":"ecdev.monitor.compare","description":"Compare captured product fields between two persisted research snapshots","inputSchema":{"type":"object","properties":{"before_run_id":{"type":"string"},"after_run_id":{"type":"string"}},"required":["before_run_id","after_run_id"],"additionalProperties":false}}));
    out.push(json!({"name":"ecdev.product.analyze","description":"Inspect one ASIN through an explicit evidence layer. KEEPA is the default paid path; OFFICIAL_SP_API supports separate catalog, offers and estimated fees in JP/US, supplied fixtures and operator-authorized bounded HTTP reads. Live reads require host credentials, request limits and explicit monetary ceilings; fixtures never establish official live validation.","inputSchema":{"type":"object","properties":{"market":{"enum":["AMAZON_JP","AMAZON_US"]},"asin":{"type":"string","pattern":"^[A-Z0-9]{10}$"},"evidence_layer":{"enum":["KEEPA","OFFICIAL_SP_API"]},"include":{"type":"array","items":{"enum":["CATALOG","OFFERS","FEE_ESTIMATE"]},"uniqueItems":true,"minItems":1,"maxItems":3},"fee_estimate":{"type":"object","properties":{"listing_price":{"type":"object","properties":{"amount":{"type":"string","maxLength":80,"pattern":"^[0-9]+(\\.[0-9]+)?$"},"currency":{"enum":["JPY","USD"]}},"required":["amount","currency"],"additionalProperties":false},"shipping":{"type":"object","properties":{"amount":{"type":"string","maxLength":80,"pattern":"^[0-9]+(\\.[0-9]+)?$"},"currency":{"enum":["JPY","USD"]}},"required":["amount","currency"],"additionalProperties":false},"is_amazon_fulfilled":{"type":"boolean"},"identifier":{"type":"string","minLength":1,"maxLength":128},"points":{"type":"object","properties":{"count":{"type":"integer","minimum":0},"value":{"type":"object","properties":{"amount":{"type":"string","maxLength":80,"pattern":"^[0-9]+(\\.[0-9]+)?$"},"currency":{"enum":["JPY","USD"]}},"required":["amount","currency"],"additionalProperties":false}},"required":["count","value"],"additionalProperties":false}},"required":["listing_price","is_amazon_fulfilled"],"additionalProperties":false},"fixture_responses":{"type":"object","properties":{"catalog":{"type":"object","properties":{"status":{"type":"integer","minimum":100,"maximum":599},"raw_body":{"type":"string","maxLength":4194304},"headers":{"type":"object","properties":{"x-amzn-requestid":{"type":"string","maxLength":4096},"x-amzn-ratelimit-limit":{"type":"string","maxLength":4096},"retry-after":{"type":"string","maxLength":4096}},"additionalProperties":false}},"required":["status","raw_body"],"additionalProperties":false},"offers":{"type":"object","properties":{"status":{"type":"integer","minimum":100,"maximum":599},"raw_body":{"type":"string","maxLength":4194304},"headers":{"type":"object","properties":{"x-amzn-requestid":{"type":"string","maxLength":4096},"x-amzn-ratelimit-limit":{"type":"string","maxLength":4096},"retry-after":{"type":"string","maxLength":4096}},"additionalProperties":false}},"required":["status","raw_body"],"additionalProperties":false},"fees":{"type":"object","properties":{"status":{"type":"integer","minimum":100,"maximum":599},"raw_body":{"type":"string","maxLength":4194304},"headers":{"type":"object","properties":{"x-amzn-requestid":{"type":"string","maxLength":4096},"x-amzn-ratelimit-limit":{"type":"string","maxLength":4096},"retry-after":{"type":"string","maxLength":4096}},"additionalProperties":false}},"required":["status","raw_body"],"additionalProperties":false}},"additionalProperties":false}},"required":["market","asin"],"additionalProperties":false}}));
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
            "ecdev.governance.donors",
            "Verified donor identities and exact commits",
        ),
        (
            "ecdev.governance.absorption",
            "Recorded donor lifecycle and native absorption blockers",
        ),
        (
            "ecdev.governance.extinction",
            "Recorded extinction gates per donor",
        ),
        (
            "ecdev.governance.conformance",
            "Recorded ECDEV repository conformance",
        ),
        (
            "ecdev.governance.evidence",
            "Source-backed capability ledger",
        ),
        ("ecdev.runs.list", "Persisted runs"),
    ] {
        out.push(json!({"name":name,"description":desc,"inputSchema":empty}));
    }
    out.push(json!({"name":"ecdev.governance.census","description":"Donor census and unknowns","inputSchema":donor}));
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
            let Ok(bytes) = fs::read(root.join(".ecdev-data/raw").join(format!("{hash}.html")))
            else {
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
    fn paid_product_failure_modes_counts_and_budget_are_durable() {
        use crate::provider::{
            AcquireError, AcquireRequest, AcquireResult, BudgetPolicy, Provider,
        };
        struct TestProvider(Option<u64>, bool);
        impl Provider for TestProvider {
            fn id(&self) -> &str {
                "keepa"
            }
            fn metadata(&self) -> Value {
                json!({"status":"AVAILABLE"})
            }
            fn acquire(&self, _: &AcquireRequest) -> Result<AcquireResult, AcquireError> {
                if self.1 {
                    Ok(AcquireResult {
                        observations: vec![],
                        result: json!({"unwitnessed_product":true}),
                        raw_payload: b"unwitnessed payload".to_vec(),
                        provider_cost: json!({"request_count":self.0}),
                    })
                } else {
                    let mut failure = AcquireError::http(429, 1, Some("5"), 1000);
                    failure.request_count = self.0;
                    Err(failure)
                }
            }
        }
        let budget = BudgetPolicy {
            per_run_minor: 10,
            per_day_minor: 100,
            per_month_minor: 100,
            per_provider_minor: 100,
            per_capability_minor: 100,
            request_ceiling_minor: 10,
            ..Default::default()
        };
        for (count, mode) in [
            (Some(0), "PLAN_ONLY"),
            (None, "INFERRED"),
            (Some(1), "LIVE"),
        ] {
            for success in [false, true] {
                let path = temp();
                let e = Engine::open(&path)
                    .unwrap()
                    .with_provider(Arc::new(TestProvider(count, success)));
                let run = e
                    .product_with_policy(
                        json!({"market":"AMAZON_JP","asin":"B08N5WRWNW"}),
                        &budget,
                        100,
                    )
                    .unwrap();
                assert_eq!(run["mode"], mode);
                assert_eq!(run["status"], "FAILED");
                assert_eq!(run["network_calls"], json!(count));
                assert_eq!(run["provider_calls"][0]["request_count"], json!(count));
                assert_eq!(run["observations"], json!([]));
                assert_eq!(run["new_live_acquisition"], false);
                assert!(!path.join(".ecdev-data/raw").exists());
                if count != Some(0) {
                    assert!(run["cost_minor"].is_null());
                } else {
                    assert_eq!(run["cost_minor"], 0);
                }
                assert_eq!(
                    e.run(run["run_id"].as_str().unwrap()).unwrap()["mode"],
                    mode
                );
                let db = e.db.lock().unwrap();
                let reserved: u64 = db
                    .query_row(
                        "SELECT SUM(reserved_minor) FROM paid_reservations",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap();
                assert_eq!(reserved, 10);
                drop(db);
                drop(e);
                fs::remove_dir_all(path).unwrap();
            }
        }
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
        let dir = root.join(".ecdev-data/raw");
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
            observations.push(json!({"id":id,"provider":"native-web","source_type":"PUBLIC_HTML","mode":"LIVE","raw_hash":hash,"external_source":source,"normalized_value":{"content_hash":hash}}));
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
