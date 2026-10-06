//! Bounded official HTTP reads. Operator authorization, credentials and core budget precede IO.
use super::*;
use std::{collections::BTreeMap, io::Read, sync::Mutex};

// Deliberately not Debug/Serialize. Secret-bearing forms, headers and LWA bodies stay ephemeral.
struct Credentials {
    client: String,
    secret: String,
    refresh: String,
}
struct Request {
    method: String,
    url: String,
    query: Value,
    body: Option<Value>,
    form: Option<Vec<(String, String)>>,
    token: Option<String>,
    date: String,
    limit: usize,
}
struct Response {
    status: u16,
    headers: BTreeMap<String, String>,
    raw: Vec<u8>,
    url: String,
}
struct WireError {
    reason: &'static str,
    status: Option<u16>,
    headers: BTreeMap<String, String>,
    uncertain: bool,
}
impl WireError {
    fn before(reason: &'static str) -> Self {
        Self {
            reason,
            status: None,
            headers: BTreeMap::new(),
            uncertain: false,
        }
    }
    fn unknown(reason: &'static str) -> Self {
        Self {
            uncertain: true,
            ..Self::before(reason)
        }
    }
}
trait Wire: Send {
    fn send(&mut self, request: Request) -> Result<Response, WireError>;
}
trait Clock: Send {
    fn now(&self) -> u64;
}
struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> u64 {
        ecdev_core::service::timestamp().saturating_mul(1000)
    }
}
struct Http {
    client: reqwest::blocking::Client,
    #[cfg(test)]
    mock_endpoint: Option<String>,
}
impl Http {
    fn new() -> Result<Self, &'static str> {
        Ok(Self {
            #[cfg(test)]
            mock_endpoint: None,
            client: reqwest::blocking::Client::builder()
                .https_only(true)
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(30))
                .connect_timeout(std::time::Duration::from_secs(10))
                .build()
                .map_err(|_| "OFFICIAL_HTTP_CLIENT_UNAVAILABLE")?,
        })
    }
}
impl Wire for Http {
    fn send(&mut self, request: Request) -> Result<Response, WireError> {
        if ![
            LWA_ENDPOINT,
            "https://sellingpartnerapi-na.amazon.com/",
            "https://sellingpartnerapi-fe.amazon.com/",
        ]
        .iter()
        .any(|base| {
            request.url == *base || (*base != LWA_ENDPOINT && request.url.starts_with(base))
        }) {
            return Err(WireError::before("OFFICIAL_ENDPOINT_DENIED"));
        }
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|_| WireError::before("OFFICIAL_METHOD_INVALID"))?;
        let url = request.url.clone();
        #[cfg(test)]
        let url = if let Some(endpoint) = &self.mock_endpoint {
            let original =
                reqwest::Url::parse(&url).map_err(|_| WireError::before("MOCK_URL_INVALID"))?;
            format!("{endpoint}{}", original.path())
        } else {
            url
        };
        let mut builder = self
            .client
            .request(method, &url)
            .header(
                "host",
                reqwest::Url::parse(&request.url)
                    .map_err(|_| WireError::before("OFFICIAL_ENDPOINT_INVALID"))?
                    .host_str()
                    .ok_or_else(|| WireError::before("OFFICIAL_ENDPOINT_INVALID"))?,
            )
            .header("user-agent", "ECDEV/0.1.0 (Language=Rust)")
            .header("x-amz-date", request.date);
        if let Some(token) = request.token {
            builder = builder.header("x-amz-access-token", token);
        }
        if let Some(form) = request.form {
            builder = builder.form(&form);
        }
        if let Some(body) = request.body {
            builder = builder.json(&body);
        }
        if let Some(query) = request.query.as_object() {
            let params: Vec<_> = query
                .iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.as_str(), s)))
                .collect();
            builder = builder.query(&params);
        }
        let response = builder
            .send()
            .map_err(|_| WireError::unknown("OFFICIAL_HTTP_OUTCOME_UNKNOWN"))?;
        let status = response.status().as_u16();
        let url = response.url().to_string();
        let headers: BTreeMap<String, String> =
            ["x-amzn-requestid", "x-amzn-ratelimit-limit", "retry-after"]
                .into_iter()
                .filter_map(|k| {
                    response
                        .headers()
                        .get(k)
                        .and_then(|v| v.to_str().ok())
                        .filter(|v| v.len() <= 4096 && !v.chars().any(char::is_control))
                        .map(|v| (k.to_string(), v.to_string()))
                })
                .collect();
        let mut raw = Vec::new();
        response
            .take(request.limit as u64 + 1)
            .read_to_end(&mut raw)
            .map_err(|_| WireError {
                reason: "OFFICIAL_RESPONSE_READ_FAILED",
                status: Some(status),
                headers: headers.clone(),
                uncertain: false,
            })?;
        if raw.len() > request.limit {
            return Err(WireError {
                reason: "OFFICIAL_RESPONSE_BODY_LIMIT",
                status: Some(status),
                headers,
                uncertain: false,
            });
        }
        Ok(Response {
            status,
            headers,
            raw,
            url,
        })
    }
}

