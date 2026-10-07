//! Population coverage of one social source's acquisition, from what was actually fetched. Time
//! is the trustworthy denominator: a slice whose pages ran to the source's own end covers its
//! span; anything else covers only what was seen. An item ratio is reported only when every
//! source total is exact by the source's own statement; approximate or capped totals (Algolia
//! nbHits, Bluesky hitsTotal) are carried as reported and never divided by.

use serde_json::{Value, json};

/// Stops after which a slice holds everything the source would return for it.
const ENDED: [&str; 2] = ["END_OF_RESULTS", "EMPTY_PAGE"];

fn ended(s: &Value) -> bool {
    s["stop"].as_str().is_some_and(|x| ENDED.contains(&x))
}

pub const MAX_SLICES: usize = 31;
pub const MIN_SLICE_SECONDS: u64 = 3_600;

/// (since, until] slices covering (start, end], newest first, `width` seconds each (at least an
/// hour; the oldest may be shorter), at most `MAX_SLICES`; a window that needs more slices is
/// covered newest first and the rest reported as never planned, not invented.
pub fn plan_slices(start: u64, end: u64, width: u64) -> Vec<(u64, u64)> {
    let width = width.max(MIN_SLICE_SECONDS);
    let mut out = vec![];
    let mut until = end;
    while until > start && out.len() < MAX_SLICES {
        let since = until.saturating_sub(width).max(start);
        out.push((since, until));
        until = since;
    }
    out
}

/// The narrowest slice width, at least the requested one, whose slices cover (start, end] in at
/// most `affordable` slices (and at most MAX_SLICES): a budget too tight for the requested width
/// samples the whole window more coarsely instead of leaving its oldest part unread.
pub fn fit_width(start: u64, end: u64, width: u64, affordable: u64) -> u64 {
    let span = end.saturating_sub(start);
    let slots = affordable.clamp(1, MAX_SLICES as u64);
    width.max(MIN_SLICE_SECONDS).max(span.div_ceil(slots))
}

