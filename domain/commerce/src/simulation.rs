//! Clean-room, deterministic commerce simulation (V0). Designed from ECDEV's own requirements
//! after the MiroFish study (research/commerce/mirofish-simulation-census.json); no donor code.
//! It fixes what that study found missing: every random draw comes from a seeded, named
//! substream; a result is a set of replicates with dispersion, never one stochastic run;
//! scenario comparisons pair replicates on common random numbers; every declared parameter must
//! be consumed by the run or the run is refused; the simulated clock is separate from wall time;
//! and every output is SIMULATED, which no observed store accepts and no demand evidence reads.
//! V0 aims at reproducibility, not realism: its buyer model is an uncalibrated heuristic.

use crate::sha256::Sha256;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub const SIMULATED: &str = "SIMULATED";
pub const MAX_BUYERS: u32 = 10_000;
pub const MAX_STEPS: u32 = 365;
pub const MAX_REPLICATES: u32 = 50;
/// Logit temperature of the buyer choice among affordable offers and waiting.
const CHOICE_TEMPERATURE: f64 = 0.25;

/// A deterministic random substream (SplitMix64) named by purpose, so that changing how many
/// draws one part of the model makes never shifts the draws of another part.
#[derive(Clone, Debug)]
pub struct Stream(u64);

