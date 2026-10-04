//! Observed sample projections; these do not estimate the size or demand of a market.
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// Cache events count logical acquisitions, including failed attempts, rather than wire requests.
/// Empty or incompletely classified ledgers cannot establish a hit ratio.
pub fn cache_metrics(calls: &[Value], mode: &str) -> Value {
    let hits = calls.iter().filter(|c| c["cache_hit"] == true).count();
    let misses = calls.iter().filter(|c| c["cache_hit"] == false).count();
    let unknown = calls.len() - hits - misses;
    let ratio = (!calls.is_empty() && unknown == 0).then(|| hits * 10000 / calls.len());
    json!({"scope":"THIS_RUN_LOGICAL_ACQUISITIONS_INCLUDING_FAILED_ATTEMPTS","evidence_mode":mode,"hit_count":hits,"miss_count":misses,"unknown_count":unknown,"acquisition_denominator":calls.len(),"hit_ratio_bps":ratio,"interpretation":"Cache reuse is not new live IO or research completeness; wire requests are accounted separately."})
}

const PRODUCT_FIELDS: &[&str] = &[
    "title",
    "brand",
    "sku",
    "gtin",
    "ean",
    "upc",
    "mpn",
    "price_minor",
    "currency",
    "original_price",
    "discount",
    "availability",
    "seller",
    "shipping_text",
    "rating",
    "review_count",
    "images",
    "variants",
    "breadcrumbs",
    "category",
    "description",
    "specifications",
    "canonical_url",
    "weight_g",
];

/// Field presence is an explicit data-completeness heuristic, not consumer quality or accuracy.
fn listing_observation(row: &Value) -> Value {
    let product = &row["product"];
    let mut states: BTreeMap<&str, usize> = [
        ("OBSERVED", 0),
        ("DERIVED", 0),
        ("ESTIMATED", 0),
        ("UNKNOWN", 0),
        ("CONFLICT", 0),
    ]
    .into();
    let mut supported = vec![];
    let mut unsupported = vec![];
    for name in PRODUCT_FIELDS {
        let field = &product["fields"][*name];
        let state = field["status"]
            .as_str()
            .filter(|s| states.contains_key(s))
            .unwrap_or("UNKNOWN");
        *states.get_mut(state).unwrap() += 1;
        if matches!(state, "OBSERVED" | "DERIVED") && !field["value"].is_null() {
            let has_locator = field["evidence"].as_array().is_some_and(|a| {
                a.iter().any(|e| {
                    e["page"] == row["source"]
                        && e["raw_capture_sha256"].as_str().is_some_and(|s| {
                            s.len() == 64
                                && s.bytes()
                                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                        })
                        && (e["json_pointer"].is_string() || e["selector"].is_string())
                })
            });
            if has_locator {
                supported.push(*name);
            } else {
                unsupported.push(*name);
            }
        }
    }
    let fields = [
        "availability",
        "rating",
        "review_count",
        "shipping_text",
        "variants",
        "images",
        "description",
        "specifications",
        "title",
    ]
    .iter()
    .map(|name| {
        let field = &product["fields"][*name];
        (
            (*name).to_string(),
            if field.is_object() {
                field.clone()
            } else {
                json!({"value":null,"status":"UNKNOWN","evidence":[]})
            },
        )
    })
    .collect::<serde_json::Map<_, _>>();
    let variants = &product["fields"]["variants"];
    let count = if supported.contains(&"variants") {
        variants["value"]
            .as_array()
            .map(Vec::len)
            .or_else(|| variants["value"].is_object().then_some(1))
    } else {
        None
    };
    fn declared_depth(value: &Value) -> usize {
        match value {
            Value::Array(a) => a.iter().map(declared_depth).max().unwrap_or(0),
            Value::Object(o) => 1 + o.get("hasVariant").map(declared_depth).unwrap_or(0),
            _ => 0,
        }
    }
    let depth = count.map(|_| declared_depth(&variants["value"]));
    let substantive = |name: &str| {
        supported.contains(&name)
            && match &product["fields"][name]["value"] {
                Value::String(s) => !s.trim().is_empty(),
                Value::Array(a) => !a.is_empty(),
                Value::Object(o) => !o.is_empty(),
                Value::Null => false,
                _ => true,
            }
    };
    json!({"page":row["source"],"evidence_ids":row["evidence_ids"],"fields":fields,
        "listing_completeness":{"status":"DERIVED_FIELD_PRESENCE_HEURISTIC","supported_fields":supported,"supported_field_count":supported.len(),"required_fields":PRODUCT_FIELDS,"field_denominator":PRODUCT_FIELDS.len(),"supported_field_coverage_bps":supported.len()*10000/PRODUCT_FIELDS.len(),"field_evidence_states":states,"unsupported_known_fields":unsupported,"evidence_validation":"Source locators and hash spelling checked; raw-capture integrity requires evidence.inspect or the live proof validator."},
        "variation_depth":{"status":if count.is_some(){"DERIVED_OBSERVED_DECLARED_MEMBERS"}else{"UNKNOWN"},"declared_member_count":count,"embedded_structure_depth":depth,"evidence":variants["evidence"],"limitation":"Captured hasVariant structure only; references are not dereferenced. Distinct selectable variants and total marketplace variation counts remain unverified."},
        "product_page_quality":{"status":"HEURISTIC_METADATA_SUPPORT_ONLY","title":substantive("title"),"images":substantive("images"),"description":substantive("description"),"specifications":substantive("specifications"),"limitation":"These flags measure supported nonempty metadata, not visual quality, content accuracy, accessibility or conversion performance."}})
}

