//! Metrics over dated series an external provider reports (for example Keepa price and sales-rank
//! history). The points are the provider's claims, not ECDEV observations: every result is typed
//! EXTERNAL_HISTORY, and a sales-rank series is a DEMAND_PROXY, never sales. A series is a step
//! function: each point holds from its time until the next point, the last one until `as_of`. A
//! point without a value is a span in which the provider reports the offer unavailable; it is
//! kept, never bridged by the previous price.

use serde_json::{Value, json};

pub const EXTERNAL_HISTORY: &str = "EXTERNAL_HISTORY";
pub const DEMAND_PROXY: &str = "DEMAND_PROXY";
const DAY: i64 = 86_400;
/// A drop of at least this many basis points from the previous available price is a discount.
pub const DISCOUNT_BPS: i64 = 500;

/// (start, end, value) spans of a step series up to `as_of`, ignoring zero-length spans.
fn spans(points: &[(i64, Option<i64>)], as_of: i64) -> Vec<(i64, i64, Option<i64>)> {
    let mut sorted = points.to_vec();
    sorted.sort_by_key(|p| p.0);
    sorted
        .iter()
        .enumerate()
        .filter_map(|(i, (t, v))| {
            let end = sorted.get(i + 1).map_or(as_of, |n| n.0).min(as_of);
            (end > *t).then_some((*t, end, *v))
        })
        .collect()
}

/// Time-weighted quantile of (value, seconds) pairs.
fn weighted_quantile(values: &[(i64, i64)], q: f64) -> Option<i64> {
    let total: i64 = values.iter().map(|v| v.1).sum();
    if total <= 0 {
        return None;
    }
    let mut v = values.to_vec();
    v.sort();
    let target = q * total as f64;
    let mut acc = 0f64;
    for (value, w) in &v {
        acc += *w as f64;
        if acc >= target {
            return Some(*value);
        }
    }
    v.last().map(|x| x.0)
}

fn available(spans: &[(i64, i64, Option<i64>)], from: i64, to: i64) -> Vec<(i64, i64)> {
    spans
        .iter()
        .filter_map(|(s, e, v)| {
            let (a, b) = ((*s).max(from), (*e).min(to));
            v.filter(|_| b > a).map(|v| (v, b - a))
        })
        .collect()
}

/// Price stability, discount frequency and trend from an external price series.
pub fn price_history_metrics(points: &[(i64, Option<i64>)], as_of: i64, currency: &str) -> Value {
    let s = spans(points, as_of);
    if s.is_empty() {
        return json!({"state":EXTERNAL_HISTORY,"status":"NO_HISTORY","currency":currency});
    }
    let first = s[0].0;
    let span_seconds = as_of - first;
    let avail = available(&s, first, as_of);
    let available_seconds: i64 = avail.iter().map(|a| a.1).sum();
    let mean = (available_seconds > 0).then(|| {
        avail
            .iter()
            .map(|(v, w)| *v as f64 * *w as f64)
            .sum::<f64>()
            / available_seconds as f64
    });
    let cv_bps = mean.filter(|m| *m > 0.0).map(|m| {
        let var = avail
            .iter()
            .map(|(v, w)| (*v as f64 - m).powi(2) * *w as f64)
            .sum::<f64>()
            / available_seconds as f64;
        (var.sqrt() / m * 10_000.0).round() as i64
    });
    let mut sorted = points.to_vec();
    sorted.sort_by_key(|p| p.0);
    let available_points: Vec<(i64, i64)> = sorted
        .iter()
        .filter_map(|(t, v)| v.map(|v| (*t, v)))
        .collect();
    let discounts = available_points
        .windows(2)
        .filter(|w| w[0].1 > 0 && (w[1].1 - w[0].1) * 10_000 <= -DISCOUNT_BPS * w[0].1)
        .count();
    let window = |from: i64, to: i64| {
        let a = available(&s, from, to);
        let covered: i64 = a.iter().map(|x| x.1).sum();
        (covered >= 7 * DAY)
            .then(|| weighted_quantile(&a, 0.5))
            .flatten()
    };
    let (recent, prior) = (
        window(as_of - 90 * DAY, as_of),
        window(as_of - 180 * DAY, as_of - 90 * DAY),
    );
    let trend_bps = recent
        .zip(prior)
        .filter(|(_, p)| *p > 0)
        .map(|(r, p)| (r - p) * 10_000 / p);
    let latest = available_points.last();
    json!({
        "state": EXTERNAL_HISTORY, "basis": "PROVIDER_REPORTED_STEP_SERIES_TIME_WEIGHTED", "currency": currency,
        "first_point_at": first, "as_of": as_of, "points": points.len(),
        "span_days": span_seconds as f64 / DAY as f64,
        "unavailable_share": if span_seconds > 0 { 1.0 - available_seconds as f64 / span_seconds as f64 } else { 0.0 },
        "low_minor": weighted_quantile(&avail, 0.0), "p25_minor": weighted_quantile(&avail, 0.25), "median_minor": weighted_quantile(&avail, 0.5),
        "p75_minor": weighted_quantile(&avail, 0.75), "high_minor": avail.iter().map(|a| a.0).max(),
        "latest_available_minor": latest.map(|l| l.1), "latest_available_at": latest.map(|l| l.0),
        "stability_cv_bps": cv_bps,
        "discount_events": discounts, "discount_rule_bps": DISCOUNT_BPS,
        "discount_events_per_30_days": if span_seconds > 0 { discounts as f64 * 30.0 * DAY as f64 / span_seconds as f64 } else { 0.0 },
        "trend_90d_vs_prior_90d_bps": trend_bps,
        "trend_status": if trend_bps.is_some() { "DERIVED" } else { "INSUFFICIENT_COVERAGE_7_DAYS_PER_WINDOW" },
    })
}