impl Stream {
    pub fn derive(seed: u64, name: &str, replicate: u32) -> Self {
        let d = Sha256::digest(format!("ecdev.simulation.v0|{seed}|{name}|{replicate}").as_bytes());
        Stream(u64::from_le_bytes(d[..8].try_into().unwrap()))
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform in [0, 1).
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn between(&mut self, r: &Range) -> f64 {
        r.low + (r.high - r.low) * self.unit()
    }
}

/// A uniform range a population attribute is drawn from.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Range {
    pub low: f64,
    pub high: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Population {
    pub buyers: u32,
    /// Spendable budget per buyer over the horizon, in minor units.
    pub budget_minor: Range,
    /// 0..1: how strongly price lowers an offer's appeal.
    pub price_sensitivity: Range,
    /// 0..1: preference for our offer over the competitor's.
    pub brand_loyalty: Range,
    /// 0..1: chance a buyer is in the market in a step.
    pub category_interest: Range,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Market {
    pub currency: String,
    pub our_price_minor: u64,
    pub competitor_price_minor: u64,
    /// The price at which price weighs exactly `price_sensitivity`.
    pub reference_price_minor: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum Shock {
    /// Our price changes by `bps` basis points from the step on (a promotion is negative).
    OurPriceChange {
        at_step: u32,
        bps: i64,
    },
    CompetitorPriceChange {
        at_step: u32,
        bps: i64,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(tag = "rule", deny_unknown_fields)]
pub enum Termination {
    #[default]
    Horizon,
    /// Stop once our units per step stayed at or below `max_units_per_step` for `window` steps.
    Quiescence {
        window: u32,
        max_units_per_step: u64,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SimulationScenario {
    pub name: String,
    pub horizon_steps: u32,
    /// Simulated seconds per step; never wall time.
    pub step_seconds: u64,
    pub population: Population,
    pub market: Market,
    #[serde(default)]
    pub shocks: Vec<Shock>,
    #[serde(default)]
    pub termination: Termination,
    pub review_probability: f64,
    /// Base chance a purchase is returned, scaled by price sensitivity and the price paid.
    pub return_base_probability: f64,
}

/// Declared parameters, as dotted paths, that a run must consume.
fn declared(s: &SimulationScenario) -> BTreeSet<String> {
    fn walk(prefix: &str, v: &Value, out: &mut BTreeSet<String>) {
        match v {
            Value::Object(m) => {
                for (k, x) in m {
                    let p = if prefix.is_empty() {
                        k.clone()
                    } else {
                        format!("{prefix}.{k}")
                    };
                    walk(&p, x, out);
                }
            }
            Value::Array(a) => {
                for (i, x) in a.iter().enumerate() {
                    walk(&format!("{prefix}[{i}]"), x, out);
                }
            }
            _ => {
                out.insert(prefix.to_string());
            }
        }
    }
    let mut out = BTreeSet::new();
    walk(
        "",
        &serde_json::to_value(s).unwrap_or(Value::Null),
        &mut out,
    );
    out
}

impl SimulationScenario {
    pub fn validate(&self) -> Result<(), String> {
        let unit = |r: &Range| {
            (0.0..=1.0).contains(&r.low) && (0.0..=1.0).contains(&r.high) && r.low <= r.high
        };
        let p = &self.population;
        let shocks_ok = self.shocks.iter().all(|s| match s {
            Shock::OurPriceChange { at_step, bps }
            | Shock::CompetitorPriceChange { at_step, bps } => {
                *at_step < self.horizon_steps && (-9_999..=100_000).contains(bps)
            }
        });
        let term_ok = match self.termination {
            Termination::Horizon => true,
            Termination::Quiescence { window, .. } => window >= 1 && window <= self.horizon_steps,
        };
        let ok = !self.name.is_empty()
            && (1..=MAX_STEPS).contains(&self.horizon_steps)
            && self.step_seconds > 0
            && (1..=MAX_BUYERS).contains(&p.buyers)
            && p.budget_minor.low >= 0.0
            && p.budget_minor.low <= p.budget_minor.high
            && unit(&p.price_sensitivity)
            && unit(&p.brand_loyalty)
            && unit(&p.category_interest)
            && self.market.currency.len() == 3
            && self.market.our_price_minor > 0
            && self.market.competitor_price_minor > 0
            && self.market.reference_price_minor > 0
            && (0.0..=1.0).contains(&self.review_probability)
            && (0.0..=1.0).contains(&self.return_base_probability)
            && shocks_ok
            && term_ok;
        if ok {
            Ok(())
        } else {
            Err("INVALID_SIMULATION_SCENARIO".into())
        }
    }
    /// SHA-256 over the canonical (sorted-key) JSON of the scenario.
    pub fn hash(&self) -> String {
        format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_value(self)
                    .unwrap_or(Value::Null)
                    .to_string()
                    .as_bytes()
            )
        )
    }
}

/// The closed set of actions a simulated buyer can take in a step.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Action {
    Buy,
    Wait,
    Switch,
    Review,
    Return,
}

impl Action {
    pub const ALL: [Action; 5] = [
        Action::Buy,
        Action::Wait,
        Action::Switch,
        Action::Review,
        Action::Return,
    ];
    pub fn wire(self) -> &'static str {
        match self {
            Action::Buy => "BUY",
            Action::Wait => "WAIT",
            Action::Switch => "SWITCH",
            Action::Review => "REVIEW",
            Action::Return => "RETURN",
        }
    }
}

#[derive(Clone, Debug)]
struct Buyer {
    budget_minor: f64,
    price_sensitivity: f64,
    brand_loyalty: f64,
    category_interest: f64,
    last_seller: Option<usize>,
    bought_ours: bool,
}

const METRICS: [&str; 9] = [
    "units_ours",
    "units_competitor",
    "revenue_ours_minor",
    "conversion_ours",
    "switches_to_ours",
    "switches_away",
    "reviews_ours",
    "returns_ours",
    "steps_run",
];

/// One replicate. `consumed` collects every parameter path the run read.
fn replicate(s: &SimulationScenario, seed: u64, r: u32, consumed: &mut BTreeSet<String>) -> Value {
    let mut use_ = |k: &str| {
        consumed.insert(k.to_string());
    };
    use_("name");
    let p = &s.population;
    let mut pop = Stream::derive(seed, "population", r);
    let mut act = Stream::derive(seed, "activation", r);
    let mut choice = Stream::derive(seed, "choice", r);
    use_("population.buyers");
    for k in [
        "budget_minor",
        "price_sensitivity",
        "brand_loyalty",
        "category_interest",
    ] {
        use_(&format!("population.{k}.low"));
        use_(&format!("population.{k}.high"));
    }
    let mut buyers: Vec<Buyer> = (0..p.buyers)
        .map(|_| Buyer {
            budget_minor: pop.between(&p.budget_minor),
            price_sensitivity: pop.between(&p.price_sensitivity),
            brand_loyalty: pop.between(&p.brand_loyalty),
            category_interest: pop.between(&p.category_interest),
            last_seller: None,
            bought_ours: false,
        })
        .collect();
    use_("market.currency");
    use_("market.our_price_minor");
    use_("market.competitor_price_minor");
    use_("market.reference_price_minor");
    use_("horizon_steps");
    use_("step_seconds");
    use_("review_probability");
    use_("return_base_probability");
    let mut prices = [
        s.market.our_price_minor as f64,
        s.market.competitor_price_minor as f64,
    ];
    let reference = s.market.reference_price_minor as f64;
    let (mut units, mut revenue, mut switch_in, mut switch_out, mut reviews, mut returns) =
        ([0u64; 2], 0f64, 0u64, 0u64, 0u64, 0u64);
    let mut timeline = vec![];
    let mut quiet = 0u32;
    let mut fired = "HORIZON".to_string();
    let mut steps_run = 0;
    for step in 0..s.horizon_steps {
        for (i, shock) in s.shocks.iter().enumerate() {
            match shock {
                Shock::OurPriceChange { at_step, bps }
                | Shock::CompetitorPriceChange { at_step, bps }
                    if *at_step == step =>
                {
                    let who = usize::from(matches!(shock, Shock::CompetitorPriceChange { .. }));
                    prices[who] = (prices[who] * (1.0 + *bps as f64 / 10_000.0))
                        .max(1.0)
                        .round();
                    use_(&format!("shocks[{i}].kind"));
                    use_(&format!("shocks[{i}].at_step"));
                    use_(&format!("shocks[{i}].bps"));
                }
                _ => {}
            }
        }
        let (mut step_units, mut active) = ([0u64; 2], 0u64);
        let mut acted = std::collections::BTreeMap::<Action, u64>::new();
        for b in buyers.iter_mut() {
            if act.unit() >= b.category_interest {
                continue;
            }
            active += 1;
            // Logit over affordable offers and waiting; waiting has utility 0.
            let utility = |seller: usize| {
                let pref = if seller == 0 {
                    b.brand_loyalty
                } else {
                    1.0 - b.brand_loyalty
                };
                pref - b.price_sensitivity * prices[seller] / reference
            };
            let mut options = vec![(None, 0.0f64)];
            for (seller, price) in prices.iter().enumerate() {
                if *price <= b.budget_minor {
                    options.push((Some(seller), utility(seller)));
                }
            }
            let weights: Vec<f64> = options
                .iter()
                .map(|(_, u)| (u / CHOICE_TEMPERATURE).exp())
                .collect();
            let mut x = choice.unit() * weights.iter().sum::<f64>();
            let mut pick = options[options.len() - 1].0;
            for ((option, _), w) in options.iter().zip(&weights) {
                if x < *w {
                    pick = *option;
                    break;
                }
                x -= w;
            }
            // Review and return draws are taken whatever is chosen, so a choice never shifts
            // the stream for the next buyer.
            let (review_draw, return_draw) = (choice.unit(), choice.unit());
            let Some(seller) = pick else {
                *acted.entry(Action::Wait).or_default() += 1;
                continue;
            };
            *acted.entry(Action::Buy).or_default() += 1;
            if let Some(last) = b.last_seller
                && last != seller
            {
                *acted.entry(Action::Switch).or_default() += 1;
                if seller == 0 {
                    switch_in += 1
                } else {
                    switch_out += 1
                }
            }
            b.last_seller = Some(seller);
            b.budget_minor -= prices[seller];
            step_units[seller] += 1;
            units[seller] += 1;
            if seller == 0 {
                b.bought_ours = true;
                revenue += prices[0];
                if review_draw < s.review_probability {
                    reviews += 1;
                    *acted.entry(Action::Review).or_default() += 1;
                }
                let return_chance = (s.return_base_probability * b.price_sensitivity * prices[0]
                    / reference)
                    .min(1.0);
                if return_draw < return_chance {
                    returns += 1;
                    *acted.entry(Action::Return).or_default() += 1;
                }
            }
        }
        steps_run = step + 1;
        timeline.push(json!({"step":step,"simulated_seconds":u64::from(step + 1) * s.step_seconds,"active_buyers":active,"units_ours":step_units[0],"units_competitor":step_units[1],"our_price_minor":prices[0],"competitor_price_minor":prices[1],"actions":Action::ALL.iter().map(|a| (a.wire().to_string(), json!(acted.get(a).copied().unwrap_or(0)))).collect::<serde_json::Map<_, _>>()}));
        if let Termination::Quiescence {
            window,
            max_units_per_step,
        } = s.termination
        {
            use_("termination.rule");
            use_("termination.window");
            use_("termination.max_units_per_step");
            quiet = if step_units[0] <= max_units_per_step {
                quiet + 1
            } else {
                0
            };
            if quiet >= window {
                fired = format!("QUIESCENCE_AT_STEP_{step}");
                break;
            }
        } else {
            use_("termination.rule");
        }
    }
    let converted = buyers.iter().filter(|b| b.bought_ours).count() as f64;
    json!({"replicate":r,"state":SIMULATED,"termination_fired":fired,"metrics":{
        "units_ours":units[0],"units_competitor":units[1],"revenue_ours_minor":revenue,
        "conversion_ours":converted / f64::from(p.buyers),"switches_to_ours":switch_in,"switches_away":switch_out,
        "reviews_ours":reviews,"returns_ours":returns,"steps_run":steps_run},"timeline":timeline})
}

/// Mean, median, sample standard deviation and quantiles of one metric across replicates.
pub fn dispersion(values: &[f64]) -> Value {
    if values.is_empty() {
        return json!({"n":0});
    }
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let n = v.len();
    let mean = v.iter().sum::<f64>() / n as f64;
    let sd = if n > 1 {
        (v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1) as f64).sqrt()
    } else {
        0.0
    };
    let q = |p: f64| {
        let pos = p * (n - 1) as f64;
        let (lo, hi) = (pos.floor() as usize, pos.ceil() as usize);
        v[lo] + (v[hi] - v[lo]) * (pos - lo as f64)
    };
    json!({"n":n,"mean":mean,"median":q(0.5),"sd":sd,"p10":q(0.1),"p90":q(0.9),"min":v[0],"max":v[n - 1]})
}

/// Runs `replicates` seeded replicates and refuses the run if any declared parameter went unread.
pub fn run(s: &SimulationScenario, seed: u64, replicates: u32) -> Result<Value, String> {
    s.validate()?;
    if !(1..=MAX_REPLICATES).contains(&replicates) {
        return Err("INVALID_REPLICATE_COUNT".into());
    }
    let mut consumed = BTreeSet::new();
    let runs: Vec<Value> = (0..replicates)
        .map(|r| replicate(s, seed, r, &mut consumed))
        .collect();
    let dead: Vec<String> = declared(s).difference(&consumed).cloned().collect();
    if !dead.is_empty() {
        return Err(format!("DEAD_PARAMETER: {}", dead.join(",")));
    }
    let summary: serde_json::Map<String, Value> = METRICS
        .iter()
        .map(|m| {
            (
                m.to_string(),
                dispersion(
                    &runs
                        .iter()
                        .filter_map(|r| r["metrics"][*m].as_f64())
                        .collect::<Vec<_>>(),
                ),
            )
        })
        .collect();
    Ok(
        json!({"state":SIMULATED,"evidence_class":"SIMULATED_NOT_OBSERVED_NOT_DEMAND","scenario":s,"scenario_hash":s.hash(),"seed":seed,"replicates":replicates,
        "streams":["population","activation","choice"],"summary":summary,"runs":runs,
        "clock":{"step_seconds":s.step_seconds,"simulated_horizon_seconds":u64::from(s.horizon_steps) * s.step_seconds,"basis":"SIMULATED_TIME_NOT_WALL_TIME"},
        "cost":{"agent_steps":u64::from(s.population.buyers) * u64::from(s.horizon_steps) * u64::from(replicates),"llm_calls":0},
        "parameters_consumed":consumed,"model":"V0_LOGIT_BUYERS_UNCALIBRATED_HEURISTIC"}),
    )
}

/// Baseline against variant on common random numbers: replicate r of both uses the same seeded
/// substreams, so the per-replicate difference isolates the scenario change.
pub fn compare(
    baseline: &SimulationScenario,
    variant: &SimulationScenario,
    seed: u64,
    replicates: u32,
) -> Result<Value, String> {
    let a = run(baseline, seed, replicates)?;
    let b = run(variant, seed, replicates)?;
    let diffs: serde_json::Map<String, Value> = METRICS
        .iter()
        .map(|m| {
            let d: Vec<f64> = (0..replicates as usize)
                .map(|r| {
                    b["runs"][r]["metrics"][*m].as_f64().unwrap_or(0.0)
                        - a["runs"][r]["metrics"][*m].as_f64().unwrap_or(0.0)
                })
                .collect();
            (m.to_string(), dispersion(&d))
        })
        .collect();
    Ok(
        json!({"state":SIMULATED,"evidence_class":"SIMULATED_NOT_OBSERVED_NOT_DEMAND","seed":seed,"replicates":replicates,
        "pairing":"COMMON_RANDOM_NUMBERS_PER_REPLICATE","baseline":a,"variant":b,"difference_variant_minus_baseline":diffs,
        "invariants":["SIMULATED != OBSERVED","SIMULATED != DEMAND","simulated units are not a sales forecast"]}),
    )
}

/// True when a value carries a SIMULATED marker anywhere: the observed stores refuse it.
pub fn carries_simulated(v: &Value) -> bool {
    match v {
        Value::Object(m) => m.iter().any(|(k, x)| {
            (matches!(k.as_str(), "state" | "mode" | "evidence_class")
                && x.as_str().is_some_and(|s| s.starts_with(SIMULATED)))
                || carries_simulated(x)
        }),
        Value::Array(a) => a.iter().any(carries_simulated),
        _ => false,
    }
}

pub fn initialize(db: &rusqlite::Connection) -> Result<(), String> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS simulation_runs(id TEXT PRIMARY KEY, created_at INTEGER NOT NULL, state TEXT NOT NULL CHECK(state='SIMULATED'), baseline_hash TEXT NOT NULL, variant_hash TEXT, seed INTEGER NOT NULL, replicates INTEGER NOT NULL, payload TEXT NOT NULL);")
        .map_err(|e| e.to_string())
}

