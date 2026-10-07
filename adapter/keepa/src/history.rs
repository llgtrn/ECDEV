//! Keepa csv history decoding, specified from the donor's behaviour (purahmanian/keepa-mcp at
//! 84c2145, MIT) and checked against its frozen outputs (tests/fixtures/keepa-history-oracle.json).
//! A slot alternates (Keepa minutes, value), except the buy-box slot 18, whose samples are
//! (minutes, price, shipping). ECDEV diverges from the donor on purpose: a negative value is kept
//! as a span in which Keepa reports the offer unavailable (the donor drops it, stretching the
//! previous price across the gap), every point is kept (the donor downsamples to 60), and times
//! keep their minute (the donor truncates to the date).

use serde_json::Value;

/// Keepa minutes start at 2011-01-01T00:00Z, 21 564 000 minutes after the Unix epoch.
pub const KEEPA_TIME_OFFSET_MINUTES: i64 = 21_564_000;
pub const BUY_BOX_SLOT: usize = 18;

pub fn keepa_minutes_to_unix(minutes: i64) -> i64 {
    (minutes + KEEPA_TIME_OFFSET_MINUTES) * 60
}

pub fn stride(slot: usize) -> usize {
    if slot == BUY_BOX_SLOT { 3 } else { 2 }
}

/// (Unix seconds, value) points of one csv slot; None where Keepa reports no value. A trailing
/// incomplete sample is ignored, as are non-integer entries.
pub fn decode(series: &Value, slot: usize) -> Vec<(i64, Option<i64>)> {
    series
        .as_array()
        .map(|a| {
            a.chunks_exact(stride(slot))
                .filter_map(|c| {
                    let t = c[0].as_i64()?;
                    let v = c[1].as_i64()?;
                    Some((keepa_minutes_to_unix(t), (v >= 0).then_some(v)))
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn iso_date(unix: i64) -> String {
        let days = unix.div_euclid(86_400);
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        format!(
            "{:04}-{:02}-{:02}",
            yoe + era * 400 + i64::from(m <= 2),
            m,
            d
        )
    }

    #[test]
    fn donor_oracle_agrees_where_ecdev_does_not_diverge() {
        let oracle: Value =
            serde_json::from_str(include_str!("../tests/fixtures/keepa-history-oracle.json"))
                .unwrap();
        assert_eq!(
            oracle["commit_sha"],
            "84c2145c9f74bd29a91a19b12c24abe1c2535223"
        );
        for t in oracle["times"].as_array().unwrap() {
            let unix = keepa_minutes_to_unix(t["keepa_minutes"].as_i64().unwrap());
            assert!(
                t["iso"].as_str().unwrap().starts_with(&iso_date(unix)),
                "{t}"
            );
        }
        let max = oracle["max_history_points"].as_u64().unwrap() as usize;
        for case in oracle["cases"].as_array().unwrap() {
            let id = case["case_id"].as_str().unwrap();
            let slot = if case["stride"] == 3 { BUY_BOX_SLOT } else { 0 };
            let (start, end) = (case["range_start"].as_i64(), case["range_end"].as_i64());
            let mine: Vec<(String, i64)> = decode(&case["csv"], slot)
                .into_iter()
                .filter(|(t, _)| start.is_none_or(|s| *t >= s) && end.is_none_or(|e| *t <= e))
                .filter_map(|(t, v)| v.map(|v| (iso_date(t), v)))
                .collect();
            let donor: Vec<(String, i64)> = case["donor_points"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| {
                    (
                        p["date"].as_str().unwrap().to_string(),
                        p["value"].as_i64().unwrap(),
                    )
                })
                .collect();
            if mine.len() <= max {
                // Same available points, same dates, same values.
                assert_eq!(mine, donor, "{id}");
            } else {
                // The donor kept an evenly spaced subset; every one of its points is ours too.
                assert_eq!(donor.len(), max, "{id}");
                assert!(donor.iter().all(|p| mine.contains(p)), "{id}");
            }
        }
    }

    #[test]
    fn unavailable_points_and_triples_are_kept_as_keepa_states() {
        let p = decode(&json!([0, 500, 60, -1, 120, -2, 180, 600]), 0);
        assert_eq!(p.len(), 4, "the donor would keep 2");
        assert_eq!(p[1], (keepa_minutes_to_unix(60), None));
        let bb = decode(
            &json!([6000000, 3000, 200, 6001440, -1, 0, 6002880]),
            BUY_BOX_SLOT,
        );
        assert_eq!(
            bb,
            vec![
                (keepa_minutes_to_unix(6000000), Some(3000)),
                (keepa_minutes_to_unix(6001440), None)
            ]
        );
        assert_eq!(keepa_minutes_to_unix(0), 1_293_840_000);
        assert!(decode(&json!(null), 0).is_empty());
    }
}
