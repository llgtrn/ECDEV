//! Small-count-safe growth between two disjoint windows. Mention counts are treated as Poisson
//! samples; conditional on their sum, the later count is binomial with p = t_after / (t_before +
//! t_after) when the rate did not change, so the exact one-sided tail is a valid p-value at any
//! count. A rise from zero has no ratio and no invented baseline; a handful of mentions cannot be
//! called growth however large its percentage.

use serde::Serialize;

pub const GROWTH_ALPHA: f64 = 0.05;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RateComparison {
    pub before: u64,
    pub after: u64,
    pub before_seconds: u64,
    pub after_seconds: u64,
    /// After-rate over before-rate; None when the before window has no mentions.
    pub rate_ratio: Option<f64>,
    pub p_rise: f64,
    pub p_fall: f64,
    pub alpha: f64,
    pub state: &'static str,
    pub method: &'static str,
}

fn log_sum_exp(a: f64, b: f64) -> f64 {
    if a == f64::NEG_INFINITY {
        return b;
    }
    let m = a.max(b);
    m + ((a - m).exp() + (b - m).exp()).ln()
}

/// (P(X >= k), P(X <= k)) for X ~ Binomial(n, p), 0 < p < 1.
fn binomial_tails(n: u64, k: u64, p: f64) -> (f64, f64) {
    let (lp, lq) = (p.ln(), (1. - p).ln());
    let mut log_pmf = n as f64 * lq;
    let (mut upper, mut lower) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for i in 0..=n {
        if i >= k {
            upper = log_sum_exp(upper, log_pmf);
        }
        if i <= k {
            lower = log_sum_exp(lower, log_pmf);
        }
        if i < n {
            log_pmf += ((n - i) as f64 / (i + 1) as f64).ln() + lp - lq;
        }
    }
    (upper.exp().min(1.), lower.exp().min(1.))
}

/// Compares mention rates in two disjoint windows. None when a window has no duration.
pub fn rate_comparison(
    before: u64,
    before_seconds: u64,
    after: u64,
    after_seconds: u64,
) -> Option<RateComparison> {
    if before_seconds == 0 || after_seconds == 0 {
        return None;
    }
    let n = before + after;
    let p = after_seconds as f64 / (before_seconds + after_seconds) as f64;
    let (p_rise, p_fall) = if n == 0 {
        (1., 1.)
    } else {
        binomial_tails(n, after, p)
    };
    let rate_ratio = (before > 0)
        .then(|| (after as f64 / after_seconds as f64) / (before as f64 / before_seconds as f64));
    let state = if n == 0 {
        "NO_MENTIONS_IN_EITHER_WINDOW"
    } else if p_rise < GROWTH_ALPHA {
        "RISING"
    } else if p_fall < GROWTH_ALPHA {
        "FALLING"
    } else {
        "NO_DETECTABLE_CHANGE"
    };
    Some(RateComparison {
        before,
        after,
        before_seconds,
        after_seconds,
        rate_ratio,
        p_rise,
        p_fall,
        alpha: GROWTH_ALPHA,
        state,
        method: "EXACT_CONDITIONAL_BINOMIAL_POISSON_RATE_TEST_ONE_SIDED",
    })
}

/// Growth of a source-reported daily attention series (a Mastodon tag's uses and accounts per
/// UTC day): the last three complete days against the three before them. The day still open at
/// capture is never compared. Statuses cluster, so the Poisson test is optimistic; distinct
/// accounts per day (account-days) is the steadier of the two and is reported first.
pub fn daily_attention_growth(usage: &serde_json::Value) -> serde_json::Value {
    use serde_json::json;
    let days: Vec<(u64, u64, u64)> = usage["series"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|d| d["complete"] == true)
        .filter_map(|d| {
            Some((
                d["day"].as_u64()?,
                d["uses"].as_u64()?,
                d["accounts"].as_u64()?,
            ))
        })
        .collect();
    const WINDOW: usize = 3;
    let consecutive = days.windows(2).all(|w| w[1].0 == w[0].0 + 86_400);
    if days.len() < 2 * WINDOW || !consecutive {
        return json!({"state":"INSUFFICIENT_COMPLETE_DAYS","complete_days":days.len(),"required":2 * WINDOW,"consecutive":consecutive,"evidence_class":"ATTENTION_SIGNAL_NOT_DEMAND"});
    }
    let recent = &days[days.len() - 2 * WINDOW..];
    let (before, after) = recent.split_at(WINDOW);
    let sum =
        |rows: &[(u64, u64, u64)], f: fn(&(u64, u64, u64)) -> u64| rows.iter().map(f).sum::<u64>();
    let seconds = WINDOW as u64 * 86_400;
    let accounts = rate_comparison(sum(before, |d| d.2), seconds, sum(after, |d| d.2), seconds);
    let uses = rate_comparison(sum(before, |d| d.1), seconds, sum(after, |d| d.1), seconds);
    json!({"state":accounts.as_ref().map(|a| a.state),"account_days":accounts,"uses":uses,
        "before":{"from":before[0].0,"until":before[WINDOW - 1].0 + 86_400},
        "after":{"from":after[0].0,"until":after[WINDOW - 1].0 + 86_400},
        "basis":"LAST_3_COMPLETE_UTC_DAYS_VS_PRIOR_3_OPEN_DAY_EXCLUDED","dispersion":"POISSON_ASSUMED_STATUS_CLUSTERING_UNMODELLED",
        "evidence_class":"ATTENTION_SIGNAL_NOT_DEMAND"})
}

