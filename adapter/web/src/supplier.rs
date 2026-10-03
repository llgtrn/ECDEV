//! Public supplier leads, not verified quotations or factory audits.
use scraper::{Html, Selector};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use url::Url;

fn field(value: Value, evidence: Vec<Value>, status: &str) -> Value {
    json!({"value":value,"status":if value.is_null(){"UNKNOWN"}else{status},"evidence":evidence})
}
pub fn extract(doc: &Html, base: &Url, structured: &[Value], hash: &str) -> Vec<Value> {
    let mut claims = vec![];
    let mut seen = BTreeSet::new();
    // Record explicit source language; a keyword is a lead, not proof of factory status.
    for (ordinal, node) in doc
        .select(&Selector::parse("p,li,dd,td,h1,h2,h3").unwrap())
        .enumerate()
    {
        let text = node
            .text()
            .collect::<Vec<_>>()
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if text.len() > 4000 || !seen.insert(text.clone()) {
            continue;
        }
        let lower = text.to_lowercase();
        for (kind, words) in [
            ("WHOLESALE", vec!["wholesale", "卸売", "新規取引"]),
            (
                "MANUFACTURING",
                vec!["manufacturer", "manufacturing", "製造"],
            ),
            (
                "DISTRIBUTION",
                vec!["distributor", "distribution", "販売代理"],
            ),
            (
                "CUSTOMIZATION",
                vec![
                    "customization",
                    "customisation",
                    "oem",
                    "特注",
                    "オリジナリティ",
                ],
            ),
        ] {
            if words.iter().any(|w| lower.contains(w)) {
                claims.push(json!({"kind":kind,"status":"OBSERVED","value":text,"source":"VISIBLE_HTML_TEXT","page":base.as_str(),"selector":"p,li,dd,td,h1,h2,h3","matched_element_ordinal":ordinal,"raw_capture_sha256":hash,"interpretation":"SOURCE_ASSERTION_NOT_INDEPENDENT_VERIFICATION"}));
            }
        }
        if claims.len() >= 100 {
            break;
        }
    }
    if claims.is_empty() {
        return vec![];
    }
    let title = doc
        .select(&Selector::parse("title").unwrap())
        .next()
        .map(|n| n.text().collect::<String>())
        .unwrap_or_default();
    let mut organizations = vec![];
    for script in structured {
        let mut found = vec![];
        crate::supplier_terms::organizations(&script["value"], "", &mut found);
        organizations.extend(
            found
                .into_iter()
                .map(|(v, p)| (v, p, script["script_index"].as_u64().unwrap_or(0))),
        );
    }
    let organization = (organizations.len() == 1).then(|| organizations[0].clone());
    let mut fields = serde_json::Map::new();
    if let Some((org, pointer, script)) = &organization {
        for (name, key) in [
            ("name", "name"),
            ("organization_country", "address/addressCountry"),
        ] {
            let value = org
                .pointer(&format!("/{key}"))
                .cloned()
                .unwrap_or(Value::Null);
            let evidence = if value.is_null() {
                vec![]
            } else {
                vec![
                    json!({"value":value,"source":"JSON_LD","page":base.as_str(),"script_index":script,"json_pointer":format!("{pointer}/{key}"),"raw_capture_sha256":hash}),
                ]
            };
            fields.insert(name.into(), field(value, evidence, "OBSERVED"));
        }
    } else {
        fields.insert("name".into(),field(json!(title),vec![json!({"source":"HTML_TITLE","page":base.as_str(),"selector":"title","raw_capture_sha256":hash,"interpretation":"PAGE_LABEL_NOT_VERIFIED_LEGAL_ENTITY"})],"DERIVED"));
    }
    for (name, kind) in [
        ("supplier", "WHOLESALE"),
        ("manufacturer", "MANUFACTURING"),
        ("distributor", "DISTRIBUTION"),
        ("customization", "CUSTOMIZATION"),
    ] {
        let evidence: Vec<_> = claims
            .iter()
            .filter(|c| c["kind"] == kind)
            .cloned()
            .collect();
        fields.insert(
            name.into(),
            field(
                if evidence.is_empty() {
                    Value::Null
                } else {
                    json!("SOURCE_CLAIM_PRESENT")
                },
                evidence,
                "OBSERVED",
            ),
        );
    }
    for name in [
        "offer",
        "moq",
        "unit_price",
        "currency",
        "lead_time",
        "origin",
        "shipping_term",
        "certification_claim",
        "factory_capacity",
    ] {
        fields
            .entry(name)
            .or_insert_with(|| field(Value::Null, vec![], "UNKNOWN"));
    }
    let links: BTreeSet<_> = doc
        .select(&Selector::parse("a[href]").unwrap())
        .filter_map(|n| n.value().attr("href"))
        .filter_map(|h| base.join(h).ok())
        .filter(|u| {
            u.host_str() == base.host_str()
                && matches!(u.scheme(), "http" | "https")
                && u.username().is_empty()
                && u.password().is_none()
        })
        .filter(|u| {
            [
                "contact",
                "wholesale",
                "partner",
                "collaboration",
                "about",
                "company",
                "oem",
            ]
            .iter()
            .any(|part| u.path().to_lowercase().contains(part))
        })
        .map(|mut u| {
            u.set_fragment(None);
            u.to_string()
        })
        .collect();
    vec![
        json!({"kind":"PUBLIC_SUPPLIER_LEAD","source":base.as_str(),"status":"OBSERVED_SOURCE_CLAIMS","fields":fields,"claims":claims,"follow_up_urls":links,"raw_capture_sha256":hash,"qualification":"UNVERIFIED","limitation":"Public statements identify a research lead, not a negotiated offer, proven factory or authorization to sell. Retail prices are never supplier unit prices."}),
    ]
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_claims_never_become_wholesale_prices_or_factory_facts() {
        let doc = Html::parse_document(
            "<title>Example contact</title><p>Wholesale enquiries welcome. OEM customization on request.</p><a href='/contact'>Contact</a><p>Retail price JPY 4000</p>",
        );
        let base = Url::parse("https://supplier.example/about").unwrap();
        let leads = extract(&doc, &base, &[], "fixture-hash");
        assert_eq!(leads.len(), 1);
        assert_eq!(leads[0]["fields"]["supplier"]["status"], "OBSERVED");
        assert_eq!(leads[0]["fields"]["customization"]["status"], "OBSERVED");
        for k in [
            "unit_price",
            "moq",
            "manufacturer",
            "certification_claim",
            "origin",
        ] {
            assert_eq!(leads[0]["fields"][k]["status"], "UNKNOWN");
        }
        assert_eq!(
            leads[0]["follow_up_urls"][0],
            "https://supplier.example/contact"
        );
        assert!(
            extract(
                &Html::parse_document("<p>A cup for your coffee</p>"),
                &base,
                &[],
                "fixture"
            )
            .is_empty()
        );
    }
}
