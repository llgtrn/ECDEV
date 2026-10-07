//! Instance-wide trending lists (Mastodon trends/tags, Bluesky getTrends) read whole as
//! discovery feeds. An entry is a lead to research with a suggested query: never a mention, a
//! product measurement or demand, and ranked by an algorithm the source does not disclose.
use super::growth::daily_attention_growth;
use crate::{Engine, provider::AcquireRequest, service::timestamp};
use rusqlite::params;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use uuid::Uuid;

pub const FEED_PLATFORMS: [&str; 2] = ["MASTODON_TRENDS", "BLUESKY_TRENDS"];

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub fn tool_definition() -> Value {
    let source = json!({"type":"object","properties":{"platform":{"enum":FEED_PLATFORMS},"instance":{"type":"string","maxLength":253,"description":"MASTODON_TRENDS only: public instance (default mastodon.social)"},"fixture_raw":{"type":"string","maxLength":4194304}},"required":["platform"],"additionalProperties":false});
    json!({"name":"ecdev.trend.feeds","description":"Read public trending lists (Mastodon trending tags, Bluesky trends) as discovery leads with suggested queries and next trend.discover actions; zero paid, raw captures hashed and kept, no posters kept; leads are never mentions or demand","inputSchema":{"type":"object","properties":{"sources":{"type":"array","items":source,"minItems":1,"maxItems":3},"request_budget":{"type":"integer","minimum":0,"maximum":6},"fixture_now":{"type":"integer","minimum":0}},"required":["sources"],"additionalProperties":false}})
}

pub fn initialize(db: &rusqlite::Connection) -> Result<(), String> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS trend_feeds(id TEXT PRIMARY KEY,mode TEXT NOT NULL,captured INTEGER NOT NULL,payload TEXT NOT NULL);")
        .map_err(err)
}

impl Engine {
    pub fn trend_feeds(&self, args: Value) -> Result<Value, String> {
        let sources = args["sources"].as_array().ok_or("sources array required")?;
        if sources.is_empty() || sources.len() > 3 {
            return Err("ONE_TO_THREE_FEED_SOURCES_REQUIRED".into());
        }
        let fixture = sources.iter().any(|s| s.get("fixture_raw").is_some());
        if fixture && !sources.iter().all(|s| s.get("fixture_raw").is_some()) {
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
        let mode = if fixture { "FIXTURE" } else { "LIVE" };
        let budget = args["request_budget"].as_u64().unwrap_or(6).min(6);
        let provider = self
            .providers
            .iter()
            .find(|p| p.id() == "native-social")
            .ok_or("NATIVE_SOCIAL_PROVIDER_NOT_CONFIGURED")?;
        if provider.metadata()["class"] != "PUBLIC" || provider.metadata()["cost_minor"] != 0 {
            return Err("PAID_OR_UNKNOWN_PROVIDER_COST_DENIED_BEFORE_IO".into());
        }
        let run_id = Uuid::new_v4().to_string();
        let (mut feeds, mut failures, mut requests) = (vec![], vec![], 0u64);
        for source in sources {
            let platform = source["platform"].as_str().unwrap_or("");
            if !FEED_PLATFORMS.contains(&platform) {
                failures.push(json!({"platform":source["platform"],"state":"SOURCE_UNAVAILABLE","reason":"NOT_A_TREND_FEED"}));
                continue;
            }
            if !fixture && requests + 2 > budget {
                failures.push(json!({"platform":platform,"state":"NOT_ACQUIRED_REQUEST_BUDGET"}));
                continue;
            }
            let mut payload = source.clone();
            payload["captured_at"] = json!(now);
            let acquired = match provider.acquire(&AcquireRequest {
                run_id: run_id.clone(),
                capability: "social.query".into(),
                market: "PUBLIC_SOCIAL".into(),
                query: payload,
            }) {
                Ok(a) => a,
                Err(e) => {
                    requests = requests.saturating_add(e.request_count.unwrap_or(0));
                    failures.push(json!({"platform":platform,"state":match e.http_status {Some(429) => "RATE_LIMITED", Some(403) => "SOURCE_BLOCKED", _ => "SOURCE_UNAVAILABLE"},"reason":e.reason,"http_status":e.http_status,"request_count":e.request_count}));
                    continue;
                }
            };
            let count = acquired.provider_cost["request_count"].as_u64();
            requests = requests.saturating_add(count.unwrap_or(0));
            let mut feed = acquired.result["trend_feed"].clone();
            let raw_hash = format!("{:x}", Sha256::digest(&acquired.raw_payload));
            if (fixture && count != Some(0))
                || (!fixture && count.is_none_or(|n| n == 0))
                || feed["raw_hash"] != json!(raw_hash)
                || feed["capture_mode"] != json!(mode)
            {
                failures.push(json!({"platform":platform,"state":"SOURCE_UNAVAILABLE","reason":"FEED_CAPTURE_WITNESS_MISSING_OR_MODE_MISMATCH"}));
                continue;
            }
            let rawdir = self.root.join(".ecdev-data/runtime/social-captures");
            fs::create_dir_all(&rawdir).map_err(err)?;
            fs::write(
                rawdir.join(format!("{raw_hash}.raw")),
                &acquired.raw_payload,
            )
            .map_err(err)?;
            for entry in feed["entries"].as_array_mut().into_iter().flatten() {
                if entry["series"].is_array() {
                    entry["growth"] = daily_attention_growth(entry);
                }
                let mut follow = vec![
                    json!({"platform":"HACKER_NEWS"}),
                    json!({"platform":"BLUESKY"}),
                ];
                if platform == "MASTODON_TRENDS" {
                    let mut tag = json!({"platform":"MASTODON_TAG"});
                    if let Some(i) = source.get("instance") {
                        tag["instance"] = i.clone();
                    }
                    follow.insert(0, tag);
                }
                entry["state"] = json!("DISCOVERY_LEAD_UNVERIFIED");
                entry["next_action"] = json!({"tool":"ecdev.trend.discover","input_template":{"query":entry["suggested_query"],"sources":follow},"paid":false});
            }
            feeds.push(feed);
        }
        let id = Uuid::new_v4().to_string();
        let out = json!({"feed_id":id,"capture_mode":mode,"captured_at":now,"feeds":feeds,"provider_failures":failures,
            "budget_usage":{"cost_minor":0,"request_count":requests,"request_budget":budget},
            "invariants":["a trending entry is a lead to research, never a mention, a product signal or demand","source rankings are undisclosed algorithms","who posted is not kept"]});
        self.db
            .lock()
            .map_err(err)?
            .execute(
                "INSERT INTO trend_feeds VALUES(?1,?2,?3,?4)",
                params![id, mode, now, out.to_string()],
            )
            .map_err(err)?;
        Ok(out)
    }
}
