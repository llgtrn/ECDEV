//! Durable observations, snapshots and fenced watches, owned by commerce.
use super::{ForecastScenario, SocialPost, snapshot, terms};
use crate::{
    Engine, planner::allocate_information_actions, provider::AcquireRequest, service::timestamp,
};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
};
use uuid::Uuid;
// Requested source identity is narrower than a platform label for public JSON feeds.
fn source_key(source: &Value) -> String {
    let platform = source["platform"].as_str().unwrap_or("UNKNOWN");
    if platform == "JSON_FEED" || platform == "XML_FEED" {
        let canonical = source["url"]
            .as_str()
            .and_then(|u| url::Url::parse(u).ok())
            .map(|mut u| {
                u.set_fragment(None);
                u.to_string()
            })
            .unwrap_or_default();
        format!("{platform}:{canonical}")
    } else {
        platform.to_owned()
    }
}
fn post_source_key(post: &SocialPost) -> String {
    if post.platform == "JSON_FEED" || post.platform == "XML_FEED" {
        format!(
            "{}:{}",
            post.platform,
            post.native_id
                .split_once('#')
                .map(|(source, _)| source)
                .unwrap_or("")
        )
    } else {
        post.platform.clone()
    }
}
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
    "VOLUME_SPIKE",
    "SENTIMENT_DROP",
];
pub fn tool_definitions() -> Vec<Value> {
    let source = json!({"type":"object","properties":{"platform":{"enum":["HACKER_NEWS","BLUESKY","MASTODON","JSON_FEED","XML_FEED","MASTODON_TAG"]},"url":{"type":"string","maxLength":4096,"description":"JSON_FEED and XML_FEED (RSS 2.0, RSS 1.0, Atom 1.0; UTF-8, no DTD): the public feed URL"},"instance":{"type":"string","maxLength":253,"description":"MASTODON and MASTODON_TAG: the public instance whose tag timeline or tag record is read (default mastodon.social); what it holds is that instance's federated view"},"fixture_raw":{"type":"string","maxLength":4194304},"max_pages":{"type":"integer","minimum":1,"maximum":5,"default":1,"description":"Pages walked by the source's own cursor; each page is its own capture and costs two requests; stops at the end, an empty page, a repeated cursor, the limit or the request budget"},"fixture_pages":{"type":"array","items":{"type":"string","maxLength":4194304},"maxItems":4,"description":"Fixture bodies for pages after the first"},"slice_seconds":{"type":"integer","minimum":3600,"description":"Split the requested window into time slices the source bounds itself (Hacker News, Bluesky, Mastodon), newest first, at most 31; each slice walks its own pages. Sources without time bounds (feeds) are refused. Bluesky serves only the first page of a search, so slicing is how a Bluesky window gets covered: live, 6-hour slices over 3 days read 347 posts with 67% of the window covered, against 50 posts and 15% unsliced; each slice costs 2 requests"},"reply_trees":{"type":"boolean","description":"Also read the reply trees of the roots with the most reported comments (Hacker News items, Bluesky getPostThread); JSON Feed exposes none"},"max_threads":{"type":"integer","minimum":1,"maximum":10,"default":3},"fixture_threads":{"type":"object","additionalProperties":{"type":"string","maxLength":4194304},"description":"Fixture thread bodies keyed by root native id"},"fixture_slices":{"type":"array","maxItems":31,"items":{"type":"array","maxItems":5,"items":{"type":"string","maxLength":4194304}},"description":"Fixture bodies per slice and page"}},"required":["platform"],"additionalProperties":false});
    let discover = json!({"type":"object","properties":{"query":{"type":"string","minLength":1,"maxLength":500},"sources":{"type":"array","items":source,"minItems":1,"maxItems":5},"window_seconds":{"type":"integer","minimum":60,"maximum":2592000},"request_budget":{"type":"integer","minimum":0,"maximum":20},"cache_only":{"type":"boolean"},"fixture_now":{"type":"integer","minimum":0}},"required":["query","sources"],"additionalProperties":false});
    let inspect = json!({"type":"object","properties":{"snapshot_id":{"type":"string"}},"additionalProperties":false});
    vec![
        json!({"name":"ecdev.trend.discover","description":"Bounded public social acquisition and evidence-linked sampled trends; zero paid; supplied raw data FIXTURE, cache_only makes no requests; missing counters/timestamps UNKNOWN, no social-only shortlist","inputSchema":discover}),
        json!({"name":"ecdev.trend.inspect","description":"Inspect frozen trend snapshots, source evidence, watches and simulation boundary without network; omit snapshot_id to list","inputSchema":inspect}),
        json!({"name":"ecdev.trend.explain","description":"Explain explicit heuristic components, provenance, unknowns and conflicts for a frozen snapshot","inputSchema":inspect}),
        json!({"name":"ecdev.trend.hypothesize","description":"Turn a frozen trend snapshot into product hypotheses, unverified candidate links and ranked zero-paid research actions; never demand, never shortlist; no network","inputSchema":{"type":"object","properties":{"snapshot_id":{"type":"string"}},"required":["snapshot_id"],"additionalProperties":false}}),
        json!({"name":"ecdev.trend.compare","description":"Compare two compatible snapshots with actual timestamps; fixture/live/simulation scopes cannot mix","inputSchema":{"type":"object","properties":{"before_snapshot_id":{"type":"string"},"after_snapshot_id":{"type":"string"}},"required":["before_snapshot_id","after_snapshot_id"],"additionalProperties":false}}),
        json!({"name":"ecdev.trend.watch","description":"Create/update persisted public trend watch or action=list; eleven trigger kinds (VOLUME_SPIKE and SENTIMENT_DROP use harken-parity threshold rules), fenced leases, local events, minimum sample and complete-source policy; enabled=false disables","inputSchema":{"type":"object","properties":{"action":{"enum":["create","list"]},"watch_id":{"type":"string"},"query":{"type":"string","maxLength":500},"research":discover,"triggers":{"type":"array","items":{"enum":TRIGGERS},"minItems":1},"interval_seconds":{"type":"integer","minimum":60,"maximum":604800},"minimum_mentions":{"type":"integer","minimum":2},"threshold":{"type":"number","minimum":0},"volume_multiplier":{"type":"number","minimum":0},"sentiment_drop":{"type":"number","minimum":0},"enabled":{"type":"boolean"}},"additionalProperties":false}}),
    ]
}

