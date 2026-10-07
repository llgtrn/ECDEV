//! Keepa boundary normalization derived from a reviewed, commit-locked behavior contract.
//! No donor runtime, network requests, currency inference or live observations.
use serde_json::Value;
pub mod client;
pub mod history;

fn non_negative(value: Option<&Value>) -> Option<i64> {
    value.and_then(Value::as_i64).filter(|v| *v >= 0)
}

/// Stats -> last complete usable history sample -> endpoint current[] -> legacy.
/// The slot-18 buybox CSV uses timestamp/price/shipping triples; other slots use pairs.
pub fn resolve_current(source: &Value, index: usize, legacy: Option<&Value>) -> Option<i64> {
    if let Some(value) = non_negative(
        source
            .get("stats")
            .and_then(|s| s.get("current"))
            .and_then(|a| a.get(index)),
    ) {
        return Some(value);
    }
    let stride = if index == 18 { 3 } else { 2 };
    if let Some(series) = source
        .get("csv")
        .and_then(|a| a.get(index))
        .and_then(Value::as_array)
    {
        for sample in series.chunks_exact(stride).rev() {
            if let Some(value) = non_negative(sample.get(1)) {
                return Some(value);
            }
        }
    }
    non_negative(source.get("current").and_then(|a| a.get(index))).or_else(|| non_negative(legacy))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn donor_oracle_integer_json_contract() {
        let cases: Value =
            serde_json::from_str(include_str!("../tests/fixtures/keepa-current-oracle.json"))
                .unwrap();
        let cases = cases["cases"].as_array().unwrap();
        assert!(cases.len() >= 500);
        for case in cases {
            let actual = resolve_current(
                &case["source"],
                case["index"].as_u64().unwrap() as usize,
                case.get("legacy"),
            );
            assert_eq!(
                serde_json::to_value(actual).unwrap(),
                case["expected"],
                "{}",
                case["case_id"]
            );
        }
    }
    #[test]
    fn resolve_current_regressions() {
        let v = |s: Value, i: usize| resolve_current(&s, i, None);
        // stats.current wins; Keepa's -1 sentinel never becomes a value.
        assert_eq!(
            v(json!({"stats":{"current":[7]},"current":[9]}), 0),
            Some(7)
        );
        assert_eq!(
            v(json!({"stats":{"current":[-1]},"current":[9]}), 0),
            Some(9)
        );
        // Last complete sample, skipping -1; an unpaired trailing timestamp is ignored.
        assert_eq!(v(json!({"csv":[[1,5,2,-1,3]]}), 0), Some(5));
        // Buy box (slot 18) samples are (time, price, shipping) triples.
        let mut csv = vec![Value::Null; 19];
        csv[18] = json!([1, 300, 50, 2, 400, 60]);
        assert_eq!(v(json!({ "csv": csv }), 18), Some(400));
        assert_eq!(resolve_current(&json!({}), 16, Some(&json!(45))), Some(45));
        assert_eq!(resolve_current(&json!({}), 16, Some(&json!(-1))), None);
    }
}
