//! Canonical acquisition boundary. Provider models remain inside adapters.
use crate::domain::Evidence;
use serde_json::Value;
#[derive(
    Clone, Copy, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq, PartialOrd, Ord,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProviderClass {
    Native,
    Public,
    Official,
    Paid,
}
/// An intent is not evidence of IO. Unknown outcomes never become a zero-call claim.
pub(crate) fn acquisition_mode(
    fixture: bool,
    known: u64,
    uncertain: bool,
    cached: bool,
) -> &'static str {
    if fixture {
        "FIXTURE"
    } else if known > 0 {
        "LIVE"
    } else if uncertain {
        "INFERRED"
    } else if cached {
        "CACHED"
    } else {
        "PLAN_ONLY"
    }
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct BudgetPolicy {
    pub currency: String,
    pub per_run_minor: u64,
    pub per_day_minor: u64,
    pub per_month_minor: u64,
    pub per_provider_minor: u64,
    pub per_capability_minor: u64,
    pub request_ceiling_minor: u64,
}
impl Default for BudgetPolicy {
    fn default() -> Self {
        Self {
            currency: "USD".into(),
            per_run_minor: 0,
            per_day_minor: 0,
            per_month_minor: 0,
            per_provider_minor: 0,
            per_capability_minor: 0,
            request_ceiling_minor: 0,
        }
    }
}
impl BudgetPolicy {
    pub fn from_env() -> Self {
        fn get(k: &str) -> u64 {
            std::env::var(k)
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0)
        }
        Self {
            currency: std::env::var("ECDEV_RESEARCH_BUDGET_CURRENCY").unwrap_or("USD".into()),
            per_run_minor: get("ECDEV_PAID_PER_RUN_MINOR"),
            per_day_minor: get("ECDEV_PAID_PER_DAY_MINOR"),
            per_month_minor: get("ECDEV_PAID_PER_MONTH_MINOR"),
            per_provider_minor: get("ECDEV_PAID_PER_PROVIDER_MINOR"),
            per_capability_minor: get("ECDEV_PAID_PER_CAPABILITY_MINOR"),
            request_ceiling_minor: get("ECDEV_PAID_REQUEST_CEILING_MINOR"),
        }
    }
    pub fn permits(&self, day: u64, month: u64, provider: u64, capability: u64) -> bool {
        let c = self.request_ceiling_minor;
        self.currency == "USD"
            && c <= i64::MAX as u64
            && c > 0
            && c <= self.per_run_minor
            && day.checked_add(c).is_some_and(|v| v <= self.per_day_minor)
            && month
                .checked_add(c)
                .is_some_and(|v| v <= self.per_month_minor)
            && provider
                .checked_add(c)
                .is_some_and(|v| v <= self.per_provider_minor)
            && capability
                .checked_add(c)
                .is_some_and(|v| v <= self.per_capability_minor)
    }
}

