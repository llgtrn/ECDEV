//! Instance-wide trending lists (Mastodon trends/tags, Bluesky getTrends) read whole as
//! discovery feeds. An entry is a lead to research with a suggested query: never a mention, a
//! product measurement or demand, and ranked by an algorithm the source does not disclose.
use super::growth::daily_attention_growth;
use crate::{Engine, provider::AcquireRequest, service::timestamp};
use rusqlite::params;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs};
use uuid::Uuid;

pub const FEED_PLATFORMS: [&str; 3] = ["MASTODON_TRENDS", "BLUESKY_TRENDS", "GOOGLE_TRENDS_RSS"];

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub fn tool_definition() -> Value {
    let source = json!({"type":"object","properties":{"platform":{"enum":FEED_PLATFORMS},"instance":{"type":"string","maxLength":253,"description":"MASTODON_TRENDS only: public instance (default mastodon.social)"},"geo":{"type":"string","pattern":"^[A-Z]{2}$","description":"GOOGLE_TRENDS_RSS only: country of the daily search trends (JP, US...); entries carry an approximate search traffic band, search interest not demand"},"fixture_raw":{"type":"string","maxLength":4194304}},"required":["platform"],"additionalProperties":false});
    json!({"name":"ecdev.trend.feeds","description":"Read public trending lists (Mastodon trending tags, Bluesky trends, Google daily search trends by country) as discovery leads with suggested queries and next trend.discover actions; zero paid, raw captures hashed and kept, no posters kept; leads are never mentions or demand","inputSchema":{"type":"object","properties":{"sources":{"type":"array","items":source,"minItems":1,"maxItems":3},"request_budget":{"type":"integer","minimum":0,"maximum":6},"fixture_now":{"type":"integer","minimum":0}},"required":["sources"],"additionalProperties":false}})
}

fn ranks_of(feed: &Value) -> BTreeMap<String, u64> {
    feed["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| Some((entry_key(e)?, e["rank"].as_u64()?)))
        .collect()
}

pub fn initialize(db: &rusqlite::Connection) -> Result<(), String> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS trend_feeds(id TEXT PRIMARY KEY,mode TEXT NOT NULL,captured INTEGER NOT NULL,payload TEXT NOT NULL);")
        .map_err(err)
}

/// The identity of a trending-list entry across captures: the Mastodon tag, or the Bluesky
/// topic id (its display label when the source gives none).
fn entry_key(entry: &Value) -> Option<String> {
    entry["tag"]
        .as_str()
        .or(entry["topic"].as_str())
        .or(entry["label"].as_str())
        .map(str::to_lowercase)
}

