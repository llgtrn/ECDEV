//! Credential readiness for the official SP-API boundary. Zero network, zero secret material.
use super::reads::{OPERATIONS, RESTRICTED_MODEL_SOURCES, RESTRICTED_OPERATIONS, WRITE_OPERATIONS};
use super::*;
use std::collections::BTreeMap;

pub const CLIENT_ID: &str = "SP_API_CLIENT_ID";
pub const CLIENT_SECRET: &str = "SP_API_CLIENT_SECRET";
pub const REFRESH_TOKEN: &str = "SP_API_REFRESH_TOKEN";
pub const SECRET_KEYS: [&str; 3] = [CLIENT_ID, CLIENT_SECRET, REFRESH_TOKEN];
pub const GATE_KEY: &str = "ECDEV_SPAPI_ALLOW_LIVE_READ";
pub const LIMIT_KEY: &str = "ECDEV_SPAPI_HTTP_REQUEST_LIMIT";
pub const MARKET_KEY: &str = "ECDEV_SPAPI_MARKETPLACE";

/// Host configuration source. Injected so tests never mutate the process environment.
pub trait EnvSource: Send + Sync {
    /// Existence only; implementations must not expose the value through this call.
    fn present(&self, key: &str) -> bool;
    /// Non-secret operator settings. Secret keys always yield None.
    fn setting(&self, key: &str) -> Option<String>;
    /// Secret values. Callers read these only after the explicit operator gate is on.
    fn secret(&self, key: &str) -> Option<String>;
}

pub struct ProcessEnv;
impl EnvSource for ProcessEnv {
    fn present(&self, key: &str) -> bool {
        // var_os existence only: the value is dropped unread, so an empty variable reports
        // PRESENT here and is rejected later by lwa_refresh_form once the gate allows reading.
        std::env::var_os(key).is_some()
    }
    fn setting(&self, key: &str) -> Option<String> {
        if SECRET_KEYS.contains(&key) {
            return None;
        }
        std::env::var(key).ok()
    }
    fn secret(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }
}

/// In-memory source for tests and embedding. Deliberately not Debug: it may hold secrets.
#[derive(Default)]
pub struct MapEnv(BTreeMap<String, String>);
impl MapEnv {
    pub fn new(pairs: &[(&str, &str)]) -> Self {
        Self(
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
        )
    }
}
impl EnvSource for MapEnv {
    fn present(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }
    fn setting(&self, key: &str) -> Option<String> {
        if SECRET_KEYS.contains(&key) {
            return None;
        }
        self.0.get(key).cloned()
    }
    fn secret(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }
}

/// Non-secret projection of a live session. Token contents and expiry instants are excluded.
pub struct SessionView {
    pub configured: bool,
    pub reason: &'static str,
    pub attempts: u64,
    pub limit: u64,
    pub token_state: &'static str,
}

pub const BLOCKED_BY_CREDENTIALS: &str = "LIVE_AUTH_BLOCKED_BY_CREDENTIALS";

fn presence(present: bool) -> &'static str {
    if present { "PRESENT" } else { "MISSING" }
}

