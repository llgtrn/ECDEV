//! Zero-paid research workflow, persisted evidence/cache, candidate ledger and accounting.
use crate::frontier::{CrawlLimits, Frontier, UrlPolicy, canonicalize};
use crate::{
    domain::Evidence,
    economics::{self, Scenario},
    provider::{AcquireRequest, BudgetPolicy},
    service::{Engine, timestamp},
};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, VecDeque};
use uuid::Uuid;
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    url: String,
    fixture_html: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    market: String,
    query: String,
    #[serde(default)]
    sources: Vec<Source>,
    follow_up_run_id: Option<String>,
    #[serde(default = "pages")]
    max_pages: usize,
    min_price_minor: Option<i64>,
    max_price_minor: Option<i64>,
    economics: Option<Scenario>,
    #[serde(default)]
    decision_policy: crate::decision::Policy,
    #[serde(default)]
    allow_stale: bool,
    #[serde(default)]
    force_refresh: bool,
    #[serde(default = "depth_limit")]
    max_depth: u32,
    #[serde(default = "url_limit")]
    max_urls: u32,
    #[serde(default = "run_deadline")]
    deadline_seconds: u32,
    crawl_run_id: Option<String>,
}
fn depth_limit() -> u32 {
    3
}
fn url_limit() -> u32 {
    2000
}
fn run_deadline() -> u32 {
    600
}
fn pages() -> usize {
    10
}
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
pub const ZERO_COST_STAGES: [(&str, bool); 14] = [
    ("discovery_from_seed_links", true),
    ("crawl", true),
    ("extraction", true),
    ("evidence", true),
    ("candidate_creation", true),
    ("competitor_price_comparison", true),
    ("economics_from_assumptions", true),
    ("ranking_and_filtering", true),
    ("report", true),
    ("supplier_discovery", true),
    ("official_marketplace_validation", false),
    ("demand_forecasting", false),
    ("live_ppc", false),
    ("regulatory_risk_validation", false),
];
impl Engine {
    pub fn inspect_candidate(&self, id: &str) -> Result<Value, String> {
        let candidate = self
            .rows("SELECT payload FROM candidates WHERE id=?1", Some(id))?
            .as_array()
            .and_then(|rows| rows.first())
            .cloned()
            .ok_or("CANDIDATE_NOT_FOUND")?;
        self.verify_evidence_ids(
            &candidate["evidence_ids"],
            &mut crate::capture::Verifier::new(&self.root),
            &mut BTreeMap::new(),
        )?;
        Ok(candidate)
    }
    pub fn compare_candidates(&self, args: Value) -> Result<Value, String> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Compare {
            candidate_ids: Vec<String>,
        }
        let input: Compare = serde_json::from_value(args).map_err(err)?;
        if !(2..=20).contains(&input.candidate_ids.len()) {
            return Err("COMPARE_REQUIRES_2_TO_20_CANDIDATES".into());
        }
        let candidates: Vec<_> = input
            .candidate_ids
            .iter()
            .map(|id| self.inspect_candidate(id))
            .collect::<Result<_, _>>()?;
        Ok(
            json!({"mode":"CAPTURED_COMPARISON","network_calls":0,"cost_minor":0,"candidates":candidates,"ranking":"No unsupported sales or profit ranking; compare source assertions, unknowns and uncertainty explicitly"}),
        )
    }
    pub(crate) fn reserve_paid(
        &self,
        id: &str,
        provider: &str,
        capability: &str,
        policy: &BudgetPolicy,
    ) -> Result<(), String> {
        let mut db = self.db.lock().map_err(err)?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(err)?;
        let day = timestamp() / 86400 * 86400;
        let month: u64 = tx
            .query_row(
                "SELECT CAST(strftime('%s','now','start of month') AS INTEGER)",
                [],
                |r| r.get(0),
            )
            .map_err(err)?;
        let sum = |start: u64, p: Option<&str>, c: Option<&str>| -> Result<u64, String> {
            tx.query_row("SELECT COALESCE(SUM(reserved_minor),0) FROM paid_reservations WHERE created_at>=?1 AND (?2 IS NULL OR provider=?2) AND (?3 IS NULL OR capability=?3)",params![start,p,c],|r|r.get(0)).map_err(err)
        };
        if !policy.permits(
            sum(day, None, None)?,
            sum(month, None, None)?,
            sum(month, Some(provider), None)?,
            sum(month, None, Some(capability))?,
        ) {
            return Err("PAID_PROVIDER_DENIED: zero/exhausted budget or unknown request ceiling; configure all explicit ceilings before paid IO".into());
        }
        tx.execute(
            "INSERT INTO paid_reservations VALUES(?1,?2,?3,?4,?5)",
            params![
                id,
                provider,
                capability,
                timestamp(),
                policy.request_ceiling_minor
            ],
        )
        .map_err(err)?;
        tx.commit().map_err(err)
    }
    pub fn budget_status(&self) -> Result<Value, String> {
        let policy = BudgetPolicy::from_env();
        let db = self.db.lock().map_err(err)?;
        let month: u64 = db
            .query_row(
                "SELECT CAST(strftime('%s','now','start of month') AS INTEGER)",
                [],
                |r| r.get(0),
            )
            .map_err(err)?;
        let reserved:u64=db.query_row("SELECT COALESCE(SUM(reserved_minor),0) FROM paid_reservations WHERE created_at>=?1",[month],|r|r.get(0)).map_err(err)?;
        Ok(
            json!({"policy":policy,"month_reserved_minor":reserved,"month_remaining_minor":policy.per_month_minor.saturating_sub(reserved),"actual_paid_cost_minor":null,"unknown_actual_cost_policy":"retain full conservative reservation","zero_cost_stage_count":ZERO_COST_STAGES.iter().filter(|(_,v)|*v).count(),"research_stage_count":ZERO_COST_STAGES.len(),"zero_cost_coverage_bps":ZERO_COST_STAGES.iter().filter(|(_,v)|*v).count()*10000/ZERO_COST_STAGES.len(),"coverage_denominator":ZERO_COST_STAGES}),
        )
    }
    pub fn accounting(&self) -> Result<Value, String> {
        self.rows(
            "SELECT payload FROM provider_accounting ORDER BY rowid DESC LIMIT 500",
            None,
        )
    }
    pub fn candidates(&self) -> Result<Value, String> {
        let mut verifier = crate::capture::Verifier::new(&self.root);
        let mut checked = BTreeMap::new();
        let candidates = self.rows(
            "SELECT payload FROM candidates ORDER BY rowid DESC LIMIT 500",
            None,
        )?;
        Ok(json!(
            candidates
                .as_array()
                .ok_or("Invalid candidates")?
                .iter()
                .filter(|c| self
                    .verify_evidence_ids(&c["evidence_ids"], &mut verifier, &mut checked)
                    .is_ok())
                .collect::<Vec<_>>()
        ))
    }
    pub fn evidence(&self, id: &str) -> Result<Value, String> {
        let rows = self.rows("SELECT payload FROM evidence WHERE run_id=?1", Some(id))?;
        crate::capture::verify_payload(&self.root, &json!({"observations": rows}))?;
        Ok(rows)
    }
    fn verify_evidence_ids(
        &self,
        ids: &Value,
        verifier: &mut crate::capture::Verifier<'_>,
        checked: &mut BTreeMap<String, bool>,
    ) -> Result<(), String> {
        let ids = ids
            .as_array()
            .filter(|ids| !ids.is_empty())
            .ok_or(crate::capture::UNAVAILABLE)?;
        for id in ids {
            let id = id.as_str().ok_or(crate::capture::UNAVAILABLE)?;
            if let Some(valid) = checked.get(id) {
                if !valid {
                    return Err(crate::capture::UNAVAILABLE.into());
                }
                continue;
            }
            let rows = self.rows("SELECT payload FROM evidence WHERE id=?1", Some(id))?;
            let observation = rows
                .as_array()
                .and_then(|a| a.first())
                .ok_or(crate::capture::UNAVAILABLE)?;
            let result = verifier.verify(observation);
            checked.insert(id.to_string(), result.is_ok());
            result?;
        }
        Ok(())
    }
    fn rows(&self, sql: &str, id: Option<&str>) -> Result<Value, String> {
        let db = self.db.lock().map_err(err)?;
        let mut st = db.prepare(sql).map_err(err)?;
        let mut rows = if let Some(id) = id {
            st.query([id]).map_err(err)?
        } else {
            st.query([]).map_err(err)?
        };
        let mut out = vec![];
        while let Some(r) = rows.next().map_err(err)? {
            let text: String = r.get(0).map_err(err)?;
            out.push(serde_json::from_str::<Value>(&text).map_err(err)?);
        }
        Ok(json!(out))
    }
    pub fn research(&self, args: Value) -> Result<Value, String> {
        let mut input: Input = serde_json::from_value(args).map_err(err)?;
        if input
            .decision_policy
            .currency
            .as_ref()
            .is_some_and(|s| s.len() != 3 || !s.bytes().all(|b| b.is_ascii_uppercase()))
            || input
                .decision_policy
                .excluded_categories
                .iter()
                .any(|s| s.trim().is_empty())
        {
            return Err("INVALID_DECISION_POLICY".into());
        }
        let mut executed_plan = Value::Null;
        if let Some(prior_id) = &input.follow_up_run_id {
            if !input.sources.is_empty() || input.crawl_run_id.is_some() {
                return Err("FOLLOW_UP_REQUIRES_NO_EXPLICIT_SOURCES_OR_RESUME".into());
            }
            let prior = self.run(prior_id)?;
            if prior["mode"] == "FIXTURE" {
                return Err("FIXTURE_PLAN_CANNOT_LAUNCH_LIVE_FOLLOW_UP".into());
            }
            if prior["market"] != input.market {
                return Err("FOLLOW_UP_MARKET_MISMATCH".into());
            }
            executed_plan = crate::planner::information_gain(
                prior["candidates"]
                    .as_array()
                    .ok_or("PRIOR_RUN_HAS_NO_CANDIDATES")?,
                prior["supplier_leads"]
                    .as_array()
                    .ok_or("PRIOR_RUN_HAS_NO_SUPPLIER_LEADS")?,
                0,
                input.max_pages.min(20),
            );
            input.sources = executed_plan["selected_actions"]
                .as_array()
                .ok_or("INVALID_FOLLOW_UP_PLAN")?
                .iter()
                .filter_map(|action| action["url"].as_str())
                .map(|url| Source {
                    url: url.into(),
                    fixture_html: None,
                })
                .collect();
            if input.sources.is_empty() {
                return Err("NO_EXECUTABLE_INFORMATION_GAIN_ACTION".into());
            }
            input.max_depth = 0;
        }
        if !matches!(
            input.market.as_str(),
            "AMAZON_JP" | "AMAZON_US" | "PUBLIC_WEB"
        ) || input.query.len() > 500
            || input.sources.is_empty()
            || input.sources.len() > 20
            || !(1..=1000).contains(&input.max_pages)
            || input.max_depth > 10
            || input.max_urls < input.max_pages as u32
            || input.max_urls > 100_000
            || !(1..=3600).contains(&input.deadline_seconds)
            || input.min_price_minor.is_some_and(|v| v < 0)
            || input.max_price_minor.is_some_and(|v| v < 0)
            || input
                .min_price_minor
                .zip(input.max_price_minor)
                .is_some_and(|(a, b)| a > b)
        {
            return Err("INVALID_RESEARCH_CONSTRAINTS".into());
        }
        if let Some(s) = &input.economics {
            economics::simulate(s)?;
        }
        let fixture = input.sources.iter().any(|s| s.fixture_html.is_some());
        if fixture && input.sources.iter().any(|s| s.fixture_html.is_none()) {
            return Err("Do not mix fixture and live sources".into());
        }
        let routes = crate::provider::routes(
            "fetch.http",
            self.providers()
                .as_array()
                .ok_or("Invalid provider catalog")?,
            0,
            input.sources.len(),
        );
        let selected = routes["routes"]
            .as_array()
            .and_then(|a| a.iter().find(|p| p["id"] == "native-web"))
            .and_then(|p| p["id"].as_str())
            .ok_or("NO_ZERO_COST_FETCH_PROVIDER")?;
        let provider = self
            .providers
            .iter()
            .find(|p| p.id() == selected)
            .ok_or("NATIVE_WEB_NOT_AVAILABLE")?;
        let id = Uuid::new_v4().to_string();
        let crawl_id = input.crawl_run_id.clone().unwrap_or_else(|| id.clone());
        if !crate::service::valid_id(&crawl_id) {
            return Err("INVALID_CRAWL_RUN_ID".into());
        }
        let mut frontier =
            Frontier::open(&self.root.join(".ynventa/materialized/runtime/ecdev.sqlite"))?;
        if input.crawl_run_id.is_none() {
            frontier.create_run(
                &crawl_id,
                &CrawlLimits {
                    max_pages: input.max_pages as u32,
                    max_urls: input.max_urls,
                    max_depth: input.max_depth,
                    global_concurrency: 1,
                    per_origin_concurrency: 1,
                    origin_interval_ms: 750,
                    lease_ms: 120_000,
                    max_retries: 2,
                    backoff_ms: 1000,
                    max_backoff_ms: 30_000,
                    deadline_ms: (timestamp() * 1000 + u64::from(input.deadline_seconds) * 1000)
                        as i64,
                },
            )?;
        } else {
            frontier.status(&crawl_id)?;
        }
        let mut fixtures = BTreeMap::new();
        for source in &input.sources {
            let normal = provider.normalize_query(&json!({"url":source.url}))?["url"]
                .as_str()
                .ok_or("Normalized URL missing")?
                .to_string();
            let identity = canonicalize(
                &normal,
                None,
                &UrlPolicy::default(),
                (timestamp() * 1000) as i64,
            )?;
            if let Some(html) = &source.fixture_html {
                fixtures.insert(identity.canonical_url.clone(), html.clone());
            }
            frontier.enqueue(&crawl_id, &identity, 0, 1000)?;
        }
        let mut recovered: VecDeque<_> = frontier.captures(&crawl_id)?.into();
        let mut observations = vec![];
        let mut candidates = vec![];
        let mut calls = vec![];
        let mut failures = vec![];
        let mut snapshots = vec![];
        loop {
            let (source, depth, checkpoint, lease) = if let Some(saved) = recovered.pop_front() {
                (
                    Source {
                        url: saved["source"]
                            .as_str()
                            .ok_or("Invalid saved source")?
                            .into(),
                        fixture_html: None,
                    },
                    saved["depth"].as_u64().unwrap_or(0) as u32,
                    Some(saved["capture"].clone()),
                    None,
                )
            } else if let Some(lease) = frontier.lease(&crawl_id, (timestamp() * 1000) as i64)? {
                (
                    Source {
                        fixture_html: fixtures.get(&lease.canonical_url).cloned(),
                        url: lease.canonical_url.clone(),
                    },
                    lease.depth,
                    None,
                    Some(lease),
                )
            } else {
                let status = frontier.status(&crawl_id)?;
                let ready = status["states"]["PENDING"].as_u64().unwrap_or(0)
                    + status["states"]["RETRYABLE"].as_u64().unwrap_or(0)
                    + status["states"]["LEASED"].as_u64().unwrap_or(0);
                if ready == 0
                    || status["acquisition_attempts"].as_u64().unwrap_or(0)
                        >= status["limits"]["max_pages"].as_u64().unwrap_or(0)
                {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
                continue;
            };
            let amazon_source = url::Url::parse(&source.url).ok().is_some_and(|u| {
                matches!(
                    u.host_str(),
                    Some("amazon.co.jp" | "www.amazon.co.jp" | "amazon.com" | "www.amazon.com")
                )
            });
            let provider = if amazon_source {
                if let Some(public) = self.providers.iter().find(|p| p.id() == "public-amazon") {
                    public
                } else {
                    let reason = "PUBLIC_AMAZON_ADAPTER_UNAVAILABLE";
                    failures.push(json!({"source":source.url,"provider":"public-amazon","status":"SOURCE_UNAVAILABLE","reason":reason}));
                    calls.push(json!({"id":Uuid::new_v4().to_string(),"provider":"public-amazon","capability":"fetch.http","status":"UNAVAILABLE","actual_cost_minor":0,"request_count":0,"cache_hit":false,"error":reason}));
                    if let Some(lease) = &lease {
                        frontier.fail(lease, (timestamp() * 1000) as i64, reason, false, None)?;
                    }
                    continue;
                }
            } else {
                provider
            };
            let checkpoint = checkpoint.filter(|capture| {
                capture["observations"].as_array().is_some_and(|a| {
                    !a.is_empty() && a.iter().all(|o| o["provider"] == provider.id())
                })
            });
            if let Some(capture) = &checkpoint
                && crate::capture::verify_payload(&self.root, capture).is_err()
            {
                failures.push(json!({"source":source.url,"reason":crate::capture::UNAVAILABLE,"origin":"FRONTIER_CHECKPOINT","request_count":0}));
                continue;
            }
            let started = timestamp() * 1000;
            let started_clock = std::time::Instant::now();
            let key=format!("{:x}",Sha256::digest(json!({"provider":provider.id(),"operation":"fetch.http","url":source.url,"market":input.market,"locale":input.market,"schema":12,"fixture_hash":source.fixture_html.as_ref().map(|h|format!("{:x}",Sha256::digest(h.as_bytes())))}).to_string().as_bytes()));
            let cached: Option<(String, u64)> = self
                .db
                .lock()
                .map_err(err)?
                .query_row(
                    "SELECT payload,expires_at FROM fetch_cache WHERE key=?1",
                    [&key],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(err)?;
            let previous = cached
                .as_ref()
                .map(|(s, _)| serde_json::from_str::<Value>(s))
                .transpose()
                .map_err(err)?
                .filter(|capture| crate::capture::verify_payload(&self.root, capture).is_ok());
            let fresh = !input.force_refresh
                && previous.is_some()
                && cached
                    .as_ref()
                    .is_some_and(|(_, expires)| *expires > timestamp());
            let request = AcquireRequest {
                run_id: id.clone(),
                capability: "fetch.http".into(),
                market: input.market.clone(),
                query: json!({"url":source.url,"fixture_html":source.fixture_html,"conditional":if input.force_refresh {None} else {previous.as_ref().map(|v|v["provider_cost"]["headers"].clone())}}),
            };
            let mut stale_used = false;
            let recovered_capture = checkpoint.is_some();
            let (mut captured, cache_hit) = if let Some(saved) = checkpoint {
                (saved, true)
            } else if fresh {
                (previous.clone().unwrap(), true)
            } else {
                match provider.acquire(&request) {
                    Ok(result) => {
                        if result.result["not_modified"] == true {
                            let Some(mut prior) = previous.clone() else {
                                failures.push(
                                    json!({"source":source.url,"reason":"304_WITHOUT_CAPTURE"}),
                                );
                                if let Some(lease) = &lease {
                                    frontier.fail(
                                        lease,
                                        (timestamp() * 1000) as i64,
                                        "304_WITHOUT_CAPTURE",
                                        false,
                                        None,
                                    )?;
                                }
                                continue;
                            };
                            prior["provider_cost"] = result.provider_cost;
                            (prior, true)
                        } else {
                            let dir = self.root.join(".ynventa/materialized/raw");
                            std::fs::create_dir_all(&dir).map_err(err)?;
                            for observation in &result.observations {
                                observation.validate()?;
                                std::fs::write(
                                    dir.join(format!("{}.html", observation.raw_hash)),
                                    &result.raw_payload,
                                )
                                .map_err(err)?;
                            }
                            (
                                json!({"observations":result.observations,"result":result.result,"provider_cost":result.provider_cost}),
                                false,
                            )
                        }
                    }
                    Err(failure) => {
                        let reason = &failure.reason;
                        if input.allow_stale
                            && cached.as_ref().is_some_and(|(_, expires)| {
                                timestamp().saturating_sub(*expires) <= 86400
                            })
                            && previous.is_some()
                        {
                            stale_used = true;
                            let mut prior = previous.clone().unwrap();
                            prior["provider_cost"]["fallback_reason"] = json!(reason);
                            prior["provider_cost"]["request_count"] = json!(failure.request_count);
                            prior["provider_cost"]["acquisition_failure"] = json!(failure);
                            (prior, true)
                        } else {
                            failures.push(json!({"source":source.url,"provider":provider.id(),"status":if reason.contains("ROBOTS") || reason.contains("HTTP_STATUS_403") || reason.contains("REDIRECT_SCOPE_DENIED") {"SOURCE_BLOCKED"}else{"SOURCE_UNAVAILABLE"},"reason":reason,"acquisition_failure":failure}));
                            calls.push(json!({"id":Uuid::new_v4().to_string(),"provider":provider.id(),"capability":"fetch.http","started_at":started,"completed_at":timestamp()*1000,"status":"FAILED","actual_cost_minor":0,"request_count":failure.request_count,"cache_hit":false,"error":reason,"acquisition_failure":failure}));
                            if let Some(lease) = &lease {
                                let retryable =
                                    matches!(failure.http_status, Some(429 | 500..=599))
                                        || reason == "FETCH_NETWORK_ERROR"
                                        || reason == "DNS_FAILED";
                                let failed_at = (timestamp() * 1000) as i64;
                                frontier.fail(
                                    lease,
                                    failed_at,
                                    reason,
                                    retryable,
                                    failure.retry_delay_ms(failed_at),
                                )?;
                            }
                            continue;
                        }
                    }
                }
            };
            if !fresh && !stale_used && !recovered_capture {
                self.db
                    .lock()
                    .map_err(err)?
                    .execute(
                        "INSERT OR REPLACE INTO fetch_cache VALUES(?1,?2,?3,?4)",
                        params![key, captured.to_string(), timestamp(), timestamp() + 3600],
                    )
                    .map_err(err)?;
            }
            let evidence = captured["observations"]
                .as_array_mut()
                .ok_or("Invalid cached evidence")?;
            for observation in evidence.iter_mut() {
                observation["id"] = json!(Uuid::new_v4().to_string());
                observation["run_id"] = json!(id);
                if cache_hit && !fixture {
                    observation["mode"] = json!("CACHED");
                }
                let typed: Evidence = serde_json::from_value(observation.clone()).map_err(err)?;
                typed.validate()?;
            }
            let evidence_ids: Vec<_> = evidence.iter().map(|v| v["id"].clone()).collect();
            observations.extend(evidence.clone());
            let result = &captured["result"];
            snapshots.push(result.clone());
            if result["source_status"] == "SOURCE_BLOCKED" {
                failures.push(json!({"source":source.url,"provider":provider.id(),"status":"SOURCE_BLOCKED","reason":result["blocked_reason"]}));
            }
            for product in result["products"].as_array().ok_or("Invalid extraction")? {
                let mut reasons = vec![];
                let mut unknowns = vec![
                    "true_sales",
                    "search_volume",
                    "supplier_moq",
                    "shipping_quote",
                    "regulatory_risk",
                    "demand_forecast",
                ];
                let price = product["price_minor"].as_i64();
                if price.is_none() {
                    unknowns.push("price");
                }
                if price
                    .zip(input.min_price_minor)
                    .is_some_and(|(p, min)| p < min)
                {
                    reasons.push("BELOW_MIN_PRICE");
                }
                if price
                    .zip(input.max_price_minor)
                    .is_some_and(|(p, max)| p > max)
                {
                    reasons.push("ABOVE_MAX_PRICE");
                }
                let economics = if let (Some(price), Some(s)) = (price, &input.economics) {
                    if product["currency"] == s.currency {
                        let mut scenario = s.clone();
                        scenario.selling_price =
                            u64::try_from(price).map_err(|_| "INVALID_OBSERVED_PRICE")?;
                        Some(
                            json!({"status":"DERIVED_FROM_SUPPLIED_ASSUMPTIONS","scenario":scenario,"result":economics::simulate(&scenario)?}),
                        )
                    } else {
                        unknowns.push("currency_compatible_economics");
                        None
                    }
                } else {
                    unknowns.push("economics");
                    None
                };
                if economics
                    .as_ref()
                    .and_then(|v| v["result"]["contribution_per_unit"].as_i64())
                    .is_some_and(|p| p <= 0)
                {
                    reasons.push("NON_POSITIVE_ASSUMED_MARGIN");
                }
                let state = if !reasons.is_empty() {
                    "REJECTED"
                } else if price.is_some() && economics.is_some() {
                    "SCREENED"
                } else {
                    "VALIDATING"
                };
                candidates.push(json!({"id":Uuid::new_v4().to_string(),"run_id":id,"state":state,"product":product,"source":result["source"],"requested_source":source.url,"evidence_ids":evidence_ids,"discovery":"OBSERVED_IN_SOURCE","economics":economics,"unknowns":unknowns,"rejection_reasons":reasons,"survival_reason":if reasons.is_empty(){"Passed supplied price constraints; requires demand/supplier/risk validation"}else{"Rejected by explicit constraints"},"confidence":"SOURCE_ASSERTION_ONLY"}));
            }
            if !fixture {
                for link in result["links"].as_array().ok_or("Invalid links")? {
                    if let Some(url) = link.as_str()
                        && let Ok(identity) = canonicalize(
                            url,
                            Some(&source.url),
                            &UrlPolicy::default(),
                            (timestamp() * 1000) as i64,
                        )
                    {
                        let priority = if url.contains("/products/") {
                            100
                        } else if url.contains("page=") {
                            90
                        } else if url.contains("/collections/") {
                            50
                        } else {
                            0
                        };
                        frontier.enqueue(&crawl_id, &identity, depth + 1, priority)?;
                    }
                }
            }
            if let Some(lease) = &lease {
                frontier.complete_with_origin_cooldown(
                    lease,
                    (timestamp() * 1000) as i64,
                    &json!({"source":source.url,"depth":depth,"capture":captured}),
                    if stale_used {
                        captured["provider_cost"]["acquisition_failure"]["retry_not_before_ms"]
                            .as_i64()
                    } else {
                        None
                    },
                )?;
            }
            calls.push(json!({"id":Uuid::new_v4().to_string(),"provider":provider.id(),"capability":"fetch.http","started_at":started,"completed_at":timestamp()*1000,"latency_ms":started_clock.elapsed().as_millis(),"request_count":if fresh || recovered_capture {json!(0)}else{captured["provider_cost"]["request_count"].clone()},"quota_before":null,"quota_after":null,"estimated_cost_minor":0,"actual_cost_minor":0,"cache_hit":cache_hit,"cache_status":if stale_used{"STALE_USABLE"}else if cache_hit{"FRESH_OR_REVALIDATED"}else{"MISS"},"result_count":result["products"].as_array().unwrap().len(),"evidence_ids":evidence_ids,"acquisition_failure":captured["provider_cost"]["acquisition_failure"],"status":if result["source_status"]=="SOURCE_BLOCKED"{"SOURCE_BLOCKED"}else{"COMPLETE"}}));
        }
        let (mut candidates, entity_resolution) = crate::resolution::resolve(candidates);
        let observed_sample = crate::intelligence::enrich(&mut candidates, &snapshots)?;
        let frontier_status = frontier.status(&crawl_id)?;
        let decision_budget_exhausted = if fixture {
            calls.len() >= input.max_pages
        } else {
            frontier_status["acquisition_attempts"]
                .as_u64()
                .unwrap_or(0)
                >= frontier_status["limits"]["max_pages"]
                    .as_u64()
                    .unwrap_or(u64::MAX)
                || frontier_status["limits"]["deadline_ms"]
                    .as_u64()
                    .is_some_and(|deadline| timestamp() * 1000 >= deadline)
        };
        crate::decision::assess(
            &mut candidates,
            &input.decision_policy,
            decision_budget_exhausted,
        );
        let supplier_leads: Vec<Value> = snapshots
            .iter()
            .flat_map(|s| s["supplier_leads"].as_array().cloned().unwrap_or_default())
            .collect();
        let next_actions = crate::planner::information_gain(
            &candidates,
            &supplier_leads,
            0,
            input.max_pages.min(20),
        );
        candidates.sort_by_key(|c| {
            (
                c["state"] == "REJECTED",
                c["product"]["price_minor"].as_i64().unwrap_or(i64::MAX),
            )
        });
        let network: u64 = calls
            .iter()
            .filter_map(|c| c["request_count"].as_u64())
            .sum();
        let completeness = crate::intelligence::completeness(
            &candidates,
            &snapshots,
            fixture,
            network,
            supplier_leads.len(),
        );

        let incomplete = [
            "DISCOVERED",
            "PENDING",
            "LEASED",
            "RETRYABLE",
            "FAILED",
            "CANCELLED",
        ]
        .iter()
        .any(|state| frontier_status["states"][*state].as_u64().unwrap_or(0) > 0);
        let accounting_mode = if fixture {
            "FIXTURE"
        } else if calls.iter().all(|c| c["cache_hit"] == true) {
            "CACHED"
        } else {
            "LIVE"
        };
        let cache_metrics = crate::intelligence::cache_metrics(&calls, accounting_mode);
        let run=self.persist(json!({"acquisition_run_id":id,"research_run":true,"supplier_leads":supplier_leads,"next_actions":next_actions,"executed_information_gain_plan":executed_plan,"entity_resolution":entity_resolution,"observed_sample":observed_sample,"cache_metrics":cache_metrics,"completeness":completeness,"crawl_run_id":crawl_id,"frontier":frontier_status,"mode":if fixture{"FIXTURE"}else if calls.iter().all(|c|c["cache_hit"]==true){"CACHED"}else{"LIVE"},"status":if observations.is_empty(){"UNAVAILABLE"}else if failures.is_empty() && !incomplete {"COMPLETE_WITH_UNKNOWNS"}else{"PARTIAL"},"market":input.market,"query":input.query,"observations":observations,"candidates":candidates,"provider_calls":calls,"snapshots":snapshots,"errors":failures,"cost_minor":0,"network_calls":if calls.iter().any(|c|c["request_count"].is_null()){serde_json::Value::Null}else{json!(network)},"known_network_calls":network,"source_routes":routes,"paid_providers":[{"provider":"semrush","status":"SKIPPED","reason":"OPTIONAL_PAID_EVIDENCE_NOT_REQUIRED"},{"provider":"keepa","status":"SKIPPED","reason":"OPTIONAL_PAID_EVIDENCE_NOT_REQUIRED"},{"provider":"hosted-firecrawl","status":"SKIPPED","reason":"NATIVE_PUBLIC_PATH"},{"provider":"hosted-apify","status":"SKIPPED","reason":"NATIVE_PUBLIC_PATH"}],"funnel":{"discovered":candidates.len(),"screened":candidates.iter().filter(|c|c["state"]=="SCREENED").count(),"validating":candidates.iter().filter(|c|c["state"]=="VALIDATING").count(),"insufficient_evidence":candidates.iter().filter(|c|c["state"]=="INSUFFICIENT_EVIDENCE").count(),"rejected":candidates.iter().filter(|c|c["state"]=="REJECTED").count(),"shortlisted":candidates.iter().filter(|c|c["state"]=="SHORTLISTED").count(),"sampling":0},"ranking_rule":"Explicit rejections last, observed price ascending; no learned sales score","coverage":self.budget_status()?,"missing_evidence":"Demand, supplier, logistics quotations, official marketplace validation, PPC, regulatory risk"}))?;
        Ok(run)
    }
    pub fn evidence_graph(&self) -> Result<Value, String> {
        let mut verifier = crate::capture::Verifier::new(&self.root);
        let mut checked = BTreeMap::new();
        let entities = self.rows(
            "SELECT payload FROM entities ORDER BY rowid DESC LIMIT 500",
            None,
        )?;
        let entities: Vec<_> = entities
            .as_array()
            .ok_or("Invalid entities")?
            .iter()
            .filter(|e| {
                self.verify_evidence_ids(&e["evidence_ids"], &mut verifier, &mut checked)
                    .is_ok()
            })
            .collect();
        let edges = self.rows("SELECT payload FROM edges ORDER BY id DESC LIMIT 500", None)?;
        let edges: Vec<_> = edges
            .as_array()
            .ok_or("Invalid edges")?
            .iter()
            .filter(|e| {
                entities.iter().any(|entity| {
                    entity["id"] == e["from"]
                        && entity["evidence_ids"]
                            .as_array()
                            .is_some_and(|ids| ids.contains(&e["to"]))
                })
            })
            .collect();
        Ok(
            json!({"entities":entities,"edges":edges,"raw_capture_verification":"VERIFIED_LOCAL_SHA256_NO_NETWORK"}),
        )
    }
    pub fn compare_snapshots(&self, args: Value) -> Result<Value, String> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Compare {
            before_run_id: String,
            after_run_id: String,
        }
        let c: Compare = serde_json::from_value(args).map_err(err)?;
        let before = self.run(&c.before_run_id)?;
        let after = self.run(&c.after_run_id)?;
        let mut changes = vec![];
        for b in before["candidates"]
            .as_array()
            .ok_or("Research run required")?
        {
            for a in after["candidates"]
                .as_array()
                .ok_or("Research run required")?
            {
                if b["source"] == a["source"]
                    && ((!b["product"]["sku"].is_null()
                        && b["product"]["sku"] == a["product"]["sku"])
                        || (b["product"]["sku"].is_null()
                            && a["product"]["sku"].is_null()
                            && b["product"]["title"] == a["product"]["title"]))
                {
                    for field in ["price_minor", "currency", "availability", "seller", "title"] {
                        if b["product"][field] != a["product"][field] {
                            changes.push(json!({"source":a["source"],"field":field,"before":b["product"][field],"after":a["product"][field],"status":"DERIVED_SNAPSHOT_DIFF"}));
                        }
                    }
                }
            }
        }
        Ok(
            json!({"before_run_id":c.before_run_id,"after_run_id":c.after_run_id,"changes":changes,"network_calls":0,"cost_minor":0,"schedule":"NOT_IMPLEMENTED"}),
        )
    }
}