/// Route selection records exclusions; uncertainty never becomes a made-up estimate.
pub fn routes(capability: &str, providers: &[Value], paid_budget: u64, candidates: usize) -> Value {
    let mut eligible = vec![];
    let mut skipped = vec![];
    for p in providers {
        if !p["capabilities"]
            .as_array()
            .is_some_and(|a| a.iter().any(|c| c == capability))
        {
            continue;
        }
        let class = p["class"].as_str().unwrap_or("PAID");
        let reason = if p["status"] != "AVAILABLE" {
            Some("UNAVAILABLE")
        } else if class == "PAID" && paid_budget == 0 {
            Some("BUDGET_ZERO")
        } else if class == "PAID" && candidates > 10 {
            Some("REDUCE_CANDIDATES_FIRST")
        } else {
            None
        };
        if let Some(reason) = reason {
            skipped.push(serde_json::json!({"provider":p["id"],"reason":reason}));
        } else {
            eligible.push(p.clone());
        }
    }
    eligible.sort_by_key(|p| match p["class"].as_str() {
        Some("NATIVE") => 0,
        Some("CACHE") => 1,
        Some("PUBLIC") => 2,
        Some("OFFICIAL") => 3,
        _ => 4,
    });
    serde_json::json!({"capability":capability,"routes":eligible,"skipped":skipped,"expected_information_gain":null,"confidence_required":"SOURCE_BACKED","freshness_required_seconds":null,"policy":"NATIVE_CACHE_PUBLIC_OFFICIAL_PAID_FINALISTS"})
}
#[cfg(test)]
mod budget_tests {
    use super::*;
    #[test]
    fn retry_after_formats_preserve_not_before_and_reject_invalid_dates() {
        let target = 784_111_777_000;
        for header in [
            "Sun, 06 Nov 1994 08:49:37 GMT",
            "Sunday, 06-Nov-94 08:49:37 GMT",
            "Sun Nov  6 08:49:37 1994",
        ] {
            assert_eq!(retry_after_not_before(header, target - 5000), Some(target));
            assert_eq!(
                retry_after_not_before(header, target + 5000),
                Some(target + 5000)
            );
        }
        assert_eq!(
            retry_after_not_before("Thu, 29 Feb 2024 00:00:00 GMT", 0),
            Some(1_709_164_800_000)
        );
        assert_eq!(
            retry_after_not_before("Sat, 03 Oct 2026 12:00:00 GMT", 0),
            Some(1_791_028_800_000)
        );
        for header in [
            "",
            "-1",
            "+1",
            "1.5",
            "1, 2",
            "Fri, 30 Feb 2024 00:00:00 GMT",
            "Thu, 29 Feb 1900 00:00:00 GMT",
            "Mon, 06 Nov 1994 08:49:37 GMT",
            "Sun,, 06 Nov 1994 08:49:37 GMT",
            "Sun, 06 Nov 1994 24:49:37 GMT",
            "Sun, 06 Nov 1994 08:60:37 GMT",
            "Sun, 06 Nov 1994 08:49:61 GMT",
            "Sun, 06 Nov 1994 08:49:37 JST",
        ] {
            assert_eq!(retry_after_not_before(header, 0), None, "{header}");
        }
        let failure = AcquireError::http(429, 2, Some("\t120 "), 10_123);
        assert_eq!(failure.retry_not_before_ms, Some(130_123));
        assert_eq!(failure.retry_delay_ms(20_123), Some(110_000));
        assert_eq!(retry_after_not_before("0", 10_123), Some(10_123));
        assert_eq!(
            retry_after_not_before("999999999999999999999999", 10_123),
            Some(i64::MAX)
        );
        let unknown: AcquireError = "DNS_FAILED".into();
        assert_eq!(unknown.request_count, None);
        assert_eq!(unknown.retry_delay_ms(100), None);
    }
    #[test]
    fn zero_budget_and_unknown_cost_deny_paid_calls() {
        assert!(!BudgetPolicy::default().permits(0, 0, 0, 0));
        let b = BudgetPolicy {
            per_run_minor: 10,
            per_day_minor: 20,
            per_month_minor: 30,
            per_provider_minor: 30,
            per_capability_minor: 30,
            request_ceiling_minor: 10,
            ..Default::default()
        };
        assert!(b.permits(10, 20, 0, 0));
        assert!(!b.permits(11, 0, 0, 0));
        assert!(!b.permits(0, u64::MAX, 0, 0));
    }
    #[test]
    fn fallback_does_not_require_paid_providers() {
        let p = vec![
            serde_json::json!({"id":"semrush","class":"PAID","status":"AVAILABLE","capabilities":["research.market"]}),
            serde_json::json!({"id":"web","class":"PUBLIC","status":"AVAILABLE","capabilities":["research.market"]}),
        ];
        let r = routes("research.market", &p, 0, 500);
        assert_eq!(r["routes"][0]["id"], "web");
        assert_eq!(r["skipped"][0]["reason"], "BUDGET_ZERO");
    }
}

#[derive(serde::Serialize)]
pub struct AcquireRequest {
    pub run_id: String,
    pub capability: String,
    pub market: String,
    pub query: Value,
}
pub struct AcquireResult {
    pub observations: Vec<Evidence>,
    pub result: Value,
    pub raw_payload: Vec<u8>,
    pub provider_cost: Value,
}
/// Acquisition failures retain machine-readable HTTP facts without disguising them as captures.
#[derive(Clone, Debug, serde::Serialize)]
pub struct AcquireError {
    pub reason: String,
    pub http_status: Option<u16>,
    pub request_count: Option<u64>,
    pub retry_after_header: Option<String>,
    pub retry_not_before_ms: Option<i64>,
}
impl From<String> for AcquireError {
    fn from(reason: String) -> Self {
        Self {
            reason,
            http_status: None,
            request_count: None,
            retry_after_header: None,
            retry_not_before_ms: None,
        }
    }
}
impl From<&str> for AcquireError {
    fn from(reason: &str) -> Self {
        reason.to_owned().into()
    }
}
impl std::fmt::Display for AcquireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reason)
    }
}
impl std::error::Error for AcquireError {}
impl AcquireError {
    pub fn http(
        status: u16,
        requests: u64,
        retry_after: Option<&str>,
        received_at_ms: i64,
    ) -> Self {
        Self {
            reason: format!("HTTP_STATUS_{status}"),
            http_status: Some(status),
            request_count: Some(requests),
            retry_after_header: retry_after.map(str::to_owned),
            retry_not_before_ms: retry_after
                .and_then(|header| retry_after_not_before(header, received_at_ms)),
        }
    }
    pub fn retry_delay_ms(&self, now_ms: i64) -> Option<i64> {
        self.retry_not_before_ms
            .map(|time| time.saturating_sub(now_ms).max(0))
    }
}

