//! Keepa boundary normalization derived from a reviewed, commit-locked behavior contract.
//! No donor runtime, network requests, currency inference or live observations.
use serde_json::Value;
pub mod client;

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
}