fn amz_date(ms: u64) -> Result<String, &'static str> {
    let seconds = ms / 1000;
    let mut days = seconds / 86400;
    let mut year = 1970u64;
    fn leap(y: u64) -> bool {
        y.is_multiple_of(4) && (!y.is_multiple_of(100) || y.is_multiple_of(400))
    }
    loop {
        let n = if leap(year) { 366 } else { 365 };
        if days < n {
            break;
        }
        days -= n;
        year += 1;
        if year > 9999 {
            return Err("OFFICIAL_CLOCK_OUT_OF_RANGE");
        }
    }
    let months = [
        31,
        if leap(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 0;
    while days >= months[month] {
        days -= months[month];
        month += 1;
    }
    let time = seconds % 86400;
    Ok(format!(
        "{year:04}{:02}{:02}T{:02}{:02}{:02}Z",
        month + 1,
        days + 1,
        time / 3600,
        time / 60 % 60,
        time % 60
    ))
}
fn headers(value: &BTreeMap<String, String>) -> Value {
    json!(value)
}

struct Session<W: Wire, C: Clock> {
    wire: W,
    clock: C,
    credentials: Credentials,
    authorized: bool,
    limit: u64,
    attempts: u64,
    token: Option<LwaToken>,
    next: BTreeMap<String, u64>,
    interval: BTreeMap<String, u64>,
    last_now: u64,
    auth_next: u64,
    lwa_failed: bool,
}
fn min_interval_ms(key: &str) -> u64 {
    reads::spec(key).map_or(1000, |s| s.min_interval_ms)
}
impl<W: Wire, C: Clock> Session<W, C> {
    /// Non-secret token lifecycle state; never exposes the token or its expiry instant.
    fn token_state(&self) -> &'static str {
        let now = self.clock.now();
        if now < self.auth_next {
            "LWA_COOLDOWN"
        } else if self
            .token
            .as_ref()
            .is_some_and(|t| t.access_token_at(now).is_some())
        {
            "VALID_IN_MEMORY"
        } else if self.lwa_failed {
            "LAST_REFRESH_FAILED"
        } else if self.token.is_some() {
            "EXPIRED_RENEWAL_REQUIRED"
        } else {
            "NEVER_REQUESTED"
        }
    }
    fn execute(&mut self, request: &AcquireRequest) -> Result<AcquireResult, AcquireError> {
        let plan = plan(request).map_err(AcquireError::from)?;
        let now = self.clock.now();
        let mut denied =
            AcquireError::from("OFFICIAL_LIVE_AUTHORIZATION_OR_REQUEST_BUDGET_REQUIRED");
        denied.request_count = Some(0);
        if !self.authorized || self.limit == 0 {
            return Err(denied);
        }
        if now < self.last_now {
            denied.reason = "OFFICIAL_CLOCK_MOVED_BACKWARDS".into();
            return Err(denied);
        }
        self.last_now = now;
        if now < self.auth_next {
            denied.reason = "OFFICIAL_LWA_COOLDOWN".into();
            denied.retry_not_before_ms = self.auth_next.try_into().ok();
            return Err(denied);
        }
        let operations = plan["operations"]
            .as_array()
            .ok_or_else(|| AcquireError::from("INVALID_OFFICIAL_PLAN"))?;
        // Only GET reads plus the read-only fee estimate POST may reach the wire.
        if operations
            .iter()
            .any(|o| o["method"] != "GET" && o["operation"] != "fees")
        {
            denied.reason = "OFFICIAL_WRITES_DISABLED".into();
            return Err(denied);
        }
        for operation in operations {
            let key = operation["operation"].as_str().unwrap();
            if self.next.get(key).is_some_and(|time| now < *time) {
                let mut error = AcquireError::from("OFFICIAL_OPERATION_COOLDOWN");
                error.request_count = Some(0);
                error.retry_not_before_ms = self.next[key].try_into().ok();
                return Err(error);
            }
        }
        let refresh = self
            .token
            .as_ref()
            .and_then(|t| t.access_token_at(now))
            .is_none();
        let needed = operations.len() as u64 + u64::from(refresh);
        if self
            .attempts
            .checked_add(needed)
            .is_none_or(|n| n > self.limit)
        {
            return Err(denied);
        }
        let mut completed = 0u64;
        let mut uncertain = false;
        let mut records = Vec::new();
        let mut captures = serde_json::Map::new();
        let mut failure: Option<Value> = None;
        if refresh {
            let form = lwa_refresh_form(
                &self.credentials.client,
                &self.credentials.secret,
                &self.credentials.refresh,
            )
            .map_err(AcquireError::from)?;
            let request = Request {
                method: "POST".into(),
                url: LWA_ENDPOINT.into(),
                query: json!({}),
                body: None,
                form: Some(
                    form.into_iter()
                        .map(|(k, v)| (k.into(), v.into()))
                        .collect(),
                ),
                token: None,
                date: amz_date(now).map_err(AcquireError::from)?.to_string(),
                limit: 16 * 1024,
            };
            self.attempts += 1;
            match self.wire.send(request) {
                Ok(response) => {
                    completed += 1;
                    if response.status != 200 {
                        let error = AcquireError::http(
                            response.status,
                            completed,
                            response.headers.get("retry-after").map(String::as_str),
                            self.clock.now().min(i64::MAX as u64) as i64,
                        );
                        self.auth_next = error
                            .retry_not_before_ms
                            .and_then(|n| u64::try_from(n).ok())
                            .unwrap_or(0);
                        self.lwa_failed = true;
                        failure = Some(
                            json!({"phase":"LWA","error":error,"raw_capture":"NEVER_PERSIST_AUTHENTICATION_BODY"}),
                        );
                    } else {
                        let token = serde_json::from_slice::<Value>(&response.raw)
                            .ok()
                            .and_then(|v| LwaToken::from_response(&v, self.clock.now()).ok());
                        if token
                            .as_ref()
                            .and_then(|t| t.access_token_at(self.clock.now()))
                            .is_none()
                        {
                            failure = Some(
                                json!({"phase":"LWA","reason":"LWA_RESPONSE_INVALID_OR_TOKEN_TOO_SHORT","raw_capture":"NEVER_PERSIST_AUTHENTICATION_BODY"}),
                            );
                        }
                        self.lwa_failed = failure.is_some();
                        self.token = token;
                    }
                }
                Err(error) => {
                    if error.status.is_some() {
                        completed += 1;
                    }
                    self.auth_next = error
                        .headers
                        .get("retry-after")
                        .and_then(|s| {
                            ecdev_core::provider::retry_after_not_before(
                                s,
                                self.clock.now().min(i64::MAX as u64) as i64,
                            )
                        })
                        .and_then(|n| u64::try_from(n).ok())
                        .unwrap_or(0);
                    uncertain |= error.uncertain;
                    self.lwa_failed = true;
                    failure = Some(
                        json!({"phase":"LWA","reason":error.reason,"http_status":error.status,"response_headers":error.headers,"raw_capture":"NEVER_PERSIST_AUTHENTICATION_BODY"}),
                    );
                }
            }
        }
        if failure.is_none() {
            for operation in operations {
                let now = self.clock.now();
                if now < self.last_now {
                    failure =
                        Some(json!({"phase":"SP_API","reason":"OFFICIAL_CLOCK_MOVED_BACKWARDS"}));
                    break;
                }
                self.last_now = now;
                let Some(token) = self.token.as_ref().and_then(|t| t.access_token_at(now)) else {
                    failure =
                        Some(json!({"phase":"SP_API","reason":"LWA_TOKEN_EXPIRED_DURING_INTENT"}));
                    break;
                };
                let key = operation["operation"].as_str().unwrap();
                let request = Request {
                    method: operation["method"].as_str().unwrap().into(),
                    url: operation["requested_url"].as_str().unwrap().into(),
                    query: operation["query"].clone(),
                    body: (!operation["body"].is_null()).then(|| operation["body"].clone()),
                    form: None,
                    token: Some(token.into()),
                    date: amz_date(now).map_err(AcquireError::from)?.to_string(),
                    limit: MAX_BODY,
                };
                self.attempts += 1;
                match self.wire.send(request) {
                    Ok(response) => {
                        completed += 1;
                        let received = self.clock.now();
                        let base = min_interval_ms(key);
                        let interval = response
                            .headers
                            .get("x-amzn-ratelimit-limit")
                            .and_then(|s| s.parse::<f64>().ok())
                            .filter(|n| n.is_finite() && *n > 0.0)
                            .map(|rate| (1000.0 / rate).ceil().min(u64::MAX as f64) as u64)
                            .unwrap_or(base)
                            .max(base);
                        let interval = interval.max(*self.interval.get(key).unwrap_or(&base));
                        self.interval.insert(key.into(), interval);
                        let retry = response
                            .headers
                            .get("retry-after")
                            .and_then(|s| {
                                ecdev_core::provider::retry_after_not_before(
                                    s,
                                    received.min(i64::MAX as u64) as i64,
                                )
                            })
                            .and_then(|n| u64::try_from(n).ok())
                            .unwrap_or(0);
                        self.next
                            .insert(key.into(), received.saturating_add(interval).max(retry));
                        let raw = match String::from_utf8(response.raw) {
                            Ok(raw) => raw,
                            Err(_) => {
                                failure = Some(
                                    json!({"phase":"SP_API","operation":key,"reason":"OFFICIAL_JSON_RESPONSE_NOT_UTF8","http_status":response.status}),
                                );
                                break;
                            }
                        };
                        captures.insert(key.into(),json!({"status":response.status,"raw_body":raw,"headers":headers(&response.headers)}));
                        records.push(json!({"operation":key,"final_url":response.url,"http_status":response.status,"received_at_ms":received,"retry_not_before_ms":retry,"minimum_next_request_ms":self.next[key]}));
                    }
                    Err(error) => {
                        if error.status.is_some() {
                            completed += 1;
                        }
                        let received = self.clock.now();
                        let base = min_interval_ms(key);
                        let interval = error
                            .headers
                            .get("x-amzn-ratelimit-limit")
                            .and_then(|s| s.parse::<f64>().ok())
                            .filter(|n| n.is_finite() && *n > 0.0)
                            .map(|rate| (1000.0 / rate).ceil().min(u64::MAX as f64) as u64)
                            .unwrap_or(base)
                            .max(base)
                            .max(*self.interval.get(key).unwrap_or(&base));
                        self.interval.insert(key.into(), interval);
                        let retry = error
                            .headers
                            .get("retry-after")
                            .and_then(|s| {
                                ecdev_core::provider::retry_after_not_before(
                                    s,
                                    received.min(i64::MAX as u64) as i64,
                                )
                            })
                            .and_then(|n| u64::try_from(n).ok())
                            .unwrap_or(0);
                        self.next
                            .insert(key.into(), received.saturating_add(interval).max(retry));
                        uncertain |= error.uncertain;
                        failure = Some(
                            json!({"phase":"SP_API","operation":key,"reason":error.reason,"http_status":error.status,"response_headers":error.headers}),
                        );
                        break;
                    }
                }
            }
        }
        let mut fixture_request = AcquireRequest {
            run_id: request.run_id.clone(),
            capability: request.capability.clone(),
            market: request.market.clone(),
            query: request.query.clone(),
        };
        fixture_request.query["fixture_responses"] = json!(captures);
        let mut result = match Amazon.acquire(&fixture_request) {
            Ok(result) => result,
            Err(error) => AcquireResult {
                observations: vec![],
                result: json!({"status":"UNAVAILABLE","plan":plan,"records":[],"normalization_error":error.reason}),
                raw_payload: vec![],
                provider_cost: json!({}),
            },
        };
        let mode = if completed > 0 { "LIVE" } else { "INFERRED" };
        for record in result.result["records"]
            .as_array_mut()
            .into_iter()
            .flatten()
        {
            if let Some(witness) = records
                .iter()
                .find(|r| r["operation"] == record["operation"])
            {
                record["final_url"] = witness["final_url"].clone();
                record["received_at_ms"] = witness["received_at_ms"].clone();
                record["minimum_next_request_ms"] = witness["minimum_next_request_ms"].clone();
                record["request_count"] = json!(1);
                record["state"] = json!("LIVE");
                record["source_authenticity"] =
                    json!("OFFICIAL_ENDPOINT_HTTP_CAPTURE_NOT_COMMERCIAL_VALIDATION");
                record["provenance"]["observation_mode"] = json!("LIVE");
                reads::set_field_mode(&mut record["normalized"], "LIVE");
            }
        }
        for observation in &mut result.observations {
            observation.mode = ObservationMode::Live;
            observation.cost_minor = None;
            observation.unit = "OFFICIAL_HTTP_RESPONSE".into();
            observation.normalized_value["source_authenticity"] =
                json!("OFFICIAL_ENDPOINT_HTTP_CAPTURE_NOT_COMMERCIAL_VALIDATION");
            reads::set_field_mode(&mut observation.normalized_value["value"], "LIVE");
        }
        result.result["mode"] = json!(mode);
        result.result["new_live_acquisition"] = json!(completed > 0);
        result.result["official_live_validation"] = json!(if !result.observations.is_empty() {
            "SOURCE_RESPONSE_CAPTURED_NOT_COMMERCIAL_VALIDATION"
        } else {
            "UNAVAILABLE"
        });
        result.result["acquisition_failure"] = json!(failure);
        result.result["status"] = json!(if failure.is_some() {
            "UNAVAILABLE_OR_PARTIAL"
        } else if result.result["records"]
            .as_array()
            .is_some_and(|a| a.iter().all(|r| r["status"] == "COMPLETE"))
        {
            "COMPLETE_WITH_UNKNOWNS"
        } else {
            "PARTIAL"
        });
        result.provider_cost = json!({"mode":mode,"request_count":if uncertain{Value::Null}else{json!(completed)},"known_request_count":completed,"attempts_this_session":self.attempts,"actual_cost_minor":null,"automatic_retries":0,"live_account_quota":null,"account_rate_scope":"APP_ACCOUNT_OPERATION_ONLY_OTHER_LIMITS_UNKNOWN","lwa_body_captured":false});
        result.raw_payload = vec![]; // Individual SP-API bodies are in records; LWA is never a capture.
        Ok(result)
    }
}

