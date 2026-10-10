//! Conservative product identity from source assertions; title similarity is not identity.
use crate::sha256::Sha256;
use serde_json::{Value, json};
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
    // An all-zero code passes the check digit but is a placeholder, never a product.
    if ![8, 12, 13, 14].contains(&s.len())
        || !s.bytes().all(|b| b.is_ascii_digit())
        || s.bytes().all(|b| b == b'0')
    {
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
/// An ISBN-13 as GTIN-14. ISBN-13 is an EAN-13 (prefix 978 or 979), and ISBNs are written with
/// hyphens or spaces, so only here are those two separators removed. The stated value is kept
/// beside it; an ISBN-10 is not a GTIN and stays a stated claim.
pub fn isbn13(v: &Value) -> Option<String> {
    let s: String = v
        .as_str()?
        .chars()
        .filter(|c| *c != '-' && *c != ' ')
        .collect();
    if s.len() == 13 && (s.starts_with("978") || s.starts_with("979")) {
        gtin(&json!(s))
    } else {
        isbn10_as_gtin(&s)
    }
}
/// A valid ISBN-10 (mod-11 check, final X allowed) converted by the ISO 2108 rule: prefix 978,
/// the first nine digits, a new EAN-13 check digit. One failing the ISBN-10 check is not converted.
fn isbn10_as_gtin(s: &str) -> Option<String> {
    let b = s.as_bytes();
    if b.len() != 10
        || !b[..9].iter().all(u8::is_ascii_digit)
        || !(b[9].is_ascii_digit() || b[9] == b'X')
    {
        return None;
    }
    let sum: u32 = b
        .iter()
        .enumerate()
        .map(|(i, c)| (10 - i as u32) * if *c == b'X' { 10 } else { u32::from(c - b'0') })
        .sum();
    if !sum.is_multiple_of(11) {
        return None;
    }
    let body = format!("978{}", &s[..9]);
    let t: u32 = body
        .bytes()
        .enumerate()
        .map(|(i, c)| u32::from(c - b'0') * if i % 2 == 1 { 3 } else { 1 })
        .sum();
    gtin(&json!(format!("{body}{}", (10 - t % 10) % 10)))
}
const GTIN_FIELDS: [&str; 5] = ["gtin", "ean", "upc", "gtin8", "gtin14"];
fn valid_codes(p: &Value) -> BTreeSet<String> {
    GTIN_FIELDS
        .iter()
        .filter_map(|k| gtin(&p[*k]))
        .chain(isbn13(&p["isbn"]))
        .collect()
}
/// Every stated product code with what ECDEV can say about it. Nothing is repaired: a code that
/// fails its check digit stays a claim and never becomes identity.
pub fn identifier_claims(p: &Value) -> Vec<Value> {
    let mut out = vec![];
    for k in GTIN_FIELDS.iter().chain(["isbn"].iter()) {
        let v = &p[*k];
        let stated = match v {
            // A blank field is not a stated code.
            Value::String(s) if s.trim().is_empty() => continue,
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            _ => continue,
        };
        let (state, normalized) = if *k == "isbn" {
            let compact: String = stated.chars().filter(|c| *c != '-' && *c != ' ').collect();
            match isbn13(&json!(stated)) {
                Some(g) if compact.len() == 10 => ("VALID_ISBN10_CONVERTED_TO_GTIN13", Some(g)),
                Some(g) => ("VALID_ISBN13_AS_GTIN13", Some(g)),
                None if compact.len() == 10 => ("INVALID_ISBN10", None),
                None if compact.len() == 13 && compact.bytes().all(|b| b.is_ascii_digit()) => {
                    ("INVALID_CHECK_DIGIT_OR_PREFIX", None)
                }
                None => ("MALFORMED", None),
            }
        } else if let Some(g) = gtin(&json!(stated)) {
            ("VALID_GTIN", Some(g))
        } else if stated.bytes().all(|b| b.is_ascii_digit())
            && [8, 12, 13, 14].contains(&stated.len())
        {
            ("INVALID_CHECK_DIGIT", None)
        } else {
            ("MALFORMED", None)
        };
        out.push(json!({"field":k,"stated":stated,"state":state,"gtin14_normalized":normalized}));
    }
    out
}
/// What identity uncertainty asks for next. Certainty about identity is a separate question from
/// whether the product is worth pursuing: nothing here changes a candidate's state.
fn identity_next_step(state: &str, identity_key: &str, product: &Value) -> Value {
    match state {
        "IDENTIFIER_CONFLICT" => {
            json!({"step":"RESOLVE_CONFLICT_BEFORE_ANY_MERGE","note":"the source states different valid codes; find the manufacturer or an independent source that names this product, and do not choose between them"})
        }
        "VALID_GTIN_STATED" => {
            let code = identity_key.split(':').nth(1).unwrap_or("");
            json!({"step":"CONFIRM_CODE_IN_A_SECOND_SOURCE","official_lookup":crate::price_observations::catalog_action(code.trim_start_matches('0')),"note":"a code stated by one source is a source assertion until a second source or an official lookup lists the same code"})
        }
        "INVALID_IDENTIFIER_STATED_NOT_USED" => {
            json!({"step":"RECHECK_SOURCE_CODE_OR_FIND_ANOTHER_LISTING","note":"the stated code fails its check digit or format and was not used"})
        }
        _ if !text(&product["mpn"]).is_empty() => {
            json!({"step":"FIND_MANUFACTURER_OR_SECOND_SOURCE_FOR_MPN","note":"an MPN needs its brand and a second source before it can identify the product"})
        }
        _ => {
            json!({"step":"FIND_LISTING_WITH_A_PRODUCT_CODE","note":"no stable identifier stated"})
        }
    }
}
fn key(c: &Value) -> (String, &'static str) {
    let p = &c["product"];
    let variant = format!("{}|{}", text(&p["color"]), text(&p["size"]));
    let codes = valid_codes(p);
    if codes.len() == 1 {
        return (
            format!("gtin:{}:{variant}", codes.first().unwrap()),
            "CHECKSUM_VALID_GTIN_AND_VARIANT",
        );
    }
    if codes.len() > 1 {
        // Two different valid codes on one product are a conflict between claims. Neither is
        // chosen, nothing merges on either, and the record stays its own entity.
        return (
            format!(
                "gtin_conflict:{}:{}:{variant}",
                codes.iter().cloned().collect::<Vec<_>>().join("+"),
                c["source"].as_str().unwrap_or("")
            ),
            "CONFLICTING_GTIN_CLAIMS",
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
            && ["gtin", "ean", "upc", "gtin8", "gtin14", "isbn"]
                .iter()
                .all(|k| {
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
        let claims = identifier_claims(&primary["product"]);
        let identifier_state = if basis == "CONFLICTING_GTIN_CLAIMS" {
            let codes: Vec<_> = claims
                .iter()
                .filter_map(|c| c["gtin14_normalized"].as_str())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            conflicts.insert(
                "gtin14".into(),
                json!({"status":"CONFLICT","codes":codes,"claims":claims}),
            );
            "IDENTIFIER_CONFLICT"
        } else if basis == "CHECKSUM_VALID_GTIN_AND_VARIANT" {
            "VALID_GTIN_STATED"
        } else if claims
            .iter()
            .any(|c| !c["state"].as_str().unwrap_or("").starts_with("VALID"))
        {
            "INVALID_IDENTIFIER_STATED_NOT_USED"
        } else {
            "NO_GTIN_STATED"
        };
        let identity_next_step =
            identity_next_step(identifier_state, &identity, &primary["product"]);
        primary["evidence_ids"] = json!(evidence);
        primary["entity_id"] = json!(entity_id);
        primary["resolution"] = json!({"status":"DERIVED_IDENTITY_FROM_SOURCE_ASSERTIONS","basis":basis,"identity_key":identity,"identifier_state":identifier_state,"identity_next_step":identity_next_step,"identifier_claims":claims,"source_pages":sources,"observation_count":rows.len(),"observations":rows,"conflicts":conflicts,"independently_verified_identity":false});
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
    fn identifier_matrix_normalizes_by_standard_and_never_repairs() {
        // Valid forms of one product, written four ways, share a GTIN-14.
        let ean = "4006381333931";
        for (stated, normalized) in [
            (ean, "04006381333931"),
            ("04006381333931", "04006381333931"),
            ("036000291452", "00036000291452"),
            ("0036000291452", "00036000291452"),
            ("73513537", "00000073513537"),
            ("9780306406157", "09780306406157"),
        ] {
            assert_eq!(
                gtin(&json!(stated)).as_deref(),
                Some(normalized),
                "{stated}"
            );
        }
        // Not repaired: bad check digit, bad length, separators, letters, numbers, all-zero.
        for bad in [
            "4006381333932",
            "400638133393",
            "40063813339311",
            "4006381-333931",
            " 4006381333931",
            "4006381333931 ",
            "40063813339A1",
            "",
            "0000000000000",
            "00000000",
            "000000000000",
            "00000000000000",
        ] {
            assert_eq!(gtin(&json!(bad)), None, "{bad:?}");
        }
        assert_eq!(
            gtin(&json!(4006381333931_u64)),
            None,
            "a JSON number is not a stated string"
        );
        // ISBN-13 is an EAN-13 and is written with hyphens; ISBN-10 and bad ones are not GTINs.
        assert_eq!(
            isbn13(&json!("978-3-473-48878-0")).as_deref(),
            Some("09783473488780")
        );
        assert_eq!(
            isbn13(&json!("978 0 306 40615 7")).as_deref(),
            Some("09780306406157")
        );
        // A valid ISBN-10 converts by the ISO 2108 rule (live: exlibris.ch, whose URL carries the
        // ISBN-13 of the ISBN-10 it states).
        assert_eq!(
            isbn13(&json!("0306406152")).as_deref(),
            Some("09780306406157")
        );
        assert_eq!(
            isbn13(&json!("3387700237")).as_deref(),
            Some("09783387700237")
        );
        assert_eq!(
            isbn13(&json!("0-8044-2957-X")).as_deref(),
            Some("09780804429573")
        );
        for bad in [
            "0306406153",
            "030640615X",
            "978-0-306-40615-8",
            "4006381333931",
            "9770306406157",
            "",
        ] {
            assert_eq!(isbn13(&json!(bad)), None, "{bad}");
        }
        let claims = identifier_claims(
            &json!({"ean":"4006381333932","upc":"036000291452","gtin":"abc","isbn":"0306406153","gtin14":"  "}),
        );
        let state = |f: &str| claims.iter().find(|c| c["field"] == f).unwrap()["state"].clone();
        assert_eq!(state("ean"), "INVALID_CHECK_DIGIT");
        assert_eq!(state("upc"), "VALID_GTIN");
        assert_eq!(state("gtin"), "MALFORMED");
        assert_eq!(state("isbn"), "INVALID_ISBN10");
        assert!(
            claims.iter().all(|c| c["field"] != "gtin14"),
            "a blank field is not a claim"
        );
    }
    #[test]
    fn conflicting_valid_codes_stay_a_visible_conflict_and_never_merge() {
        let mut both = candidate("both", "one.example", "C1", "");
        both["product"]["ean"] = json!("4006381333931");
        both["product"]["upc"] = json!("036000291452");
        let mut plain = candidate("plain", "one.example", "C1", "");
        plain["product"]["ean"] = json!("4006381333931");
        let (rows, _) = resolve(vec![both, plain]);
        assert_eq!(
            rows.len(),
            2,
            "a conflicted record is not merged into either code"
        );
        let conflicted = rows
            .iter()
            .find(|r| r["resolution"]["basis"] == "CONFLICTING_GTIN_CLAIMS")
            .unwrap();
        assert_eq!(
            conflicted["resolution"]["identifier_state"],
            "IDENTIFIER_CONFLICT"
        );
        assert_eq!(
            conflicted["resolution"]["conflicts"]["gtin14"]["codes"],
            json!(["00036000291452", "04006381333931"])
        );
        let other = rows
            .iter()
            .find(|r| r["resolution"]["basis"] == "CHECKSUM_VALID_GTIN_AND_VARIANT")
            .unwrap();
        assert_eq!(other["resolution"]["identifier_state"], "VALID_GTIN_STATED");
        assert_eq!(
            other["resolution"]["identity_next_step"]["step"],
            "CONFIRM_CODE_IN_A_SECOND_SOURCE"
        );
        assert_eq!(
            conflicted["resolution"]["identity_next_step"]["step"],
            "RESOLVE_CONFLICT_BEFORE_ANY_MERGE"
        );
        assert_eq!(
            other["state"], conflicted["state"],
            "identity uncertainty never changes the candidate state"
        );
        // The same code in ISBN form and EAN form is one identity, not a conflict.
        let mut book = candidate("book", "two.example", "B1", "");
        book["product"]["isbn"] = json!("978-3-473-48878-0");
        book["product"]["ean"] = json!("9783473488780");
        let (rows, _) = resolve(vec![book]);
        assert_eq!(
            rows[0]["resolution"]["identity_key"],
            "gtin:09783473488780:|"
        );
        // An invalid code is reported and not used.
        let mut bad = candidate("bad", "three.example", "D1", "");
        bad["product"]["ean"] = json!("4006381333932");
        let (rows, _) = resolve(vec![bad]);
        assert_eq!(
            rows[0]["resolution"]["identifier_state"],
            "INVALID_IDENTIFIER_STATED_NOT_USED"
        );
        assert_eq!(
            rows[0]["resolution"]["basis"],
            "ORIGIN_SCOPED_SKU_AND_VARIANT"
        );
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