pub fn population_coverage(start: u64, end: u64, sliced: bool, slices: &[Value]) -> Value {
    let span = end.saturating_sub(start).max(1) as f64;
    let acquired: Vec<&Value> = slices.iter().filter(|s| s["acquired"] == true).collect();
    let stops: Vec<&str> = slices.iter().filter_map(|s| s["stop"].as_str()).collect();
    let earliest = acquired.iter().filter_map(|s| s["earliest"].as_u64()).min();
    let latest = acquired.iter().filter_map(|s| s["latest"].as_u64()).max();
    let covered_seconds: f64 = if sliced {
        // Each slice is read newest first: one cut short (page limit, refused cursor) still
        // covers its span back to its oldest item, as an unsliced query does.
        acquired
            .iter()
            .map(|s| {
                let (a, b) = (
                    s["since"].as_u64().unwrap_or(start).max(start),
                    s["until"].as_u64().unwrap_or(end).min(end),
                );
                let a = if ended(s) {
                    a
                } else {
                    s["earliest"].as_u64().map_or(b, |e| e.clamp(a, b))
                };
                b.saturating_sub(a) as f64
            })
            .sum::<f64>()
            // An empty float sum is -0.0; reported coverage is never negative zero.
            + 0.
    } else if acquired.iter().all(|s| ended(s)) && !acquired.is_empty() {
        span
    } else {
        // An unbounded newest-first query covers the window back to its oldest item only.
        earliest.map_or(0., |e| end.saturating_sub(e.max(start)) as f64)
    };
    let temporal = (covered_seconds / span).min(1.);
    let gaps: Vec<Value> = slices
        .iter()
        .filter(|s| !(s["acquired"] == true && ended(s)))
        .map(|s| {
            // A slice read newest first and cut short is a gap only below its oldest item.
            let until = match (s["acquired"] == true && sliced, s["earliest"].as_u64()) {
                (true, Some(e)) => json!(e),
                _ => s["until"].clone(),
            };
            json!({"since":s["since"],"until":until,"reason":s["stop"]})
        })
        .collect();
    let totals: Vec<&Value> = acquired
        .iter()
        .map(|s| &s["source_total"])
        .filter(|t| !t.is_null())
        .collect();
    let exact = !totals.is_empty()
        && acquired.len() == totals.len()
        && totals.iter().all(|t| t["exactness"] == "EXACT_BY_SOURCE");
    let items: u64 = acquired.iter().filter_map(|s| s["posts"].as_u64()).sum();
    let reported: Option<u64> =
        exact.then(|| totals.iter().filter_map(|t| t["value"].as_u64()).sum());
    let any = |x: &str| stops.contains(&x);
    let state = if any("PAGE_FAILED") {
        "NETWORK_INTERRUPTED"
    } else if any("REQUEST_BUDGET") || any("NOT_ACQUIRED_REQUEST_BUDGET") {
        "PARTIAL_REQUEST_BUDGET"
    } else if any("PAGE_LIMIT") {
        "PARTIAL_PAGE_LIMIT"
    } else if any("SOURCE_REFUSED_FURTHER_PAGES") {
        "PARTIAL_SOURCE_REFUSED_PAGES"
    } else if temporal < 1. {
        "PARTIAL_TIME_WINDOW"
    } else {
        "COMPLETE_BY_SOURCE"
    };
    json!({
        "requested_start": start, "requested_end": end, "sliced": sliced,
        "slices_planned": slices.len(), "slices_acquired": acquired.len(),
        "items_observed": items, "pages_observed": acquired.iter().filter_map(|s| s["pages"].as_u64()).sum::<u64>(),
        "observed_earliest": earliest, "observed_latest": latest,
        "temporal_span_coverage": temporal, "temporal_basis": "SECONDS_OF_REQUESTED_WINDOW_FETCHED_TO_THE_SOURCE_END",
        "pagination_complete": !acquired.is_empty() && acquired.iter().all(|s| ended(s)) && acquired.len() == slices.len(),
        "source_reported_totals": totals,
        "coverage_ratio": reported.filter(|r| *r > 0).map(|r| items as f64 / r as f64),
        "coverage_ratio_basis": if exact { "SOURCE_TOTAL_EXACT_BY_SOURCE" } else { "NONE_SOURCE_TOTAL_UNKNOWN_OR_APPROXIMATE" },
        "total_state": if exact { "SOURCE_TOTAL_EXACT" } else { "SOURCE_TOTAL_UNKNOWN" },
        "gaps": gaps, "state": state,
    })
}