/// Rank history of every entry in the latest capture of one feed, across that feed's earlier
/// successful captures (oldest first, the latest last). Only successful captures count: a failed
/// read is no capture, never an absence. A list shows its top N only, so an entry missing from a
/// capture is OFF_TOP_N_IN_CAPTURE, not gone. Every rank is kept, repeats included, and identity
/// does not reset at day boundaries.
pub fn rank_persistence(captures: &[(u64, BTreeMap<String, u64>)]) -> BTreeMap<String, Value> {
    let Some((latest_at, latest)) = captures.last() else {
        return BTreeMap::new();
    };
    let mut out = BTreeMap::new();
    for key in latest.keys() {
        let first = captures
            .iter()
            .position(|(_, ranks)| ranks.contains_key(key))
            .unwrap_or(captures.len() - 1);
        let span = &captures[first..];
        let ranks: Vec<Value> = span
            .iter()
            .map(|(at, r)| json!({"captured_at":at,"rank":r.get(key)}))
            .collect();
        let present: Vec<bool> = span.iter().map(|(_, r)| r.contains_key(key)).collect();
        let re_entries = present.windows(2).filter(|w| !w[0] && w[1]).count();
        let streak = present.iter().rev().take_while(|p| **p).count();
        let streak_start = span[span.len() - streak].0;
        let best = span.iter().filter_map(|(_, r)| r.get(key)).min();
        let state = if span.len() == 1 {
            "NEW_ON_LIST"
        } else if re_entries > 0 && streak == 1 {
            "RE_ENTERED"
        } else {
            "SUSTAINED"
        };
        out.insert(key.clone(), json!({"state":state,"first_seen":span[0].0,"last_seen":latest_at,"captures_since_first_seen":span.len(),"captures_on_list":present.iter().filter(|p| **p).count(),"current_streak_captures":streak,"current_streak_seconds":latest_at - streak_start,"re_entries":re_entries,"best_rank":best,"rank_history":ranks,"absence_meaning":"OFF_TOP_N_IN_CAPTURE","failed_reads":"NOT_CAPTURES_NEVER_ABSENCE"}));
    }
    out
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
                if platform == "GOOGLE_TRENDS_RSS" && source["geo"] == "JP" {
                    entry["listing_action"] = json!({"tool":"ecdev.listing.search","input_template":{"market":"YAHOO_SHOPPING_JP","query":entry["suggested_query"]},"requires":"YAHOO_SHOPPING_APP_ID","paid":false});
                }
                entry["next_action"] = json!({"tool":"ecdev.trend.discover","input_template":{"query":entry["suggested_query"],"sources":follow},"paid":false});
            }
            // Cross-run identity: this feed's earlier successful captures in the same mode.
            let scope = feed["source_url"].clone();
            let mut captures: Vec<(u64, BTreeMap<String, u64>)> = vec![];
            {
                let db = self.db.lock().map_err(err)?;
                let mut stmt = db
                    .prepare("SELECT payload FROM (SELECT payload,captured,rowid AS r FROM trend_feeds WHERE mode=?1 AND captured<=?2 ORDER BY captured DESC,rowid DESC LIMIT 500) ORDER BY captured,r")
                    .map_err(err)?;
                for row in stmt
                    .query_map(params![mode, now], |r| r.get::<_, String>(0))
                    .map_err(err)?
                {
                    let past: Value = serde_json::from_str(&row.map_err(err)?).map_err(err)?;
                    for f in past["feeds"].as_array().into_iter().flatten() {
                        if f["source_url"] == scope {
                            captures.push((f["captured_at"].as_u64().unwrap_or(0), ranks_of(f)));
                        }
                    }
                }
            }
            captures.push((now, ranks_of(&feed)));
            let history = rank_persistence(&captures);
            for entry in feed["entries"].as_array_mut().into_iter().flatten() {
                if let Some(h) = entry_key(entry).and_then(|k| history.get(&k)) {
                    entry["key"] = json!(entry_key(entry));
                    entry["list_persistence"] = h.clone();
                }
            }
            feed["prior_captures_of_this_feed"] = json!(captures.len() - 1);
            feeds.push(feed);
        }
        let id = Uuid::new_v4().to_string();
        let out = json!({"feed_id":id,"capture_mode":mode,"captured_at":now,"egress":provider.metadata()["egress"],"feeds":feeds,"provider_failures":failures,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn capture(at: u64, keys: &[&str]) -> (u64, BTreeMap<String, u64>) {
        (
            at,
            keys.iter()
                .enumerate()
                .map(|(i, k)| (k.to_string(), i as u64 + 1))
                .collect(),
        )
    }

    #[test]
    fn list_presence_survives_days_and_absence_is_only_off_top_n() {
        let day = 86_400;
        let caps = vec![
            capture(0, &["matcha", "ogre"]),
            capture(day, &["ogre", "matcha"]),
            capture(2 * day, &["ogre"]),
            capture(3 * day, &["hojicha", "matcha", "ogre"]),
        ];
        let h = rank_persistence(&caps);
        let m = &h["matcha"];
        assert_eq!(m["state"], "RE_ENTERED");
        assert_eq!(
            (
                m["first_seen"].clone(),
                m["captures_since_first_seen"].clone(),
                m["captures_on_list"].clone()
            ),
            (json!(0), json!(4), json!(3))
        );
        assert_eq!(m["re_entries"], 1);
        assert_eq!(m["best_rank"], 1);
        assert_eq!(
            m["rank_history"][2]["rank"],
            Value::Null,
            "off the top N in that capture"
        );
        let o = &h["ogre"];
        assert_eq!(o["state"], "SUSTAINED");
        assert_eq!(
            (
                o["current_streak_captures"].clone(),
                o["current_streak_seconds"].clone()
            ),
            (json!(4), json!(3 * day))
        );
        assert_eq!(
            o["rank_history"].as_array().unwrap().len(),
            4,
            "repeated ranks are kept"
        );
        assert_eq!(h["hojicha"]["state"], "NEW_ON_LIST");
        assert!(rank_persistence(&[]).is_empty());
    }
}