impl crate::Engine {
    /// Runs a scenario, or a baseline against a variant on common random numbers, and keeps the
    /// result in simulation_runs only. Wall time is reported beside the result, never inside it.
    pub fn simulation_compare(&self, args: Value) -> Result<Value, String> {
        let seed = args["seed"]
            .as_u64()
            .ok_or("Required unsigned integer: seed")?;
        let replicates = args["replicates"].as_u64().unwrap_or(10) as u32;
        let baseline: SimulationScenario = serde_json::from_value(args["baseline"].clone())
            .map_err(|e| format!("INVALID_SIMULATION_SCENARIO: {e}"))?;
        let started = std::time::Instant::now();
        let (mut result, variant_hash) = match args.get("variant").filter(|v| !v.is_null()) {
            Some(v) => {
                let variant: SimulationScenario = serde_json::from_value(v.clone())
                    .map_err(|e| format!("INVALID_SIMULATION_SCENARIO: {e}"))?;
                (
                    compare(&baseline, &variant, seed, replicates)?,
                    Some(variant.hash()),
                )
            }
            None => (run(&baseline, seed, replicates)?, None),
        };
        let id = crate::identifier::Uuid::new_v4().to_string();
        self.db
            .lock()
            .map_err(|e| e.to_string())?
            .execute(
                "INSERT INTO simulation_runs VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                rusqlite::params![
                    id,
                    crate::service::timestamp(),
                    SIMULATED,
                    baseline.hash(),
                    variant_hash,
                    seed as i64,
                    replicates,
                    result.to_string()
                ],
            )
            .map_err(|e| e.to_string())?;
        result["simulation_id"] = json!(id);
        result["wall_ms"] = json!(started.elapsed().as_millis());
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn baseline() -> SimulationScenario {
        serde_json::from_value(json!({
            "name":"baseline price","horizon_steps":30,"step_seconds":86400,
            "population":{"buyers":400,"budget_minor":{"low":2000.0,"high":20000.0},"price_sensitivity":{"low":0.2,"high":0.9},
                "brand_loyalty":{"low":0.3,"high":0.7},"category_interest":{"low":0.02,"high":0.12}},
            "market":{"currency":"JPY","our_price_minor":3000,"competitor_price_minor":3000,"reference_price_minor":3000},
            "review_probability":0.1,"return_base_probability":0.05}))
        .unwrap()
    }