pub struct ConfiguredAmazon {
    session: Option<Mutex<Session<Http, SystemClock>>>,
    reason: &'static str,
    env: std::sync::Arc<dyn doctor::EnvSource>,
}
impl ConfiguredAmazon {
    pub fn from_env() -> Self {
        Self::from_source(std::sync::Arc::new(doctor::ProcessEnv))
    }
    /// Operator gate and request budget are checked before any secret value is read.
    pub fn from_source(env: std::sync::Arc<dyn doctor::EnvSource>) -> Self {
        let denied = |reason, env| Self {
            session: None,
            reason,
            env,
        };
        if env.setting(doctor::GATE_KEY).as_deref() != Some("true") {
            return denied("EXPLICIT_OPERATOR_LIVE_AUTHORIZATION_REQUIRED", env);
        }
        let Some(limit) = env
            .setting(doctor::LIMIT_KEY)
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|n| (1..=1000).contains(n))
        else {
            return denied("BOUNDED_SPAPI_HTTP_REQUEST_LIMIT_REQUIRED", env);
        };
        let credentials = Credentials {
            client: env.secret(doctor::CLIENT_ID).unwrap_or_default(),
            secret: env.secret(doctor::CLIENT_SECRET).unwrap_or_default(),
            refresh: env.secret(doctor::REFRESH_TOKEN).unwrap_or_default(),
        };
        if lwa_refresh_form(
            &credentials.client,
            &credentials.secret,
            &credentials.refresh,
        )
        .is_err()
        {
            return denied("SP_API_CREDENTIALS_REQUIRED_OR_INVALID", env);
        }
        let Ok(wire) = Http::new() else {
            return denied("OFFICIAL_HTTP_CLIENT_UNAVAILABLE", env);
        };
        Self {
            session: Some(Mutex::new(Session {
                wire,
                clock: SystemClock,
                credentials,
                authorized: true,
                limit,
                attempts: 0,
                token: None,
                next: BTreeMap::new(),
                interval: BTreeMap::new(),
                last_now: 0,
                auth_next: 0,
                lwa_failed: false,
            })),
            reason: "EXPLICIT_AUTHORIZATION_CONFIGURED_LIVE_ACCOUNT_UNVERIFIED",
            env,
        }
    }
    /// Readiness report: credential presence only, session counters and token lifecycle state.
    pub fn doctor_report(&self) -> Value {
        let view = match &self.session {
            Some(session) => match session.lock() {
                Ok(s) => doctor::SessionView {
                    configured: true,
                    reason: self.reason,
                    attempts: s.attempts,
                    limit: s.limit,
                    token_state: s.token_state(),
                },
                Err(_) => doctor::SessionView {
                    configured: false,
                    reason: "OFFICIAL_SESSION_UNAVAILABLE",
                    attempts: 0,
                    limit: 0,
                    token_state: "UNKNOWN_SESSION_UNAVAILABLE",
                },
            },
            None => doctor::SessionView {
                configured: false,
                reason: self.reason,
                attempts: 0,
                limit: 0,
                token_state: "NEVER_REQUESTED",
            },
        };
        doctor::report(self.env.as_ref(), Some(view))
    }
}
impl Provider for ConfiguredAmazon {
    fn id(&self) -> &str {
        "amazon-sp-api"
    }
    fn metadata(&self) -> Value {
        let mut value = Amazon.metadata();
        value["adapter_state"] = json!("NATIVE_HTTP_AND_FIXTURE_BOUNDARY");
        value["live_supported"] = json!(true);
        value["live_authorized"] = json!(self.session.is_some());
        value["status"] = json!(if self.session.is_some() {
            "AVAILABLE"
        } else {
            "UNAVAILABLE"
        });
        value["auth_state"] = json!(self.reason);
        value["doctor"] = json!("ecdev.provider.doctor");
        value["reason"] = json!(
            "Authorized reads additionally require core explicit monetary ceilings; tokens memory-only, fixed HTTPS endpoints, no retries, no public/Keepa substitution"
        );
        value
    }
    fn doctor(&self) -> Option<Value> {
        Some(self.doctor_report())
    }
    fn acquire(&self, request: &AcquireRequest) -> Result<AcquireResult, AcquireError> {
        if request.query["fixture_responses"].is_object() {
            return Amazon.acquire(request);
        }
        if !matches!(
            request.capability.as_str(),
            "product.analyze.official" | reads::READ_CAPABILITY
        ) {
            return Err("UNSUPPORTED_OFFICIAL_CAPABILITY".into());
        }
        let Some(session) = &self.session else {
            let mut error = AcquireError::from(self.reason);
            error.request_count = Some(0);
            return Err(error);
        };
        session
            .lock()
            .map_err(|_| AcquireError::from("OFFICIAL_SESSION_UNAVAILABLE"))?
            .execute(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Write,
        net::TcpListener,
        sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        },
        thread,
        time::{Duration, Instant},
    };
    struct TestClock(Arc<AtomicU64>);
    impl Clock for TestClock {
        fn now(&self) -> u64 {
            self.0.load(Ordering::SeqCst)
        }
    }
    fn mock(responses: Vec<(u16, String, String)>) -> (Http, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let mut requests = vec![];
            for (status, headers, body) in responses {
                let deadline = Instant::now() + Duration::from_secs(5);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((s, _)) => break s,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "mock request missing");
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(e) => panic!("{e}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut bytes = vec![];
                loop {
                    let mut chunk = [0; 4096];
                    let n = stream.read(&mut chunk).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&chunk[..n]);
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let text = String::from_utf8_lossy(&bytes[..end]);
                        let length = text
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|s| s.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if bytes.len() >= end + 4 + length {
                            break;
                        }
                    }
                    assert!(bytes.len() < 100_000);
                }
                requests.push(String::from_utf8(bytes).unwrap());
                write!(stream,"HTTP/1.1 {status} Mock\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}",body.len()).unwrap();
            }
            requests
        });
        (
            Http {
                client: reqwest::blocking::Client::builder()
                    .no_proxy()
                    .redirect(reqwest::redirect::Policy::none())
                    .timeout(Duration::from_secs(3))
                    .build()
                    .unwrap(),
                mock_endpoint: Some(endpoint),
            },
            handle,
        )
    }
    fn token() -> (u16, String, String) {
        (
            200,
            String::new(),
            json!({"access_token":"mock-access-token","token_type":"bearer","expires_in":3600})
                .to_string(),
        )
    }
    fn offer() -> (u16, String, String) {
        let v: Value =
            serde_json::from_str(include_str!("../tests/fixtures/official-responses.json"))
                .unwrap();
        (
            200,
            String::new(),
            v["cases"][1]["response"]["raw_body"]
                .as_str()
                .unwrap()
                .into(),
        )
    }
    fn intent() -> AcquireRequest {
        AcquireRequest {
            run_id: "mock-run".into(),
            capability: "product.analyze.official".into(),
            market: "AMAZON_US".into(),
            query: json!({"asin":"B00V5DG6IQ","include":["OFFERS"]}),
        }
    }
    fn session(wire: Http) -> (Session<Http, TestClock>, Arc<AtomicU64>) {
        let clock = Arc::new(AtomicU64::new(1_700_000_000_000));
        (
            Session {
                wire,
                clock: TestClock(clock.clone()),
                credentials: Credentials {
                    client: "mock-client".into(),
                    secret: "mock-secret".into(),
                    refresh: "mock-refresh".into(),
                },
                authorized: true,
                limit: 20,
                attempts: 0,
                token: None,
                next: BTreeMap::new(),
                interval: BTreeMap::new(),
                last_now: 0,
                auth_next: 0,
                lwa_failed: false,
            },
            clock,
        )
    }
    #[test]
    fn official_http_refresh_reuse_expiry_and_secret_exclusion() {
        let (wire, server) = mock(vec![token(), offer(), offer(), token(), offer()]);
        let (mut s, clock) = session(wire);
        for (advance, count) in [(0, 2), (2000, 1), (3_600_000, 2)] {
            clock.fetch_add(advance, Ordering::SeqCst);
            let result = s.execute(&intent()).unwrap();
            assert_eq!(result.provider_cost["request_count"], count);
            assert_eq!(result.result["mode"], "LIVE");
            assert_eq!(result.observations.len(), 1);
            let serialized = serde_json::to_string(&result.result).unwrap();
            for secret in ["mock-secret", "mock-refresh", "mock-access-token"] {
                assert!(!serialized.contains(secret));
            }
            assert!(result.provider_cost["actual_cost_minor"].is_null());
        }
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 5);
        let lwa = &requests[0];
        assert!(lwa.starts_with("POST /auth/o2/token"));
        assert!(lwa.contains("grant_type=refresh_token"));
        assert!(!lwa.contains("scope="));
        assert!(!lwa.contains("x-amz-access-token"));
        let api = requests[1].to_ascii_lowercase();
        assert!(api.starts_with("get /products/pricing/v0/items/b00v5dg6iq/offers?"));
        assert!(api.contains("marketplaceid=atvpdkikx0der"));
        assert!(api.contains("host: sellingpartnerapi-na.amazon.com"));
        assert!(api.contains("x-amz-access-token: mock-access-token"));
        assert!(api.contains("x-amz-date: 20231114t221320z"));
    }
    #[test]
    fn official_http_throttle_preserves_receipt_and_denies_early_retry() {
        let (wire, server) = mock(vec![
            token(),
            (
                429,
                "Retry-After: 7\r\nx-amzn-RateLimit-Limit: 0.2\r\n".into(),
                "{\"errors\":[]}".into(),
            ),
            offer(),
        ]);
        let (mut s, clock) = session(wire);
        let result = s.execute(&intent()).unwrap();
        assert_eq!(result.result["records"][0]["http_status"], 429);
        assert_eq!(result.provider_cost["request_count"], 2);
        assert_eq!(s.attempts, 2);
        let error = s.execute(&intent()).err().unwrap();
        assert_eq!(error.request_count, Some(0));
        assert_eq!(error.reason, "OFFICIAL_OPERATION_COOLDOWN");
        assert_eq!(s.attempts, 2);
        clock.fetch_add(8000, Ordering::SeqCst);
        assert_eq!(
            s.execute(&intent()).unwrap().provider_cost["request_count"],
            1
        );
        assert_eq!(s.interval["offers"], 5000);
        assert_eq!(server.join().unwrap().len(), 3);
    }
    #[test]
    fn official_http_lwa_failure_budget_and_redirect_never_retry() {
        let (wire, server) = mock(vec![(
            429,
            "Retry-After: 10\r\n".into(),
            "mock-secret mock-access-token".into(),
        )]);
        let (mut s, _) = session(wire);
        let result = s.execute(&intent()).unwrap();
        assert!(result.observations.is_empty());
        assert_eq!(result.provider_cost["request_count"], 1);
        assert!(!result.result.to_string().contains("mock-secret"));
        assert_eq!(s.execute(&intent()).err().unwrap().request_count, Some(0));
        assert_eq!(server.join().unwrap().len(), 1);
        let (wire, server) = mock(vec![
            token(),
            (
                301,
                "Location: https://example.invalid/\r\n".into(),
                "{}".into(),
            ),
        ]);
        let (mut s, _) = session(wire);
        s.limit = 1;
        assert_eq!(s.execute(&intent()).err().unwrap().request_count, Some(0));
        assert_eq!(s.attempts, 0);
        s.limit = 2;
        let result = s.execute(&intent()).unwrap();
        assert_eq!(result.result["records"][0]["http_status"], 301);
        assert_eq!(result.provider_cost["request_count"], 2);
        assert_eq!(server.join().unwrap().len(), 2);
    }
    #[test]
    fn official_http_body_bound_keeps_known_receipt() {
        let (mut wire, server) = mock(vec![(
            200,
            "Retry-After: 4\r\n".into(),
            "0123456789012345".into(),
        )]);
        let error = wire
            .send(Request {
                method: "POST".into(),
                url: LWA_ENDPOINT.into(),
                query: json!({}),
                body: None,
                form: None,
                token: None,
                date: amz_date(0).unwrap(),
                limit: 10,
            })
            .err()
            .unwrap();
        assert_eq!(error.status, Some(200));
        assert!(!error.uncertain);
        assert_eq!(error.headers["retry-after"], "4");
        assert_eq!(server.join().unwrap().len(), 1);
        assert_eq!(amz_date(0).unwrap(), "19700101T000000Z");
        assert!(amz_date(u64::MAX).is_err());
    }
    #[test]
    fn official_http_fee_post_preserves_explicit_assumptions() {
        let fixtures: Value =
            serde_json::from_str(include_str!("../tests/fixtures/official-responses.json"))
                .unwrap();
        let (wire, server) = mock(vec![
            token(),
            (
                200,
                String::new(),
                fixtures["cases"][2]["response"]["raw_body"]
                    .as_str()
                    .unwrap()
                    .into(),
            ),
        ]);
        let (mut s, _) = session(wire);
        let mut request = intent();
        request.query["include"] = json!(["FEE_ESTIMATE"]);
        request.query["fee_estimate"] = json!({"listing_price":{"currency":"USD","amount":"10"},"shipping":{"currency":"USD","amount":"10"},"is_amazon_fulfilled":false,"identifier":"UmaS1","points":{"count":0,"value":{"currency":"USD","amount":"0"}}});
        let result = s.execute(&request).unwrap();
        assert_eq!(
            result.result["records"][0]["normalized"]["state"],
            "ESTIMATED"
        );
        assert!(result.result["profit_expected"].is_null());
        let requests = server.join().unwrap();
        assert!(requests[1].starts_with("POST /products/fees/v0/items/B00V5DG6IQ/feesEstimate"));
        let body: Value =
            serde_json::from_str(requests[1].split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(body, protocol(&request).unwrap()["operations"][0]["body"]);
    }
    #[test]
    fn official_unknown_transport_outcome_never_invents_zero_or_live() {
        struct Unknown;
        impl Wire for Unknown {
            fn send(&mut self, _: Request) -> Result<Response, WireError> {
                Err(WireError::unknown("OFFICIAL_HTTP_OUTCOME_UNKNOWN"))
            }
        }
        let mut s = Session {
            wire: Unknown,
            clock: TestClock(Arc::new(AtomicU64::new(1_700_000_000_000))),
            credentials: Credentials {
                client: "c".into(),
                secret: "s".into(),
                refresh: "r".into(),
            },
            authorized: true,
            limit: 2,
            attempts: 0,
            token: None,
            next: BTreeMap::new(),
            interval: BTreeMap::new(),
            last_now: 0,
            auth_next: 0,
            lwa_failed: false,
        };
        let result = s.execute(&intent()).unwrap();
        assert!(result.provider_cost["request_count"].is_null());
        assert_eq!(result.provider_cost["known_request_count"], 0);
        assert_eq!(result.result["new_live_acquisition"], false);
        assert_eq!(result.result["mode"], "INFERRED");
        assert!(result.observations.is_empty());
        s.authorized = false;
        assert_eq!(s.execute(&intent()).err().unwrap().request_count, Some(0));
        assert_eq!(s.attempts, 1);
    }
    fn read_case(op: &str) -> Value {
        reads::tests::cases()["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["operation"] == op)
            .unwrap()
            .clone()
    }
    fn read_body(op: &str) -> (u16, String, String) {
        (
            200,
            "x-amzn-RequestId: mock-receipt-1\r\n".into(),
            read_case(op)["response"]["raw_body"]
                .as_str()
                .unwrap()
                .into(),
        )
    }
    #[test]
    fn official_http_seller_reads_use_locked_paths_and_stamp_live_provenance() {
        let ops = [
            "listings_item",
            "inventory_summaries",
            "marketplace_participations",
            "product_type_search",
            "product_type_definition",
        ];
        let mut responses = vec![token()];
        responses.extend(ops.iter().map(|op| read_body(op)));
        let (wire, server) = mock(responses);
        let (mut s, _) = session(wire);
        for (i, op) in ops.iter().enumerate() {
            let request = reads::tests::read_request("AMAZON_US", read_case(op)["query"].clone());
            let result = s.execute(&request).unwrap();
            assert_eq!(result.result["mode"], "LIVE", "{op}");
            assert_eq!(result.result["status"], "COMPLETE_WITH_UNKNOWNS", "{op}");
            assert_eq!(
                result.provider_cost["request_count"],
                if i == 0 { 2 } else { 1 }
            );
            assert!(result.provider_cost["actual_cost_minor"].is_null());
            let record = &result.result["records"][0];
            assert_eq!(record["state"], "LIVE");
            assert_eq!(record["provenance"]["observation_mode"], "LIVE");
            assert_eq!(
                record["response_headers"]["x-amzn-requestid"],
                "mock-receipt-1"
            );
            for f in record["normalized"]["fields"].as_array().unwrap() {
                assert_eq!(f["observation_mode"], "LIVE");
                assert_eq!(f["source_layer"], "OFFICIAL_SP_API");
                assert_eq!(f["receipt"]["x-amzn-requestid"], "mock-receipt-1");
                assert_eq!(f["raw_capture_sha256"], record["raw_capture_sha256"]);
            }
            let observation = &result.observations[0];
            assert_eq!(observation.mode, ObservationMode::Live);
            observation.validate().unwrap();
            assert!(
                observation.normalized_value["value"]["fields"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|f| f["observation_mode"] == "LIVE")
            );
            let text = result.result.to_string();
            for secret in ["mock-secret", "mock-refresh", "mock-access-token"] {
                assert!(!text.contains(secret));
            }
            assert_eq!(
                s.next[*op] - 1_700_000_000_000,
                reads::spec(op).unwrap().min_interval_ms
            );
        }
        assert_eq!(s.token_state(), "VALID_IN_MEMORY");
        let requests: Vec<String> = server
            .join()
            .unwrap()
            .iter()
            .map(|r| r.to_ascii_lowercase())
            .collect();
        assert_eq!(requests.len(), 6);
        assert!(
            requests[1].starts_with("get /listings/2021-08-01/items/a1exampleseller/gm-zdpi-9b4e?")
        );
        assert!(requests[1].contains("marketplaceids=atvpdkikx0der"));
        assert!(
            requests[1]
                .contains("includeddata=summaries%2coffers%2cfulfillmentavailability%2cissues")
        );
        assert!(requests[2].starts_with("get /fba/inventory/v1/summaries?"));
        assert!(requests[2].contains("granularitytype=marketplace"));
        assert!(requests[2].contains("sellerskus=synth-sku-1%2csynth-sku-2"));
        assert!(requests[2].contains("details=true"));
        assert!(requests[3].starts_with("get /sellers/v1/marketplaceparticipations"));
        assert!(requests[4].starts_with("get /definitions/2020-09-01/producttypes?"));
        assert!(requests[4].contains("keywords=luggage"));
        assert!(requests[5].starts_with("get /definitions/2020-09-01/producttypes/luggage?"));
        assert!(requests[5].contains("requirements=listing"));
        for api in &requests[1..] {
            assert!(api.contains("x-amz-access-token: mock-access-token"));
            assert!(api.contains("host: sellingpartnerapi-na.amazon.com"));
            assert!(!api.contains("client_secret"));
        }
    }
    #[test]
    fn official_restricted_and_write_intents_never_reach_wire() {
        let (wire, server) = mock(vec![]);
        let (mut s, _) = session(wire);
        for query in [
            json!({"operation":"ORDERS"}),
            json!({"operation":"RESTRICTED_DATA_TOKEN"}),
            json!({"operation":"PATCH_LISTINGS_ITEM"}),
        ] {
            let error = s
                .execute(&reads::tests::read_request("AMAZON_US", query))
                .err()
                .unwrap();
            assert!(matches!(
                error.reason.as_str(),
                "RESTRICTED_DOMAIN_DISABLED" | "OFFICIAL_WRITES_DISABLED"
            ));
        }
        assert_eq!(s.attempts, 0);
        assert_eq!(s.token_state(), "NEVER_REQUESTED");
        assert!(server.join().unwrap().is_empty());
    }
    #[test]
    fn official_token_state_transitions_are_reported_without_material() {
        let (wire, server) = mock(vec![(429, "Retry-After: 10\r\n".into(), String::new())]);
        let (mut s, clock) = session(wire);
        assert_eq!(s.token_state(), "NEVER_REQUESTED");
        s.execute(&intent()).unwrap();
        assert_eq!(s.token_state(), "LWA_COOLDOWN");
        let view = doctor::SessionView {
            configured: true,
            reason: "TEST",
            attempts: s.attempts,
            limit: s.limit,
            token_state: s.token_state(),
        };
        let report = doctor::report(&doctor::MapEnv::new(&[]), Some(view));
        assert!(
            report["blockers"]
                .as_array()
                .unwrap()
                .contains(&json!("LWA_COOLDOWN"))
        );
        assert_eq!(report["request_budget"]["attempts_this_session"], 1);
        clock.fetch_add(11_000, Ordering::SeqCst);
        assert_eq!(s.token_state(), "LAST_REFRESH_FAILED");
        server.join().unwrap();
        let (wire, server) = mock(vec![token(), offer()]);
        let (mut s, clock) = session(wire);
        s.execute(&intent()).unwrap();
        assert_eq!(s.token_state(), "VALID_IN_MEMORY");
        let view = doctor::SessionView {
            configured: true,
            reason: "TEST",
            attempts: s.attempts,
            limit: s.limit,
            token_state: s.token_state(),
        };
        let text = doctor::report(&doctor::MapEnv::new(&[]), Some(view)).to_string();
        assert!(!text.contains("mock-access-token") && !text.contains("mock-secret"));
        clock.fetch_add(3_600_000, Ordering::SeqCst);
        assert_eq!(s.token_state(), "EXPIRED_RENEWAL_REQUIRED");
        server.join().unwrap();
    }
}
