//! Commerce metadata projections. Values remain source assertions with field locators.
use scraper::{Html, Selector};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use url::Url;

pub fn annotate(product: &mut Value, page: &str, script: usize, hash: &str) {
    let raw = product["raw_json_ld"].clone();
    let pointer = product["json_pointer"].as_str().unwrap_or("").to_string();
    let mut fields = serde_json::Map::new();
    for (name, key) in [
        ("title", "name"),
        ("brand", "brand"),
        ("sku", "sku"),
        ("gtin", "gtin"),
        ("ean", "gtin13"),
        ("upc", "gtin12"),
        ("gtin8", "gtin8"),
        ("gtin14", "gtin14"),
        ("mpn", "mpn"),
        ("model", "model"),
        ("images", "image"),
        ("description", "description"),
        ("category", "category"),
        ("specifications", "additionalProperty"),
        ("variants", "hasVariant"),
        ("variant_attributes", "variesBy"),
        ("color", "color"),
        ("size", "size"),
        ("weight", "weight"),
        ("rating", "aggregateRating/ratingValue"),
        ("review_count", "aggregateRating/reviewCount"),
    ] {
        let value = raw
            .pointer(&format!("/{key}"))
            .cloned()
            .unwrap_or(Value::Null);
        fields.insert(
            name.into(),
            field(
                value.clone(),
                vec![json!({
                    "value":value,"source":"JSON_LD","page":page,"script_index":script,
                    "json_pointer":format!("{pointer}/{key}"),"raw_capture_sha256":hash
                })],
            ),
        );
        // Retain the compatibility projection while exposing each observed source field.
        if !["title", "brand", "sku"].contains(&name) {
            product[name] = value;
        }
    }
    let offers = product["observed_offers"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    for (name, key) in [
        ("price_minor", "price_minor"),
        ("currency", "currency"),
        ("availability", "availability"),
        ("seller", "seller"),
    ] {
        let observations: Vec<_> = offers.iter().enumerate().filter_map(|(i,o)| {
            if o[key].is_null() { return None; }
            let raw_key = if key == "price_minor" { "price" } else if key == "currency" { "priceCurrency" } else { key };
            let suffix = if raw["offers"].is_array() {format!("/offers/{i}/{raw_key}")} else {format!("/offers/{raw_key}")};
            Some(json!({"value":o[key],"raw_value":if key=="price_minor"{o["raw_price"].clone()}else{o[key].clone()},"source":"JSON_LD","page":page,"script_index":script,"json_pointer":format!("{pointer}{suffix}"),"raw_capture_sha256":hash}))
        }).collect();
        let values: BTreeSet<_> = observations
            .iter()
            .map(|o| o["value"].to_string())
            .collect();
        let value = if values.len() == 1 {
            observations[0]["value"].clone()
        } else {
            Value::Null
        };
        let mut record = field(value.clone(), observations);
        if values.len() > 1 {
            record["status"] = json!("CONFLICT");
        }
        // A single known offer price is not evidence that every offer has that price.
        if name == "price_minor" {
            record["value"] = product[name].clone();
            record["status"] = product["price_status"].clone();
        }
        if name == "availability" || name == "seller" {
            product[name] = value;
        }
        fields.insert(name.into(), record);
    }
    for name in [
        "original_price",
        "discount",
        "shipping_text",
        "breadcrumbs",
        "canonical_url",
        "shipping_cost",
        "supplier_moq",
        "supplier_capacity",
        "sales",
        "search_volume",
        "weight_g",
    ] {
        fields
            .entry(name)
            .or_insert_with(|| field(Value::Null, vec![]));
    }
    product["fields"] = Value::Object(fields);
}

fn field(value: Value, mut evidence: Vec<Value>) -> Value {
    evidence.retain(|e| !e["value"].is_null());
    json!({"status":if value.is_null(){"UNKNOWN"}else{"OBSERVED"},"value":value,"evidence":evidence,"confidence":"SOURCE_ASSERTION_ONLY"})
}

pub fn page_metadata(doc: &Html, base: &Url, products: &[Value], hash: &str) -> Value {
    let mut og = vec![];
    for element in doc.select(&Selector::parse("meta[property],meta[name]").unwrap()) {
        let property = element
            .value()
            .attr("property")
            .or_else(|| element.value().attr("name"))
            .unwrap_or("");
        if (property.starts_with("og:")
            || property.starts_with("product:")
            || property.starts_with("twitter:"))
            && let Some(content) = element.value().attr("content")
        {
            og.push(json!({"property":property,"value":content,"source":"HTML_META","page":base.as_str(),"selector":format!("meta[{}='{}']",if element.value().attr("property").is_some(){"property"}else{"name"},property),"raw_capture_sha256":hash}));
        }
    }
    let mut canonicals = vec![];
    for element in doc.select(&Selector::parse("link[rel]").unwrap()) {
        if element
            .value()
            .attr("rel")
            .unwrap_or("")
            .split_whitespace()
            .any(|t| t.eq_ignore_ascii_case("canonical"))
            && let Some(href) = element.value().attr("href")
            && let Ok(url) = base.join(href)
        {
            canonicals.push(json!({"value":url.as_str(),"raw_value":href,"source":"HTML_LINK","selector":"link[rel~=canonical]","page":base.as_str(),"raw_capture_sha256":hash}));
        }
    }
    let h1 = doc
        .select(&Selector::parse("h1").unwrap())
        .map(|e| e.text().collect::<String>())
        .collect::<Vec<_>>()
        .join(" ");
    let path = base.path().to_ascii_lowercase();
    let article = og
        .iter()
        .any(|o| o["property"] == "og:type" && o["value"] == "article");
    let product_og = og.iter().any(|o| {
        o["property"] == "og:type"
            && matches!(o["value"].as_str(), Some("product" | "product.item"))
    });
    let mut signals = vec![];
    let role = if !products.is_empty() && !article {
        if products
            .iter()
            .any(|p| p["provenance"]["source"] == "JSON_LD")
        {
            signals.push("JSON_LD_PRODUCT");
        }
        if products
            .iter()
            .any(|p| p["provenance"]["source"] == "MICRODATA")
        {
            signals.push("MICRODATA_PRODUCT");
        }
        "PRODUCT"
    } else if article || path.contains("/blog") || path.contains("/article") {
        signals.push("ARTICLE_METADATA_OR_PATH");
        "ARTICLE"
    } else if product_og {
        signals.push("OPEN_GRAPH_PRODUCT");
        "PRODUCT"
    } else if base.query_pairs().any(|(k, _)| k == "page" || k == "p") {
        signals.push("PAGINATION_QUERY");
        "PAGINATION"
    } else if path.contains("/search")
        || base.query_pairs().any(|(k, _)| k == "q" || k == "query")
        || h1.to_ascii_lowercase().contains("search results")
    {
        signals.push("SEARCH_PATH_QUERY_OR_HEADING");
        "SEARCH_RESULT"
    } else if path.contains("/collections")
        || path.contains("/category")
        || path.contains("/categories")
    {
        signals.push("CATEGORY_PATH");
        "CATEGORY"
    } else if path.contains("/brands/") || path.contains("/brand/") {
        signals.push("BRAND_PATH");
        "BRAND"
    } else if path.contains("/seller/") || path.contains("/store/") {
        signals.push("SELLER_PATH");
        "SELLER"
    } else if path == "/" {
        signals.push("ROOT_PATH");
        "HOME"
    } else {
        signals.push("NO_RECOGNIZED_COMMERCE_SIGNALS");
        "UNKNOWN"
    };
    json!({"classification":{"role":role,"method":"DETERMINISTIC_HEURISTIC_V1","signals":signals,"page":base.as_str(),"raw_capture_sha256":hash,"confidence":"HEURISTIC_NOT_TRAINED_MODEL","heading":h1},"open_graph":og,"canonical_urls":canonicals})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn page_roles_and_field_conflicts_are_inspectable() {
        for (url, role) in [
            ("/", "HOME"),
            ("/collections/all", "CATEGORY"),
            ("/collections/all?page=2", "PAGINATION"),
            ("/search?q=coffee", "SEARCH_RESULT"),
            ("/brand/hario", "BRAND"),
            ("/seller/shop", "SELLER"),
            ("/blog/news", "ARTICLE"),
            ("/other", "UNKNOWN"),
        ] {
            let base = Url::parse(&format!("https://shop.example{url}")).unwrap();
            assert_eq!(
                page_metadata(
                    &Html::parse_document("<h1>Catalog</h1>"),
                    &base,
                    &[],
                    "hash"
                )["classification"]["role"],
                role
            );
        }
        let html = r#"<script type="application/ld+json">{"@graph":[{"@type":"Product","name":"Cup","gtin13":"1234567890128","mpn":"C-1","aggregateRating":{"ratingValue":4.4,"reviewCount":12},"offers":[{"price":"3300","priceCurrency":"JPY","availability":"InStock"},{"price":"4400","priceCurrency":"JPY","availability":"OutOfStock"}]}]}</script>"#;
        let result = crate::extract(html, "https://shop.example/products/cup").unwrap();
        let p = &result["products"][0];
        assert_eq!(result["page_metadata"]["classification"]["role"], "PRODUCT");
        assert_eq!(
            p["fields"]["ean"]["evidence"][0]["json_pointer"],
            "/@graph/0/gtin13"
        );
        assert_eq!(p["fields"]["review_count"]["value"], 12);
        assert_eq!(p["fields"]["availability"]["status"], "CONFLICT");
        assert!(p["availability"].is_null());
        assert!(p["fields"]["supplier_moq"]["value"].is_null());
    }
}