    fn promotion() -> SimulationScenario {
        let mut s = baseline();
        s.name = "10% promotion".into();
        s.shocks = vec![Shock::OurPriceChange {
            at_step: 0,
            bps: -1000,
        }];
        s
    }

    #[test]
    fn simulations_persist_apart_and_observed_stores_refuse_them() {
        let root =
            std::env::temp_dir().join(format!("ecdev-sim-{}", crate::identifier::Uuid::new_v4()));
        let e = crate::Engine::open(&root).unwrap();
        let before = e.candidates().unwrap();
        let out = e
            .simulation_compare(
                json!({"baseline":baseline(),"variant":promotion(),"seed":11,"replicates":4}),
            )
            .unwrap();
        assert_eq!(out["state"], SIMULATED);
        let id = out["simulation_id"].as_str().unwrap();
        let (state, stored): (String, String) =
            e.db.lock()
                .unwrap()
                .query_row(
                    "SELECT state,payload FROM simulation_runs WHERE id=?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
        assert_eq!(state, SIMULATED);
        assert!(
            !stored.contains("wall_ms"),
            "wall time stays outside the stored result"
        );
        // Nothing reached the observed stores or the candidates.
        let observed: i64 =
            e.db.lock()
                .unwrap()
                .query_row(
                    "SELECT (SELECT count(*) FROM runs)+(SELECT count(*) FROM evidence)",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
        assert_eq!(observed, 0);
        assert_eq!(e.candidates().unwrap(), before);
        // A simulated record cannot be written as an observation or a candidate.
        for payload in [
            json!({"mode":"LIVE","research_run":true,"observations":[{"id":"x","mode":"SIMULATED"}]}),
            json!({"mode":"PLAN_ONLY","observations":[],"candidates":[{"product":{"units":3},"evidence_class":"SIMULATED_NOT_OBSERVED_NOT_DEMAND"}]}),
            json!({"mode":"PLAN_ONLY","observations":[],"candidates":[{"product":{"title":"x","units_estimate":{"state":"SIMULATED"}}}]}),
            json!({"mode":"LIVE","research_run":true,"observations":[{"id":"y","normalized_value":out["variant"]["runs"][0].clone()}]}),
        ] {
            assert_eq!(
                e.persist(payload).unwrap_err(),
                "SIMULATED_RECORD_REFUSED_BY_OBSERVED_STORE"
            );
        }
        // The table itself refuses any other state.
        assert!(
            e.db.lock()
                .unwrap()
                .execute(
                    "INSERT INTO simulation_runs VALUES('z',0,'OBSERVED','h',NULL,1,1,'{}')",
                    []
                )
                .is_err()
        );
        drop(e);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn same_seed_same_result_other_seed_other_result() {
        let a = run(&baseline(), 7, 5).unwrap();
        assert_eq!(a, run(&baseline(), 7, 5).unwrap());
        assert_ne!(a["runs"], run(&baseline(), 8, 5).unwrap()["runs"]);
        assert_eq!(a["state"], SIMULATED);
        assert_eq!(a["summary"]["units_ours"]["n"], 5);
        assert!(
            a["summary"]["units_ours"]["sd"].as_f64().unwrap() > 0.0,
            "replicates must differ"
        );
        assert_eq!(a["cost"]["llm_calls"], 0);
    }

    #[test]
    fn substreams_keep_unrelated_parts_fixed() {
        // A promotion changes prices, never who the buyers are.
        let a = run(&baseline(), 7, 1).unwrap();
        let b = run(&promotion(), 7, 1).unwrap();
        assert_eq!(
            a["runs"][0]["timeline"][0]["active_buyers"],
            b["runs"][0]["timeline"][0]["active_buyers"]
        );
        assert_eq!(b["runs"][0]["timeline"][0]["our_price_minor"], 2700.0);
    }

    #[test]
    fn promotion_is_compared_on_common_random_numbers() {
        let c = compare(&baseline(), &promotion(), 11, 20).unwrap();
        let d = &c["difference_variant_minus_baseline"]["units_ours"];
        assert!(
            d["mean"].as_f64().unwrap() > 0.0,
            "a cheaper offer sells more in this model"
        );
        // Pairing on common random numbers: paired differences vary less than unpaired ones.
        let unpaired: Vec<f64> = {
            let a = run(&baseline(), 11, 20).unwrap();
            let b = run(&promotion(), 999, 20).unwrap();
            (0..20)
                .map(|r| {
                    b["runs"][r]["metrics"]["units_ours"].as_f64().unwrap()
                        - a["runs"][r]["metrics"]["units_ours"].as_f64().unwrap()
                })
                .collect()
        };
        assert!(d["sd"].as_f64().unwrap() < dispersion(&unpaired)["sd"].as_f64().unwrap());
        assert_eq!(c["pairing"], "COMMON_RANDOM_NUMBERS_PER_REPLICATE");
    }

    #[test]
    fn every_declared_parameter_is_consumed_or_the_run_is_refused() {
        let r = run(&promotion(), 1, 2).unwrap();
        let consumed: BTreeSet<String> = r["parameters_consumed"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert_eq!(consumed, declared(&promotion()));
        // Unknown parameters never parse; a shock past the horizon could never act.
        let mut v = serde_json::to_value(baseline()).unwrap();
        v["agent_mood"] = json!(0.5);
        assert!(serde_json::from_value::<SimulationScenario>(v).is_err());
        let mut late = baseline();
        late.shocks = vec![Shock::CompetitorPriceChange {
            at_step: 30,
            bps: 500,
        }];
        assert_eq!(run(&late, 1, 1).unwrap_err(), "INVALID_SIMULATION_SCENARIO");
        // Each numeric parameter changes the outcome for a fixed seed.
        let base = run(&baseline(), 3, 2).unwrap()["runs"].clone();
        let perturb: Vec<fn(&mut SimulationScenario)> = vec![
            |s| s.population.buyers += 1,
            |s| s.population.budget_minor.low = 4000.0,
            |s| s.population.price_sensitivity.high = 0.5,
            |s| s.population.brand_loyalty.low = 0.5,
            |s| s.population.category_interest.high = 0.3,
            |s| s.market.our_price_minor = 3500,
            |s| s.market.competitor_price_minor = 2500,
            |s| s.market.reference_price_minor = 6000,
            |s| s.review_probability = 0.5,
            |s| s.return_base_probability = 0.6,
            |s| s.horizon_steps = 29,
            |s| s.step_seconds = 3600,
        ];
        for (i, f) in perturb.iter().enumerate() {
            let mut s = baseline();
            f(&mut s);
            assert_ne!(
                run(&s, 3, 2).unwrap()["runs"],
                base,
                "parameter {i} had no effect"
            );
        }
    }

    #[test]
    fn actions_come_from_a_closed_set_and_add_up() {
        let r = run(&baseline(), 9, 1).unwrap();
        let mut buys = 0;
        for step in r["runs"][0]["timeline"].as_array().unwrap() {
            let a = &step["actions"];
            assert_eq!(
                a.as_object().unwrap().keys().cloned().collect::<Vec<_>>(),
                ["BUY", "RETURN", "REVIEW", "SWITCH", "WAIT"]
            );
            assert_eq!(
                a["BUY"].as_u64().unwrap() + a["WAIT"].as_u64().unwrap(),
                step["active_buyers"].as_u64().unwrap()
            );
            assert_eq!(
                a["BUY"].as_u64().unwrap(),
                step["units_ours"].as_u64().unwrap() + step["units_competitor"].as_u64().unwrap()
            );
            buys += a["BUY"].as_u64().unwrap();
        }
        let m = &r["runs"][0]["metrics"];
        assert_eq!(
            buys,
            m["units_ours"].as_u64().unwrap() + m["units_competitor"].as_u64().unwrap()
        );
    }

    #[test]
    fn termination_records_which_rule_fired() {
        let mut s = baseline();
        s.termination = Termination::Quiescence {
            window: 3,
            max_units_per_step: 1_000,
        };
        let r = run(&s, 5, 1).unwrap();
        assert_eq!(r["runs"][0]["termination_fired"], "QUIESCENCE_AT_STEP_2");
        assert_eq!(r["runs"][0]["metrics"]["steps_run"], 3);
        assert_eq!(
            run(&baseline(), 5, 1).unwrap()["runs"][0]["termination_fired"],
            "HORIZON"
        );
    }

    #[test]
    fn simulated_markers_are_detected_anywhere() {
        let r = run(&baseline(), 1, 1).unwrap();
        assert!(carries_simulated(&r));
        assert!(carries_simulated(
            &json!({"observations":[{"nested":{"state":"SIMULATED"}}]})
        ));
        assert!(!carries_simulated(
            &json!({"observations":[{"mode":"LIVE","note":"SIMULATED in prose is not a marker"}]})
        ));
    }
}
