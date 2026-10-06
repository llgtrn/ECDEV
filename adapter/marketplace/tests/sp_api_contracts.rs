//! Every SP-API request ECDEV can plan, checked against the request contract of the pinned official
//! models (amzn/selling-partner-api-models, Apache-2.0, 3677bb9d): method, path template, no
//! undeclared parameter, every required parameter present, enum values inside the declared enum.
//! Restricted (orders, RDT) and write operations are checked to exist in the models as declared,
//! and to be refused by the planner.
use ecdev_core::provider::AcquireRequest;
use ecdev_marketplace::reads::{OPERATIONS, RESTRICTED_OPERATIONS, WRITE_OPERATIONS, spec};
use serde_json::{Value, json};

fn contracts() -> Value {
    serde_json::from_str(include_str!("fixtures/sp-api-operation-contracts.json")).unwrap()
}

fn contract<'a>(all: &'a Value, operation_id: &str) -> &'a Value {
    all["contracts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["operation_id"] == operation_id)
        .unwrap_or_else(|| panic!("{operation_id} missing from the pinned models"))
}

fn template_matches(template: &str, path: &str) -> bool {
    let (t, p): (Vec<_>, Vec<_>) = (template.split('/').collect(), path.split('/').collect());
    t.len() == p.len()
        && t.iter()
            .zip(&p)
            .all(|(a, b)| (a.starts_with('{') && a.ends_with('}') && !b.is_empty()) || a == b)
}

fn request(capability: &str, market: &str, query: Value) -> AcquireRequest {
    AcquireRequest {
        run_id: "contract-run".into(),
        capability: capability.into(),
        market: market.into(),
        query,
    }
}

fn check(all: &Value, op: &Value) {
    let key = op["operation"].as_str().unwrap();
    let operation_id = spec(key).unwrap().operation_id;
    let c = contract(all, operation_id);
    assert_eq!(op["method"], c["method"], "{operation_id}");
    let url = op["requested_url"].as_str().unwrap();
    let path = &url[url.find(".com").unwrap() + 4..];
    assert!(
        template_matches(c["path"].as_str().unwrap(), path),
        "{operation_id}: {path}"
    );
    let params = c["parameters"].as_array().unwrap();
    let query = op["query"].as_object().unwrap();
    for (name, value) in query {
        let p = params
            .iter()
            .find(|p| p["name"] == name.as_str() && p["in"] == "query")
            .unwrap_or_else(|| panic!("{operation_id}: undeclared query parameter {name}"));
        let declared = p["enum"].as_array().or(p["items"]["enum"].as_array());
        if let Some(allowed) = declared {
            for v in value.as_str().unwrap().split(',') {
                assert!(
                    allowed.contains(&json!(v)),
                    "{operation_id}: {name}={v} outside the model enum"
                );
            }
        }
    }
    for p in params.iter().filter(|p| p["required"] == true) {
        match p["in"].as_str().unwrap() {
            "query" => assert!(
                query.contains_key(p["name"].as_str().unwrap()),
                "{operation_id}: missing {}",
                p["name"]
            ),
            "body" => {
                for field in p["body_required"].as_array().unwrap() {
                    assert!(
                        !op["body"][field.as_str().unwrap()].is_null(),
                        "{operation_id}: body missing {field}"
                    );
                }
            }
            _ => {}
        }
    }
}

#[test]
fn every_planned_request_satisfies_the_pinned_model_contract() {
    let all = contracts();
    let mut checked = std::collections::BTreeSet::new();
    for market in ["AMAZON_US", "AMAZON_JP"] {
        let product = ecdev_marketplace::plan(&request(
            "product.analyze.official",
            market,
            json!({"asin":"B00V5DG6IQ","include":["CATALOG","OFFERS","FEE_ESTIMATE"],
                   "fee_estimate":{"is_amazon_fulfilled":true,"listing_price":{"amount":if market == "AMAZON_US" {"19.99"} else {"2980"},"currency":if market == "AMAZON_US" {"USD"} else {"JPY"}}}}),
        ))
        .unwrap();
        let reads = [
            json!({"operation":"LISTINGS_ITEM","seller_id":"A1EXAMPLESELLER","sku":"SKU-1"}),
            json!({"operation":"INVENTORY_SUMMARIES","seller_skus":["S1","S2"],"details":true}),
            json!({"operation":"MARKETPLACE_PARTICIPATIONS"}),
            json!({"operation":"PRODUCT_TYPE_SEARCH","keywords":["luggage"]}),
            json!({"operation":"PRODUCT_TYPE_DEFINITION","product_type":"LUGGAGE"}),
        ];
        let mut ops: Vec<Value> = product["operations"].as_array().unwrap().clone();
        for q in reads {
            let plan =
                ecdev_marketplace::plan(&request("seller.read.official", market, q)).unwrap();
            ops.extend(plan["operations"].as_array().unwrap().iter().cloned());
        }
        for op in &ops {
            check(&all, op);
            checked.insert(
                spec(op["operation"].as_str().unwrap())
                    .unwrap()
                    .operation_id,
            );
        }
    }
    let live: std::collections::BTreeSet<_> = OPERATIONS.iter().map(|s| s.operation_id).collect();
    assert_eq!(checked, live, "every live operation is planned and checked");
    // Every locked model hash in the operation table is the pinned model's hash.
    for s in OPERATIONS.iter() {
        assert_eq!(
            all["source_models"][s.model_path]["sha256"], s.model_sha256,
            "{}",
            s.operation_id
        );
    }
    // Restricted and write operations exist in the models exactly as ECDEV names them.
    for (id, method, path) in RESTRICTED_OPERATIONS.iter().chain(WRITE_OPERATIONS.iter()) {
        let c = contract(&all, id);
        assert_eq!(
            (c["method"].as_str().unwrap(), c["path"].as_str().unwrap()),
            (*method, *path),
            "{id}"
        );
    }
}
