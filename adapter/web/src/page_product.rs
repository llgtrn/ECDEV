//! Page assertions enrich only a single associated product; no variant-wide price assignment.
use serde_json::{Value, json};
use std::collections::BTreeSet;

fn merge(product: &mut Value, name: &str, observations: Vec<Value>, status: &str) {
    if observations.is_empty() {
        return;
    }
    let mut evidence = product["fields"][name]["evidence"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    evidence.extend(observations);
    let values: BTreeSet<_> = evidence
        .iter()
        .filter(|e| !e["value"].is_null())
        .map(|e| {
            if name == "brand" && e["value"]["name"].is_string() {
                e["value"]["name"].to_string()
            } else {
                e["value"].to_string()
            }
        })
        .collect();
    let conflict = values.len() > 1;
    let value = if values.len() == 1 {
        evidence.iter().find(|e| !e["value"].is_null()).unwrap()["value"].clone()
    } else {
        Value::Null
    };
    product["fields"][name] = json!({"status":if conflict {"CONFLICT"} else {status},"value":value,"evidence":evidence,"confidence":"SOURCE_ASSERTION_ONLY"});
    if name == "brand" {
        product["fields"][name]["comparison"] =
            json!("Brand.name and literal brand labels compared; raw shapes retained");
    }
    // Keep the existing title label for identity, while field truth exposes any conflict.
    if name != "title" || product["title"].is_null() {
        product[name] = value;
    }
    if name == "price_minor" {
        product["price_status"] = product["fields"][name]["status"].clone();
    }
}
fn assertions(metadata: &Value, property: &str) -> Vec<Value> {
    metadata["open_graph"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["property"] == property)
        .cloned()
        .collect()
}
fn associated(mut observation: Value) -> Value {
    observation["association"] =
        json!("DERIVED_SINGLE_PRODUCT_PAGE_NOT_VARIANT_LEVEL_VERIFICATION");
    observation
}
fn breadcrumbs(
    value: &Value,
    pointer: &str,
    script: usize,
    page: &str,
    hash: &str,
    out: &mut Vec<Value>,
) {
    if value["@type"] == "BreadcrumbList" {
        out.push(json!({"value":value,"source":"JSON_LD","script_index":script,"json_pointer":pointer,"page":page,"raw_capture_sha256":hash,"association":"PAGE_BREADCRUMB_ASSERTION_NOT_TAXONOMY_VERIFICATION"}));
    }
    match value {
        Value::Array(array) => {
            for (i, child) in array.iter().enumerate() {
                breadcrumbs(child, &format!("{pointer}/{i}"), script, page, hash, out);
            }
        }
        Value::Object(object) => {
            for (key, child) in object {
                if child.is_array() || child.is_object() {
                    breadcrumbs(
                        child,
                        &format!("{pointer}/{}", key.replace('~', "~0").replace('/', "~1")),
                        script,
                        page,
                        hash,
                        out,
                    );
                }
            }
        }
        _ => {}
    }
}
pub fn enrich(
    products: &mut Vec<Value>,
    metadata: &Value,
    structured: &[Value],
    page: &str,
    hash: &str,
) {
    if products.is_empty()
        && metadata["classification"]["role"] == "PRODUCT"
        && !assertions(metadata, "og:title").is_empty()
    {
        let mut product = json!({"kind":"PRODUCT","title":null,"brand":null,"sku":null,"currency":null,"price_minor":null,"price_status":"UNKNOWN","observed_offers":[],"provenance":{"source":"OPEN_GRAPH","page":page,"raw_capture_sha256":hash,"association":"EXPLICIT_OPEN_GRAPH_PRODUCT_TYPE"},"raw_open_graph":metadata["open_graph"]});
        crate::commerce::annotate(&mut product, page, 0, hash);
        products.push(product);
    }
    if products.len() != 1 {
        return;
    }
    let product = &mut products[0];
    let incomplete_structured_price = product["observed_offers"].as_array().is_some_and(|offers| {
        !offers.is_empty() && offers.iter().any(|o| o["price_minor"].is_null())
    });
    for (name, property) in [
        ("title", "og:title"),
        ("description", "og:description"),
        ("brand", "product:brand"),
        ("retailer_item_id", "product:retailer_item_id"),
    ] {
        merge(
            product,
            name,
            assertions(metadata, property)
                .into_iter()
                .map(associated)
                .collect(),
            "OBSERVED",
        );
    }
    for name in ["title", "description"] {
        merge(
            product,
            name,
            metadata["page_assertions"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|assertion| assertion["field"] == name)
                .cloned()
                .map(associated)
                .collect(),
            "OBSERVED",
        );
    }
    if product["fields"]["title"]["status"] == "UNKNOWN" {
        merge(
            product,
            "title",
            metadata["document_titles"]
                .as_array()
                .into_iter()
                .flatten()
                .cloned()
                .map(associated)
                .collect(),
            "DERIVED",
        );
    }
    let images = assertions(metadata, "og:image");
    if product["fields"]["images"]["value"].is_null() && !images.is_empty() {
        let urls: Vec<_> = images.iter().map(|o| o["value"].clone()).collect();
        let evidence = images.into_iter().map(associated).collect::<Vec<_>>();
        product["images"] = json!(urls);
        product["fields"]["images"] = json!({"status":"OBSERVED","value":urls,"evidence":evidence,"confidence":"SOURCE_ASSERTION_ONLY"});
    }
    merge(
        product,
        "canonical_url",
        metadata["canonical_urls"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(associated)
            .collect(),
        "OBSERVED",
    );
    let currencies = assertions(metadata, "product:price:currency");
    merge(
        product,
        "currency",
        currencies.clone().into_iter().map(associated).collect(),
        "OBSERVED",
    );
    let codes: BTreeSet<_> = currencies
        .iter()
        .filter_map(|o| o["value"].as_str())
        .collect();
    if codes.len() == 1 {
        let code = *codes.first().unwrap();
        let mut incomplete_meta_price = false;
        let prices = assertions(metadata, "product:price:amount")
            .into_iter()
            .map(|mut o| {
                let interpretation =
                    crate::price::formatted_money(o["value"].as_str().unwrap_or(""), code);
                incomplete_meta_price |= interpretation["minor"].is_null();
                o["raw_value"] = o["value"].clone();
                o["value"] = interpretation["minor"].clone();
                o["numeric_interpretation"] = interpretation;
                o["currency_evidence"] = json!(currencies);
                associated(o)
            })
            .collect();
        merge(product, "price_minor", prices, "DERIVED");
        if (incomplete_meta_price || incomplete_structured_price)
            && product["fields"]["price_minor"]["status"] != "CONFLICT"
        {
            product["price_minor"] = Value::Null;
            product["price_status"] = json!("UNKNOWN");
            product["fields"]["price_minor"]["value"] = Value::Null;
            product["fields"]["price_minor"]["status"] = json!("UNKNOWN");
            product["fields"]["price_minor"]["reason"] =
                json!("UNPARSED_OR_INCOMPLETE_PRICE_ASSERTIONS_RETAINED");
        }
    }
    if product["fields"]["currency"]["status"] == "CONFLICT" {
        product["price_minor"] = Value::Null;
        product["price_status"] = json!("CONFLICT");
        product["fields"]["price_minor"]["value"] = Value::Null;
        product["fields"]["price_minor"]["status"] = json!("CONFLICT");
    }
    let original_codes = assertions(metadata, "product:original_price:currency");
    let original_set: BTreeSet<_> = original_codes
        .iter()
        .filter_map(|o| o["value"].as_str())
        .collect();
    if original_set.len() == 1 {
        let code = *original_set.first().unwrap();
        let mut incomplete_original = false;
        let amounts = assertions(metadata, "product:original_price:amount")
            .into_iter()
            .map(|mut o| {
                let parsed = crate::price::formatted_money(o["value"].as_str().unwrap_or(""), code);
                incomplete_original |= parsed["minor"].is_null();
                o["raw_value"] = o["value"].clone();
                o["value"] = json!({"minor":parsed["minor"],"currency":code});
                o["numeric_interpretation"] = parsed;
                o["currency_evidence"] = json!(original_codes);
                associated(o)
            })
            .collect();
        merge(product, "original_price", amounts, "DERIVED");
        if incomplete_original {
            product["original_price"] = Value::Null;
            product["fields"]["original_price"]["value"] = Value::Null;
            product["fields"]["original_price"]["status"] = json!("UNKNOWN");
        }
    }
    if let (Some(original), Some(current)) = (
        product["original_price"]["minor"].as_i64(),
        product["price_minor"].as_i64(),
    ) && original > 0
        && original >= current
        && product["original_price"]["currency"] == product["currency"]
    {
        let value = json!({"amount_minor":original-current,"currency":product["currency"],"fraction_bps_floor":(i128::from(original-current)*10000/i128::from(original)) as i64});
        let evidence = json!({"value":value,"source":"DERIVED_PRICE_DIFFERENCE","page":page,"raw_capture_sha256":hash,"inputs":{"original_price":product["fields"]["original_price"]["evidence"],"price":product["fields"]["price_minor"]["evidence"]},"interpretation":"Public list/current price difference; no realized savings or margin claim"});
        merge(product, "discount", vec![evidence], "DERIVED");
    }
    let mut crumbs = vec![];
    for script in structured {
        breadcrumbs(
            &script["value"],
            "",
            script["script_index"].as_u64().unwrap_or(0) as usize,
            page,
            hash,
            &mut crumbs,
        );
    }
    merge(product, "breadcrumbs", crumbs, "OBSERVED");
    let raw = product["raw_json_ld"].clone();
    let offers: Vec<_> = if let Some(array) = raw["offers"].as_array() {
        array
            .iter()
            .enumerate()
            .map(|(i, o)| (format!("/offers/{i}"), o))
            .collect()
    } else {
        vec![("/offers".into(), &raw["offers"])]
    };
    let shipping=offers.into_iter().filter(|(_,o)| !o["shippingDetails"].is_null()).map(|(path,o)|{
        json!({"value":o["shippingDetails"],"source":"JSON_LD","page":page,"raw_capture_sha256":hash,"script_index":product["provenance"]["script_index"],"json_pointer":format!("{}{path}/shippingDetails",product["json_pointer"].as_str().unwrap_or("")),"interpretation":"Offer shipping assertion; no independent delivery or cost verification"})
    }).collect();
    merge(product, "shipping_text", shipping, "OBSERVED");
}

#[cfg(test)]
mod tests {
    #[test]
    fn named_metadata_preserves_repeated_claims_conflicts_and_exact_locators() {
        let html = r#"<meta name="viewport" content="width=device-width"><meta name=" DC.Description " content=" First &amp; second "><meta property="twitter:description" content="Another description"><meta name="description" content=""><meta name="title" content="Cup"><script type="application/ld+json">{"@type":"Product","name":"Cup"}</script>"#;
        let result = crate::extract(html, "https://shop.example/cup").unwrap();
        let field = &result["products"][0]["fields"]["description"];
        assert_eq!(field["status"], "CONFLICT");
        assert!(field["value"].is_null());
        let evidence = field["evidence"].as_array().unwrap();
        assert_eq!(evidence.len(), 2);
        assert_eq!(evidence[0]["value"], "First & second");
        assert_eq!(evidence[0]["raw_property"], " DC.Description ");
        assert_eq!(evidence[0]["match_index"], 1);
        assert_eq!(evidence[1]["match_index"], 2);
        assert_eq!(evidence[0]["selector"], "meta");
        assert_eq!(
            result["products"][0]["fields"]["title"]["status"],
            "OBSERVED"
        );
        let tags: Vec<_> = scraper::Html::parse_document(html)
            .select(&scraper::Selector::parse("meta").unwrap())
            .map(|e| e.value().attr("content").unwrap().to_owned())
            .collect();
        for e in evidence {
            assert_eq!(
                tags[e["match_index"].as_u64().unwrap() as usize],
                e["raw_value"].as_str().unwrap()
            );
            assert_eq!(e["raw_capture_sha256"], result["content_hash"]);
        }
    }
    #[test]
    fn document_title_fallback_requires_single_product_and_remains_derived() {
        let title = "<title> Cup &amp; saucer </title>";
        assert!(
            crate::extract(title, "https://shop.example/article").unwrap()["products"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let one =
            format!("{title}<script type='application/ld+json'>{{\"@type\":\"Product\"}}</script>");
        let result = crate::extract(&one, "https://shop.example/cup").unwrap();
        assert_eq!(
            result["products"][0]["fields"]["title"]["status"],
            "DERIVED"
        );
        assert_eq!(
            result["products"][0]["fields"]["title"]["value"],
            "Cup & saucer"
        );
        let many = format!(
            "{title}<meta name='description' content='Catalog'><script type='application/ld+json'>[{{\"@type\":\"Product\",\"name\":\"A\"}},{{\"@type\":\"Product\",\"name\":\"B\"}}]</script>"
        );
        let result = crate::extract(&many, "https://shop.example/catalog").unwrap();
        for p in result["products"].as_array().unwrap() {
            assert_eq!(p["fields"]["description"]["status"], "UNKNOWN");
        }
    }
    #[test]
    fn breadcrumb_and_shipping_fields_resolve_to_source_assertions() {
        let html = r#"<script type="application/ld+json">{"@graph":[{"@type":"BreadcrumbList","itemListElement":[{"position":1,"name":"Kitchen"}]},{"@type":"Product","name":"Cup","offers":{"shippingDetails":{"shippingDestination":{"addressCountry":"JP"}}}}]}</script>"#;
        let result = crate::extract(html, "https://shop.example/cup").unwrap();
        let fields = &result["products"][0]["fields"];
        for name in ["breadcrumbs", "shipping_text"] {
            assert_eq!(fields[name]["status"], "OBSERVED");
            let e = &fields[name]["evidence"][0];
            let pointer = e["json_pointer"].as_str().unwrap();
            assert_eq!(
                result["structured_data"][0]["value"]
                    .pointer(pointer)
                    .unwrap(),
                &e["value"]
            );
            assert_eq!(e["raw_capture_sha256"], result["content_hash"]);
        }
        assert!(fields["shipping_cost"]["value"].is_null());
    }
    #[test]
    fn standalone_meta_and_cross_format_conflicts_are_explicit() {
        let meta = r#"<meta property="og:type" content="product"><meta property="og:title" content="Cup"><meta property="product:price:amount" content="2,980"><meta property="product:price:currency" content="JPY"><meta property="product:original_price:amount" content="3,980"><meta property="product:original_price:currency" content="JPY"><link rel="canonical" href="/cup">"#;
        let value = crate::extract(meta, "https://shop.example/cup").unwrap();
        let p = &value["products"][0];
        assert_eq!(p["title"], "Cup");
        assert_eq!(p["price_minor"], 2980);
        assert_eq!(p["discount"]["amount_minor"], 1000);
        assert_eq!(p["canonical_url"], "https://shop.example/cup");
        assert_eq!(
            p["fields"]["price_minor"]["evidence"][0]["raw_value"],
            "2,980"
        );
        assert!(p["fields"]["sales"]["value"].is_null());
        let mixed = format!(
            "{meta}<script type='application/ld+json'>{{\"@type\":\"Product\",\"name\":\"Cup\",\"offers\":{{\"price\":\"3200\",\"priceCurrency\":\"JPY\"}}}}</script>"
        );
        let value = crate::extract(&mixed, "https://shop.example/cup").unwrap();
        assert!(value["products"][0]["price_minor"].is_null());
        assert_eq!(
            value["products"][0]["fields"]["price_minor"]["status"],
            "CONFLICT"
        );
    }
    #[test]
    fn ambiguous_page_association_and_article_do_not_create_prices() {
        let meta = r#"<meta property="og:type" content="product"><meta property="og:title" content="Catalog"><meta property="product:price:amount" content="2980"><meta property="product:price:currency" content="JPY">"#;
        let html = format!(
            "{meta}<script type='application/ld+json'>[{{\"@type\":\"Product\",\"name\":\"A\"}},{{\"@type\":\"Product\",\"name\":\"B\"}}]</script>"
        );
        let result = crate::extract(&html, "https://shop.example/catalog").unwrap();
        assert_eq!(result["products"].as_array().unwrap().len(), 2);
        assert!(result["products"][0]["price_minor"].is_null());
        let article = meta.replace("content=\"product\"", "content=\"article\"");
        assert!(
            crate::extract(&article, "https://shop.example/article").unwrap()["products"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let partial = format!("{meta}<meta property='product:price:amount' content='unknown'>");
        let result = crate::extract(&partial, "https://shop.example/cup").unwrap();
        assert!(result["products"][0]["price_minor"].is_null());
        assert_eq!(
            result["products"][0]["fields"]["price_minor"]["evidence"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }
}
