//! ECDEV-owned durable frontier. SQLite transactions fence workers across processes.
//! Time is supplied in Unix milliseconds so scheduling is reproducible in tests.
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, path::Path, time::Duration};
use url::Url;
use uuid::Uuid;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Defaults preserve identity-bearing parameters and repeated-slash path semantics.
/// Sorting is opt-in: some sites treat repeated query parameter order as significant.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UrlPolicy {
    pub sort_query: bool,
    pub collapse_slashes: bool,
    pub tracking_parameters: BTreeSet<String>,
    pub identity_parameters: BTreeSet<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UrlIdentity {
    pub raw_url: String,
    pub canonical_url: String,
    pub identity_hash: String,
    pub origin: String,
    pub source_page: Option<String>,
    pub discovered_at: i64,
}
/// The longest URL ECDEV will name, stored or request (the bound robots evaluation already
/// applies). Scrapy's URLLENGTH_LIMIT is 2,083; measured, the longest of 2,611 live page links
/// and 646 post links was 247 characters, so this bounds abuse and never real links.
pub const MAX_URL_BYTES: usize = 8192;

pub fn canonicalize(
    raw: &str,
    source: Option<&str>,
    policy: &UrlPolicy,
    now: i64,
) -> Result<UrlIdentity, String> {
    if raw.len() > MAX_URL_BYTES {
        return Err("URL_TOO_LONG".into());
    }
    let mut url = match source {
        Some(base) => Url::parse(base).map_err(err)?.join(raw).map_err(err)?,
        None => Url::parse(raw).map_err(err)?,
    };
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("INVALID_PUBLIC_URL".into());
    }
    url.set_fragment(None);
    if policy.collapse_slashes {
        let mut path = url.path().to_owned();
        while path.contains("//") {
            path = path.replace("//", "/");
        }
        url.set_path(&path);
    }
    // Preserve the encoded original query unless policy explicitly asks for mutation.
    if policy.sort_query || !policy.tracking_parameters.is_empty() {
        let mut pairs: Vec<(String, String)> = url
            .query_pairs()
            .filter(|(k, _)| {
                policy.identity_parameters.contains(k.as_ref())
                    || !policy.tracking_parameters.contains(k.as_ref())
            })
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        if policy.sort_query {
            pairs.sort();
        }
        url.set_query(None);
        if !pairs.is_empty() {
            url.query_pairs_mut().extend_pairs(pairs);
        }
    }
    // Uppercase escapes and decode only RFC3986 unreserved bytes. Encoded '/' stays encoded.
    let canonical_url = normalize_escapes(url.as_str());
    let origin = url.origin().ascii_serialization();
    Ok(UrlIdentity {
        raw_url: raw.into(),
        identity_hash: format!("{:x}", Sha256::digest(canonical_url.as_bytes())),
        canonical_url,
        origin,
        source_page: source.map(str::to_owned),
        discovered_at: now,
    })
}
fn normalize_escapes(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(n) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            if n.is_ascii_alphanumeric() || b"-._~".contains(&n) {
                out.push(n as char);
            } else {
                out.push_str(&format!("%{n:02X}"));
            }
            i += 3;
            continue;
        }
        let c = s[i..].chars().next().expect("character boundary");
        out.push(c);
        i += c.len_utf8();
    }
    out
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrawlLimits {
    pub max_pages: u32,
    pub max_urls: u32,
    pub max_depth: u32,
    pub global_concurrency: u32,
    pub per_origin_concurrency: u32,
    pub origin_interval_ms: i64,
    pub lease_ms: i64,
    pub max_retries: u32,
    pub backoff_ms: i64,
    pub max_backoff_ms: i64,
    pub deadline_ms: i64,
}
impl CrawlLimits {
    fn validate(&self) -> Result<(), String> {
        if self.max_pages == 0
            || self.max_urls < self.max_pages
            || self.max_urls > 100_000
            || self.global_concurrency == 0
            || self.per_origin_concurrency == 0
            || self.lease_ms <= 0
            || self.origin_interval_ms < 0
            || self.backoff_ms < 0
            || self.max_backoff_ms < self.backoff_ms
            || self.max_retries > 20
            || self.deadline_ms <= 0
        {
            return Err("INVALID_CRAWL_LIMITS".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Lease {
    pub run_id: String,
    pub identity_hash: String,
    pub canonical_url: String,
    pub token: String,
    pub depth: u32,
    pub attempts: u32,
    pub expires_at: i64,
}
pub struct Frontier {
    db: Connection,
}
struct Transition<'a> {
    state: &'a str,
    next: i64,
    payload: Option<&'a Value>,
    reason: Option<&'a str>,
    priority: Option<i64>,
    origin_not_before: Option<i64>,
}
impl Frontier {
    pub fn open(path: &Path) -> Result<Self, String> {
        let db = Connection::open(path).map_err(err)?;
        db.busy_timeout(Duration::from_secs(5)).map_err(err)?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
          CREATE TABLE IF NOT EXISTS crawl_runs(id TEXT PRIMARY KEY, limits TEXT NOT NULL, cancelled INTEGER NOT NULL DEFAULT 0, starts INTEGER NOT NULL DEFAULT 0);
          CREATE TABLE IF NOT EXISTS crawl_urls(run_id TEXT NOT NULL REFERENCES crawl_runs(id), identity_hash TEXT NOT NULL, canonical_url TEXT NOT NULL, raw_url TEXT NOT NULL, source_page TEXT, discovered_at INTEGER NOT NULL, origin TEXT NOT NULL, depth INTEGER NOT NULL, priority INTEGER NOT NULL, state TEXT NOT NULL CHECK(state IN ('DISCOVERED','PENDING','LEASED','HANDLED','RETRYABLE','FAILED','CANCELLED')), attempts INTEGER NOT NULL DEFAULT 0, next_at INTEGER NOT NULL, lease_token TEXT, lease_until INTEGER, payload TEXT, last_error TEXT, PRIMARY KEY(run_id,identity_hash));
          CREATE INDEX IF NOT EXISTS crawl_ready ON crawl_urls(run_id,state,next_at,priority);
          CREATE TABLE IF NOT EXISTS crawl_origins(run_id TEXT NOT NULL, origin TEXT NOT NULL, next_at INTEGER NOT NULL, PRIMARY KEY(run_id,origin));
          CREATE TABLE IF NOT EXISTS crawl_events(id INTEGER PRIMARY KEY, run_id TEXT NOT NULL, identity_hash TEXT, at INTEGER NOT NULL, state TEXT NOT NULL, reason TEXT);
        ").map_err(err)?;
        Ok(Self { db })
    }
    pub fn create_run(&self, id: &str, limits: &CrawlLimits) -> Result<(), String> {
        limits.validate()?;
        self.db
            .execute(
                "INSERT INTO crawl_runs(id,limits) VALUES(?1,?2)",
                params![id, serde_json::to_string(limits).map_err(err)?],
            )
            .map_err(err)?;
        Ok(())
    }
    pub fn enqueue(
        &mut self,
        run: &str,
        url: &UrlIdentity,
        depth: u32,
        priority: i64,
    ) -> Result<bool, String> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        let (limits, cancelled): (String, bool) = tx
            .query_row(
                "SELECT limits,cancelled FROM crawl_runs WHERE id=?1",
                [run],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(err)?;
        let limits: CrawlLimits = serde_json::from_str(&limits).map_err(err)?;
        let count: u32 = tx
            .query_row(
                "SELECT count(*) FROM crawl_urls WHERE run_id=?1",
                [run],
                |r| r.get(0),
            )
            .map_err(err)?;
        if cancelled
            || depth > limits.max_depth
            || count >= limits.max_urls
            || url.discovered_at >= limits.deadline_ms
        {
            return Ok(false);
        }
        let added=tx.execute("INSERT OR IGNORE INTO crawl_urls(run_id,identity_hash,canonical_url,raw_url,source_page,discovered_at,origin,depth,priority,state,next_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'DISCOVERED',?6)", params![run,url.identity_hash,url.canonical_url,url.raw_url,url.source_page,url.discovered_at,url.origin,depth,priority]).map_err(err)?==1;
        if added {
            for state in ["DISCOVERED", "PENDING"] {
                tx.execute(
                    "INSERT INTO crawl_events(run_id,identity_hash,at,state) VALUES(?1,?2,?3,?4)",
                    params![run, url.identity_hash, url.discovered_at, state],
                )
                .map_err(err)?;
            }
            tx.execute(
                "UPDATE crawl_urls SET state='PENDING' WHERE run_id=?1 AND identity_hash=?2",
                params![run, url.identity_hash],
            )
            .map_err(err)?;
        }
        tx.commit().map_err(err)?;
        Ok(added)
    }
    pub fn lease(&mut self, run: &str, now: i64) -> Result<Option<Lease>, String> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        let (limits, cancelled, starts): (String, bool, u32) = tx
            .query_row(
                "SELECT limits,cancelled,starts FROM crawl_runs WHERE id=?1",
                [run],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(err)?;
        let limits: CrawlLimits = serde_json::from_str(&limits).map_err(err)?;
        if cancelled || now >= limits.deadline_ms {
            tx.execute("UPDATE crawl_urls SET state='CANCELLED',lease_token=NULL,lease_until=NULL,last_error=?2 WHERE run_id=?1 AND state IN ('PENDING','LEASED','RETRYABLE','DISCOVERED')",params![run,if cancelled {"CANCELLED"}else{"DEADLINE"}]).map_err(err)?;
            tx.commit().map_err(err)?;
            return Ok(None);
        }
        // Every expired lease is observable; no other process's live lease is stolen.
        tx.execute("INSERT INTO crawl_events(run_id,identity_hash,at,state,reason) SELECT run_id,identity_hash,?2,CASE WHEN attempts>?3 THEN 'FAILED' ELSE 'RETRYABLE' END,'LEASE_EXPIRED' FROM crawl_urls WHERE run_id=?1 AND state='LEASED' AND lease_until<=?2", params![run,now,limits.max_retries]).map_err(err)?;
        tx.execute("UPDATE crawl_urls SET state=CASE WHEN attempts>?3 THEN 'FAILED' ELSE 'RETRYABLE' END,lease_token=NULL,lease_until=NULL,next_at=?2,last_error='LEASE_EXPIRED' WHERE run_id=?1 AND state='LEASED' AND lease_until<=?2",params![run,now,limits.max_retries]).map_err(err)?;
        let active: u32 = tx
            .query_row(
                "SELECT count(*) FROM crawl_urls WHERE run_id=?1 AND state='LEASED'",
                [run],
                |r| r.get(0),
            )
            .map_err(err)?;
        // The budget bounds acquisition attempts, including retries, not just successful pages.
        if active >= limits.global_concurrency || starts >= limits.max_pages {
            tx.commit().map_err(err)?;
            return Ok(None);
        }
        let row:Option<(String,String,u32,u32,String)>=tx.query_row("SELECT u.identity_hash,u.canonical_url,u.depth,u.attempts,u.origin FROM crawl_urls u LEFT JOIN crawl_origins o ON o.run_id=u.run_id AND o.origin=u.origin WHERE u.run_id=?1 AND u.state IN ('PENDING','RETRYABLE') AND u.next_at<=?2 AND COALESCE(o.next_at,0)<=?2 AND (SELECT count(*) FROM crawl_urls l WHERE l.run_id=u.run_id AND l.origin=u.origin AND l.state='LEASED')<?3 ORDER BY u.priority DESC,u.next_at,u.discovered_at,u.rowid LIMIT 1",params![run,now,limits.per_origin_concurrency],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(err)?;
        let Some((hash, url, depth, attempts, origin)) = row else {
            tx.commit().map_err(err)?;
            return Ok(None);
        };
        let token = Uuid::new_v4().to_string();
        let expires = now
            .checked_add(limits.lease_ms)
            .ok_or("TIME_OVERFLOW")?
            .min(limits.deadline_ms);
        tx.execute("UPDATE crawl_urls SET state='LEASED',lease_token=?3,lease_until=?4,attempts=attempts+1 WHERE run_id=?1 AND identity_hash=?2",params![run,hash,token,expires]).map_err(err)?;
        tx.execute("UPDATE crawl_runs SET starts=starts+1 WHERE id=?1", [run])
            .map_err(err)?;
        tx.execute(
            "INSERT OR REPLACE INTO crawl_origins VALUES(?1,?2,?3)",
            params![
                run,
                origin,
                now.checked_add(limits.origin_interval_ms)
                    .ok_or("TIME_OVERFLOW")?
            ],
        )
        .map_err(err)?;
        tx.execute(
            "INSERT INTO crawl_events(run_id,identity_hash,at,state) VALUES(?1,?2,?3,'LEASED')",
            params![run, hash, now],
        )
        .map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(Some(Lease {
            run_id: run.into(),
            identity_hash: hash,
            canonical_url: url,
            token,
            depth,
            attempts: attempts + 1,
            expires_at: expires,
        }))
    }
    pub fn complete(&mut self, lease: &Lease, now: i64, payload: &Value) -> Result<(), String> {
        self.complete_with_origin_cooldown(lease, now, payload, None)
    }
    pub fn complete_with_origin_cooldown(
        &mut self,
        lease: &Lease,
        now: i64,
        payload: &Value,
        origin_not_before: Option<i64>,
    ) -> Result<(), String> {
        self.transition(
            lease,
            now,
            Transition {
                state: "HANDLED",
                next: now,
                payload: Some(payload),
                reason: None,
                priority: None,
                origin_not_before,
            },
        )
    }
    pub fn fail(
        &mut self,
        lease: &Lease,
        now: i64,
        reason: &str,
        retryable: bool,
        retry_after_ms: Option<i64>,
    ) -> Result<(), String> {
        let text: String = self
            .db
            .query_row(
                "SELECT limits FROM crawl_runs WHERE id=?1",
                [&lease.run_id],
                |r| r.get(0),
            )
            .map_err(err)?;
        let limits: CrawlLimits = serde_json::from_str(&text).map_err(err)?;
        let backoff = limits
            .backoff_ms
            .saturating_mul(1_i64 << lease.attempts.saturating_sub(1).min(20))
            .min(limits.max_backoff_ms);
        let delay = backoff.max(retry_after_ms.unwrap_or(0).max(0));
        self.transition(
            lease,
            now,
            Transition {
                state: if retryable && lease.attempts <= limits.max_retries {
                    "RETRYABLE"
                } else {
                    "FAILED"
                },
                next: now.checked_add(delay).ok_or("TIME_OVERFLOW")?,
                payload: None,
                reason: Some(reason),
                priority: None,
                origin_not_before: if retryable {
                    retry_after_ms
                        .filter(|delay| *delay > 0)
                        .map(|delay| now.saturating_add(delay))
                } else {
                    None
                },
            },
        )
    }
    /// End a lease as FAILED when the attempt was refused by policy before any request was
    /// made (a robots rule answered from the cache). The attempt does not spend the page
    /// budget and does not delay the origin: live, 16 of 60 attempts (27%) on one catalogue
    /// were robots-denied add-to-cart URLs that cost no request but used the budget.
    pub fn fail_unspent(&mut self, lease: &Lease, now: i64, reason: &str) -> Result<(), String> {
        self.transition(
            lease,
            now,
            Transition {
                state: "FAILED",
                next: now,
                payload: None,
                reason: Some(reason),
                priority: None,
                origin_not_before: None,
            },
        )?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        tx.execute(
            "UPDATE crawl_runs SET starts=MAX(starts-1,0) WHERE id=?1",
            [&lease.run_id],
        )
        .map_err(err)?;
        tx.execute("UPDATE crawl_origins SET next_at=MIN(next_at,?3) WHERE run_id=?1 AND origin=(SELECT origin FROM crawl_urls WHERE run_id=?1 AND identity_hash=?2)", params![lease.run_id, lease.identity_hash, now]).map_err(err)?;
        tx.commit().map_err(err)
    }
    /// Release a live lease for retry without delay; an explicit priority can promote it.
    /// Lease fencing, acquisition budget and retry exhaustion still apply.
    /// `fail` remains the route for exponential backoff and Retry-After.
    pub fn reclaim(
        &mut self,
        lease: &Lease,
        now: i64,
        priority: Option<i64>,
    ) -> Result<(), String> {
        let text: String = self
            .db
            .query_row(
                "SELECT limits FROM crawl_runs WHERE id=?1",
                [&lease.run_id],
                |r| r.get(0),
            )
            .map_err(err)?;
        let limits: CrawlLimits = serde_json::from_str(&text).map_err(err)?;
        let state = if lease.attempts <= limits.max_retries {
            "RETRYABLE"
        } else {
            "FAILED"
        };
        self.transition(
            lease,
            now,
            Transition {
                state,
                next: now,
                payload: None,
                reason: Some("RECLAIM"),
                priority,
                origin_not_before: None,
            },
        )
    }
    fn transition(
        &mut self,
        lease: &Lease,
        now: i64,
        change: Transition<'_>,
    ) -> Result<(), String> {
        let Transition {
            state,
            next,
            payload,
            reason,
            priority,
            origin_not_before,
        } = change;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        let n=tx.execute("UPDATE crawl_urls SET state=?4,next_at=?5,payload=?6,last_error=?7,priority=COALESCE(?9,priority),lease_token=NULL,lease_until=NULL WHERE run_id=?1 AND identity_hash=?2 AND lease_token=?3 AND state='LEASED' AND lease_until>?8 AND EXISTS(SELECT 1 FROM crawl_runs WHERE id=?1 AND cancelled=0)", params![lease.run_id,lease.identity_hash,lease.token,state,next,payload.map(Value::to_string),reason,now,priority]).map_err(err)?;
        if n != 1 {
            return Err("STALE_OR_CANCELLED_LEASE".into());
        }
        if let Some(until) = origin_not_before {
            // This cooldown and lease completion commit together. A stale worker cannot
            // change origin scheduling, and other processes observe the same deadline.
            tx.execute("INSERT INTO crawl_origins(run_id,origin,next_at) SELECT run_id,origin,?3 FROM crawl_urls WHERE run_id=?1 AND identity_hash=?2 ON CONFLICT(run_id,origin) DO UPDATE SET next_at=MAX(crawl_origins.next_at,excluded.next_at)", params![lease.run_id,lease.identity_hash,until]).map_err(err)?;
        }
        tx.execute(
            "INSERT INTO crawl_events(run_id,identity_hash,at,state,reason) VALUES(?1,?2,?3,?4,?5)",
            params![lease.run_id, lease.identity_hash, now, state, reason],
        )
        .map_err(err)?;
        tx.commit().map_err(err)
    }
    pub fn cancel(&mut self, run: &str, now: i64) -> Result<(), String> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        if tx
            .execute("UPDATE crawl_runs SET cancelled=1 WHERE id=?1", [run])
            .map_err(err)?
            != 1
        {
            return Err("UNKNOWN_CRAWL_RUN".into());
        }
        tx.execute("INSERT INTO crawl_events(run_id,identity_hash,at,state,reason) SELECT run_id,identity_hash,?2,'CANCELLED','USER_CANCELLED' FROM crawl_urls WHERE run_id=?1 AND state IN ('DISCOVERED','PENDING','LEASED','RETRYABLE')",params![run,now]).map_err(err)?;
        tx.execute("UPDATE crawl_urls SET state='CANCELLED',lease_token=NULL,lease_until=NULL WHERE run_id=?1 AND state IN ('DISCOVERED','PENDING','LEASED','RETRYABLE')",[run]).map_err(err)?;
        tx.commit().map_err(err)
    }
    pub fn status(&self, run: &str) -> Result<Value, String> {
        let mut states = serde_json::Map::new();
        for name in [
            "DISCOVERED",
            "PENDING",
            "LEASED",
            "HANDLED",
            "RETRYABLE",
            "FAILED",
            "CANCELLED",
        ] {
            states.insert(name.into(), json!(0));
        }
        let mut statement = self
            .db
            .prepare("SELECT state,count(*) FROM crawl_urls WHERE run_id=?1 GROUP BY state")
            .map_err(err)?;
        for row in statement
            .query_map([run], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))
            .map_err(err)?
        {
            let (s, n) = row.map_err(err)?;
            states.insert(s, json!(n));
        }
        let (starts, limits, cancelled): (u32, String, bool) = self
            .db
            .query_row(
                "SELECT starts,limits,cancelled FROM crawl_runs WHERE id=?1",
                [run],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(err)?;
        let retries: u32 = self
            .db
            .query_row(
                "SELECT count(*) FROM crawl_events WHERE run_id=?1 AND state='RETRYABLE'",
                [run],
                |r| r.get(0),
            )
            .map_err(err)?;
        let mut origins =
            std::collections::BTreeMap::<String, serde_json::Map<String, Value>>::new();
        let mut origin_rows = self.db.prepare("SELECT origin,state,count(*) FROM crawl_urls WHERE run_id=?1 GROUP BY origin,state ORDER BY origin,state").map_err(err)?;
        for row in origin_rows
            .query_map([run], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, u64>(2)?,
                ))
            })
            .map_err(err)?
        {
            let (origin, state, count) = row.map_err(err)?;
            let counts = origins
                .entry(origin)
                .or_insert_with(|| states.keys().map(|s| (s.clone(), json!(0))).collect());
            counts.insert(state, json!(count));
        }
        let origins: Vec<_> = origins
            .into_iter()
            .map(|(origin, counts)| {
                let urls: u64 = counts.values().filter_map(Value::as_u64).sum();
                json!({"origin":origin,"url_count":urls,"states":counts})
            })
            .collect();
        Ok(
            json!({"run_id":run,"states":states,"origins":origins,"origin_scope":"ENQUEUED_URLS_IN_THIS_FRONTIER_RUN_NOT_INDEPENDENT_PUBLISHERS","acquisition_attempts":starts,"retry_events":retries,"cancelled":cancelled,"limits":serde_json::from_str::<Value>(&limits).map_err(err)?}),
        )
    }
    /// Captures are committed with HANDLED so an interrupted report can be rebuilt.
    pub fn captures(&self, run: &str) -> Result<Vec<Value>, String> {
        let mut st=self.db.prepare("SELECT payload FROM crawl_urls WHERE run_id=?1 AND state='HANDLED' AND payload IS NOT NULL ORDER BY rowid").map_err(err)?;
        st.query_map([run], |r| r.get::<_, String>(0))
            .map_err(err)?
            .map(|row| serde_json::from_str(&row.map_err(err)?).map_err(err))
            .collect()
    }
}
