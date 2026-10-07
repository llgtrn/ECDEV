//! Conservative product identity from source assertions; title similarity is not identity.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

fn text(v: &Value) -> String {
    v.as_str()
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
fn brand(p: &Value) -> String {
    text(if p["brand"].is_object() {
        &p["brand"]["name"]
    } else {
        &p["brand"]
    })
}
/// A checksum-valid GTIN-8/12/13/14 as GTIN-14; None otherwise.
pub fn gtin(v: &Value) -> Option<String> {
    let s = v.as_str()?;
    if ![8, 12, 13, 14].contains(&s.len()) || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let sum: u32 = s
        .bytes()
        .rev()
        .enumerate()
        .map(|(i, b)| u32::from(b - b'0') * if i % 2 == 1 { 3 } else { 1 })
        .sum();
    (sum.is_multiple_of(10)).then(|| format!("{s:0>14}"))
}
fn key(c: &Value) -> (String, &'static str) {
    let p = &c["product"];
    let variant = format!("{}|{}", text(&p["color"]), text(&p["size"]));
    let codes: BTreeSet<_> = ["gtin", "ean", "upc", "gtin8", "gtin14"]
        .iter()
        .filter_map(|k| gtin(&p[*k]))
        .collect();
    if codes.len() == 1 {
        return (
            format!("gtin:{}:{variant}", codes.first().unwrap()),
            "CHECKSUM_VALID_GTIN_AND_VARIANT",
        );
    }
    let asin = p["asin"].as_str().unwrap_or("").to_ascii_uppercase();
    if asin.len() == 10 && asin.bytes().all(|b| b.is_ascii_alphanumeric()) {
        let scope = url::Url::parse(c["source"].as_str().unwrap_or(""))
            .ok()
            .map(|u| match u.host_str() {
                Some("amazon.co.jp" | "www.amazon.co.jp") => "AMAZON_JP".into(),
                Some("amazon.com" | "www.amazon.com") => "AMAZON_US".into(),
                _ => u.origin().ascii_serialization(),
            })
            .unwrap_or_else(|| "UNKNOWN_ORIGIN".into());
        return (format!("asin:{scope}:{asin}:{variant}"), "ASIN_AND_VARIANT");
    }
    let b = brand(p);
    for k in ["mpn", "model"] {
        let id = text(&p[k]);
        if !b.is_empty() && !id.is_empty() {
            return (
                format!("{k}:{b}:{id}:{variant}:sku:{}", text(&p["sku"])),
                "BRAND_MANUFACTURER_IDENTIFIER_AND_VARIANT",
            );
        }
    }
    let sku = text(&p["sku"]);
    if !sku.is_empty()
        && let Ok(url) = url::Url::parse(c["source"].as_str().unwrap_or(""))
    {
        return (
            format!(
                "sku:{}:{b}:{sku}:{variant}",
                url.origin().ascii_serialization()
            ),
            "ORIGIN_SCOPED_SKU_AND_VARIANT",
        );
    }
    (
        format!(
            "page:{}:{}:{variant}",
            c["source"].as_str().unwrap_or(""),
            text(&p["title"])
        ),
        "EXACT_PAGE_TITLE_AND_VARIANT_ONLY",
    )
}

pub fn resolve(records: Vec<Value>) -> (Vec<Value>, Value) {
    let input_count = records.len();
    let page_title = |c: &Value| {
        format!(
            "{}|{}",
            c["source"].as_str().unwrap_or(""),
            text(&c["product"]["title"])
        )
    };
    let mut anchors: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let sku_anchor = |c: &Value| {
        let sku = text(&c["product"]["sku"]);
        let origin = url::Url::parse(c["source"].as_str().unwrap_or("")).ok()?;
        (!sku.is_empty()).then(|| {
            format!(
                "{}|{sku}|{}|{}",
                origin.origin().ascii_serialization(),
                text(&c["product"]["color"]),
                text(&c["product"]["size"])
            )
        })
    };
    let mut gtin_skus: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for c in &records {
        let (identity, basis) = key(c);
        if basis == "CHECKSUM_VALID_GTIN_AND_VARIANT"
            && let Some(sku) = sku_anchor(c)
        {
            gtin_skus.entry(sku).or_default().insert(identity.clone());
        }
        if basis != "EXACT_PAGE_TITLE_AND_VARIANT_ONLY"
            && text(&c["product"]["color"]).is_empty()
            && text(&c["product"]["size"]).is_empty()
        {
            anchors.entry(page_title(c)).or_default().insert(identity);
        }
    }
    let mut groups: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for c in records {
        let (mut identity, basis) = key(&c);
        if basis == "ORIGIN_SCOPED_SKU_AND_VARIANT"
            && ["gtin", "ean", "upc", "gtin8", "gtin14"].iter().all(|k| {
                c["product"][*k].is_null() && c["product"]["fields"][*k]["status"] != "CONFLICT"
            })
            && let Some(ids) = sku_anchor(&c).and_then(|sku| gtin_skus.get(&sku))
            && ids.len() == 1
        {
            identity = ids.first().unwrap().clone();
        }
        if basis == "EXACT_PAGE_TITLE_AND_VARIANT_ONLY"
            && text(&c["product"]["color"]).is_empty()
            && text(&c["product"]["size"]).is_empty()
            && let Some(ids) = anchors.get(&page_title(&c))
            && ids.len() == 1
        {
            identity = ids.first().unwrap().clone();
        }
        groups.entry(identity).or_default().push(c);
    }
    let mut candidates = vec![];
    let mut title_groups: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for (identity, rows) in groups {
        let mut primary = rows
            .iter()
            .max_by_key(|r| {
                r["product"]
                    .as_object()
                    .map(|p| p.values().filter(|v| !v.is_null()).count())
                    .unwrap_or(0)
            })
            .unwrap()
            .clone();
        let evidence: BTreeSet<_> = rows
            .iter()
            .flat_map(|r| r["evidence_ids"].as_array().into_iter().flatten())
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        let sources: BTreeSet<_> = rows
            .iter()
            .filter_map(|r| r["source"].as_str().map(str::to_string))
            .collect();
        let mut conflicts = serde_json::Map::new();
        for name in [
            "title",
            "brand",
            "sku",
            "price_minor",
            "currency",
            "availability",
            "seller",
            "mpn",
            "model",
            "rating",
            "review_count",
        ] {
            let values: BTreeSet<_> = rows
                .iter()
                .map(|r| &r["product"][name])
                .filter(|v| !v.is_null())
                .map(Value::to_string)
                .collect();
            if values.len() > 1 {
                conflicts.insert(name.into(),json!({"status":"CONFLICT","observations":rows.iter().map(|r|json!({"value":r["product"][name],"source":r["source"],"evidence_ids":r["evidence_ids"]})).collect::<Vec<_>>() }));
                primary["product"][name] = Value::Null;
                if name == "price_minor" || name == "currency" {
                    primary["product"]["price_status"] = json!("CONFLICT");
                    primary["economics"] = Value::Null;
                    primary["state"] = json!("INSUFFICIENT_EVIDENCE");
                    primary["rejection_reasons"] = json!([]);
                    primary["survival_reason"] = json!(
                        "Conflicting observations require resolution; earlier assessments retained in resolution.observations"
                    );
                }
            }
        }
        let entity_id = format!("product:{:x}", Sha256::digest(identity.as_bytes()));
        let basis = if identity.starts_with("gtin:") {
            "CHECKSUM_VALID_GTIN_AND_VARIANT"
        } else {
            key(&primary).1
        };
        // Field projections must not retain a precise first-source value after merging
        // contradictory source assertions. Preserve every original locator alongside it.
        let field_names: BTreeSet<String> = rows
            .iter()
            .flat_map(|r| {
                r["product"]["fields"]
                    .as_object()
                    .into_iter()
                    .flat_map(|f| f.keys().cloned())
            })
            .collect();
        for name in field_names {
            let evidence: Vec<Value> = rows
                .iter()
                .flat_map(|r| {
                    r["product"]["fields"][&name]["evidence"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .cloned()
                })
                .collect();
            // Evidence can describe an element of a collection or a failed conversion.
            // Compare field projections, and retain the status of each source assertion.
            let states: BTreeSet<_> = rows
                .iter()
                .filter_map(|r| r["product"]["fields"][&name]["status"].as_str())
                .collect();
            let values: BTreeSet<_> = rows
                .iter()
                .map(|r| &r["product"]["fields"][&name]["value"])
                .filter(|v| !v.is_null())
                .map(|v| {
                    if name == "brand" && v["name"].is_string() {
                        v["name"].to_string()
                    } else {
                        v.to_string()
                    }
                })
                .collect();
            if !primary["product"]["fields"].is_object() {
                primary["product"]["fields"] = json!({});
            }
            if values.len() > 1 || conflicts.contains_key(&name) || states.contains("CONFLICT") {
                primary["product"]["fields"][&name] =
                    json!({"status":"CONFLICT","value":null,"evidence":evidence});
            } else if values.len() == 1 {
                let value = rows
                    .iter()
                    .map(|r| &r["product"]["fields"][&name]["value"])
                    .find(|v| !v.is_null())
                    .unwrap();
                let status = if states.contains("OBSERVED") {
                    "OBSERVED"
                } else if states.contains("DERIVED") {
                    "DERIVED"
                } else if states.contains("ESTIMATED") {
                    "ESTIMATED"
                } else {
                    "UNKNOWN"
                };
                primary["product"]["fields"][&name] =
                    json!({"status":status,"value":value,"evidence":evidence});
            }
            // Known individual offers do not establish a complete product price.
            if name == "price_minor" && primary["product"]["price_minor"].is_null() {
                primary["product"]["fields"][&name]["value"] = Value::Null;
                primary["product"]["fields"][&name]["status"] =
                    primary["product"]["price_status"].clone();
            }
        }
        primary["evidence_ids"] = json!(evidence);
        primary["entity_id"] = json!(entity_id);
        primary["resolution"] = json!({"status":"DERIVED_IDENTITY_FROM_SOURCE_ASSERTIONS","basis":basis,"identity_key":identity,"source_pages":sources,"observation_count":rows.len(),"observations":rows,"conflicts":conflicts,"independently_verified_identity":false});
        let title = text(&primary["product"]["title"]);
        if !title.is_empty() {
            title_groups.entry(title).or_default().push(json!({"candidate_id":primary["id"],"entity_id":entity_id,"brand":brand(&primary["product"])}));
        }
        candidates.push(primary);
    }
    // Distinct titles that are near-duplicates (MinHash/LSH candidates with Jaccard >= 0.9 over
    // character 3-grams) are proposed as uncertain links, never merged.
    let mut known: Vec<crate::memory::NamedEntity> = Vec::new();
    let mut similar = Vec::new();
    for (title, ids) in &title_groups {
        let incoming = crate::memory::NamedEntity {
            id: title.clone(),
            name: title.clone(),
            labels: vec!["Entity".into()],
        };
        if let Some(crate::memory::NameMatch::Similar(i, score)) =
            crate::memory::match_names(&mut known, std::slice::from_ref(&incoming)).pop()
        {
            similar.push(json!({"relation":"POSSIBLE_EQUIVALENCE","status":"UNCERTAIN_NOT_MERGED","basis":"TITLE_NAME_SIMILARITY_MINHASH_JACCARD","jaccard":score,"threshold":crate::memory::FUZZY_JACCARD_THRESHOLD,"titles":[known[i].name.clone(),title.clone()],"entities":[title_groups[&known[i].name].clone(),ids.clone()]}));
        }
        known.push(incoming);
    }
    let mut possible:Vec<_>=title_groups.into_iter().filter(|(_,ids)|ids.len()>1).map(|(title,ids)|json!({"relation":"POSSIBLE_EQUIVALENCE","status":"UNCERTAIN_NOT_MERGED","basis":"NORMALIZED_TITLE_ONLY","title":title,"entities":ids})).collect();
    possible.extend(similar);
    let metrics = json!({"input_observations":input_count,"resolved_candidates":candidates.len(),"merged_observations":input_count-candidates.len(),"possible_equivalence_groups":possible,"policy":"Strong identifiers plus variant attributes; SKU confined to source origin; title alone never merges across pages"});
    (candidates, metrics)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn candidate(id: &str, origin: &str, sku: &str, color: &str) -> Value {
        json!({"id":id,"source":format!("https://{origin}/products/{id}"),"product":{"title":"Coffee Cup","sku":sku,"color":color,"brand":"Hario","price_minor":3000,"currency":"JPY"},"evidence_ids":[id],"state":"VALIDATING"})
    }
    #[test]
    fn near_duplicate_titles_are_uncertain_links_never_merges() {
        let titled = |id: &str, origin: &str, title: &str| {
            let mut c = candidate(id, origin, id, "white");
            c["product"]["title"] = json!(title);
            c
        };
        let (rows, report) = resolve(vec![
            titled(
                "a",
                "one.example",
                "Hario V60 Ceramic Coffee Dripper 02 White",
            ),
            titled(
                "b",
                "two.example",
                "HARIO V60 Ceramic Coffee Dripper 02 White!",
            ),
            titled("c", "three.example", "Kalita Wave 185 Stainless Dripper"),
        ]);
        assert_eq!(rows.len(), 3, "similar titles never merge");
        let links: Vec<&Value> = report["possible_equivalence_groups"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|g| g["basis"] == "TITLE_NAME_SIMILARITY_MINHASH_JACCARD")
            .collect();
        assert_eq!(links.len(), 1, "{report}");
        assert_eq!(links[0]["status"], "UNCERTAIN_NOT_MERGED");
        assert!(links[0]["jaccard"].as_f64().unwrap() >= 0.9);
    }
    #[test]
    fn identity_does_not_merge_title_only_cross_origin_or_variants() {
        let (rows, report) = resolve(vec![
            candidate("a", "one.example", "C1", "red"),
            candidate("b", "one.example", "C1", "red"),
            candidate("c", "two.example", "C1", "red"),
            candidate("d", "one.example", "C1", "blue"),
        ]);
        assert_eq!(rows.len(), 3);
        assert_eq!(report["merged_observations"], 1);
        assert_eq!(
            report["possible_equivalence_groups"][0]["status"],
            "UNCERTAIN_NOT_MERGED"
        );
        assert!(
            rows.iter()
                .any(|r| r["evidence_ids"].as_array().unwrap().len() == 2)
        );
        assert_eq!(gtin(&json!("1234567890128")), Some("01234567890128".into()));
        assert!(gtin(&json!("1234567890129")).is_none());
        let mut a = candidate("a", "one.example", "C1", "red");
        let mut b = candidate("b", "one.example", "C1", "red");
        a["product"]["price_minor"] = json!(3000);
        b["product"]["price_minor"] = json!(6000);
        let (rows, _) = resolve(vec![a, b]);
        assert!(rows[0]["product"]["price_minor"].is_null());
        assert_eq!(rows[0]["state"], "INSUFFICIENT_EVIDENCE");
        let strong = candidate("same", "one.example", "C1", "");
        let mut weak = strong.clone();
        weak["id"] = json!("weak");
        weak["product"]["sku"] = Value::Null;
        weak["product"]["brand"] = Value::Null;
        let (rows, report) = resolve(vec![strong, weak]);
        assert_eq!(rows.len(), 1);
        assert_eq!(report["merged_observations"], 1);
        assert_eq!(rows[0]["resolution"]["observation_count"], 2);
        let mut mug = candidate("mug", "one.example", "TCM-300-GG", "");
        let mut part = candidate("part", "one.example", "S-TCM-300-GG", "");
        mug["product"]["mpn"] = json!("4573533861937");
        part["product"]["mpn"] = json!("4573533861937");
        let (rows, _) = resolve(vec![mug, part]);
        assert_eq!(
            rows.len(),
            2,
            "different source SKUs disambiguate a shared manufacturer-number assertion"
        );
        let mut partial = candidate("partial", "one.example", "P1", "");
        partial["product"]["price_minor"] = Value::Null;
        partial["product"]["price_status"] = json!("UNKNOWN");
        partial["product"]["fields"] = json!({"price_minor":{"status":"UNKNOWN","value":null,"evidence":[{"value":3000,"raw_value":"3000","source":"JSON_LD"}]}});
        let (rows, _) = resolve(vec![partial]);
        assert!(rows[0]["product"]["fields"]["price_minor"]["value"].is_null());
        assert_eq!(
            rows[0]["product"]["fields"]["price_minor"]["status"],
            "UNKNOWN"
        );
        assert_eq!(
            rows[0]["product"]["fields"]["price_minor"]["evidence"][0]["value"],
            3000
        );
        let mut jp = candidate("jp", "www.amazon.co.jp", "", "red");
        let mut us = candidate("us", "www.amazon.com", "", "red");
        jp["product"]["asin"] = json!("B012345678");
        us["product"]["asin"] = json!("B012345678");
        let (rows, _) = resolve(vec![jp, us]);
        assert_eq!(
            rows.len(),
            2,
            "ASIN assertions remain scoped to the source marketplace"
        );
        let mut identified = candidate("gtin", "one.example", "C1", "red");
        identified["product"]["gtin"] = json!("4963264503563");
        let weak = candidate("sku", "one.example", "C1", "red");
        let (rows, _) = resolve(vec![identified.clone(), weak.clone()]);
        assert_eq!(
            rows.len(),
            1,
            "same origin SKU observation joins the unique GTIN anchor"
        );
        assert_eq!(
            rows[0]["resolution"]["basis"],
            "CHECKSUM_VALID_GTIN_AND_VARIANT"
        );
        let mut ambiguous = identified.clone();
        ambiguous["product"]["gtin"] = json!("4963264503556");
        let (rows, _) = resolve(vec![identified, ambiguous, weak]);
        assert_eq!(
            rows.len(),
            3,
            "two GTIN anchors prevent an ambiguous SKU bridge"
        );
    }
}