/// RFC 9110 delay-seconds and the three HTTP-date forms. Unsupported syntax remains unknown.
/// Delays beyond the clock range saturate at its last instant rather than wrapping into an early retry.
pub fn retry_after_not_before(header: &str, received_at_ms: i64) -> Option<i64> {
    let header = header.trim_matches([' ', '\t']);
    if !header.is_empty() && header.bytes().all(|b| b.is_ascii_digit()) {
        let seconds = header.bytes().fold(0_i64, |n, b| {
            n.saturating_mul(10).saturating_add((b - b'0') as i64)
        });
        return Some(received_at_ms.saturating_add(seconds.saturating_mul(1000)));
    }
    fn number(value: &str, width: usize) -> Option<i64> {
        (value.len() == width && value.bytes().all(|b| b.is_ascii_digit()))
            .then(|| value.parse().ok())
            .flatten()
    }
    fn year_start(year: i64) -> i64 {
        let last = year - 1;
        365 * last + last / 4 - last / 100 + last / 400
    }
    let fields: Vec<_> = header.split_ascii_whitespace().collect();
    let (weekday, day, month, mut year, time, obsolete) = match fields.as_slice() {
        [weekday, day, month, year, time, "GMT"] if weekday.ends_with(',') => (
            weekday.strip_suffix(',').unwrap(),
            number(day, 2)?,
            *month,
            number(year, 4)?,
            *time,
            false,
        ),
        [weekday, date, time, "GMT"] if weekday.ends_with(',') => {
            let parts: Vec<_> = date.split('-').collect();
            let [day, month, year] = parts.as_slice() else {
                return None;
            };
            (
                weekday.strip_suffix(',').unwrap(),
                number(day, 2)?,
                *month,
                number(year, 2)?,
                *time,
                true,
            )
        }
        [weekday, month, day, time, year] => (
            *weekday,
            number(day, day.len().min(2))?,
            *month,
            number(year, 4)?,
            *time,
            false,
        ),
        _ => return None,
    };
    if obsolete {
        // Interpret the two-digit year in the current century, rolling a date more than
        // fifty years ahead back to the most recent matching year in the past.
        let days = received_at_ms.div_euclid(86_400_000) + year_start(1970);
        let current_year = (1..=9999).rev().find(|y| year_start(*y) <= days)?;
        year += current_year / 100 * 100;
        if year > current_year + 50 {
            year -= 100;
        }
    }
    if !(1..=9999).contains(&year) {
        return None;
    }
    let months = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let month = months.iter().position(|m| *m == month)?;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let mut lengths = [31_i64, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if leap {
        lengths[1] = 29;
    }
    if day < 1 || day > lengths[month] {
        return None;
    }
    let clock: Vec<_> = time.split(':').collect();
    let [hour, minute, second] = clock.as_slice() else {
        return None;
    };
    let (hour, minute, second) = (number(hour, 2)?, number(minute, 2)?, number(second, 2)?);
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let days = year_start(year) - year_start(1970) + lengths[..month].iter().sum::<i64>() + day - 1;
    let expected = (days + 4).rem_euclid(7) as usize;
    let short = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    let long = [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ];
    if weekday
        != if obsolete {
            long[expected]
        } else {
            short[expected]
        }
    {
        return None;
    }
    Some(((days * 86400 + hour * 3600 + minute * 60 + second) * 1000).max(received_at_ms))
}

pub trait Provider: Send + Sync {
    fn id(&self) -> &str;
    fn metadata(&self) -> Value;
    /// Credential/readiness report without secrets or network. None when unsupported.
    fn doctor(&self) -> Option<Value> {
        None
    }
    fn normalize_query(&self, query: &Value) -> Result<Value, String> {
        Ok(query.clone())
    }
    /// Blocking IO; application transports execute this on a blocking worker.
    fn acquire(&self, request: &AcquireRequest) -> Result<AcquireResult, AcquireError>;
}