pub fn enrich(candidates: &mut [Value], snapshots: &[Value]) -> Result<Value, String> {
    let mut brands: BTreeMap<String, usize> = BTreeMap::new();
    let origins: BTreeSet<_> = snapshots
        .iter()
        .filter_map(|s| s["source"].as_str())
        .filter_map(|s| url::Url::parse(s).ok())
        .map(|u| u.origin().ascii_serialization())
        .collect();
    for c in candidates.iter() {
        let b = &c["product"]["brand"];
        let name = if b.is_object() {
            b["name"].as_str()
        } else {
            b.as_str()
        };
        if let Some(name) = name.map(str::trim).filter(|name| !name.is_empty()) {
            *brands.entry(name.to_string()).or_default() += 1;
        }
    }
    for c in candidates.iter_mut() {
        let rows = c["resolution"]["observations"]
            .as_array()
            .cloned()
            .unwrap_or_else(|| vec![c.clone()]);
        let pages: BTreeSet<_> = rows
            .iter()
            .filter_map(|r| r["source"].as_str())
            .map(str::to_string)
            .collect();
        let listing_origins: BTreeSet<_> = pages
            .iter()
            .filter_map(|page| url::Url::parse(page).ok())
            .map(|u| u.origin().ascii_serialization())
            .collect();
        let mut sellers = BTreeSet::new();
        let mut prices: BTreeMap<String, BTreeSet<i64>> = BTreeMap::new();
        let mut offers = vec![];
        for row in &rows {
            for offer in row["product"]["observed_offers"]
                .as_array()
                .into_iter()
                .flatten()
            {
                if !offer["seller"].is_null() {
                    sellers.insert(offer["seller"].to_string());
                }
                if let (Some(currency), Some(price)) =
                    (offer["currency"].as_str(), offer["price_minor"].as_i64())
                {
                    prices
                        .entry(currency.to_string())
                        .or_default()
                        .insert(price);
                }
                offers.push(
                    json!({"offer":offer,"page":row["source"],"evidence_ids":row["evidence_ids"]}),
                );
            }
        }
        let ranges:Vec<_>=prices.iter().map(|(currency,values)|json!({"currency":currency,"min_minor":values.first(),"max_minor":values.last(),"status":"OBSERVED_IN_CAPTURED_OFFERS"})).collect();
        let category_pages:Vec<_>=snapshots.iter().filter(|s|matches!(s["page_metadata"]["classification"]["role"].as_str(),Some("CATEGORY"|"PAGINATION"|"SEARCH_RESULT"))).filter(|s|s["links"].as_array().is_some_and(|links|links.iter().any(|l|l.as_str().is_some_and(|u|pages.contains(u))))).map(|s|json!({"page":s["source"],"role":s["page_metadata"]["classification"]["role"],"raw_capture_sha256":s["content_hash"]})).collect();
        c["competition_evidence"] = json!({"status":"OBSERVED_SAMPLE_ONLY","listing_pages_observed":pages.len(),"source_pages":pages,"listing_origins_observed":listing_origins,"publisher_independence":"UNVERIFIED","seller_assertions_observed":sellers.len(),"seller_assertions":sellers,"observed_offer_price_ranges":ranges,"offers":offers,"rating":c["product"]["rating"],"review_count":c["product"]["review_count"],"listing_observations":rows.iter().map(listing_observation).collect::<Vec<_>>(),"limitation":"These are captured listings/offers, not a total competitor count. A single manufacturer catalog does not establish competitive intensity."});
        c["demand_evidence"] = json!({"status":"PROXY","kind":"PUBLIC_LINK_AND_MATCHED_PRODUCT_LISTING_PRESENCE","observed_surfaces":category_pages,"observed_listing_origins":listing_origins,"listing_origin_evidence":rows.iter().map(|r|json!({"page":r["source"],"evidence_ids":r["evidence_ids"]})).collect::<Vec<_>>(),"publisher_independence":"UNVERIFIED","search_volume":null,"true_sales":null,"review_velocity":null,"limitation":"Link presence is a visibility proxy; no volume, sales, trend or velocity inference."});
        c["economics_uncertainty"] = crate::uncertainty::observed_candidate(&c["product"])?;
    }
    let known_brands = brands.values().sum::<usize>();
    let brand_shares: Vec<_> = brands.iter().map(|(label,count)| json!({"label":label,"candidate_count":count,"all_candidate_share_bps":(count*10000).checked_div(candidates.len()),"known_label_share_bps":(count*10000).checked_div(known_brands)})).collect();
    Ok(
        json!({"scope":"RESOLVED_CANDIDATES_IN_THIS_CAPTURED_RUN","candidate_count":candidates.len(),"origins_observed":origins,"brand_counts":brands,"brand_count_denominator":candidates.len(),"brand_concentration":{"status":"DERIVED_ASSERTED_LABEL_FREQUENCY_IN_CAPTURED_SAMPLE","known_brand_candidate_count":known_brands,"unknown_brand_candidate_count":candidates.len()-known_brands,"labels":brand_shares,"limitation":"Exact published labels after trimming; brand ownership and aliases are unverified. These sample proportions are not total market share."},"status":"OBSERVED_SAMPLE_ONLY","independent_market_coverage":"UNVERIFIED","total_market_competitors":null,"total_market_demand":null}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cache_ratios_keep_empty_unknown_and_fixture_ledgers_distinct() {
        let calls = vec![
            json!({"cache_hit":true,"request_count":0}),
            json!({"cache_hit":false,"status":"FAILED","request_count":2}),
            json!({"cache_hit":false,"request_count":7}),
        ];
        let metric = cache_metrics(&calls, "LIVE");
        assert_eq!(metric["hit_count"], 1);
        assert_eq!(metric["miss_count"], 2);
        assert_eq!(metric["acquisition_denominator"], 3);
        assert_eq!(metric["hit_ratio_bps"], 3333);
        assert!(cache_metrics(&[], "CACHED")["hit_ratio_bps"].is_null());
        assert!(
            cache_metrics(&[json!({"cache_hit":true}), json!({})], "LIVE")["hit_ratio_bps"]
                .is_null()
        );
        assert_eq!(cache_metrics(&calls, "FIXTURE")["evidence_mode"], "FIXTURE");
    }
    #[test]
    fn per_listing_quality_preserves_claims_and_missing_variations() {
        let hash = "a".repeat(64);
        let field = |value: Value, name: &str, page: &str| json!({"value":value,"status":"OBSERVED","evidence":[{"source":"JSON_LD","page":page,"json_pointer":format!("/{name}"),"raw_capture_sha256":hash}]});
        let a = "https://manufacturer.example/cup";
        let b = "https://retailer.example/cup";
        let row = |page: &str| json!({"source":page,"evidence_ids":[page],"product":{"fields":{"title":field(json!("Cup"),"name",page),"availability":field(json!(if page == a {"OutOfStock"}else{"InStock"}),"availability",page),"shipping_text":field(json!({"shippingDestination":{"addressCountry":"JP"}}),"shippingDetails",page),"images":field(json!([]),"image",page),"rating":{"value":5,"status":"OBSERVED","evidence":[]}}}});
        let mut first = row(a);
        first["product"]["fields"]["variants"] = field(
            json!([{"hasVariant":[{"sku":"red"},{"sku":"blue"}]},{"@id":"/unresolved"}]),
            "hasVariant",
            a,
        );
        let second = row(b);
        let mut candidates =
            vec![json!({"product":{"rating":null},"resolution":{"observations":[first,second]}})];
        enrich(&mut candidates, &[]).unwrap();
        let listings = &candidates[0]["competition_evidence"]["listing_observations"];
        assert_eq!(listings[0]["fields"]["availability"]["value"], "OutOfStock");
        assert_eq!(listings[1]["fields"]["availability"]["value"], "InStock");
        assert_eq!(
            listings[0]["fields"]["shipping_text"]["evidence"][0]["raw_capture_sha256"],
            hash
        );
        assert_eq!(listings[0]["variation_depth"]["declared_member_count"], 2);
        assert_eq!(
            listings[0]["variation_depth"]["embedded_structure_depth"],
            2
        );
        assert!(listings[1]["variation_depth"]["declared_member_count"].is_null());
        assert_eq!(listings[1]["variation_depth"]["status"], "UNKNOWN");
        assert_eq!(listings[0]["listing_completeness"]["field_denominator"], 24);
        assert_eq!(
            listings[0]["listing_completeness"]["supported_field_count"],
            5
        );
        assert_eq!(
            listings[0]["listing_completeness"]["unsupported_known_fields"][0],
            "rating"
        );
        assert_eq!(listings[0]["product_page_quality"]["images"], false);
        assert!(candidates[0]["demand_evidence"]["true_sales"].is_null());
        assert!(candidates[0]["economics_uncertainty"]["profit_expected"].is_null());
        let missing = listing_observation(&json!({"source":a,"product":{}}));
        assert_eq!(missing["listing_completeness"]["supported_field_count"], 0);
        assert!(missing["variation_depth"]["embedded_structure_depth"].is_null());
    }
    #[test]
    fn captured_presence_and_prices_never_become_sales_or_profit_forecasts() {
        let mut candidates = vec![
            json!({"product":{"currency":"JPY","price_minor":4000,"brand":"Hario","provenance":{"page":"https://shop.example/product/a"}},"resolution":{"observations":[{"source":"https://shop.example/product/a","product":{"observed_offers":[{"currency":"JPY","price_minor":4000,"seller":"Shop"}]},"evidence_ids":["one"]}]}}),
        ];
        let snapshots = vec![
            json!({"source":"https://shop.example/catalog","content_hash":"hash","page_metadata":{"classification":{"role":"CATEGORY"}},"links":["https://shop.example/product/a"]}),
        ];
        let summary = enrich(&mut candidates, &snapshots).unwrap();
        assert_eq!(
            candidates[0]["competition_evidence"]["listing_pages_observed"],
            1
        );
        assert_eq!(
            candidates[0]["demand_evidence"]["observed_surfaces"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(candidates[0]["demand_evidence"]["search_volume"].is_null());
        assert!(candidates[0]["economics_uncertainty"]["profit_expected"].is_null());
        assert!(summary["total_market_competitors"].is_null());
        assert_eq!(
            summary["brand_concentration"]["labels"][0]["all_candidate_share_bps"],
            10000
        );
        let mut sample = vec![
            json!({"product":{"brand":"A"}}),
            json!({"product":{"brand":"B"}}),
            json!({"product":{"brand":" "}}),
        ];
        let report = enrich(&mut sample, &[]).unwrap();
        assert_eq!(
            report["brand_concentration"]["unknown_brand_candidate_count"],
            1
        );
        assert_eq!(
            report["brand_concentration"]["labels"][0]["all_candidate_share_bps"],
            3333
        );
        assert_eq!(
            report["brand_concentration"]["labels"][0]["known_label_share_bps"],
            5000
        );
        let report = enrich(&mut [], &[]).unwrap();
        assert!(
            report["brand_concentration"]["labels"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}

/// Per-run observations are kept separate from executable-capability coverage.
pub fn completeness(
    candidates: &[Value],
    snapshots: &[Value],
    fixture: bool,
    network_calls: u64,
    supplier_count: usize,
) -> Value {
    let required = PRODUCT_FIELDS;
    let mut counts: BTreeMap<&str, usize> = [
        ("OBSERVED", 0),
        ("DERIVED", 0),
        ("ESTIMATED", 0),
        ("UNKNOWN", 0),
        ("CONFLICT", 0),
    ]
    .into();
    let mut with_evidence = 0;
    let mut known = 0;
    for candidate in candidates {
        for key in required {
            let field = &candidate["product"]["fields"][*key];
            let status = field["status"]
                .as_str()
                .filter(|s| counts.contains_key(*s))
                .unwrap_or("UNKNOWN");
            *counts.get_mut(status).unwrap() += 1;
            if matches!(status, "OBSERVED" | "DERIVED" | "ESTIMATED") && !field["value"].is_null() {
                known += 1;
                if field["evidence"].as_array().is_some_and(|a| {
                    a.iter()
                        .any(|e| e["page"].is_string() && e["raw_capture_sha256"].is_string())
                }) {
                    with_evidence += 1;
                }
            }
        }
    }
    let denominator = candidates.len() * required.len();
    let ratio = |n: usize, d: usize| (n * 10000).checked_div(d).map_or(Value::Null, |v| json!(v));
    let stages: Vec<Value> = crate::research::ZERO_COST_STAGES
        .iter()
        .map(|(name, _)| {
            let observed = match *name {
                "discovery_from_seed_links" => snapshots
                    .iter()
                    .any(|s| s["links"].as_array().is_some_and(|a| !a.is_empty())),
                "crawl" | "extraction" | "evidence" => !snapshots.is_empty(),
                "candidate_creation" | "ranking_and_filtering" => !candidates.is_empty(),
                "competitor_price_comparison" => candidates.iter().any(|c| {
                    c["competition_evidence"]["observed_offer_price_ranges"]
                        .as_array()
                        .is_some_and(|a| !a.is_empty())
                }),
                "economics_from_assumptions" => {
                    candidates.iter().any(|c| !c["economics"].is_null())
                }
                "report" => true,
                "supplier_discovery" => supplier_count > 0,
                _ => false,
            };
            json!({"stage":name,"observed_in_run":observed})
        })
        .collect();
    let observed_stages = stages
        .iter()
        .filter(|s| s["observed_in_run"] == true)
        .count();
    let live = !fixture && network_calls > 0;
    json!({"scope":"THIS_RUN_ONLY","evidence_mode":if fixture{"FIXTURE"}else if live{"LIVE_NETWORK"}else{"CACHED_NO_NEW_NETWORK"},"live_zero_paid_research_coverage_bps":if live{ratio(observed_stages,stages.len())}else{Value::Null},"fixture_zero_paid_research_coverage_bps":if fixture{ratio(observed_stages,stages.len())}else{Value::Null},"stage_denominator":stages,"live_data_completeness_bps":if live{ratio(known,denominator)}else{Value::Null},"candidate_field_observability_bps":ratio(known,denominator),"evidence_completeness_bps":ratio(with_evidence,known),"evidence_supported_known_fields":with_evidence,"known_fields":known,"field_count_denominator":denominator,"required_fields":required,"field_evidence_states":counts,"definitions":"Field observability counts known nonconflicting required product fields. Evidence completeness counts known fields with page and raw-capture hash. Stage coverage records operations evidenced in this run, not accuracy or total market coverage. Empty denominators remain unknown; live coverage requires new network IO. Cached and fixture observations never increment live coverage."})
}
#[cfg(test)]
mod completeness_tests {
    use super::*;
    #[test]
    fn fixture_and_empty_runs_cannot_claim_live_completeness() {
        let rows = vec![
            json!({"product":{"fields":{"title":{"status":"OBSERVED","value":"Cup","evidence":[{"page":"https://example.org/p","raw_capture_sha256":"fixture"}]}}}}),
        ];
        let fixture = completeness(&rows, &[], true, 0, 0);
        assert!(fixture["live_zero_paid_research_coverage_bps"].is_null());
        assert_eq!(fixture["known_fields"], 1);
        assert_eq!(fixture["field_count_denominator"], 24);
        assert_eq!(fixture["evidence_completeness_bps"], 10000);
        let empty = completeness(&[], &[], false, 1, 0);
        assert!(empty["live_data_completeness_bps"].is_null());
        assert!(completeness(&rows, &[], false, 0, 0)["live_data_completeness_bps"].is_null());
        assert_eq!(
            completeness(&rows, &[], false, 2, 0)["evidence_mode"],
            "LIVE_NETWORK"
        );
    }
}
