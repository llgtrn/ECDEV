//! Provider-independent ontology; schema conversion never asserts live authenticity.
mod runtime;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MarketplaceKind {
    Marketplace,
    Seller,
    Product,
    CatalogItem,
    Offer,
    Price,
    BuyBox,
    Inventory,
    Order,
    Shipment,
    Fulfillment,
    Report,
    Listing,
    Variation,
    Brand,
    Fee,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct MarketplaceEntity {
    pub id: String,
    pub kind: MarketplaceKind,
    pub marketplace_id: String,
    pub fields: Value,
    pub evidence_ids: Vec<String>,
}
pub fn amazon_catalog(value: &Value, marketplace_id: &str) -> Result<MarketplaceEntity, String> {
    let asin = value["asin"].as_str().ok_or("CATALOG_ASIN_REQUIRED")?;
    if asin.len() != 10
        || !asin
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    {
        return Err("INVALID_ASIN".into());
    }
    let summary = value["summaries"]
        .as_array()
        .and_then(|a| a.iter().find(|s| s["marketplaceId"] == marketplace_id));
    let s = summary.cloned().unwrap_or(Value::Null);
    Ok(MarketplaceEntity {
        id: format!("{marketplace_id}:{asin}"),
        kind: MarketplaceKind::CatalogItem,
        marketplace_id: marketplace_id.into(),
        fields: json!({"external_id":asin,"title":s["itemName"],"brand":s["brand"],"manufacturer":s["manufacturer"],"model_number":s["modelNumber"],"color":s["color"],"size":s["size"],"variation_theme":s["variationTheme"],"price_minor":null,"inventory":null,"sales":null,"status":"DERIVED_SCHEMA_MAPPING","source_authenticity":"UNVERIFIED_SUPPLIED_PAYLOAD","source_schema":"Amazon Catalog Items 2022-04-01"}),
        evidence_ids: vec![],
    })
}
#[derive(Debug, Serialize, Deserialize)]
pub struct BrowserTask {
    pub id: String,
    pub url: String,
    pub timeout_ms: u64,
    pub max_actions: u32,
    pub budget_minor: u64,
    pub session_id: Option<String>,
    pub cancellation_requested: bool,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct BrowserObservation {
    pub task_id: String,
    pub source: String,
    pub evidence_ids: Vec<String>,
    pub value: Value,
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn amazon_mapping_selects_market_and_keeps_unknowns() {
        let e=amazon_catalog(&json!({"asin":"B08N5WRWNW","summaries":[{"marketplaceId":"JP","itemName":"JP fixture"},{"marketplaceId":"US","itemName":"US fixture"}]}),"JP").unwrap();
        assert_eq!(e.fields["title"], "JP fixture");
        assert!(e.fields["sales"].is_null());
        assert!(e.fields["price_minor"].is_null());
    }
}
