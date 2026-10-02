use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ObservationMode {
    Live,
    Cached,
    Replay,
    Fixture,
    Simulated,
    Inferred,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub id: String,
    pub mode: ObservationMode,
    pub source_type: String,
    pub provider: String,
    pub external_source: String,
    pub market: String,
    pub query: Value,
    pub timestamp: String,
    pub retrieved_at: String,
    pub raw_hash: String,
    pub normalized_value: Value,
    pub unit: String,
    pub currency: Option<String>,
    pub confidence: Option<f64>,
    pub freshness_seconds: Option<u64>,
    pub cost_minor: Option<u64>,
    pub run_id: String,
}
impl Evidence {
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty()
            || self.provider.is_empty()
            || self.external_source.is_empty()
            || self.run_id.is_empty()
        {
            return Err("Evidence identity, source, provider and run are required".into());
        }
        if self
            .confidence
            .is_some_and(|c| !c.is_finite() || !(0.0..=1.0).contains(&c))
        {
            return Err("Confidence must be within [0,1]".into());
        }
        if self.raw_hash.len() != 64 || !self.raw_hash.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err("Evidence requires a SHA-256 raw hash".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphEdge {
    pub from: String,
    pub relation: String,
    pub to: String,
    pub evidence_ids: Vec<String>,
    pub mode: ObservationMode,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProductFingerprint {
    pub concepts: Vec<String>,
    pub category: Option<String>,
    pub weight_g: Option<u64>,
    pub dimensions_mm: Option<[u64; 3]>,
    pub materials: Vec<String>,
    pub attributes: Value,
    pub pack_quantity: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MatchConfidence {
    Exact,
    HighConfidenceMatch,
    ProbableMatch,
    WeakMatch,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entity {
    pub id: String,
    pub kind: String,
    pub attributes: Value,
    pub evidence_ids: Vec<String>,
}
