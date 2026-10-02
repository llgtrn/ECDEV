use serde::{Deserialize, Serialize};

/// All monetary inputs use one currency's minor units, all rates basis points.
/// Fee and tax values are supplied assumptions, never current marketplace quotes.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub currency: String,
    pub fulfillment: Fulfillment,
    pub selling_price: u64,
    pub product_cost: u64,
    #[serde(default)]
    pub packaging: u64,
    #[serde(default)]
    pub sample_amortization: u64,
    #[serde(default)]
    pub freight: u64,
    #[serde(default)]
    pub insurance: u64,
    #[serde(default)]
    pub duty: u64,
    #[serde(default)]
    pub import_tax: u64,
    #[serde(default)]
    pub brokerage: u64,
    #[serde(default)]
    pub prep: u64,
    #[serde(default)]
    pub label: u64,
    pub fulfillment_fee: u64,
    pub referral_bps: u64,
    #[serde(default)]
    pub storage_fee: u64,
    #[serde(default)]
    pub payment_cost: u64,
    #[serde(default)]
    pub ppc: u64,
    #[serde(default)]
    pub returns: u64,
    #[serde(default)]
    pub fx_cost: u64,
    #[serde(default)]
    pub reserve: u64,
    pub units: u64,
    #[serde(default)]
    pub fixed_launch_cost: u64,
    #[serde(default)]
    pub lead_time_days: u64,
    #[serde(default)]
    pub inventory_days: u64,
    #[serde(default)]
    pub payout_days: u64,
    #[serde(default)]
    pub supplier_credit_days: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Fulfillment {
    Fba,
    Fbm,
    EproloDropship,
    ThreePl,
    BulkImport,
    FactoryDirect,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Economics {
    pub mode: &'static str,
    pub currency: String,
    pub landed_cost: i64,
    pub referral_fee: i64,
    pub contribution_per_unit: i64,
    pub contribution_margin_bps: i64,
    pub net_margin_bps: i64,
    pub break_even_acos_bps: i64,
    pub roi_bps: Option<i64>,
    pub cash_required: i64,
    pub cash_conversion_cycle_days: i64,
    pub break_even_units: Option<i64>,
    pub expected_profit: i64,
    pub assumptions: &'static str,
}
fn narrow(x: i128) -> Result<i64, String> {
    i64::try_from(x).map_err(|_| "Monetary overflow".into())
}
pub fn simulate(s: &Scenario) -> Result<Economics, String> {
    if s.currency.len() != 3 || !s.currency.bytes().all(|b| b.is_ascii_uppercase()) {
        return Err("Use an uppercase ISO currency code".into());
    }
    if s.selling_price == 0 || s.units == 0 || s.referral_bps > 10_000 {
        return Err("Positive price and units, and referral_bps <= 10000 required".into());
    }
    let price = s.selling_price as i128;
    let units = s.units as i128;
    let landed = [
        s.product_cost,
        s.packaging,
        s.sample_amortization,
        s.freight,
        s.insurance,
        s.duty,
        s.import_tax,
        s.brokerage,
        s.prep,
        s.label,
        s.fx_cost,
    ]
    .iter()
    .map(|x| *x as i128)
    .sum::<i128>();
    let referral = (price * s.referral_bps as i128 + 5000) / 10000;
    let before_ads = price
        - landed
        - referral
        - s.fulfillment_fee as i128
        - s.storage_fee as i128
        - s.payment_cost as i128
        - s.returns as i128
        - s.reserve as i128;
    let contribution = before_ads - s.ppc as i128;
    let profit = contribution
        .checked_mul(units)
        .ok_or("Monetary overflow")?
        .checked_sub(s.fixed_launch_cost as i128)
        .ok_or("Monetary overflow")?;
    // Conservative upfront budget includes inventory, assumed PPC and launch costs.
    let cash = (landed + s.ppc as i128)
        .checked_mul(units)
        .ok_or("Monetary overflow")?
        .checked_add(s.fixed_launch_cost as i128)
        .ok_or("Monetary overflow")?;
    let revenue = price.checked_mul(units).ok_or("Monetary overflow")?;
    narrow(profit)?;
    narrow(cash)?;
    Ok(Economics {
        mode: "SIMULATED",
        currency: s.currency.clone(),
        landed_cost: narrow(landed)?,
        referral_fee: narrow(referral)?,
        contribution_per_unit: narrow(contribution)?,
        contribution_margin_bps: narrow(contribution * 10000 / price)?,
        net_margin_bps: narrow(profit * 10000 / revenue)?,
        break_even_acos_bps: narrow(before_ads.max(0) * 10000 / price)?,
        roi_bps: if cash == 0 {
            None
        } else {
            Some(narrow(profit * 10000 / cash)?)
        },
        cash_required: narrow(cash)?,
        cash_conversion_cycle_days: narrow(
            s.lead_time_days as i128 + s.inventory_days as i128 + s.payout_days as i128
                - s.supplier_credit_days as i128,
        )?,
        break_even_units: if contribution <= 0 {
            None
        } else {
            Some(narrow(
                (s.fixed_launch_cost as i128 + contribution - 1) / contribution,
            )?)
        },
        expected_profit: narrow(profit)?,
        assumptions: "User-supplied costs and fees in currency minor units; half-up referral rounding; no live fee lookup or FX conversion.",
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn scenario() -> Scenario {
        serde_json::from_value(serde_json::json!({"currency":"JPY","fulfillment":"FBA","selling_price":4000,"product_cost":800,"freight":200,"fulfillment_fee":500,"referral_bps":1500,"ppc":400,"units":100,"fixed_launch_cost":10000})).unwrap()
    }
    #[test]
    fn known_unit_economics() {
        let e = simulate(&scenario()).unwrap();
        assert_eq!(e.landed_cost, 1000);
        assert_eq!(e.contribution_per_unit, 1500);
        assert_eq!(e.net_margin_bps, 3500);
        assert_eq!(e.cash_required, 150000);
        assert_eq!(e.expected_profit, 140000);
        assert_eq!(e.break_even_units, Some(7));
    }
    #[test]
    fn loss_never_has_a_break_even() {
        let mut s = scenario();
        s.product_cost = 9000;
        let e = simulate(&s).unwrap();
        assert_eq!(e.break_even_units, None);
        assert!(e.expected_profit < 0);
        assert_eq!(e.break_even_acos_bps, 0);
    }
    #[test]
    fn validates_rates_and_overflow() {
        let mut s = scenario();
        s.referral_bps = 10001;
        assert!(simulate(&s).is_err());
        s.referral_bps = 1500;
        s.units = u64::MAX;
        assert!(simulate(&s).is_err());
    }
    #[test]
    fn rounding_is_explicit() {
        let mut s = scenario();
        s.selling_price = 3999;
        assert_eq!(simulate(&s).unwrap().referral_fee, 600);
    }
    #[test]
    fn extreme_inputs_return_errors_without_panicking() {
        let mut s = scenario();
        s.product_cost = u64::MAX;
        s.freight = u64::MAX;
        s.units = u64::MAX;
        assert!(simulate(&s).is_err());
    }
}