#[cfg(test)]
mod budget_storage_tests {
    use super::*;
    #[test]
    fn response_retry_after_reaches_durable_frontier_and_known_request_accounting() {
        use crate::provider::{AcquireError, AcquireResult, Provider};
        struct FailureProvider(u16);
        impl Provider for FailureProvider {
            fn id(&self) -> &str {
                "native-web"
            }
            fn metadata(&self) -> Value {
                json!({"id":self.id(),"class":"PUBLIC","status":"AVAILABLE","capabilities":["fetch.http"],"markets":["PUBLIC_WEB"]})
            }
            fn acquire(&self, _: &AcquireRequest) -> Result<AcquireResult, AcquireError> {
                Err(AcquireError::http(
                    self.0,
                    2,
                    Some("120"),
                    (timestamp() * 1000) as i64,
                ))
            }
        }
        for status in [429, 503] {
            let root = std::env::temp_dir().join(format!("ecdev-http-retry-{}", Uuid::new_v4()));
            let e = Engine::open(&root)
                .unwrap()
                .with_provider(std::sync::Arc::new(FailureProvider(status)));
            let run = e.research(json!({"market":"PUBLIC_WEB","query":"Synthetic response failure, no network IO","sources":[{"url":"https://shop.example/product","fixture_html":""}],"max_pages":1,"deadline_seconds":300})).unwrap();
            assert_eq!(run["mode"], "FIXTURE");
            assert_eq!(run["network_calls"], 2); // Synthetic provider's declared counter, not live proof.
            assert_eq!(run["known_network_calls"], 2);
            assert_eq!(
                run["provider_calls"][0]["acquisition_failure"]["http_status"],
                status
            );
            assert_eq!(run["frontier"]["states"]["RETRYABLE"], 1);
            let id = run["crawl_run_id"].as_str().unwrap();
            let next_at = run["provider_calls"][0]["acquisition_failure"]["retry_not_before_ms"]
                .as_i64()
                .unwrap();
            {
                let db = e.db.lock().unwrap();
                let saved: i64 = db
                    .query_row(
                        "SELECT next_at FROM crawl_urls WHERE run_id=?1",
                        [id],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(saved, next_at);
                // Permit another attempt so the page budget cannot mask the not-before assertion.
                let text: String = db
                    .query_row("SELECT limits FROM crawl_runs WHERE id=?1", [id], |row| {
                        row.get(0)
                    })
                    .unwrap();
                let mut limits: CrawlLimits = serde_json::from_str(&text).unwrap();
                limits.max_pages = 2;
                db.execute(
                    "UPDATE crawl_runs SET limits=?2 WHERE id=?1",
                    params![id, serde_json::to_string(&limits).unwrap()],
                )
                .unwrap();
            }
            drop(e);
            let mut reopened =
                Frontier::open(&root.join(".ynventa/materialized/runtime/ecdev.sqlite")).unwrap();
            assert!(reopened.lease(id, next_at - 1).unwrap().is_none());
            assert_eq!(reopened.lease(id, next_at).unwrap().unwrap().attempts, 2);
        }
    }
    #[test]
    fn budget_reservations_are_durable_and_zero_denies_io() {
        let root = std::env::temp_dir().join(format!("ecdev-budget-{}", Uuid::new_v4()));
        let e = Engine::open(&root).unwrap();
        assert!(
            e.reserve_paid(
                "denied",
                "keepa",
                "product.analyze",
                &BudgetPolicy::default()
            )
            .is_err()
        );
        let policy = BudgetPolicy {
            per_run_minor: 10,
            per_day_minor: 20,
            per_month_minor: 20,
            per_provider_minor: 20,
            per_capability_minor: 20,
            request_ceiling_minor: 10,
            ..Default::default()
        };
        e.reserve_paid("first", "keepa", "product.analyze", &policy)
            .unwrap();
        e.reserve_paid("second", "keepa", "product.analyze", &policy)
            .unwrap();
        drop(e);
        let e = Engine::open(&root).unwrap();
        assert!(
            e.reserve_paid("third", "keepa", "product.analyze", &policy)
                .is_err()
        );
        let db = e.db.lock().unwrap();
        let count: u64 = db
            .query_row("SELECT COUNT(*) FROM paid_reservations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2);
    }
}