impl Engine {
    fn verified_social_capture(
        &self,
        post: &SocialPost,
        checked: &mut BTreeMap<String, bool>,
    ) -> bool {
        if post.validate().is_err() {
            return false;
        }
        *checked.entry(post.raw_hash.clone()).or_insert_with(|| {
            let path = self
                .root
                .join(".ecdev-data/runtime/social-captures")
                .join(format!("{}.raw", post.raw_hash));
            fs::metadata(&path).is_ok_and(|m| m.is_file() && m.len() <= 4194304)
                && fs::read(path)
                    .is_ok_and(|raw| format!("{:x}", Sha256::digest(raw)) == post.raw_hash)
        })
    }
    fn verified_social_snapshot(
        &self,
        value: &Value,
        checked: &mut BTreeMap<String, bool>,
    ) -> bool {
        let counters = value["source_counters"]
            .as_array()
            .into_iter()
            .flatten()
            .all(|c| {
                c["raw_hash"].as_str().is_some_and(|hash| {
                    *checked.entry(hash.to_string()).or_insert_with(|| {
                        let path = self
                            .root
                            .join(".ecdev-data/runtime/social-captures")
                            .join(format!("{hash}.raw"));
                        fs::metadata(&path).is_ok_and(|m| m.is_file() && m.len() <= 4194304)
                            && fs::read(path)
                                .is_ok_and(|raw| format!("{:x}", Sha256::digest(raw)) == hash)
                    })
                })
            });
        counters
            && value["captured_posts"].as_array().is_some_and(|posts| {
                posts.iter().all(|p| {
                    serde_json::from_value::<SocialPost>(p.clone())
                        .is_ok_and(|p| self.verified_social_capture(&p, checked))
                })
            })
    }
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
    /// Hypotheses from a verified frozen snapshot; persisted as a zero-network PLAN_ONLY run.
    pub fn trend_hypothesize(&self, args: Value) -> Result<Value, String> {
        let id = args["snapshot_id"]
            .as_str()
            .ok_or("snapshot_id is required")?;
        let snapshot = self.trend_inspect(json!({"snapshot_id": id}))?;
        let candidates = self.candidates()?;
        let candidates = candidates.as_array().map(Vec::as_slice).unwrap_or(&[]);
        let mut out = crate::hypothesis::hypothesize(&snapshot, candidates);
        // Recurrence against the previous snapshot of the same query and capture mode: the
        // same lead again, and how many of its posts that snapshot had not captured. Posts
        // captured before are the same evidence seen twice, not continued attention.
        let prior: Option<Value> = {
            let db = self.db.lock().map_err(err)?;
            db.query_row(
                "SELECT payload FROM trend_snapshots WHERE query=?1 AND mode=?2 AND captured<?3 AND id<>?4 ORDER BY captured DESC LIMIT 1",
                rusqlite::params![
                    snapshot["query"].as_str().unwrap_or_default(),
                    snapshot["capture_mode"].as_str().unwrap_or_default(),
                    snapshot["captured_at"].as_u64().unwrap_or(0) as i64,
                    id
                ],
                |r| r.get::<_, String>(0),
            )
            .ok()
            .and_then(|p| serde_json::from_str(&p).ok())
        };
        crate::hypothesis::mark_recurrence(&mut out, &snapshot, prior.as_ref(), candidates);
        let price_runs: Vec<Value> = {
            let db = self.db.lock().map_err(err)?;
            let mut stmt = db
                .prepare("SELECT payload FROM price_observation_runs WHERE mode=?1 ORDER BY captured DESC LIMIT 200")
                .map_err(err)?;
            stmt.query_map(
                [snapshot["capture_mode"].as_str().unwrap_or_default()],
                |r| r.get::<_, String>(0),
            )
            .map_err(err)?
            .filter_map(Result::ok)
            .filter_map(|p| serde_json::from_str(&p).ok())
            .collect()
        };
        crate::hypothesis::attach_store_prices(&mut out, &price_runs);
        let run = self.persist(json!({"mode":"PLAN_ONLY","run_kind":"TREND_HYPOTHESIS","snapshot_id":id,"capture_mode":snapshot["capture_mode"],"hypotheses":out["hypotheses"],"observations":[],"cost_minor":0,"network_calls":0}))?;
        out["run_id"] = run["run_id"].clone();
        Ok(out)
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
            if !self.verified_social_snapshot(&value, &mut BTreeMap::new()) {
                return Err("SNAPSHOT_RAW_CAPTURE_HASH_UNAVAILABLE_OR_MISMATCH".into());
            }
            value["raw_capture_verification"] = json!("VERIFIED_LOCAL_SHA256_NO_NETWORK");
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
        let mut checked = BTreeMap::new();
        let mut unavailable = vec![];
        let snapshots: Vec<_> = rows?.into_iter().filter(|row| {
            let valid = self.verified_social_snapshot(row, &mut checked);
            if !valid { unavailable.push(json!({"snapshot_id":row["snapshot_id"],"state":"SOURCE_UNAVAILABLE","reason":"SNAPSHOT_RAW_CAPTURE_HASH_UNAVAILABLE_OR_MISMATCH"})); }
            valid
        }).collect();
        Ok(
            json!({"snapshots":snapshots,"unavailable_snapshots":unavailable,"raw_capture_verification":"VERIFIED_LOCAL_SHA256_NO_NETWORK","watches":watches?,"simulation":{"state":"SIMULATED","execution":"UNAVAILABLE_NO_SIMULATION_RUNTIME","observed_contribution":0},"platforms_declared":["HACKER_NEWS","BLUESKY","MASTODON","JSON_FEED","XML_FEED","MASTODON_TAG"],"donor_provenance":"research/commerce/social-capability-graph.json","license_review":"research/commerce/social-license-review.json"}),
        )
    }
    pub fn trend_compare(&self, args: Value) -> Result<Value, String> {
        let before =
            self.trend_inspect(json!({"snapshot_id":required(&args,"before_snapshot_id")?}))?;
        let after =
            self.trend_inspect(json!({"snapshot_id":required(&args,"after_snapshot_id")?}))?;
        if before["source_scope_version"] != 2
            || after["source_scope_version"] != 2
            || before["capture_mode"] != after["capture_mode"]
            || before["query"] != after["query"]
            || before["window_seconds"] != after["window_seconds"]
            || before["source_scope"] != after["source_scope"]
            || before["source_scope_version"] != after["source_scope_version"]
        {
            return Err("INCOMPATIBLE_TREND_SNAPSHOT_SCOPE".into());
        }
        Ok(
            json!({"before":before,"after":after,"mention_delta":after["mention_count"].as_i64().unwrap_or(0)-before["mention_count"].as_i64().unwrap_or(0),"state":"DERIVED","method":"CAPTURED_SAMPLE_WINDOW_DIFFERENCE_NOT_PLATFORM_TOTAL","simulation_contribution":0}),
        )
    }
    pub fn trend_discover(&self, args: Value) -> Result<Value, String> {
        let query = required(&args, "query")?.trim().to_owned();
        if query.len() > 500 || !super::searchable(&query) {
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
        let is_fixture =
            |s: &Value| s.get("fixture_raw").is_some() || s.get("fixture_slices").is_some();
        let fixture = sources.iter().any(is_fixture);
        if fixture && !sources.iter().all(is_fixture) {
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
        let proposals:Vec<_>=sources.iter().enumerate().map(|(i,s)|json!({"candidate_id":query,"action":"PUBLIC_SOCIAL_QUERY","provider":"native-social","class":"PUBLIC","target_unknown":"SOCIAL_TOPIC_EVIDENCE","evidence_gap":format!("{}:{}",query,source_key(s)),"source_group":source_key(s),"url":format!("source-{i}"),"expected_cost_minor":0,"expected_requests":if fixture||mode=="CACHED"{0}else{2},"expected_latency_ms":if fixture{1}else{1000},"uncertainty_reduction_points":10})).collect();
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
        let mut checked = BTreeMap::new();
        let author_key = super::pseudonym::installation_key(&self.root)?;
        let run_id = Uuid::new_v4().to_string();
        let mut paginations = vec![];
        // Source-reported counters that are not posts (a Mastodon tag's daily uses), each tied
        // to its raw capture.
        let mut counters = vec![];
        for (i, source) in sources.iter().enumerate() {
            if !selected.contains(format!("source-{i}").as_str()) {
                continue;
            }
            if !matches!(
                source["platform"].as_str(),
                Some(
                    "HACKER_NEWS"
                        | "BLUESKY"
                        | "MASTODON"
                        | "JSON_FEED"
                        | "XML_FEED"
                        | "MASTODON_TAG"
                )
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
                    if !posts
                        .iter()
                        .all(|p| self.verified_social_capture(p, &mut checked))
                    {
                        failures.push(json!({"platform":source["platform"],"state":"SOURCE_UNAVAILABLE","reason":"CACHE_CAPTURE_HASH_UNAVAILABLE_OR_MISMATCH","request_count":0}));
                        continue;
                    }
                    for p in &mut posts {
                        super::pseudonym::pseudonymise(p, &author_key);
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
            // A requested window may be split into time slices the source bounds itself; each
            // slice walks its own pages, one acquisition and one raw capture per page.
            // Live slices cost two requests each; when the requested width needs more slices than
            // this source may afford, they are widened so the whole window is still sampled.
            let affordable = if fixture {
                super::coverage::MAX_SLICES as u64
            } else {
                let reserved = (i + 1..sources.len())
                    .filter(|j| selected.contains(format!("source-{j}").as_str()))
                    .count() as u64
                    * 2;
                (request_budget as u64).saturating_sub(requests + reserved) / 2
            };
            let mut slice_plan = Value::Null;
            let planned: Vec<Option<(u64, u64)>> = match source["slice_seconds"].as_u64() {
                Some(width) => {
                    let used = super::coverage::fit_width(
                        now.saturating_sub(window),
                        now,
                        width,
                        affordable,
                    );
                    slice_plan = json!({"requested_seconds":width,"used_seconds":used,"widened":used > width.max(super::coverage::MIN_SLICE_SECONDS),"reason":if used > width.max(super::coverage::MIN_SLICE_SECONDS) {"WIDENED_TO_COVER_THE_WINDOW_WITHIN_REQUEST_BUDGET_OR_SLICE_CAP"} else {"AS_REQUESTED"}});
                    super::coverage::plan_slices(now.saturating_sub(window), now, used)
                        .into_iter()
                        .map(Some)
                        .collect()
                }
                None => vec![None],
            };
            let sliced = planned.iter().any(Option::is_some);
            // Time-uniform sampling: every slice gets the same page quota out of the requests
            // this source may spend, so a tight budget samples every day instead of letting the
            // newest slice take it all. Fixtures cost no requests and keep max_pages.
            let max_pages = source["max_pages"].as_u64().unwrap_or(1);
            let page_quota = if sliced && !fixture {
                let reserved = (i + 1..sources.len())
                    .filter(|j| selected.contains(format!("source-{j}").as_str()))
                    .count() as u64
                    * 2;
                let pages = (request_budget as u64).saturating_sub(requests + reserved) / 2;
                (pages / planned.len() as u64).clamp(1, max_pages.max(1))
            } else {
                max_pages
            };
            let mut source_posts: Vec<SocialPost> = vec![];
            let mut slices = vec![];
            for (slice_index, bounds) in planned.iter().enumerate() {
                let reserved = (i + 1..sources.len())
                    .filter(|j| selected.contains(format!("source-{j}").as_str()))
                    .count() as u64
                    * 2;
                if slice_index > 0 && !fixture && requests + 2 + reserved > request_budget as u64 {
                    slices.push(json!({"since":bounds.map(|b|b.0),"until":bounds.map(|b|b.1),"acquired":false,"stop":"NOT_ACQUIRED_REQUEST_BUDGET","pages":0,"posts":0}));
                    continue;
                }
                let fixture_body = |page: usize| -> Option<String> {
                    if sliced {
                        source["fixture_slices"][slice_index][page]
                            .as_str()
                            .map(str::to_string)
                    } else if page == 0 {
                        source["fixture_raw"].as_str().map(str::to_string)
                    } else {
                        source["fixture_pages"][page - 1]
                            .as_str()
                            .map(str::to_string)
                    }
                };
                let mut walk = super::pagination::PageWalk::new(page_quota);
                let mut cursor: Option<String> = None;
                let mut slice_posts: Vec<SocialPost> = vec![];
                let mut total = Value::Null;
                let stop: &str = loop {
                    let mut payload = source.clone();
                    if let Some(o) = payload.as_object_mut() {
                        for k in [
                            "max_pages",
                            "fixture_pages",
                            "fixture_slices",
                            "slice_seconds",
                            "reply_trees",
                            "max_threads",
                            "fixture_threads",
                        ] {
                            o.remove(k);
                        }
                    }
                    payload["query"] = json!(query);
                    payload["captured_at"] = json!(now);
                    if let Some((since, until)) = bounds {
                        payload["since"] = json!(since);
                        payload["until"] = json!(until);
                    }
                    if let Some(c) = &cursor {
                        payload["page_cursor"] = json!(c);
                    }
                    if fixture {
                        match fixture_body(walk.pages() as usize) {
                            Some(raw) => payload["fixture_raw"] = json!(raw),
                            None => break "FIXTURE_PAGE_MISSING",
                        }
                    }
                    let acquired = provider.acquire(&AcquireRequest {
                        run_id: run_id.clone(),
                        capability: "social.query".into(),
                        market: "PUBLIC_SOCIAL".into(),
                        query: payload,
                    });
                    match acquired {
                        Ok(acquired) => {
                            let count = acquired.provider_cost["request_count"].as_u64();
                            requests = requests.saturating_add(count.unwrap_or(0));
                            if (fixture && count != Some(0))
                                || (!fixture && count.is_none_or(|n| n == 0))
                            {
                                failures.push(json!({"platform":source["platform"],"state":"SOURCE_UNAVAILABLE","reason":"SOCIAL_CAPTURE_HTTP_WITNESS_MISSING_OR_MODE_MISMATCH","request_count":count}));
                                break "PAGE_FAILED";
                            }
                            let posts: Vec<SocialPost> =
                                serde_json::from_value(acquired.result["posts"].clone())
                                    .map_err(err)?;
                            let mut posts = posts;
                            for p in &mut posts {
                                super::pseudonym::pseudonymise(p, &author_key);
                            }
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
                            let rawdir = self.root.join(".ecdev-data/runtime/social-captures");
                            fs::create_dir_all(&rawdir).map_err(err)?;
                            fs::write(
                                rawdir.join(format!("{raw_hash}.raw")),
                                &acquired.raw_payload,
                            )
                            .map_err(err)?;
                            if total.is_null() {
                                total = acquired.result["source_total"].clone();
                            }
                            let usage = &acquired.result["tag_usage"];
                            if !usage.is_null() {
                                if usage["raw_hash"] != json!(raw_hash)
                                    || usage["capture_mode"] != json!(mode)
                                {
                                    return Err("SOCIAL_CAPTURE_HASH_MISMATCH".into());
                                }
                                let mut counter = usage.clone();
                                counter["platform"] = source["platform"].clone();
                                counter["source_group"] = json!(source_key(source));
                                counter["growth"] = super::growth::daily_attention_growth(usage);
                                counters.push(counter);
                            }
                            let next = acquired.result["pagination"]["next_cursor"].as_str();
                            let n = posts.len();
                            slice_posts.extend(posts);
                            match walk.after_page(n, next) {
                                Ok(c) => {
                                    // Every selected source still to come keeps its two requests.
                                    if !fixture && requests + 2 + reserved > request_budget as u64 {
                                        break "REQUEST_BUDGET";
                                    }
                                    cursor = Some(c);
                                }
                                Err(reason) => break reason,
                            }
                        }
                        Err(e) => {
                            requests = requests.saturating_add(e.request_count.unwrap_or(0));
                            // A source that serves a first page but refuses its own next cursor
                            // (Bluesky unauthenticated search) limits coverage; it is not an outage.
                            if walk.pages() >= 1 && e.http_status == Some(403) {
                                break "SOURCE_REFUSED_FURTHER_PAGES";
                            }
                            let state = match e.http_status {
                                Some(401) => "AUTH_REQUIRED",
                                Some(429) => "RATE_LIMITED",
                                Some(403) => "SOURCE_BLOCKED",
                                // The site's own policy (robots, AI-agent refusal, crawl delay).
                                _ if super::policy_refusal(&e.reason) => "SOURCE_BLOCKED",
                                _ => "SOURCE_UNAVAILABLE",
                            };
                            failures.push(json!({"platform":source["platform"],"state":state,"reason":e.reason,"http_status":e.http_status,"request_count":e.request_count,"retry_not_before_ms":e.retry_not_before_ms,"page":walk.pages() + 1,"slice":{"since":bounds.map(|b|b.0),"until":bounds.map(|b|b.1)}}));
                            break "PAGE_FAILED";
                        }
                    }
                };
                let times: Vec<u64> = slice_posts.iter().filter_map(|p| p.published_at).collect();
                slices.push(json!({"since":bounds.map(|b|b.0),"until":bounds.map(|b|b.1),"acquired":walk.pages() > 0,"stop":stop,"pages":walk.pages(),"posts":slice_posts.len(),"earliest":times.iter().min(),"latest":times.iter().max(),"source_total":total}));
                source_posts.extend(slice_posts);
            }
            // Reply trees of the roots with the most reported comments, one acquisition and one
            // raw capture per thread, inside the same budget.
            let mut threads = vec![];
            if source["reply_trees"] == true {
                let mut roots: Vec<&SocialPost> = source_posts
                    .iter()
                    .filter(|p| {
                        p.parent_id.is_none() && p.engagement.comments.is_some_and(|n| n > 0)
                    })
                    .collect();
                roots.sort_by(|a, b| {
                    b.engagement
                        .comments
                        .cmp(&a.engagement.comments)
                        .then(a.native_id.cmp(&b.native_id))
                });
                let max_threads = source["max_threads"].as_u64().unwrap_or(3).clamp(1, 10) as usize;
                let roots: Vec<(String, Option<u64>, String, String)> = roots
                    .into_iter()
                    .take(max_threads)
                    .map(|p| {
                        (
                            p.native_id.clone(),
                            p.engagement.comments,
                            p.raw_hash.clone(),
                            p.raw_locator.clone(),
                        )
                    })
                    .collect();
                let reserved = (i + 1..sources.len())
                    .filter(|j| selected.contains(format!("source-{j}").as_str()))
                    .count() as u64
                    * 2;
                let mut thread_posts = vec![];
                for (root, reported, root_hash, root_locator) in roots {
                    if !fixture && requests + 2 + reserved > request_budget as u64 {
                        threads.push(json!({"root":root,"reported_comments":reported,"state":"NOT_ACQUIRED_REQUEST_BUDGET"}));
                        continue;
                    }
                    let mut payload = json!({"platform":source["platform"],"url":source["url"],"query":query,"captured_at":now,"thread_of":root});
                    if source["platform"] == "MASTODON" {
                        // Context takes the instance-local id, read from the root's own capture.
                        let local = fs::read(
                            self.root
                                .join(".ecdev-data/runtime/social-captures")
                                .join(format!("{root_hash}.raw")),
                        )
                        .ok()
                        .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
                        .and_then(|v| v.pointer(&root_locator)?["id"].as_str().map(str::to_string));
                        let Some(local) = local else {
                            threads.push(json!({"root":root,"reported_comments":reported,"state":"THREAD_FAILED","reason":"ROOT_LOCAL_ID_NOT_IN_CAPTURE"}));
                            continue;
                        };
                        payload["thread_of"] = json!(local);
                        payload["thread_root_uri"] = json!(root);
                        if let Some(i) = source.get("instance") {
                            payload["instance"] = i.clone();
                        }
                    }
                    if fixture {
                        match source["fixture_threads"][root.as_str()].as_str() {
                            Some(raw) => payload["fixture_raw"] = json!(raw),
                            None => {
                                threads.push(json!({"root":root,"reported_comments":reported,"state":"FIXTURE_THREAD_MISSING"}));
                                continue;
                            }
                        }
                    }
                    match provider.acquire(&AcquireRequest {
                        run_id: run_id.clone(),
                        capability: "social.query".into(),
                        market: "PUBLIC_SOCIAL".into(),
                        query: payload,
                    }) {
                        Ok(acquired) => {
                            let count = acquired.provider_cost["request_count"].as_u64();
                            requests = requests.saturating_add(count.unwrap_or(0));
                            if (fixture && count != Some(0))
                                || (!fixture && count.is_none_or(|n| n == 0))
                            {
                                threads.push(json!({"root":root,"state":"THREAD_FAILED","reason":"SOCIAL_CAPTURE_HTTP_WITNESS_MISSING_OR_MODE_MISMATCH"}));
                                continue;
                            }
                            let posts: Vec<SocialPost> =
                                serde_json::from_value(acquired.result["posts"].clone())
                                    .map_err(err)?;
                            let mut posts = posts;
                            for p in &mut posts {
                                super::pseudonym::pseudonymise(p, &author_key);
                            }
                            let raw_hash = format!("{:x}", Sha256::digest(&acquired.raw_payload));
                            for p in &posts {
                                p.validate()?;
                                if p.capture_mode != mode || p.raw_hash != raw_hash {
                                    return Err("SOCIAL_CAPTURE_HASH_MISMATCH".into());
                                }
                            }
                            let rawdir = self.root.join(".ecdev-data/runtime/social-captures");
                            fs::create_dir_all(&rawdir).map_err(err)?;
                            fs::write(
                                rawdir.join(format!("{raw_hash}.raw")),
                                &acquired.raw_payload,
                            )
                            .map_err(err)?;
                            let tree = &acquired.result["reply_tree"];
                            let comments = posts
                                .iter()
                                .filter(|p| p.depth.is_some_and(|d| d > 0))
                                .count();
                            threads.push(json!({"root":root,"reported_comments":reported,"comments_observed":comments,"nodes":tree["nodes"],"deleted":tree["deleted"],"not_found":tree["not_found"],"blocked":tree["blocked"],"max_depth":tree["max_depth"],"depth_cut_nodes":tree["depth_cut_nodes"],"state":tree["state"],"raw_hash":raw_hash}));
                            thread_posts.extend(
                                posts.into_iter().filter(|p| p.depth.is_some_and(|d| d > 0)),
                            );
                        }
                        Err(e) => {
                            requests = requests.saturating_add(e.request_count.unwrap_or(0));
                            threads.push(json!({"root":root,"reported_comments":reported,"state":if e.reason == "SOURCE_DOES_NOT_EXPOSE_TREE" {"SOURCE_DOES_NOT_EXPOSE_TREE"} else {"THREAD_FAILED"},"reason":e.reason,"http_status":e.http_status}));
                        }
                    }
                }
                source_posts.extend(thread_posts);
            }
            let coverage = super::coverage::population_coverage(
                now.saturating_sub(window),
                now,
                sliced,
                &slices,
            );
            let mut coverage = coverage;
            if source["reply_trees"] == true {
                let states: Vec<&str> =
                    threads.iter().filter_map(|t| t["state"].as_str()).collect();
                let tree_state =
                    if source["platform"] == "JSON_FEED" || source["platform"] == "XML_FEED" {
                        "SOURCE_DOES_NOT_EXPOSE_TREE"
                    } else if states.is_empty() {
                        "NO_ROOTS_WITH_REPORTED_COMMENTS"
                    } else if states.iter().all(|s| *s == "COMPLETE_BY_SOURCE") {
                        "COMPLETE_BY_SOURCE"
                    } else {
                        "PARTIAL_COMMENT_TREE"
                    };
                coverage["comments_observed"] = json!(
                    threads
                        .iter()
                        .filter_map(|t| t["comments_observed"].as_u64())
                        .sum::<u64>()
                );
                coverage["threads_requested"] = json!(threads.len());
                coverage["comment_tree_state"] = json!(tree_state);
                coverage["comment_tree_complete"] = json!(tree_state == "COMPLETE_BY_SOURCE");
                if coverage["state"] == "COMPLETE_BY_SOURCE" && tree_state == "PARTIAL_COMMENT_TREE"
                {
                    coverage["state"] = json!("PARTIAL_COMMENT_TREE");
                }
            } else {
                coverage["comment_tree_state"] = json!("NOT_REQUESTED");
            }
            let pages: u64 = slices.iter().filter_map(|s| s["pages"].as_u64()).sum();
            paginations.push(json!({"platform":source["platform"],"source_group":source_key(source),"pages":pages,"stop":if slices.len() == 1 {slices[0]["stop"].clone()} else {json!("SLICED")},"posts":source_posts.len(),"slices":slices,"threads":threads,"page_quota_per_slice":page_quota,"sampling":if sliced {"TIME_UNIFORM_EQUAL_PAGE_QUOTA_PER_SLICE"} else {"SINGLE_WINDOW"},"slice_plan":slice_plan,"population_coverage":coverage}));
            if !fixture && !source_posts.is_empty() {
                self.db
                    .lock()
                    .map_err(err)?
                    .execute(
                        "INSERT OR REPLACE INTO social_cache VALUES(?1,?2,?3)",
                        params![key, now, serde_json::to_string(&source_posts).map_err(err)?],
                    )
                    .map_err(err)?;
            }
            captured.extend(source_posts);
        }
        // LIVE snapshots describe the completed acquisition, rather than its start time.
        let now = if mode == "LIVE" { timestamp() } else { now };
        let selected_scope: BTreeSet<String> = sources.iter().map(source_key).collect();
        let source_scope = json!(selected_scope);
        let mut posts = self.social_rows(mode)?;
        // Rows stored before pseudonyms existed are pseudonymised on read; idempotent.
        for p in &mut posts {
            super::pseudonym::pseudonymise(p, &author_key);
        }
        posts.extend(captured.clone());
        posts.retain(|post| selected_scope.contains(&post_source_key(post)));
        let mut invalid = BTreeSet::new();
        posts.retain(|post| {
            let valid = self.verified_social_capture(post, &mut checked);
            if !valid && invalid.insert(post.raw_hash.clone()) {
                failures.push(json!({"platform":post.platform,"state":"SOURCE_UNAVAILABLE","reason":"HISTORICAL_CAPTURE_HASH_UNAVAILABLE_OR_MISMATCH","raw_hash":post.raw_hash,"request_count":0}));
            }
            valid
        });
        let allocation_complete = allocation["skipped"].as_array().is_some_and(|rows| {
            rows.iter()
                .all(|row| row["reason"] == "REDUNDANT_EVIDENCE_GAP_SOURCE")
        });
        let acquisition_complete = failures.is_empty() && allocation_complete;
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
                    && previous["source_scope_version"] == 2
                    && self.verified_social_snapshot(&previous, &mut checked)
                {
                    history.push(previous);
                }
            }
        }
        history.reverse();
        let mut snap = snapshot(&posts, &history, &query, now, window, mode);
        let snapshot_id = Uuid::new_v4().to_string();
        snap["source_scope"] = source_scope;
        snap["source_scope_version"] = json!(2);
        snap["snapshot_id"] = json!(snapshot_id);
        snap["run_id"] = json!(run_id);
        snap["raw_capture_verification"] = json!("VERIFIED_LOCAL_SHA256_NO_NETWORK");
        snap["acquisition_provenance"] = json!({"requested_mode":mode,"new_observation_count":if mode=="CACHED"{0}else{captured.len()},"new_live_observation_count":if mode=="LIVE" && requests>0{captured.len()}else{0},"live_acquisition_established":mode=="LIVE" && requests>0 && !captured.is_empty(),"historical_projection":"VALIDATED_RAW_CAPTURE_ONLY","cache_projection_is_new_live_acquisition":false});
        snap["provider_failures"] = json!(failures);
        snap["pagination"] = json!(paginations);
        snap["population_evidence"] = super::coverage::population_evidence(&paginations, mode);
        // Mention counts of capped or partial acquisitions move with the cap and the window,
        // not with volume: velocity between them is a sampling difference. Posts published
        // since the prior capture and absent from it are an arrival count, exact only when
        // both acquisitions read their sources to the end, a lower bound otherwise.
        if let Some(prior) = history.last() {
            let complete = |s: &Value| s["population_evidence"]["grade"] == "COMPLETE_BY_SOURCE";
            let comparable = complete(&snap) && complete(prior);
            let comparability = json!(if comparable {
                "COMPARABLE_BOTH_COMPLETE_BY_SOURCE"
            } else {
                "NOT_COMPARABLE_CAPPED_OR_PARTIAL_COUNTS"
            });
            // Acceleration is built from velocities, and the score from both: each says so.
            snap["velocity"]["count_comparability"] = comparability.clone();
            snap["acceleration"]["count_comparability"] = comparability.clone();
            snap["score"]["velocity_count_comparability"] = comparability;
            let prior_at = prior["captured_at"].as_u64().unwrap_or(now);
            let prior_keys: BTreeSet<&str> = prior["post_keys"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let arrived = posts
                .iter()
                .filter(|p| p.published_at.is_some_and(|t| t > prior_at && t <= now))
                .filter(|p| !prior_keys.contains(p.key().as_str()))
                .map(|p| p.key())
                .collect::<BTreeSet<_>>()
                .len();
            let hours = now.saturating_sub(prior_at) as f64 / 3600.;
            snap["arrivals_since_prior_snapshot"] = json!({"prior_snapshot_id":prior["snapshot_id"],"prior_captured_at":prior_at,"posts":arrived,
                "per_hour":(hours > 0.).then(|| arrived as f64 / hours),
                "bound":if comparable {"EXACT_BY_SOURCE"} else {"LOWER_BOUND_CAPPED_OR_PARTIAL"},
                "basis":"POSTS_PUBLISHED_AFTER_PRIOR_CAPTURE_AND_ABSENT_FROM_IT"});
        }
        snap["source_counters"] = json!(counters);
        snap["budget_usage"] = json!({"cost_minor":0,"request_count":if failures.iter().any(|f|f.get("request_count").is_some_and(Value::is_null)){Value::Null}else{json!(requests)},"known_request_count":requests,"request_budget":request_budget,"cache_hits":hits,"paid_budget_minor":0,"paid_execution":"NOT_IMPLEMENTED_PAID_PROPOSALS_ONLY","allocation":allocation});
        let acquisition_mode = if mode != "LIVE" || requests > 0 {
            mode
        } else if snap["budget_usage"]["request_count"].is_null() {
            "INFERRED"
        } else {
            "PLAN_ONLY"
        };
        snap["acquisition_provenance"]["actual_run_mode"] = json!(acquisition_mode);
        snap["acquisition_provenance"]["live_io_established"] =
            json!(mode == "LIVE" && requests > 0);
        snap["acquisition_provenance"]["egress"] = self
            .providers
            .iter()
            .find(|p| p.id() == "native-social")
            .map_or(Value::Null, |p| p.metadata()["egress"].clone());
        snap["acquisition_provenance"]["capture_mode_scope"] =
            json!("REQUESTED_SOURCE_COHORT_HISTORICAL_POSTS_KEEP_ORIGINAL_CAPTURE_MODE");
        snap["source_complete"] = json!(
            snap["provider_failures"]
                .as_array()
                .is_some_and(Vec::is_empty)
                && allocation_complete
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
        let qterms = terms(&query);
        // Only matching candidates can contribute a topic link. Validate their capture hashes
        // before exposing a link, without hashing unrelated commercial captures on each trend read.
        let candidates = if snap["evidence_ids"]
            .as_array()
            .is_some_and(|ids| !ids.is_empty())
        {
            self.verified_candidates_where(|c| {
                let text = ["title", "brand", "category"]
                    .iter()
                    .filter_map(|key| c["product"][key].as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                !qterms.is_disjoint(&terms(&text))
            })?
        } else {
            json!([])
        };
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
        let run=self.persist(json!({"acquisition_run_id":run_id,"mode":acquisition_mode,"requested_mode":mode,"run_kind":"SOCIAL_TREND","social_run":true,"status":if snap["source_complete"]==true{"COMPLETE"}else{"PARTIAL"},"snapshot_id":snapshot_id,"observations":captured.iter().map(|p|json!({"id":p.evidence_id,"mode":mode,"source_type":"PUBLIC_SOCIAL_JSON","provider":p.provider,"external_source":p.source_url,"market":"PUBLIC_SOCIAL","query":query,"timestamp":p.published_at,"retrieved_at":p.captured_at,"raw_hash":p.raw_hash,"raw_locator":p.raw_locator,"normalized_value":p,"state":"OBSERVED","run_id":run_id})).collect::<Vec<_>>(),"cost_minor":0,"provider_failures":snap["provider_failures"],"budget_usage":snap["budget_usage"],"network_calls":snap["budget_usage"]["request_count"],"known_network_calls":requests,"acquisition_provenance":snap["acquisition_provenance"]}))?;
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
        if query.len() > 500 || !super::searchable(query) {
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
        let value = json!({"watch_id":id,"query":query,"research":research,"triggers":triggers,"interval_seconds":interval,"minimum_mentions":args["minimum_mentions"].as_u64().unwrap_or(3).max(2),"threshold":args["threshold"].as_f64().unwrap_or(1.).max(0.),"volume_multiplier":args["volume_multiplier"].as_f64().unwrap_or(2.).max(0.),"sentiment_drop":args["sentiment_drop"].as_f64().unwrap_or(0.5).max(0.),"enabled":args["enabled"].as_bool().unwrap_or(true),"baseline":if same{old["baseline"].clone()}else{Value::Null},"last_snapshot":if same{old["last_snapshot"].clone()}else{Value::Null},"events":if same{old["events"].clone()}else{json!([])},"lease_state":"IDLE","notification_policy":"PERSIST_LOCAL_EVENTS_ONLY_MINIMUM_SAMPLE_AND_COMPLETE_ACQUISITION"});
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
        || before["source_scope"] != after["source_scope"]
        || before["source_scope_version"] != after["source_scope_version"]
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
            // harken thresholds.py semantics (parity: social.rs::independent_social_donor_oracles):
            // the previous complete snapshot is the baseline window.
            "VOLUME_SPIKE" | "SENTIMENT_DROP" => {
                let events = crate::social::threshold_events(&json!({
                    "metrics": {
                        "current_count": n, "baseline_count": old, "baseline_average": old,
                        "current_net_sentiment": sentiment_net(after),
                        "baseline_net_sentiment": sentiment_net(before),
                    },
                    "minimum_mentions": min,
                    "volume_multiplier": policy["volume_multiplier"].as_f64().unwrap_or(2.),
                    "sentiment_drop": policy["sentiment_drop"].as_f64().unwrap_or(0.5),
                }));
                events[if t == "VOLUME_SPIKE" {
                    "volume_spike"
                } else {
                    "sentiment_drop"
                }] == true
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