/// Seconds of (start, end] covered by the union of (s, e] intervals.
pub fn covered_seconds(start: u64, end: u64, intervals: &[(u64, u64)]) -> u64 {
    let mut clipped: Vec<(u64, u64)> = intervals
        .iter()
        .map(|&(s, e)| (s.max(start), e.min(end)))
        .filter(|(s, e)| s < e)
        .collect();
    clipped.sort_unstable();
    let (mut total, mut reach) = (0, start);
    for (s, e) in clipped {
        let s = s.max(reach);
        if e > s {
            total += e - s;
            reach = e;
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_counts_are_not_growth_however_large_the_percentage() {
        // 1 -> 3 is +200 %, 0 -> 4 is unbounded; neither is distinguishable from noise.
        let a = rate_comparison(1, 3600, 3, 3600).unwrap();
        assert_eq!((a.state, a.rate_ratio), ("NO_DETECTABLE_CHANGE", Some(3.)));
        assert!((a.p_rise - 5. / 16.).abs() < 1e-12);
        let z = rate_comparison(0, 3600, 4, 3600).unwrap();
        assert_eq!((z.state, z.rate_ratio), ("NO_DETECTABLE_CHANGE", None));
        assert!((z.p_rise - 1. / 16.).abs() < 1e-12);
    }

    #[test]
    fn sustained_counts_are_detected_both_ways() {
        let up = rate_comparison(0, 3600, 5, 3600).unwrap();
        assert_eq!(up.state, "RISING");
        assert!((up.p_rise - 1. / 32.).abs() < 1e-12);
        assert_eq!(
            rate_comparison(40, 3600, 20, 3600).unwrap().state,
            "FALLING"
        );
        assert_eq!(
            rate_comparison(20, 3600, 24, 3600).unwrap().state,
            "NO_DETECTABLE_CHANGE"
        );
        // Unequal windows: the same count over twice the time is no rise.
        assert_eq!(
            rate_comparison(10, 7200, 10, 3600).unwrap().rate_ratio,
            Some(2.)
        );
        assert_eq!(rate_comparison(0, 0, 3, 3600), None);
        assert_eq!(
            rate_comparison(0, 60, 0, 60).unwrap().state,
            "NO_MENTIONS_IN_EITHER_WINDOW"
        );
        let big = rate_comparison(2000, 3600, 2100, 3600).unwrap();
        assert!(big.p_rise.is_finite() && big.p_fall.is_finite());
    }

    #[test]
    fn daily_attention_compares_complete_days_only() {
        use serde_json::json;
        let day = |d: u64, uses: u64, accounts: u64, complete: bool| json!({"day":d * 86_400,"uses":uses,"accounts":accounts,"complete":complete});
        let mut series: Vec<_> = (0..3).map(|d| day(d, 4, 2, true)).collect();
        series.extend((3..6).map(|d| day(d, 40, 12, true)));
        series.push(day(6, 0, 0, false));
        let g = daily_attention_growth(&json!({"series":series}));
        assert_eq!(g["state"], "RISING");
        assert_eq!(
            (
                g["account_days"]["before"].clone(),
                g["account_days"]["after"].clone()
            ),
            (json!(6), json!(36))
        );
        assert_eq!(g["after"]["until"], 6 * 86_400, "the open day is excluded");
        assert_eq!(g["evidence_class"], "ATTENTION_SIGNAL_NOT_DEMAND");
        // Fewer than six complete days, or a gap, is not compared.
        let short = daily_attention_growth(&json!({"series":&series[1..]}));
        assert_eq!(short["state"], "INSUFFICIENT_COMPLETE_DAYS");
        let gap: Vec<_> = (0..3).chain(4..8).map(|d| day(d, 9, 3, true)).collect();
        assert_eq!(
            daily_attention_growth(&json!({"series":gap}))["state"],
            "INSUFFICIENT_COMPLETE_DAYS"
        );
    }

    #[test]
    fn coverage_is_the_union_of_captured_windows() {
        assert_eq!(
            covered_seconds(100, 200, &[(50, 120), (110, 150), (180, 400)]),
            70
        );
        assert_eq!(covered_seconds(100, 200, &[]), 0);
        assert_eq!(covered_seconds(100, 200, &[(0, 1000)]), 100);
    }
}
