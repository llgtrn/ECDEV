//! Interval arithmetic on explicit monetary evidence. Missing costs are never zero estimates.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Bound {
    pub min: u64,
    pub max: u64,
    pub expected: Option<u64>,
    pub status: String,
    pub evidence: Vec<Value>,
}

pub const REQUIRED_COSTS: [&str; 6] = [
    "product_cost",
    "freight",
    "fulfillment_fee",
    "referral_fee",
    "ppc",
    "other_costs",
];

pub fn calculate(
    currency: &str,
    price: Option<&Bound>,
    costs: &BTreeMap<String, Bound>,
) -> Result<Value, String> {
    if currency.len() != 3 || !currency.bytes().all(|b| b.is_ascii_uppercase()) {
        return Err("INVALID_CURRENCY".into());
    }
    if costs.keys().any(|k| !REQUIRED_COSTS.contains(&k.as_str())) {
        return Err("UNRECOGNIZED_COST_COMPONENT".into());
    }
    for b in price.into_iter().chain(costs.values()) {
        if b.min > b.max
            || b.expected.is_some_and(|v| v < b.min || v > b.max)
            || !["OBSERVED", "DERIVED", "ESTIMATED"].contains(&b.status.as_str())
            || b.evidence.is_empty()
        {
            return Err("INVALID_OR_UNSUPPORTED_MONETARY_BOUND".into());
        }
    }
    let unknown: Vec<_> = REQUIRED_COSTS
        .iter()
        .filter(|k| !costs.contains_key(**k))
        .copied()
        .collect();
    let low: i128 = costs.values().map(|b| i128::from(b.min)).sum();
    let high: i128 = costs.values().map(|b| i128::from(b.max)).sum();
    let narrow = |v: i128| i64::try_from(v).map_err(|_| "MONETARY_OVERFLOW".to_string());
    let profit_min = if unknown.is_empty() {
        price
            .map(|p| narrow(i128::from(p.min) - high))
            .transpose()?
    } else {
        None
    };
    // Unknown costs have a mathematical nonnegative lower bound, not an estimated value.
    let profit_max = price.map(|p| narrow(i128::from(p.max) - low)).transpose()?;
    let profit_expected = if unknown.is_empty() && costs.values().all(|b| b.expected.is_some()) {
        price
            .and_then(|p| p.expected)
            .map(|p| {
                narrow(
                    i128::from(p)
                        - costs
                            .values()
                            .map(|b| i128::from(b.expected.unwrap()))
                            .sum::<i128>(),
                )
            })
            .transpose()?
    } else {
        None
    };
    let mut dominant:Vec<Value>=unknown.iter().map(|k|json!({"variable":k,"status":"UNKNOWN","width_minor":null,"priority":"UNBOUNDED_UNCERTAINTY"})).collect();
    let mut bounded: Vec<_> = costs.iter().filter(|(_, b)| b.min != b.max).collect();
    bounded.sort_by_key(|(_, b)| std::cmp::Reverse(b.max - b.min));
    dominant.extend(bounded.into_iter().map(|(k,b)|json!({"variable":k,"status":b.status,"width_minor":b.max-b.min,"priority":"BOUNDED_UNCERTAINTY"})));
    Ok(
        json!({"currency":currency,"unit":"PER_UNIT_MINOR_UNITS","status":if price.is_none(){"UNKNOWN_SELLING_PRICE"}else if unknown.is_empty(){"BOUNDED_FROM_EXPLICIT_EVIDENCE"}else{"PARTIAL_OPEN_INTERVAL"},"profit_min":profit_min,"profit_expected":profit_expected,"profit_max":profit_max,"profit_max_kind":"MATHEMATICAL_UPPER_BOUND_NOT_FORECAST","selling_price":price,"costs":costs,"unknown_costs":unknown,"dominant_uncertainty":dominant,"assumptions":"All cost components are nonnegative. Missing costs remain unknown. other_costs must explicitly cover packaging, tax, returns, reserves and other omitted costs; no currency conversion."}),
    )
}

pub fn observed_candidate(product: &Value) -> Result<Value, String> {
    let currency = product["currency"].as_str().unwrap_or("");
    if currency.is_empty() {
        return Ok(
            json!({"status":"UNKNOWN_CURRENCY","profit_min":null,"profit_expected":null,"profit_max":null,"unknown_costs":REQUIRED_COSTS}),
        );
    }
    let price = product["price_minor"].as_u64().map(|p| Bound {
        min: p,
        max: p,
        expected: Some(p),
        status: "OBSERVED".into(),
        evidence: vec![product["provenance"].clone()],
    });
    calculate(currency, price.as_ref(), &BTreeMap::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bound(min: u64, max: u64, expected: Option<u64>) -> Bound {
        Bound {
            min,
            max,
            expected,
            status: "ESTIMATED".into(),
            evidence: vec![json!({"source":"SUPPLIED_TEST_ASSUMPTION"})],
        }
    }
    #[test]
    fn unknown_costs_do_not_become_precise_profit() {
        let price = bound(4000, 4000, Some(4000));
        let v = calculate("JPY", Some(&price), &BTreeMap::new()).unwrap();
        assert!(v["profit_min"].is_null());
        assert!(v["profit_expected"].is_null());
        assert_eq!(v["profit_max"], 4000);
        assert_eq!(v["unknown_costs"].as_array().unwrap().len(), 6);
        let mut costs: BTreeMap<_, _> = REQUIRED_COSTS
            .iter()
            .map(|k| (k.to_string(), bound(0, 0, Some(0))))
            .collect();
        costs.insert("product_cost".into(), bound(800, 1200, Some(1000)));
        costs.insert("freight".into(), bound(100, 300, Some(200)));
        let v = calculate("JPY", Some(&price), &costs).unwrap();
        assert_eq!(v["profit_min"], 2500);
        assert_eq!(v["profit_expected"], 2800);
        assert_eq!(v["profit_max"], 3100);
        assert_eq!(v["dominant_uncertainty"][0]["variable"], "product_cost");
        costs.get_mut("ppc").unwrap().expected = None;
        assert!(calculate("JPY", Some(&price), &costs).unwrap()["profit_expected"].is_null());
        costs.get_mut("freight").unwrap().max = 0;
        assert!(calculate("JPY", Some(&price), &costs).is_err());
    }
}