/// Rank velocity and volatility over the last `days` days, sampled daily from the step series.
/// Lower rank is better, so a negative velocity is an improving rank. It is a demand proxy only.
pub fn rank_history_metrics(points: &[(i64, Option<i64>)], as_of: i64, days: i64) -> Value {
    let s = spans(points, as_of);
    let at = |t: i64| {
        s.iter()
            .find(|(a, b, _)| *a <= t && t < *b)
            .and_then(|x| x.2)
            .filter(|r| *r > 0)
    };
    let daily: Vec<(i64, f64)> = (0..=days)
        .rev()
        .map(|d| as_of - d * DAY - 1)
        .filter_map(|t| at(t).map(|r| (t, (r as f64).ln())))
        .collect();
    let changes: Vec<f64> = daily
        .windows(2)
        .filter(|w| w[1].0 - w[0].0 == DAY)
        .map(|w| w[1].1 - w[0].1)
        .collect();
    let n = changes.len();
    let velocity = (daily.len() >= 2).then(|| {
        let (a, b) = (daily[0], daily[daily.len() - 1]);
        (b.1 - a.1) / ((b.0 - a.0) as f64 / DAY as f64)
    });
    let volatility = (n >= 2).then(|| {
        let m = changes.iter().sum::<f64>() / n as f64;
        (changes.iter().map(|c| (c - m).powi(2)).sum::<f64>() / (n - 1) as f64).sqrt()
    });
    json!({
        "state": EXTERNAL_HISTORY, "evidence_class": DEMAND_PROXY, "meaning": "SALES_RANK_NOT_SALES",
        "window_days": days, "daily_samples": daily.len(),
        "latest_rank": daily.last().map(|d| d.1.exp().round() as i64),
        "log_rank_velocity_per_day": velocity, "log_rank_volatility_daily": volatility,
        "direction": velocity.map(|v| if v < -0.005 { "IMPROVING" } else if v > 0.005 { "WORSENING" } else { "FLAT" }),
        "status": if daily.len() >= 2 { "DERIVED" } else { "INSUFFICIENT_DAILY_SAMPLES" },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_spans_are_kept_not_bridged() {
        // 1000 for 10 days, unavailable for 10 days, 800 for 10 days.
        let p = vec![(0, Some(1000)), (10 * DAY, None), (20 * DAY, Some(800))];
        let m = price_history_metrics(&p, 30 * DAY, "JPY");
        assert!((m["unavailable_share"].as_f64().unwrap() - 1.0 / 3.0).abs() < 1e-9);
        assert_eq!(
            (m["low_minor"].clone(), m["high_minor"].clone()),
            (json!(800), json!(1000))
        );
        assert_eq!(
            m["discount_events"], 1,
            "1000 -> 800 across the gap is a 20% drop"
        );
        assert_eq!(m["latest_available_minor"], 800);
        assert_eq!(m["state"], EXTERNAL_HISTORY);
        assert_eq!(m["trend_status"], "INSUFFICIENT_COVERAGE_7_DAYS_PER_WINDOW");
    }

    #[test]
    fn trend_compares_recent_and_prior_quarters() {
        let p = vec![(0, Some(1000)), (90 * DAY, Some(1100))];
        let m = price_history_metrics(&p, 180 * DAY, "USD");
        assert_eq!(m["trend_90d_vs_prior_90d_bps"], 1000);
        assert_eq!(m["discount_events"], 0);
        assert_eq!(m["stability_cv_bps"], 476);
        assert_eq!(
            price_history_metrics(&[], 10, "USD")["status"],
            "NO_HISTORY"
        );
    }

    #[test]
    fn rank_is_a_demand_proxy_with_direction() {
        // Rank halves over 10 days: improving.
        let p: Vec<(i64, Option<i64>)> = (0..=10).map(|d| (d * DAY, Some(1000 - 50 * d))).collect();
        let m = rank_history_metrics(&p, 11 * DAY, 10);
        assert_eq!(m["evidence_class"], DEMAND_PROXY);
        assert_eq!(m["meaning"], "SALES_RANK_NOT_SALES");
        assert_eq!(m["direction"], "IMPROVING");
        assert!(m["log_rank_velocity_per_day"].as_f64().unwrap() < 0.0);
        // Rank 0 and unavailable spans give no samples.
        let none = rank_history_metrics(&[(0, Some(0)), (DAY, None)], 3 * DAY, 2);
        assert_eq!(none["status"], "INSUFFICIENT_DAILY_SAMPLES");
    }
}