pub fn report(env: &dyn EnvSource, session: Option<SessionView>) -> Value {
    let client = env.present(CLIENT_ID);
    let secret = env.present(CLIENT_SECRET);
    let refresh = env.present(REFRESH_TOKEN);
    let app_state = match (client, secret) {
        (true, true) => "PRESENT_UNVALIDATED",
        (false, false) => "MISSING",
        _ => "PARTIAL",
    };
    let gate = env.setting(GATE_KEY).as_deref() == Some("true");
    let limit_setting = env.setting(LIMIT_KEY);
    let limit = limit_setting
        .as_deref()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|n| (1..=1000).contains(n));
    let budget_state = match (&limit_setting, limit) {
        (None, _) => "MISSING",
        (Some(_), None) => "INVALID_OUT_OF_RANGE_1_TO_1000",
        (Some(_), Some(_)) => "CONFIGURED",
    };
    let attempts = session.as_ref().map_or(0, |s| s.attempts);
    let token_state = session
        .as_ref()
        .map_or("NEVER_REQUESTED", |s| s.token_state);
    let configured = session.as_ref().is_some_and(|s| s.configured);

    let mut blockers: Vec<&str> = Vec::new();
    if !(client && secret && refresh) {
        blockers.push(BLOCKED_BY_CREDENTIALS);
    }
    if !gate {
        blockers.push("LIVE_READ_BLOCKED_BY_OPERATOR_GATE");
    }
    if limit.is_none() {
        blockers.push("LIVE_READ_BLOCKED_BY_REQUEST_BUDGET");
    } else if configured && session.as_ref().is_some_and(|s| s.attempts >= s.limit) {
        blockers.push("LIVE_READ_BLOCKED_BY_REQUEST_BUDGET_EXHAUSTED");
    }
    if blockers.is_empty() && !configured {
        blockers.push(match session.as_ref().map(|s| s.reason) {
            Some("SP_API_CREDENTIALS_REQUIRED_OR_INVALID") => {
                "LIVE_AUTH_BLOCKED_BY_INVALID_CREDENTIALS"
            }
            Some("OFFICIAL_HTTP_CLIENT_UNAVAILABLE") => "OFFICIAL_HTTP_CLIENT_UNAVAILABLE",
            _ => "LIVE_SESSION_NOT_CONFIGURED_RESTART_REQUIRED",
        });
    }
    if matches!(token_state, "LWA_COOLDOWN") {
        blockers.push("LWA_COOLDOWN");
    }
    let status = blockers
        .first()
        .copied()
        .unwrap_or("LIVE_READ_CONFIGURED_UNVERIFIED");

    let market = match env.setting(MARKET_KEY) {
        None => json!({"state":"SELECTED_PER_REQUEST","configured_default":null}),
        Some(m) => match crate::market(&m) {
            Ok((id, endpoint, currency)) => {
                json!({"state":"CONFIGURED","configured_default":m,"marketplace_id":id,"region_endpoint":endpoint,"currency":currency})
            }
            Err(_) => json!({"state":"UNSUPPORTED_MARKET_SETTING","configured_default":null}),
        },
    };
    let region_state = if blockers.is_empty() {
        "READY_NOT_PROBED"
    } else {
        "BLOCKED"
    };
    let regions = json!([
        {"region":"NA","endpoint":"https://sellingpartnerapi-na.amazon.com","markets":["AMAZON_US"],"marketplace_ids":["ATVPDKIKX0DER"],"allowlisted":true,"readiness":region_state,"network_probe":"NOT_PERFORMED"},
        {"region":"FE","endpoint":"https://sellingpartnerapi-fe.amazon.com","markets":["AMAZON_JP"],"marketplace_ids":["A1VC38T7YXB528"],"allowlisted":true,"readiness":region_state,"network_probe":"NOT_PERFORMED"},
        {"region":"EU","endpoint":"https://sellingpartnerapi-eu.amazon.com","markets":[],"marketplace_ids":[],"allowlisted":false,"readiness":"NOT_SUPPORTED_BY_ADAPTER","network_probe":"NOT_PERFORMED"},
        {"region":"LWA","endpoint":LWA_ENDPOINT,"markets":[],"marketplace_ids":[],"allowlisted":true,"readiness":region_state,"network_probe":"NOT_PERFORMED"}
    ]);
    let operations: Vec<Value> = OPERATIONS
        .iter()
        .map(|s| {
            let route = if s.capability == reads::READ_CAPABILITY {
                "ecdev.seller.read"
            } else {
                "ecdev.product.analyze evidence_layer=OFFICIAL_SP_API (also requires core monetary ceilings)"
            };
            json!({"operation":s.key,"operation_id":s.operation_id,"api":s.api,"capability":s.capability,"method":s.method,"path":s.path,"access":"READ","live_state":if blockers.is_empty(){"AVAILABLE_LIVE_UNVERIFIED"}else{"BLOCKED"},"blocked_by":blockers,"fixture_state":"AVAILABLE","engine_route":route,"default_rate_per_second":s.default_rate_per_second,"default_burst":s.default_burst,"min_session_interval_ms":s.min_interval_ms,"model_source":{"path":s.model_path,"sha256":s.model_sha256,"commit":MODEL_COMMIT},"provenance_level":"LOCKED_MODEL_SHA256_MATCHES_CENSUS","fixture_provenance":s.fixture_provenance})
        })
        .collect();
    let restricted: Vec<Value> = RESTRICTED_OPERATIONS
        .iter()
        .map(|(id, method, path)| json!({"operation_id":id,"method":method,"path":path,"state":"RESTRICTED_DOMAIN_DISABLED"}))
        .collect();
    let writes: Vec<Value> = WRITE_OPERATIONS
        .iter()
        .map(|(id, method, path)| json!({"operation_id":id,"method":method,"path":path,"state":"WRITE_DISABLED"}))
        .collect();
    json!({
        "provider":"amazon-sp-api",
        "source_layer":"OFFICIAL_SP_API",
        "status":status,
        "blockers":blockers,
        "app_credentials":{"state":app_state,"SP_API_CLIENT_ID":presence(client),"SP_API_CLIENT_SECRET":presence(secret),"detection":"ENV_VAR_EXISTENCE_ONLY_VALUES_NOT_READ"},
        "seller_authorization":{"state":if refresh{"PRESENT_UNVALIDATED"}else{"MISSING"},"SP_API_REFRESH_TOKEN":presence(refresh),"grant":"LWA_REFRESH_TOKEN_SELLER_AUTHORIZED","detection":"ENV_VAR_EXISTENCE_ONLY_VALUES_NOT_READ"},
        "marketplace":market,
        "supported_markets":["AMAZON_US","AMAZON_JP"],
        "operator_gate":{"key":GATE_KEY,"state":if gate{"ON"}else{"OFF"},"required_value":"true","secrets_read_only_after_gate":true},
        "request_budget":{"key":LIMIT_KEY,"state":budget_state,"limit":limit,"attempts_this_session":attempts,"remaining_this_session":limit.map(|l|l.saturating_sub(attempts)),"automatic_retries":0,"account_quota":"UNKNOWN_NOT_PROBED"},
        "live_session":{"configured":configured,"reason":session.as_ref().map(|s|s.reason)},
        "token_state":token_state,
        "token_storage":"MEMORY_ONLY_NEVER_PERSISTED",
        "endpoints":regions,
        "operations":operations,
        "restricted_domain":{"state":"RESTRICTED_DOMAIN_DISABLED","reason":"Orders/PII and Restricted Data Token flows are a separate security domain: no restricted-role approval, RDT exchange or PII data-protection controls exist here","operations":restricted,"model_sources":RESTRICTED_MODEL_SOURCES.iter().map(|(p,h)|json!({"path":p,"sha256":h})).collect::<Vec<_>>()},
        "writes":{"state":"WRITES_DISABLED","operations":writes},
        "grantless_operations":"NOT_SUPPORTED",
        "network":"NOT_PROBED",
        "live_calls_made_by_doctor":0,
        "secret_material":"EXCLUDED_PRESENCE_ONLY",
        "model_commit":MODEL_COMMIT
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::ConfiguredAmazon;
    use ecdev_core::provider::Provider;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    /// Counts secret reads so tests can prove the operator gate precedes them.
    struct Recording {
        inner: MapEnv,
        secret_reads: AtomicUsize,
    }
    impl EnvSource for Recording {
        fn present(&self, key: &str) -> bool {
            self.inner.present(key)
        }
        fn setting(&self, key: &str) -> Option<String> {
            self.inner.setting(key)
        }
        fn secret(&self, key: &str) -> Option<String> {
            self.secret_reads.fetch_add(1, Ordering::SeqCst);
            self.inner.secret(key)
        }
    }
    fn recording(pairs: &[(&str, &str)]) -> Arc<Recording> {
        Arc::new(Recording {
            inner: MapEnv::new(pairs),
            secret_reads: AtomicUsize::new(0),
        })
    }
    const FAKE_CLIENT: &str = "qzxv-FAKE-cid-WQJK-8h3k-Zp9w";
    const FAKE_SECRET: &str = "qzxv-FAKE-csec-ZZPL-MNB4-Yy7t";
    const FAKE_REFRESH: &str = "Atzr|qzxv-FAKE-rtok-VVKW-XYJ2-Qq5r";
    fn secrets() -> [(&'static str, &'static str); 3] {
        [
            (CLIENT_ID, FAKE_CLIENT),
            (CLIENT_SECRET, FAKE_SECRET),
            (REFRESH_TOKEN, FAKE_REFRESH),
        ]
    }
    /// No value, prefix, suffix or 6-byte window of any secret may appear in the report.
    fn assert_secret_free(report: &Value) {
        let text = serde_json::to_string(report).unwrap();
        for (_, secret) in secrets() {
            assert!(!text.contains(secret));
            for window in secret.as_bytes().windows(6) {
                let w = std::str::from_utf8(window).unwrap();
                assert!(!text.contains(w), "secret fragment {w} leaked");
            }
        }
        for forbidden in ["length", "prefix", "suffix", "_len", "masked"] {
            assert!(!text.contains(forbidden), "{forbidden}");
        }
    }
    #[test]
    fn doctor_without_credentials_blocks_live_auth_and_reads_no_secrets() {
        let env = recording(&[]);
        let provider = ConfiguredAmazon::from_source(env.clone());
        let report = provider.doctor().unwrap();
        assert_eq!(env.secret_reads.load(Ordering::SeqCst), 0);
        assert_eq!(report["status"], BLOCKED_BY_CREDENTIALS);
        assert_eq!(report["source_layer"], "OFFICIAL_SP_API");
        assert_eq!(report["app_credentials"]["state"], "MISSING");
        assert_eq!(report["app_credentials"]["SP_API_CLIENT_ID"], "MISSING");
        assert_eq!(report["seller_authorization"]["state"], "MISSING");
        assert_eq!(report["operator_gate"]["state"], "OFF");
        assert_eq!(report["request_budget"]["state"], "MISSING");
        assert_eq!(report["token_state"], "NEVER_REQUESTED");
        assert_eq!(report["marketplace"]["state"], "SELECTED_PER_REQUEST");
        assert_eq!(report["network"], "NOT_PROBED");
        assert_eq!(report["live_calls_made_by_doctor"], 0);
        assert_eq!(
            report["blockers"],
            json!([
                "LIVE_AUTH_BLOCKED_BY_CREDENTIALS",
                "LIVE_READ_BLOCKED_BY_OPERATOR_GATE",
                "LIVE_READ_BLOCKED_BY_REQUEST_BUDGET"
            ])
        );
        let operations = report["operations"].as_array().unwrap();
        assert_eq!(operations.len(), 8);
        for op in operations {
            assert_eq!(op["live_state"], "BLOCKED");
            assert_eq!(op["fixture_state"], "AVAILABLE");
            assert_eq!(op["access"], "READ");
            assert!(
                op["blocked_by"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(BLOCKED_BY_CREDENTIALS))
            );
        }
        assert_eq!(
            report["restricted_domain"]["state"],
            "RESTRICTED_DOMAIN_DISABLED"
        );
        assert!(
            report["restricted_domain"]["operations"]
                .as_array()
                .unwrap()
                .iter()
                .all(|o| o["state"] == "RESTRICTED_DOMAIN_DISABLED")
        );
        assert_eq!(report["writes"]["state"], "WRITES_DISABLED");
        let regions = report["endpoints"].as_array().unwrap();
        assert!(
            regions
                .iter()
                .all(|r| r["network_probe"] == "NOT_PERFORMED")
        );
        assert_eq!(regions[2]["readiness"], "NOT_SUPPORTED_BY_ADAPTER");
        // Partial credentials stay blocked; the gate being on does not change credential state.
        let env = recording(&[
            (CLIENT_ID, FAKE_CLIENT),
            (GATE_KEY, "true"),
            (LIMIT_KEY, "5"),
        ]);
        let report = ConfiguredAmazon::from_source(env.clone()).doctor().unwrap();
        assert_eq!(report["status"], BLOCKED_BY_CREDENTIALS);
        assert_eq!(report["app_credentials"]["state"], "PARTIAL");
        assert_eq!(report["operator_gate"]["state"], "ON");
        assert_eq!(report["request_budget"]["limit"], 5);
        assert_eq!(report["blockers"], json!([BLOCKED_BY_CREDENTIALS]));
        assert_secret_free(&report);
        // Fixture-only adapter reports through the same doctor (process env, no session).
        assert!(crate::Amazon.doctor().unwrap()["operations"].is_array());
    }
    #[test]
    fn doctor_gate_precedes_secret_reads_and_output_never_contains_secret_values() {
        let mut pairs = secrets().to_vec();
        let env = recording(&pairs);
        let report = ConfiguredAmazon::from_source(env.clone()).doctor().unwrap();
        assert_eq!(env.secret_reads.load(Ordering::SeqCst), 0);
        assert_eq!(report["status"], "LIVE_READ_BLOCKED_BY_OPERATOR_GATE");
        assert_eq!(report["app_credentials"]["state"], "PRESENT_UNVALIDATED");
        assert_eq!(
            report["seller_authorization"]["state"],
            "PRESENT_UNVALIDATED"
        );
        assert_secret_free(&report);
        pairs.push((GATE_KEY, "true"));
        let env = recording(&pairs);
        let report = ConfiguredAmazon::from_source(env.clone()).doctor().unwrap();
        assert_eq!(env.secret_reads.load(Ordering::SeqCst), 0);
        assert_eq!(report["status"], "LIVE_READ_BLOCKED_BY_REQUEST_BUDGET");
        assert_secret_free(&report);
        pairs.push((LIMIT_KEY, "0"));
        let report = ConfiguredAmazon::from_source(recording(&pairs))
            .doctor()
            .unwrap();
        assert_eq!(
            report["request_budget"]["state"],
            "INVALID_OUT_OF_RANGE_1_TO_1000"
        );
        pairs.pop();
        pairs.push((LIMIT_KEY, "7"));
        pairs.push((MARKET_KEY, "AMAZON_JP"));
        let env = recording(&pairs);
        let provider = ConfiguredAmazon::from_source(env.clone());
        // Gate and budget allow the session to load credentials into memory exactly once.
        assert_eq!(env.secret_reads.load(Ordering::SeqCst), 3);
        let report = provider.doctor().unwrap();
        assert_eq!(env.secret_reads.load(Ordering::SeqCst), 3);
        assert_eq!(report["status"], "LIVE_READ_CONFIGURED_UNVERIFIED");
        assert_eq!(report["blockers"], json!([]));
        assert_eq!(report["live_session"]["configured"], true);
        assert_eq!(report["token_state"], "NEVER_REQUESTED");
        assert_eq!(report["request_budget"]["remaining_this_session"], 7);
        assert_eq!(report["marketplace"]["marketplace_id"], "A1VC38T7YXB528");
        assert!(
            report["operations"]
                .as_array()
                .unwrap()
                .iter()
                .all(|o| o["live_state"] == "AVAILABLE_LIVE_UNVERIFIED")
        );
        assert_secret_free(&report);
        assert_secret_free(&provider.metadata());
        // Present-but-empty secrets are detected as present yet rejected once read after the gate.
        let env = recording(&[
            (CLIENT_ID, ""),
            (CLIENT_SECRET, FAKE_SECRET),
            (REFRESH_TOKEN, FAKE_REFRESH),
            (GATE_KEY, "true"),
            (LIMIT_KEY, "7"),
        ]);
        let report = ConfiguredAmazon::from_source(env).doctor().unwrap();
        assert_eq!(report["status"], "LIVE_AUTH_BLOCKED_BY_INVALID_CREDENTIALS");
        assert_secret_free(&report);
    }
    #[test]
    fn non_secret_settings_never_expose_secret_keys() {
        let env = MapEnv::new(&secrets());
        for key in SECRET_KEYS {
            assert!(env.present(key));
            assert!(env.setting(key).is_none());
            assert!(ProcessEnv.setting(key).is_none());
        }
    }
}
