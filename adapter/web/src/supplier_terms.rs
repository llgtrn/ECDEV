//! Published organization offers. No negotiated wholesale or commercial validation is implied.
use serde_json::{Value, json};
use std::collections::BTreeMap;
use url::Url;

struct Source<'a> {
    base: &'a Url,
    hash: &'a str,
    script: u64,
}
impl Source<'_> {
    fn field(&self, value: Value, pointer: &str, status: &str) -> Value {
        let evidence = if value.is_null() {
            vec![]
        } else {
            vec![self.evidence(value.clone(), pointer)]
        };
        json!({"status":if value.is_null(){"UNKNOWN"}else{status},"value":value,"evidence":evidence})
    }
    fn evidence(&self, value: Value, pointer: &str) -> Value {
        json!({"value":value,"source":"JSON_LD","page":self.base.as_str(),"script_index":self.script,"json_pointer":pointer,"raw_capture_sha256":self.hash,"interpretation":"PUBLISHED_SOURCE_ASSERTION_NOT_NEGOTIATED_OR_INDEPENDENTLY_VERIFIED"})
    }
    fn offer(&self, offer: &Value, pointer: &str) -> Value {
        let mut fields = serde_json::Map::new();
        for (name, key) in [
            ("item", "itemOffered"),
            ("lead_time", "deliveryLeadTime"),
            ("origin", "itemOffered/countryOfOrigin"),
            ("shipping_term", "shippingDetails"),
            ("delivery_method", "availableDeliveryMethod"),
            ("availability", "availability"),
            ("published_price", "price"),
            ("published_currency", "priceCurrency"),
        ] {
            fields.insert(
                name.into(),
                self.field(
                    offer
                        .pointer(&format!("/{key}"))
                        .cloned()
                        .unwrap_or(Value::Null),
                    &format!("{pointer}/{key}"),
                    "OBSERVED",
                ),
            );
        }
        fields["shipping_term"]["interpretation"] =
            json!("PUBLISHED_SHIPPING_DETAILS_NOT_VERIFIED_INCOTERMS");
        let minimum = &offer["eligibleQuantity"]["minValue"];
        let valid_minimum = minimum.as_u64().is_some_and(|n| n > 0)
            || minimum.as_str().is_some_and(|s| {
                !s.is_empty()
                    && s.bytes().all(|b| b.is_ascii_digit())
                    && s.parse::<u64>().is_ok_and(|n| n > 0)
            });
        let moq = if valid_minimum {
            json!({"minimum":minimum,"unit_code":offer["eligibleQuantity"]["unitCode"],"unit_text":offer["eligibleQuantity"]["unitText"],"interpretation":"ELIGIBLE_ORDER_QUANTITY_LOWER_BOUND_NOT_VERIFIED_WHOLESALE_MOQ"})
        } else {
            Value::Null
        };
        fields.insert(
            "moq".into(),
            self.field(moq, &format!("{pointer}/eligibleQuantity"), "DERIVED"),
        );
        if valid_minimum {
            fields["moq"]["evidence"][0]["value"] = offer["eligibleQuantity"].clone();
        }
        let specs: Vec<_> = if let Some(items) = offer["priceSpecification"].as_array() {
            items
                .iter()
                .enumerate()
                .map(|(i, v)| (v, format!("{pointer}/priceSpecification/{i}")))
                .collect()
        } else {
            vec![(
                &offer["priceSpecification"],
                format!("{pointer}/priceSpecification"),
            )]
        };
        let mut values = BTreeMap::new();
        let mut evidence = vec![];
        let mut invalid = false;
        for (spec, path) in specs {
            if !schema_type(spec, "UnitPriceSpecification") {
                continue;
            }
            let currency = spec["priceCurrency"].as_str().unwrap_or("");
            evidence.push(self.evidence(spec.clone(), &path));
            if let Some(minor) = crate::money(&spec["price"], currency).filter(|n| *n >= 0) {
                let value = json!({"minor":minor,"currency":currency,"reference_quantity":spec["referenceQuantity"],"unit_code":spec["unitCode"],"unit_text":spec["unitText"],"value_added_tax_included":spec["valueAddedTaxIncluded"],"interpretation":"PUBLISHED_UNIT_PRICE_SPECIFICATION_NOT_PRODUCT_COST"});
                values.insert(value.to_string(), value);
            } else {
                invalid = true;
            }
        }
        let value = if values.len() == 1 && !invalid {
            values.values().next().unwrap().clone()
        } else {
            Value::Null
        };
        let status = if values.len() > 1 {
            "CONFLICT"
        } else if value.is_null() {
            "UNKNOWN"
        } else {
            "DERIVED"
        };
        fields.insert("currency".into(),json!({"value":value["currency"],"status":if value.is_null(){"UNKNOWN"}else{"DERIVED"},"evidence":evidence}));
        fields.insert(
            "unit_price".into(),
            json!({"value":value,"status":status,"evidence":evidence}),
        );
        json!({"source":self.base.as_str(),"script_index":self.script,"json_pointer":pointer,"raw_capture_sha256":self.hash,"fields":fields,"raw_offer":offer,"qualification":"UNVERIFIED_PUBLISHED_OFFER"})
    }
}
pub(crate) fn schema_type(value: &Value, expected: &str) -> bool {
    let matches = |s: &str| {
        s == expected
            || s == format!("https://schema.org/{expected}")
            || s == format!("http://schema.org/{expected}")
    };
    value["@type"].as_str().is_some_and(matches)
        || value["@type"]
            .as_array()
            .is_some_and(|a| a.iter().filter_map(Value::as_str).any(matches))
}
pub(crate) fn organizations<'a>(
    value: &'a Value,
    pointer: &str,
    out: &mut Vec<(&'a Value, String)>,
) {
    if schema_type(value, "Organization") {
        out.push((value, pointer.into()));
    }
    match value {
        Value::Array(a) => {
            for (i, v) in a.iter().enumerate() {
                organizations(v, &format!("{pointer}/{i}"), out);
            }
        }
        Value::Object(o) => {
            for (k, v) in o {
                organizations(
                    v,
                    &format!("{pointer}/{}", k.replace('~', "~0").replace('/', "~1")),
                    out,
                );
            }
        }
        _ => {}
    }
}
fn offers<'a>(value: &'a Value, pointer: &str, out: &mut Vec<(&'a Value, String)>) {
    if out.len() >= 100 {
        return;
    }
    if schema_type(value, "Offer") {
        out.push((value, pointer.into()));
        return;
    }
    if let Some(a) = value.as_array() {
        for (i, v) in a.iter().enumerate() {
            offers(v, &format!("{pointer}/{i}"), out);
        }
    } else if schema_type(value, "OfferCatalog") || schema_type(value, "ItemList") {
        offers(
            &value["itemListElement"],
            &format!("{pointer}/itemListElement"),
            out,
        );
    } else if schema_type(value, "ListItem") {
        offers(&value["item"], &format!("{pointer}/item"), out);
    }
}
pub fn extract(base: &Url, structured: &[Value], hash: &str) -> Vec<Value> {
    let mut leads = vec![];
    for script in structured {
        let source = Source {
            base,
            hash,
            script: script["script_index"].as_u64().unwrap_or(0),
        };
        let mut orgs = vec![];
        organizations(&script["value"], "", &mut orgs);
        for (org, pointer) in orgs {
            let mut raw = vec![];
            offers(
                &org["makesOffer"],
                &format!("{pointer}/makesOffer"),
                &mut raw,
            );
            offers(
                &org["hasOfferCatalog"],
                &format!("{pointer}/hasOfferCatalog"),
                &mut raw,
            );
            if raw.is_empty() {
                continue;
            }
            let terms: Vec<_> = raw.iter().map(|(v, p)| source.offer(v, p)).collect();
            let mut fields = serde_json::Map::new();
            for (name, key) in [
                ("name", "name"),
                ("organization_country", "address/addressCountry"),
                ("certification_claim", "hasCertification"),
            ] {
                fields.insert(
                    name.into(),
                    source.field(
                        org.pointer(&format!("/{key}"))
                            .cloned()
                            .unwrap_or(Value::Null),
                        &format!("{pointer}/{key}"),
                        "OBSERVED",
                    ),
                );
            }
            fields.insert("offer".into(),json!({"value":raw.iter().map(|(v,_)|*v).collect::<Vec<_>>(),"status":"OBSERVED","evidence":raw.iter().map(|(v,p)|source.evidence((*v).clone(),p)).collect::<Vec<_>>()}));
            for name in [
                "supplier",
                "manufacturer",
                "distributor",
                "customization",
                "moq",
                "unit_price",
                "currency",
                "lead_time",
                "origin",
                "shipping_term",
                "factory_capacity",
            ] {
                let field = if terms.len() == 1 && terms[0]["fields"][name].is_object() {
                    terms[0]["fields"][name].clone()
                } else {
                    json!({"value":null,"status":"UNKNOWN","evidence":[]})
                };
                fields.insert(name.into(), field);
            }
            leads.push(json!({"kind":"PUBLIC_SUPPLIER_LEAD","source":base.as_str(),"status":"OBSERVED_SOURCE_CLAIMS","fields":fields,"offers":terms,"claims":[],"follow_up_urls":[],"raw_capture_sha256":hash,"qualification":"UNVERIFIED","limitation":"Published organization offers are not verified wholesale eligibility, negotiated quotations or product-cost inputs. Terms stay attached to each offer; different offers and publishers are never collapsed."}));
        }
    }
    leads
}
pub fn microdata(
    base: &Url,
    normalized: &Value,
    paths: &BTreeMap<String, String>,
    raw: &Value,
    hash: &str,
) -> Vec<Value> {
    let mut leads = extract(base, &[json!({"value":normalized,"script_index":0})], hash);
    fn relabel(value: &mut Value, paths: &BTreeMap<String, String>, raw: &Value) {
        match value {
            Value::Object(o) => {
                if o.contains_key("script_index") && o.contains_key("json_pointer") {
                    let pointer = o["json_pointer"]
                        .as_str()
                        .and_then(|p| paths.get(p))
                        .cloned();
                    o.remove("script_index");
                    o.insert("data_container".into(), json!("microdata"));
                    o.insert(
                        "normalization".into(),
                        json!(
                            "Native microdata schema projection; original graph pointer retained"
                        ),
                    );
                    if o.get("source").is_some_and(|s| s == "JSON_LD") {
                        o.insert("source".into(), json!("MICRODATA"));
                    }
                    if let Some(p) = pointer {
                        o.insert("json_pointer".into(), json!(p));
                        if o.remove("raw_offer").is_some() {
                            o.insert(
                                "raw_microdata".into(),
                                raw.pointer(&p).cloned().unwrap_or(Value::Null),
                            );
                        }
                    } else {
                        o.insert("json_pointer".into(), Value::Null);
                        o.insert(
                            "provenance_status".into(),
                            json!("NORMALIZED_POINTER_MAPPING_UNAVAILABLE"),
                        );
                    }
                }
                for child in o.values_mut() {
                    relabel(child, paths, raw);
                }
            }
            Value::Array(a) => {
                for v in a {
                    relabel(v, paths, raw);
                }
            }
            _ => {}
        }
    }
    for lead in &mut leads {
        relabel(lead, paths, raw);
    }
    leads
}
#[cfg(test)]
mod tests {
    use super::*;
    use scraper::Html;
    #[test]
    fn explicit_supplier_offers_preserve_terms_and_source_pointers() {
        let raw = json!({"@graph":[{"@type":"Organization","name":"Supplier","address":{"addressCountry":"JP"},"hasCertification":{"name":"ISO source claim"},"makesOffer":{"@type":"Offer","itemOffered":{"@type":"Product","countryOfOrigin":"VN"},"eligibleQuantity":{"minValue":24,"unitCode":"C62"},"deliveryLeadTime":{"minValue":7,"maxValue":14,"unitCode":"DAY"},"priceSpecification":{"@type":"UnitPriceSpecification","price":"12.50","priceCurrency":"USD","referenceQuantity":{"value":1,"unitCode":"C62"}}}}]});
        let base = Url::parse("https://supplier.example/terms").unwrap();
        let leads = extract(&base, &[json!({"script_index":3,"value":raw})], "fixture");
        let fields = &leads[0]["fields"];
        assert_eq!(fields["unit_price"]["value"]["minor"], 1250);
        assert_eq!(fields["moq"]["value"]["minimum"], 24);
        assert_eq!(fields["unit_price"]["status"], "DERIVED");
        assert_eq!(fields["lead_time"]["value"]["unitCode"], "DAY");
        assert_eq!(fields["origin"]["value"], "VN");
        assert_eq!(fields["organization_country"]["value"], "JP");
        assert_eq!(fields["certification_claim"]["status"], "OBSERVED");
        for name in ["unit_price", "moq", "origin", "lead_time"] {
            let e = &fields[name]["evidence"][0];
            assert_eq!(e["script_index"], 3);
            assert_eq!(e["raw_capture_sha256"], "fixture");
            assert!(raw.pointer(e["json_pointer"].as_str().unwrap()).is_some());
        }
        assert_eq!(
            fields["unit_price"]["evidence"][0]["value"],
            raw["@graph"][0]["makesOffer"]["priceSpecification"]
        );
    }
    #[test]
    fn offers_and_organizations_do_not_mix_prices_or_unverified_origins() {
        let raw = json!({"@graph":[{"@type":"Organization","name":"Retail","address":{"addressCountry":"JP"},"makesOffer":{"@type":"Offer","price":3000,"priceCurrency":"JPY"}},{"@type":"Organization","name":"Other","hasOfferCatalog":{"@type":"OfferCatalog","itemListElement":[{"@type":"ListItem","item":{"@type":"Offer","priceSpecification":[{"@type":"UnitPriceSpecification","price":"10.00","priceCurrency":"USD"},{"@type":"UnitPriceSpecification","price":"11.00","priceCurrency":"USD"}]}},{"@type":"Offer","priceSpecification":[{"@type":"UnitPriceSpecification","price":"10.00","priceCurrency":"USD"},{"@type":"UnitPriceSpecification","price":"2980.0000000000001","priceCurrency":"JPY"}]}]}}]});
        let base = Url::parse("https://supplier.example/terms").unwrap();
        let leads = extract(&base, &[json!({"value":raw,"script_index":0})], "fixture");
        assert_eq!(leads.len(), 2);
        assert_eq!(leads[0]["fields"]["origin"]["status"], "UNKNOWN");
        assert_eq!(leads[0]["fields"]["unit_price"]["status"], "UNKNOWN");
        assert_eq!(
            leads[0]["offers"][0]["fields"]["published_price"]["value"],
            3000
        );
        assert_eq!(leads[1]["fields"]["unit_price"]["status"], "UNKNOWN");
        assert_eq!(
            leads[1]["offers"][0]["fields"]["unit_price"]["status"],
            "CONFLICT"
        );
        assert_eq!(
            leads[1]["offers"][1]["fields"]["unit_price"]["status"],
            "UNKNOWN"
        );
        assert_eq!(
            leads[1]["offers"][1]["fields"]["unit_price"]["evidence"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert!(extract(&base,&[json!({"value":{"@type":"Product","offers":{"@type":"Offer","price":3000,"priceCurrency":"JPY"}}})],"fixture").is_empty());
        let page = Html::parse_document("<p>Wholesale</p><title>Page label</title>");
        let visible = crate::supplier::extract(
            &page,
            &base,
            &[json!({"value":raw,"script_index":0})],
            "fixture",
        );
        assert_eq!(visible[0]["fields"]["name"]["value"], "Page label");
    }
    #[test]
    fn microdata_supplier_offers_retain_original_graph_locators() {
        let html = r#"<div itemscope itemtype="https://schema.org/Organization"><span itemprop="name">Supplier</span><div itemprop="makesOffer" itemscope itemtype="https://schema.org/Offer"><div itemprop="priceSpecification" itemscope itemtype="https://schema.org/UnitPriceSpecification"><meta itemprop="price" content="12.50"><meta itemprop="priceCurrency" content="USD"></div><div itemprop="eligibleQuantity" itemscope itemtype="https://schema.org/QuantitativeValue"><meta itemprop="minValue" content="24"><meta itemprop="unitCode" content="C62"></div></div></div>"#;
        let result = crate::extract(html, "https://supplier.example/terms").unwrap();
        let fields = &result["supplier_leads"][0]["fields"];
        assert_eq!(fields["unit_price"]["value"]["minor"], 1250);
        assert_eq!(fields["moq"]["value"]["minimum"], "24");
        for name in ["unit_price", "moq"] {
            let e = &fields[name]["evidence"][0];
            assert_eq!(e["source"], "MICRODATA");
            assert!(e.get("script_index").is_none());
            assert!(
                result["microdata"]
                    .pointer(e["json_pointer"].as_str().unwrap())
                    .is_some()
            );
        }
        assert!(result["supplier_leads"][0]["offers"][0]["raw_microdata"].is_object());
    }
}