/// What a trend snapshot rests on, across its sources: how it was acquired and how complete the
/// sources say it is. It qualifies the snapshot's signals and never changes them: mention counts,
/// velocity and the score are computed from the posts alone.
pub fn population_evidence(paginations: &[Value], mode: &str) -> Value {
    let sources: Vec<Value> = paginations
        .iter()
        .map(|p| {
            let c = &p["population_coverage"];
            let slices = p["slices"].as_array().map_or(0, Vec::len);
            let basis = if c["sliced"] == true {
                "TIME_SLICED"
            } else if p["pages"].as_u64().unwrap_or(0) > 1 {
                "MULTI_PAGE"
            } else {
                "FIRST_PAGE_ONLY"
            };
            json!({"platform":p["platform"],"source_group":p["source_group"],"basis":basis,"slices":slices,"pages":p["pages"],"items_observed":c["items_observed"],"state":c["state"],"temporal_span_coverage":c["temporal_span_coverage"],"comment_tree_state":c["comment_tree_state"],"total_state":c["total_state"]})
        })
        .collect();
    let states: Vec<&str> = sources.iter().filter_map(|s| s["state"].as_str()).collect();
    let grade = if mode == "CACHED" || sources.is_empty() {
        "UNKNOWN_NOT_ACQUIRED_IN_THIS_RUN"
    } else if states.iter().all(|s| *s == "COMPLETE_BY_SOURCE") {
        "COMPLETE_BY_SOURCE"
    } else if states.contains(&"NETWORK_INTERRUPTED") {
        "INTERRUPTED"
    } else {
        "PARTIAL"
    };
    let spans: Vec<f64> = sources
        .iter()
        .filter_map(|s| s["temporal_span_coverage"].as_f64())
        .collect();
    json!({
        "grade": grade,
        "sources": sources,
        "min_temporal_span_coverage": spans.iter().copied().reduce(f64::min),
        "items_observed": paginations.iter().filter_map(|p| p["population_coverage"]["items_observed"].as_u64()).sum::<u64>(),
        "role": "QUALIFIES_SIGNALS_NEVER_CHANGES_THEM",
        "total_state": "SOURCE_TOTALS_NEVER_USED_AS_DENOMINATORS_UNLESS_EXACT",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slices_widen_to_cover_the_window_within_the_budget() {
        let day = 86_400;
        // 3 days in 6-hour slices need 12; 10 affordable widen them to 7.2 hours, all covered.
        let w = fit_width(0, 3 * day, 6 * 3600, 10);
        assert_eq!(w, 25_920);
        assert_eq!(plan_slices(0, 3 * day, w).len(), 10);
        // Enough budget keeps the requested width; the 31-slice cap applies too.
        assert_eq!(fit_width(0, 3 * day, 6 * 3600, 20), 6 * 3600);
        assert!(plan_slices(0, 30 * day, fit_width(0, 30 * day, 3600, 100)).len() <= MAX_SLICES);
        assert_eq!(
            plan_slices(0, 30 * day, fit_width(0, 30 * day, 3600, 100))
                .last()
                .unwrap()
                .0,
            0
        );
    }

    fn slice(since: u64, until: u64, stop: &str, posts: u64) -> Value {
        json!({"since":since,"until":until,"acquired":stop != "NOT_ACQUIRED_REQUEST_BUDGET","stop":stop,"pages":1,"posts":posts,"earliest":since + 1,"latest":until,"source_total":{"value":posts * 10,"exactness":"APPROXIMATE"}})
    }

    #[test]
    fn evidence_grades_say_what_a_snapshot_rests_on() {
        let p = |pages: u64, sliced: bool, state: &str| json!({"platform":"HACKER_NEWS","pages":pages,"slices":[{}],"population_coverage":{"sliced":sliced,"state":state,"items_observed":50,"temporal_span_coverage":0.25}});
        let first = population_evidence(&[p(1, false, "PARTIAL_PAGE_LIMIT")], "LIVE");
        assert_eq!(
            (first["grade"].clone(), first["sources"][0]["basis"].clone()),
            (json!("PARTIAL"), json!("FIRST_PAGE_ONLY"))
        );
        assert_eq!(
            population_evidence(&[p(4, false, "COMPLETE_BY_SOURCE")], "LIVE")["sources"][0]["basis"],
            "MULTI_PAGE"
        );
        assert_eq!(
            population_evidence(&[p(7, true, "COMPLETE_BY_SOURCE")], "LIVE")["grade"],
            "COMPLETE_BY_SOURCE"
        );
        assert_eq!(
            population_evidence(
                &[
                    p(1, false, "NETWORK_INTERRUPTED"),
                    p(1, true, "COMPLETE_BY_SOURCE")
                ],
                "LIVE"
            )["grade"],
            "INTERRUPTED"
        );
        assert_eq!(
            population_evidence(&[], "CACHED")["grade"],
            "UNKNOWN_NOT_ACQUIRED_IN_THIS_RUN"
        );
        assert_eq!(first["min_temporal_span_coverage"], 0.25);
    }

    #[test]
    fn slices_tile_the_window_newest_first_without_overlap() {
        assert_eq!(plan_slices(0, 7 * 86_400, 86_400).len(), 7);
        assert_eq!(
            plan_slices(0, 7 * 86_400, 86_400)[0],
            (6 * 86_400, 7 * 86_400)
        );
        assert_eq!(
            plan_slices(100, 10_000, 3_600),
            vec![(6_400, 10_000), (2_800, 6_400), (100, 2_800)]
        );
        assert_eq!(plan_slices(0, 100 * 86_400, 86_400).len(), MAX_SLICES);
        assert_eq!(plan_slices(0, 7_200, 60), vec![(3_600, 7_200), (0, 3_600)]);
        assert!(plan_slices(5, 5, 3_600).is_empty());
    }

    #[test]
    fn time_is_the_denominator_and_approximate_totals_never_are() {
        let full = population_coverage(
            0,
            200,
            true,
            &[
                slice(100, 200, "END_OF_RESULTS", 5),
                slice(0, 100, "EMPTY_PAGE", 0),
            ],
        );
        assert_eq!(
            (
                full["state"].clone(),
                full["temporal_span_coverage"].clone()
            ),
            (json!("COMPLETE_BY_SOURCE"), json!(1.0))
        );
        assert!(full["coverage_ratio"].is_null());
        assert_eq!(full["total_state"], "SOURCE_TOTAL_UNKNOWN");
        assert_eq!(full["gaps"], json!([]));
        // A budget-cut slice is a gap, not data, and halves the covered time.
        let cut = population_coverage(
            0,
            200,
            true,
            &[
                slice(100, 200, "END_OF_RESULTS", 5),
                slice(0, 100, "NOT_ACQUIRED_REQUEST_BUDGET", 0),
            ],
        );
        assert_eq!(
            (cut["state"].clone(), cut["temporal_span_coverage"].clone()),
            (json!("PARTIAL_REQUEST_BUDGET"), json!(0.5))
        );
        assert_eq!(cut["gaps"][0]["reason"], "NOT_ACQUIRED_REQUEST_BUDGET");
        assert_eq!(cut["slices_acquired"], 1);
        // A slice cut by the page limit, read newest first, covers its span back to its
        // oldest item; the rest of it is a gap.
        let mut cut_short = slice(100, 200, "PAGE_LIMIT", 250);
        cut_short["earliest"] = json!(160);
        let paged = population_coverage(
            0,
            200,
            true,
            &[cut_short, slice(0, 100, "END_OF_RESULTS", 3)],
        );
        assert_eq!(
            (
                paged["state"].clone(),
                paged["temporal_span_coverage"].clone()
            ),
            (json!("PARTIAL_PAGE_LIMIT"), json!(0.7))
        );
        assert_eq!(
            paged["gaps"],
            json!([{"since":100,"until":160,"reason":"PAGE_LIMIT"}])
        );
        // A slice that failed before any item covers nothing, and nothing is never -0.0.
        let failed = json!({"since":0,"until":200,"acquired":false,"stop":"PAGE_FAILED","pages":0,"posts":0});
        let f = population_coverage(0, 200, true, &[failed]);
        assert!(
            f["temporal_span_coverage"]
                .as_f64()
                .unwrap()
                .is_sign_positive()
        );
        assert_eq!(f["state"], "NETWORK_INTERRUPTED");
    }

    #[test]
    fn unsliced_queries_cover_back_to_their_oldest_item_only() {
        let one = json!({"acquired":true,"stop":"PAGE_LIMIT","pages":1,"posts":50,"earliest":150,"latest":200,"source_total":{"value":318608,"exactness":"APPROXIMATE"}});
        let c = population_coverage(0, 200, false, &[one]);
        assert_eq!(
            (c["state"].clone(), c["temporal_span_coverage"].clone()),
            (json!("PARTIAL_PAGE_LIMIT"), json!(0.25))
        );
        let refused = json!({"acquired":true,"stop":"SOURCE_REFUSED_FURTHER_PAGES","pages":1,"posts":25,"earliest":190,"latest":200});
        assert_eq!(
            population_coverage(0, 200, false, &[refused])["state"],
            "PARTIAL_SOURCE_REFUSED_PAGES"
        );
        let ended = json!({"acquired":true,"stop":"END_OF_RESULTS","pages":2,"posts":7,"earliest":150,"latest":200,"source_total":{"value":7,"exactness":"EXACT_BY_SOURCE"}});
        let e = population_coverage(0, 200, false, &[ended]);
        assert_eq!(
            (
                e["state"].clone(),
                e["coverage_ratio"].clone(),
                e["total_state"].clone()
            ),
            (
                json!("COMPLETE_BY_SOURCE"),
                json!(1.0),
                json!("SOURCE_TOTAL_EXACT")
            )
        );
        let failed = json!({"acquired":true,"stop":"PAGE_FAILED","pages":1,"posts":50,"earliest":100,"latest":200});
        assert_eq!(
            population_coverage(0, 200, false, &[failed])["state"],
            "NETWORK_INTERRUPTED"
        );
    }
}
