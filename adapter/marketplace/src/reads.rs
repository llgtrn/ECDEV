//! Seller-scoped official reads (Listings Items, FBA Inventory, Sellers, Product Type Definitions).
//! Paths, parameters and default usage plans come from the locked SP-API models; reads only,
//! no writes, no PII and no Restricted Data Token flow.
use super::*;

pub const READ_CAPABILITY: &str = "seller.read.official";

/// One official operation known to this adapter, with its locked-model source.
pub struct OperationSpec {
    pub key: &'static str,
    pub operation_id: &'static str,
    pub api: &'static str,
    pub capability: &'static str,
    pub method: &'static str,
    pub path: &'static str,
    pub model_path: &'static str,
    pub model_sha256: &'static str,
    pub default_rate_per_second: f64,
    pub default_burst: u64,
    pub min_interval_ms: u64,
    pub fixture_provenance: &'static str,
}

const PRODUCT: &str = "product.analyze.official";
const LOCKED_EXAMPLE: &str = "LOCKED_MODEL_EXAMPLE_SHA256_MATCHES_CENSUS";

/// Every live-capable operation. Order is stable and reported by `doctor`.
pub const OPERATIONS: [OperationSpec; 9] = [
    OperationSpec {
        key: "catalog",
        operation_id: "getCatalogItem",
        api: "catalogItems_2022-04-01",
        capability: PRODUCT,
        method: "GET",
        path: "/catalog/2022-04-01/items/{asin}",
        model_path: "models/catalog-items-api-model/catalogItems_2022-04-01.json",
        model_sha256: "1a029b01df1d847d3057740a6e877f89f2ab78104b5b4d8f4b839f8d00f600c2",
        default_rate_per_second: 2.0,
        default_burst: 2,
        min_interval_ms: 500,
        fixture_provenance: LOCKED_EXAMPLE,
    },
    OperationSpec {
        key: "catalog_search",
        operation_id: "searchCatalogItems",
        api: "catalogItems_2022-04-01",
        capability: READ_CAPABILITY,
        method: "GET",
        path: "/catalog/2022-04-01/items",
        model_path: "models/catalog-items-api-model/catalogItems_2022-04-01.json",
        model_sha256: "1a029b01df1d847d3057740a6e877f89f2ab78104b5b4d8f4b839f8d00f600c2",
        default_rate_per_second: 2.0,
        default_burst: 2,
        min_interval_ms: 500,
        fixture_provenance: LOCKED_EXAMPLE,
    },
    OperationSpec {
        key: "offers",
        operation_id: "getItemOffers",
        api: "productPricingV0",
        capability: PRODUCT,
        method: "GET",
        path: "/products/pricing/v0/items/{Asin}/offers",
        model_path: "models/product-pricing-api-model/productPricingV0.json",
        model_sha256: "86d65f691a5611f951a4e61df206e518ec663064ecfa8733105f2ca259d4ded6",
        default_rate_per_second: 0.5,
        default_burst: 1,
        min_interval_ms: 2000,
        fixture_provenance: LOCKED_EXAMPLE,
    },
    OperationSpec {
        key: "fees",
        operation_id: "getMyFeesEstimateForASIN",
        api: "productFeesV0",
        capability: PRODUCT,
        method: "POST",
        path: "/products/fees/v0/items/{Asin}/feesEstimate",
        model_path: "models/product-fees-api-model/productFeesV0.json",
        model_sha256: "d06ad35f909d8c0985845f21420c1f75599531465b27f46d4946f7d7f522fc35",
        default_rate_per_second: 1.0,
        default_burst: 2,
        min_interval_ms: 1000,
        fixture_provenance: LOCKED_EXAMPLE,
    },
    OperationSpec {
        key: "listings_item",
        operation_id: "getListingsItem",
        api: "listingsItems_2021-08-01",
        capability: READ_CAPABILITY,
        method: "GET",
        path: "/listings/2021-08-01/items/{sellerId}/{sku}",
        model_path: "models/listings-items-api-model/listingsItems_2021-08-01.json",
        model_sha256: "117617f4c86dbd5c1708913103806a24c0ef0bbcfb6054e415d044d07761faeb",
        default_rate_per_second: 5.0,
        default_burst: 10,
        min_interval_ms: 200,
        fixture_provenance: LOCKED_EXAMPLE,
    },
    OperationSpec {
        key: "inventory_summaries",
        operation_id: "getInventorySummaries",
        api: "fbaInventory",
        capability: READ_CAPABILITY,
        method: "GET",
        path: "/fba/inventory/v1/summaries",
        model_path: "models/fba-inventory-api-model/fbaInventory.json",
        model_sha256: "7c14bcdb22de8ca2df45e5a40f2a422cff344d45985a68b9515b2e800edcc5ab",
        default_rate_per_second: 2.0,
        default_burst: 2,
        min_interval_ms: 500,
        fixture_provenance: "SCHEMA_SHAPED_SYNTHETIC_LOCKED_MODEL_HAS_NO_200_EXAMPLE",
    },
    OperationSpec {
        key: "marketplace_participations",
        operation_id: "getMarketplaceParticipations",
        api: "sellers",
        capability: READ_CAPABILITY,
        method: "GET",
        path: "/sellers/v1/marketplaceParticipations",
        model_path: "models/sellers-api-model/sellers.json",
        model_sha256: "497862ea32de8040453649986e2cd7c6fcc15b55e8022becc783e4a6d6ffcffd",
        default_rate_per_second: 0.016,
        default_burst: 15,
        min_interval_ms: 62_500,
        fixture_provenance: LOCKED_EXAMPLE,
    },
    OperationSpec {
        key: "product_type_search",
        operation_id: "searchDefinitionsProductTypes",
        api: "definitionsProductTypes_2020-09-01",
        capability: READ_CAPABILITY,
        method: "GET",
        path: "/definitions/2020-09-01/productTypes",
        model_path: "models/product-type-definitions-api-model/definitionsProductTypes_2020-09-01.json",
        model_sha256: "9f54478fd9c145be65fd5b266d13770e4eb41f3d19f1ade5342ce64786af0aaa",
        default_rate_per_second: 5.0,
        default_burst: 10,
        min_interval_ms: 200,
        fixture_provenance: LOCKED_EXAMPLE,
    },
    OperationSpec {
        key: "product_type_definition",
        operation_id: "getDefinitionsProductType",
        api: "definitionsProductTypes_2020-09-01",
        capability: READ_CAPABILITY,
        method: "GET",
        path: "/definitions/2020-09-01/productTypes/{productType}",
        model_path: "models/product-type-definitions-api-model/definitionsProductTypes_2020-09-01.json",
        model_sha256: "9f54478fd9c145be65fd5b266d13770e4eb41f3d19f1ade5342ce64786af0aaa",
        default_rate_per_second: 5.0,
        default_burst: 10,
        min_interval_ms: 200,
        fixture_provenance: LOCKED_EXAMPLE,
    },
];

pub fn spec(key: &str) -> Option<&'static OperationSpec> {
    OPERATIONS.iter().find(|s| s.key == key)
}

/// Restricted (PII / RDT) operations. Modelled as a separate security domain: never planned.
pub const RESTRICTED_OPERATIONS: [(&str, &str, &str); 7] = [
    ("getOrders", "GET", "/orders/v0/orders"),
    ("getOrder", "GET", "/orders/v0/orders/{orderId}"),
    (
        "getOrderBuyerInfo",
        "GET",
        "/orders/v0/orders/{orderId}/buyerInfo",
    ),
    (
        "getOrderAddress",
        "GET",
        "/orders/v0/orders/{orderId}/address",
    ),
    (
        "getOrderItems",
        "GET",
        "/orders/v0/orders/{orderId}/orderItems",
    ),
    (
        "getOrderItemsBuyerInfo",
        "GET",
        "/orders/v0/orders/{orderId}/orderItems/buyerInfo",
    ),
    (
        "createRestrictedDataToken",
        "POST",
        "/tokens/2021-03-01/restrictedDataToken",
    ),
];
pub const RESTRICTED_MODEL_SOURCES: [(&str, &str); 2] = [
    (
        "models/orders-api-model/ordersV0.json",
        "027ac6f5c97126647c6925db9be09f78c7c741cd1d8727a5367374a1846bedc5",
    ),
    (
        "models/tokens-api-model/tokens_2021-03-01.json",
        "3cd09ae7f218c83f32536a894cb8c42f2191c94b9c27f6bcf0a164442089b061",
    ),
];
/// Writes are never planned, whatever credentials or gates are present.
pub const WRITE_OPERATIONS: [(&str, &str, &str); 6] = [
    (
        "putListingsItem",
        "PUT",
        "/listings/2021-08-01/items/{sellerId}/{sku}",
    ),
    (
        "patchListingsItem",
        "PATCH",
        "/listings/2021-08-01/items/{sellerId}/{sku}",
    ),
    (
        "deleteListingsItem",
        "DELETE",
        "/listings/2021-08-01/items/{sellerId}/{sku}",
    ),
    (
        "updateShipmentStatus",
        "POST",
        "/orders/v0/orders/{orderId}/shipment",
    ),
    (
        "confirmShipment",
        "POST",
        "/orders/v0/orders/{orderId}/shipmentConfirmation",
    ),
    (
        "updateVerificationStatus",
        "PATCH",
        "/orders/v0/orders/{orderId}/regulatedInfo",
    ),
];

const LISTINGS_INCLUDED_DATA: [&str; 7] = [
    "summaries",
    "attributes",
    "issues",
    "offers",
    "fulfillmentAvailability",
    "relationships",
    "productTypes",
];

fn clean(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && !s.chars().any(char::is_control)
}
fn csv_item(s: &str, max: usize) -> bool {
    clean(s, max) && !s.contains(',')
}
/// RFC 3986 unreserved characters pass; every other byte is percent-encoded for a path segment.
pub fn encode_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
fn string_list(value: &Value, max_items: usize, max_len: usize) -> Result<Vec<String>, String> {
    let items = value
        .as_array()
        .filter(|a| !a.is_empty() && a.len() <= max_items)
        .ok_or("INVALID_OFFICIAL_READ_LIST")?;
    let mut out: Vec<String> = Vec::new();
    for item in items {
        let s = item
            .as_str()
            .filter(|s| csv_item(s, max_len))
            .ok_or("INVALID_OFFICIAL_READ_LIST_ITEM")?;
        if out.iter().any(|o| o == s) {
            return Err("DUPLICATE_OFFICIAL_READ_LIST_ITEM".into());
        }
        out.push(s.into());
    }
    Ok(out)
}
fn optional_str<'a>(
    query: &'a Value,
    key: &str,
    max: usize,
    reason: &'static str,
) -> Result<Option<&'a str>, String> {
    match &query[key] {
        Value::Null => Ok(None),
        v => v
            .as_str()
            .filter(|s| clean(s, max))
            .map(Some)
            .ok_or_else(|| reason.into()),
    }
}
fn locale<'a>(query: &'a Value, key: &str) -> Result<Option<&'a str>, String> {
    let value = optional_str(query, key, 10, "INVALID_LOCALE")?;
    if value.is_some_and(|s| !s.bytes().all(|c| c.is_ascii_alphabetic() || c == b'_')) {
        return Err("INVALID_LOCALE".into());
    }
    Ok(value)
}

/// Build one read operation plan. Restricted and write operations are refused before any IO.
pub fn protocol(request: &AcquireRequest) -> Result<Value, String> {
    let (marketplace, endpoint, currency) = market(&request.market)?;
    let query = &request.query;
    let name = query["operation"]
        .as_str()
        .ok_or("OFFICIAL_READ_OPERATION_REQUIRED")?;
    let allowed: &[&str] = match name {
        "LISTINGS_ITEM" => &["seller_id", "sku", "included_data", "issue_locale"],
        "INVENTORY_SUMMARIES" => &["seller_skus", "details", "next_token"],
        "MARKETPLACE_PARTICIPATIONS" => &[],
        "CATALOG_SEARCH_BY_IDENTIFIER" => &["identifiers", "identifiers_type", "page_size"],
        "PRODUCT_TYPE_SEARCH" => &["keywords", "item_name", "locale", "search_locale"],
        "PRODUCT_TYPE_DEFINITION" => &[
            "product_type",
            "product_type_version",
            "requirements",
            "requirements_enforced",
            "locale",
        ],
        "ORDERS"
        | "ORDER"
        | "ORDER_ITEMS"
        | "ORDER_BUYER_INFO"
        | "ORDER_ADDRESS"
        | "ORDER_ITEMS_BUYER_INFO"
        | "RESTRICTED_DATA_TOKEN" => {
            return Err("RESTRICTED_DOMAIN_DISABLED".into());
        }
        "PUT_LISTINGS_ITEM"
        | "PATCH_LISTINGS_ITEM"
        | "DELETE_LISTINGS_ITEM"
        | "CONFIRM_SHIPMENT"
        | "UPDATE_SHIPMENT_STATUS" => return Err("OFFICIAL_WRITES_DISABLED".into()),
        _ => return Err("UNSUPPORTED_OFFICIAL_READ_OPERATION".into()),
    };
    if let Some(object) = query.as_object()
        && object
            .keys()
            .any(|k| k != "operation" && k != "fixture_responses" && !allowed.contains(&k.as_str()))
    {
        return Err("UNKNOWN_OFFICIAL_READ_PARAMETER".into());
    }
    let (key, path, params, context) = match name {
        "LISTINGS_ITEM" => {
            let seller = query["seller_id"]
                .as_str()
                .filter(|s| {
                    !s.is_empty() && s.len() <= 64 && s.bytes().all(|c| c.is_ascii_alphanumeric())
                })
                .ok_or("INVALID_SELLER_ID")?;
            let sku = query["sku"]
                .as_str()
                .filter(|s| clean(s, 200))
                .ok_or("INVALID_SKU")?;
            let included = if query["included_data"].is_null() {
                vec![
                    "summaries".to_string(),
                    "offers".into(),
                    "fulfillmentAvailability".into(),
                    "issues".into(),
                ]
            } else {
                string_list(&query["included_data"], 7, 40)?
            };
            if included
                .iter()
                .any(|i| !LISTINGS_INCLUDED_DATA.contains(&i.as_str()))
            {
                return Err("INVALID_LISTINGS_INCLUDED_DATA".into());
            }
            let mut params =
                json!({"marketplaceIds":marketplace,"includedData":included.join(",")});
            if let Some(l) = locale(query, "issue_locale")? {
                params["issueLocale"] = json!(l);
            }
            (
                "listings_item",
                format!(
                    "/listings/2021-08-01/items/{}/{}",
                    encode_segment(seller),
                    encode_segment(sku)
                ),
                params,
                json!({"seller_id":seller,"sku":sku,"included_data":included}),
            )
        }
        "INVENTORY_SUMMARIES" => {
            let details = match &query["details"] {
                Value::Null => false,
                v => v.as_bool().ok_or("INVALID_INVENTORY_DETAILS_FLAG")?,
            };
            let mut params = json!({"details":details.to_string(),"granularityType":"Marketplace","granularityId":marketplace,"marketplaceIds":marketplace});
            let skus = if query["seller_skus"].is_null() {
                None
            } else {
                Some(string_list(&query["seller_skus"], 50, 200)?)
            };
            if let Some(skus) = &skus {
                params["sellerSkus"] = json!(skus.join(","));
            }
            if let Some(token) =
                optional_str(query, "next_token", 4096, "INVALID_INVENTORY_NEXT_TOKEN")?
            {
                params["nextToken"] = json!(token);
            }
            (
                "inventory_summaries",
                "/fba/inventory/v1/summaries".to_string(),
                params,
                json!({"seller_skus":skus,"details":details}),
            )
        }
        "CATALOG_SEARCH_BY_IDENTIFIER" => {
            // Product codes to catalog items: JAN, EAN, GTIN and UPC must pass their checksum,
            // ASINs their shape. SKU (seller-scoped) and keyword search are not offered here.
            let kind = query["identifiers_type"]
                .as_str()
                .filter(|k| matches!(*k, "JAN" | "EAN" | "GTIN" | "UPC" | "ASIN"))
                .ok_or("INVALID_CATALOG_IDENTIFIERS_TYPE")?;
            let ids = string_list(&query["identifiers"], 20, 14)?;
            for id in &ids {
                let ok = if kind == "ASIN" {
                    id.len() == 10
                        && id
                            .bytes()
                            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
                } else {
                    ecdev_core::resolution::gtin(&json!(id)).is_some()
                };
                if !ok {
                    return Err("INVALID_CATALOG_IDENTIFIER".into());
                }
            }
            let page_size = match &query["page_size"] {
                Value::Null => 20,
                v => v
                    .as_u64()
                    .filter(|n| (1..=20).contains(n))
                    .ok_or("INVALID_CATALOG_PAGE_SIZE")?,
            };
            (
                "catalog_search",
                "/catalog/2022-04-01/items".to_string(),
                json!({"marketplaceIds":marketplace,"identifiers":ids.join(","),"identifiersType":kind,"includedData":"identifiers,summaries","pageSize":page_size}),
                json!({"identifiers":ids,"identifiers_type":kind}),
            )
        }
        "MARKETPLACE_PARTICIPATIONS" => (
            "marketplace_participations",
            "/sellers/v1/marketplaceParticipations".to_string(),
            json!({}),
            json!({}),
        ),
        "PRODUCT_TYPE_SEARCH" => {
            let mut params = json!({"marketplaceIds":marketplace});
            let keywords = if query["keywords"].is_null() {
                None
            } else {
                Some(string_list(&query["keywords"], 20, 100)?)
            };
            let item_name =
                optional_str(query, "item_name", 500, "INVALID_PRODUCT_TYPE_ITEM_NAME")?;
            match (&keywords, item_name) {
                (Some(_), Some(_)) => return Err("KEYWORDS_AND_ITEM_NAME_EXCLUSIVE".into()),
                (Some(k), None) => params["keywords"] = json!(k.join(",")),
                (None, Some(n)) => params["itemName"] = json!(n),
                (None, None) => {}
            }
            if let Some(l) = locale(query, "locale")? {
                params["locale"] = json!(l);
            }
            if let Some(l) = locale(query, "search_locale")? {
                params["searchLocale"] = json!(l);
            }
            (
                "product_type_search",
                "/definitions/2020-09-01/productTypes".to_string(),
                params,
                json!({"keywords":keywords,"item_name":item_name}),
            )
        }
        _ => {
            let product_type = query["product_type"]
                .as_str()
                .filter(|s| {
                    !s.is_empty()
                        && s.len() <= 100
                        && s.bytes()
                            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_')
                })
                .ok_or("INVALID_PRODUCT_TYPE")?;
            let requirements = query["requirements"].as_str().unwrap_or("LISTING");
            if !matches!(
                requirements,
                "LISTING" | "LISTING_PRODUCT_ONLY" | "LISTING_OFFER_ONLY"
            ) {
                return Err("INVALID_PRODUCT_TYPE_REQUIREMENTS".into());
            }
            let enforced = query["requirements_enforced"]
                .as_str()
                .unwrap_or("ENFORCED");
            if !matches!(enforced, "ENFORCED" | "NOT_ENFORCED") {
                return Err("INVALID_PRODUCT_TYPE_REQUIREMENTS_ENFORCED".into());
            }
            let locale = locale(query, "locale")?.unwrap_or("DEFAULT");
            let mut params = json!({"marketplaceIds":marketplace,"requirements":requirements,"requirementsEnforced":enforced,"locale":locale});
            let version = optional_str(
                query,
                "product_type_version",
                100,
                "INVALID_PRODUCT_TYPE_VERSION",
            )?;
            if let Some(v) = version {
                params["productTypeVersion"] = json!(v);
            }
            (
                "product_type_definition",
                format!(
                    "/definitions/2020-09-01/productTypes/{}",
                    encode_segment(product_type)
                ),
                params,
                json!({"product_type":product_type,"requirements":requirements,"requirements_enforced":enforced,"locale":locale,"product_type_version":version}),
            )
        }
    };
    let spec = spec(key).ok_or("UNSUPPORTED_OFFICIAL_READ_OPERATION")?;
    let operation = json!({"operation":key,"operation_id":spec.operation_id,"method":spec.method,"requested_url":format!("{endpoint}{path}"),"query":params,"body":null,"request_context":context,"default_rate_per_second":spec.default_rate_per_second,"default_burst":spec.default_burst,"rate_scope":"DEFAULT_OPERATION_PLAN_NOT_GUARANTEED_ACCOUNT_QUOTA","auth":"LWA_SELLER_REFRESH_TOKEN_NON_RESTRICTED","rdt":"NOT_USED_NON_PII_OPERATION","aws_sigv4_required":false,"model_source":{"path":spec.model_path,"sha256":spec.model_sha256,"commit":MODEL_COMMIT}});
    Ok(
        json!({"capability":READ_CAPABILITY,"marketplace_id":marketplace,"region_endpoint":endpoint,"currency":currency,"asin":null,"operations":[operation],"model_commit":MODEL_COMMIT}),
    )
}

/// One normalized field. Absent values remain UNKNOWN; provenance is stamped by the caller.
fn field(path: String, value: &Value, absent: &str) -> Value {
    if value.is_null() {
        json!({"field":path,"value":null,"evidence_state":"UNKNOWN","unknown_reason":absent})
    } else {
        json!({"field":path,"value":value,"evidence_state":"OBSERVED"})
    }
}
fn exact_decimal(s: &str) -> bool {
    let mut parts = s.split('.');
    let whole = parts.next().unwrap_or("");
    let fraction = parts.next();
    s.len() <= 80
        && !whole.is_empty()
        && whole.bytes().all(|c| c.is_ascii_digit())
        && fraction.is_none_or(|f| !f.is_empty() && f.bytes().all(|c| c.is_ascii_digit()))
        && parts.next().is_none()
}
fn objects<'a>(value: &'a Value, reason: &'static str) -> Result<&'a [Value], String> {
    match value {
        Value::Null => Ok(&[]),
        Value::Array(a) if a.iter().all(Value::is_object) => Ok(a.as_slice()),
        _ => Err(reason.into()),
    }
}
fn quantity(value: &Value) -> Result<(), String> {
    if value.is_null() || value.as_u64().is_some() {
        Ok(())
    } else {
        Err("INVALID_INVENTORY_QUANTITY".into())
    }
}

pub fn normalize(operation: &str, body: &Value, plan: &Value) -> Result<Value, String> {
    let marketplace = plan["marketplace_id"].as_str().ok_or("INVALID_MARKET")?;
    let currency = plan["currency"].as_str().ok_or("INVALID_MARKET")?;
    let op = plan["operations"]
        .as_array()
        .and_then(|a| a.iter().find(|o| o["operation"] == operation))
        .ok_or("OFFICIAL_OPERATION_NOT_PLANNED")?;
    let ctx = &op["request_context"];
    let mut fields = Vec::new();
    let mut ignored = 0usize;
    match operation {
        "listings_item" => {
            if body["sku"] != ctx["sku"] {
                return Err("LISTINGS_SKU_MISMATCH".into());
            }
            fields.push(field("sku".into(), &body["sku"], "NOT_RETURNED"));
            let requested = |name: &str| {
                ctx["included_data"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|v| v == name))
            };
            let summaries = objects(&body["summaries"], "INVALID_LISTINGS_SUMMARIES")?;
            for s in summaries {
                if !s["marketplaceId"].is_string()
                    || !s["productType"].is_string()
                    || !s["status"].is_array()
                    || !s["createdDate"].is_string()
                    || !s["lastUpdatedDate"].is_string()
                {
                    return Err("INVALID_LISTINGS_SUMMARY_REQUIRED_FIELDS".into());
                }
            }
            ignored += summaries
                .iter()
                .filter(|s| s["marketplaceId"] != marketplace)
                .count();
            let summary = summaries
                .iter()
                .find(|s| s["marketplaceId"] == marketplace)
                .cloned()
                .unwrap_or(Value::Null);
            let absent = if requested("summaries") {
                "NOT_RETURNED_FOR_MARKETPLACE"
            } else {
                "NOT_REQUESTED"
            };
            for key in [
                "asin",
                "productType",
                "conditionType",
                "status",
                "itemName",
                "createdDate",
                "lastUpdatedDate",
                "fnSku",
            ] {
                fields.push(field(format!("summary.{key}"), &summary[key], absent));
            }
            let offers = objects(&body["offers"], "INVALID_LISTINGS_OFFERS")?;
            let mut selected = Vec::new();
            for o in offers {
                if !o["marketplaceId"].is_string()
                    || !o["offerType"].is_string()
                    || !o["price"]["amount"].as_str().is_some_and(exact_decimal)
                    || !o["price"]["currencyCode"].is_string()
                {
                    return Err("INVALID_LISTINGS_OFFER_REQUIRED_FIELDS".into());
                }
                if o["marketplaceId"] != marketplace {
                    ignored += 1;
                    continue;
                }
                if o["price"]["currencyCode"] != currency {
                    return Err("LISTINGS_OFFER_CURRENCY_MISMATCH".into());
                }
                selected.push(json!({"offerType":o["offerType"],"price":o["price"],"points":o["points"],"audience":o["audience"]["value"]}));
            }
            let absent = if requested("offers") {
                "NOT_RETURNED_FOR_MARKETPLACE"
            } else {
                "NOT_REQUESTED"
            };
            fields.push(field(
                "offers".into(),
                &if selected.is_empty() {
                    Value::Null
                } else {
                    json!(selected)
                },
                absent,
            ));
            let availability = objects(
                &body["fulfillmentAvailability"],
                "INVALID_FULFILLMENT_AVAILABILITY",
            )?;
            if availability.iter().any(|a| {
                !a["fulfillmentChannelCode"].is_string()
                    || !(a["quantity"].is_null() || a["quantity"].is_u64())
            }) {
                return Err("INVALID_FULFILLMENT_AVAILABILITY".into());
            }
            fields.push(field(
                "fulfillmentAvailability".into(),
                &if availability.is_empty() {
                    Value::Null
                } else {
                    json!(availability)
                },
                if requested("fulfillmentAvailability") {
                    "NOT_RETURNED"
                } else {
                    "NOT_REQUESTED"
                },
            ));
            let issues = objects(&body["issues"], "INVALID_LISTINGS_ISSUES")?;
            let mut kept = Vec::new();
            for i in issues {
                if !i["code"].is_string()
                    || !i["message"].is_string()
                    || !i["severity"].is_string()
                    || !i["categories"].is_array()
                {
                    return Err("INVALID_LISTINGS_ISSUE_REQUIRED_FIELDS".into());
                }
                let applies = i["marketplaceIds"]
                    .as_array()
                    .is_none_or(|ids| ids.iter().any(|m| m == marketplace));
                if applies {
                    kept.push(json!({"code":i["code"],"severity":i["severity"],"categories":i["categories"],"enforcement_actions":i["enforcements"]["actions"],"message":i["message"]}));
                } else {
                    ignored += 1;
                }
            }
            fields.push(field(
                "issues".into(),
                &if requested("issues") {
                    json!(kept)
                } else {
                    Value::Null
                },
                "NOT_REQUESTED",
            ));
            for key in ["attributes", "relationships", "productTypes"] {
                if requested(key) {
                    fields.push(field(key.into(), &body[key], "NOT_RETURNED"));
                }
            }
        }
        "inventory_summaries" => {
            let payload = &body["payload"];
            if payload["granularity"]["granularityType"] != "Marketplace"
                || payload["granularity"]["granularityId"] != marketplace
            {
                return Err("INVENTORY_GRANULARITY_OR_MARKET_MISMATCH".into());
            }
            let summaries = payload["inventorySummaries"]
                .as_array()
                .filter(|a| a.iter().all(Value::is_object))
                .ok_or("INVALID_INVENTORY_SUMMARIES")?;
            let requested: Option<Vec<&str>> = ctx["seller_skus"]
                .as_array()
                .map(|a| a.iter().filter_map(Value::as_str).collect());
            for s in summaries {
                let sku = s["sellerSku"]
                    .as_str()
                    .filter(|s| clean(s, 200))
                    .ok_or("INVENTORY_SELLER_SKU_REQUIRED")?;
                if requested.as_ref().is_some_and(|r| !r.contains(&sku)) {
                    return Err("INVENTORY_SKU_OUTSIDE_REQUEST".into());
                }
                quantity(&s["totalQuantity"])?;
                let details = &s["inventoryDetails"];
                for key in [
                    "fulfillableQuantity",
                    "inboundWorkingQuantity",
                    "inboundShippedQuantity",
                    "inboundReceivingQuantity",
                ] {
                    quantity(&details[key])?;
                }
                let prefix = format!("inventorySummaries[{sku}]");
                for key in ["asin", "fnSku", "condition", "productName", "totalQuantity"] {
                    fields.push(field(format!("{prefix}.{key}"), &s[key], "NOT_RETURNED"));
                }
                let updated = s["lastUpdatedTime"]
                    .as_str()
                    .filter(|t| !t.is_empty())
                    .map(|t| json!(t))
                    .unwrap_or(Value::Null);
                fields.push(field(
                    format!("{prefix}.lastUpdatedTime"),
                    &updated,
                    "NOT_RETURNED_OR_EMPTY",
                ));
                let absent = if ctx["details"] == true {
                    "NOT_RETURNED"
                } else {
                    "DETAILS_NOT_REQUESTED"
                };
                for key in [
                    "fulfillableQuantity",
                    "inboundWorkingQuantity",
                    "inboundShippedQuantity",
                    "inboundReceivingQuantity",
                    "reservedQuantity",
                    "researchingQuantity",
                    "unfulfillableQuantity",
                ] {
                    fields.push(field(
                        format!("{prefix}.inventoryDetails.{key}"),
                        &details[key],
                        absent,
                    ));
                }
            }
            return Ok(
                json!({"fields":fields,"item_count":summaries.len(),"pagination":if body["pagination"]["nextToken"].is_string(){"MORE_PAGES_NOT_FOLLOWED"}else{"NO_FURTHER_PAGE_SIGNALLED"},"ignored_other_marketplace_entries":0,"sales":null,"demand":null,"limitation":"Seller FBA inventory snapshot; not market demand, sell-through or other sellers' stock"}),
            );
        }
        "marketplace_participations" => {
            let entries = body["payload"]
                .as_array()
                .ok_or("INVALID_MARKETPLACE_PARTICIPATIONS")?;
            let mut requested = Value::Null;
            for e in entries {
                let m = &e["marketplace"];
                let p = &e["participation"];
                if ![
                    "id",
                    "name",
                    "countryCode",
                    "defaultCurrencyCode",
                    "defaultLanguageCode",
                    "domainName",
                ]
                .iter()
                .all(|k| m[*k].is_string())
                    || !p["isParticipating"].is_boolean()
                    || !p["hasSuspendedListings"].is_boolean()
                    || !e["storeName"].is_string()
                {
                    return Err("INVALID_MARKETPLACE_PARTICIPATION_REQUIRED_FIELDS".into());
                }
                let id = m["id"].as_str().unwrap();
                let prefix = format!("participations[{id}]");
                for (k, v) in [
                    ("marketplace.countryCode", &m["countryCode"]),
                    ("marketplace.defaultCurrencyCode", &m["defaultCurrencyCode"]),
                    ("marketplace.domainName", &m["domainName"]),
                    ("storeName", &e["storeName"]),
                    ("participation.isParticipating", &p["isParticipating"]),
                    (
                        "participation.hasSuspendedListings",
                        &p["hasSuspendedListings"],
                    ),
                ] {
                    fields.push(field(format!("{prefix}.{k}"), v, "NOT_RETURNED"));
                }
                if id == marketplace {
                    requested = p["isParticipating"].clone();
                }
            }
            fields.push(field(
                "requested_marketplace.isParticipating".into(),
                &requested,
                "MARKETPLACE_NOT_LISTED_IN_RESPONSE",
            ));
        }
        "catalog_search" => {
            // An item answers a requested code only when its own identifiers in this marketplace
            // list that code (compared as GTIN-14, or as the ASIN itself).
            let items = body["items"]
                .as_array()
                .filter(|_| body["numberOfResults"].is_u64())
                .ok_or("INVALID_CATALOG_SEARCH_RESULTS")?;
            fields.push(field(
                "numberOfResults".into(),
                &body["numberOfResults"],
                "NOT_RETURNED",
            ));
            let kind = ctx["identifiers_type"].as_str().unwrap_or("");
            let norm = |s: &str| -> String {
                if kind == "ASIN" {
                    s.to_string()
                } else {
                    ecdev_core::resolution::gtin(&json!(s)).unwrap_or_default()
                }
            };
            let wanted: Vec<(String, String)> = ctx["identifiers"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(|s| (s.to_string(), norm(s)))
                .collect();
            let mut answered = std::collections::BTreeSet::new();
            for item in items {
                let asin = item["asin"]
                    .as_str()
                    .filter(|a| a.len() == 10)
                    .ok_or("INVALID_CATALOG_ITEM_ASIN")?;
                let groups = objects(&item["identifiers"], "INVALID_CATALOG_ITEM_IDENTIFIERS")?;
                ignored += groups
                    .iter()
                    .filter(|g| g["marketplaceId"] != marketplace)
                    .count();
                let own: std::collections::BTreeSet<String> = groups
                    .iter()
                    .filter(|g| g["marketplaceId"] == marketplace)
                    .flat_map(|g| g["identifiers"].as_array().cloned().unwrap_or_default())
                    .filter_map(|i| i["identifier"].as_str().map(&norm))
                    .chain((kind == "ASIN").then(|| asin.to_string()))
                    .filter(|s| !s.is_empty())
                    .collect();
                let matched: Vec<&String> = wanted
                    .iter()
                    .filter(|(_, n)| own.contains(n))
                    .map(|(raw, _)| raw)
                    .collect();
                answered.extend(matched.iter().map(|m| m.to_string()));
                fields.push(field(
                    format!("items[{asin}].requested_identifiers_listed"),
                    &json!(matched),
                    "NOT_RETURNED",
                ));
                let summary = item["summaries"]
                    .as_array()
                    .and_then(|a| a.iter().find(|x| x["marketplaceId"] == marketplace))
                    .cloned()
                    .unwrap_or(Value::Null);
                for key in ["itemName", "brand"] {
                    fields.push(field(
                        format!("items[{asin}].{key}"),
                        &summary[key],
                        "NOT_RETURNED",
                    ));
                }
            }
            let unanswered: Vec<&String> = wanted
                .iter()
                .map(|(raw, _)| raw)
                .filter(|r| !answered.contains(*r))
                .collect();
            fields.push(field(
                "identifiers_without_listed_item".into(),
                &json!(unanswered),
                "NOT_RETURNED",
            ));
            if body["pagination"]["nextToken"].is_string() {
                fields.push(json!({"field":"pagination","value":"MORE_PAGES_NOT_FOLLOWED","evidence_state":"OBSERVED"}));
            }
        }
        "product_type_search" => {
            let types = body["productTypes"]
                .as_array()
                .ok_or("INVALID_PRODUCT_TYPE_LIST")?;
            if !body["productTypeVersion"].is_string() {
                return Err("INVALID_PRODUCT_TYPE_LIST".into());
            }
            fields.push(field(
                "productTypeVersion".into(),
                &body["productTypeVersion"],
                "NOT_RETURNED",
            ));
            for t in types {
                let ids = t["marketplaceIds"]
                    .as_array()
                    .filter(|_| t["name"].is_string() && t["displayName"].is_string())
                    .ok_or("INVALID_PRODUCT_TYPE_REQUIRED_FIELDS")?;
                if !ids.iter().any(|m| m == marketplace) {
                    ignored += 1;
                    continue;
                }
                fields.push(field(
                    format!("productTypes[{}].displayName", t["name"].as_str().unwrap()),
                    &t["displayName"],
                    "NOT_RETURNED",
                ));
            }
        }
        "product_type_definition" => {
            for key in [
                "locale",
                "productType",
                "displayName",
                "requirements",
                "requirementsEnforced",
            ] {
                if !body[key].is_string() {
                    return Err("INVALID_PRODUCT_TYPE_DEFINITION_REQUIRED_FIELDS".into());
                }
            }
            if !body["propertyGroups"].is_object()
                || !body["schema"]["checksum"].is_string()
                || !body["productTypeVersion"]["version"].is_string()
                || !body["productTypeVersion"]["latest"].is_boolean()
            {
                return Err("INVALID_PRODUCT_TYPE_DEFINITION_REQUIRED_FIELDS".into());
            }
            if body["productType"] != ctx["product_type"]
                || body["requirements"] != ctx["requirements"]
                || body["requirementsEnforced"] != ctx["requirements_enforced"]
                || (ctx["locale"] != "DEFAULT" && body["locale"] != ctx["locale"])
                || !body["marketplaceIds"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|m| m == marketplace))
                || ctx["product_type_version"]
                    .as_str()
                    .is_some_and(|v| body["productTypeVersion"]["version"] != v)
            {
                return Err("PRODUCT_TYPE_DEFINITION_CORRELATION_MISMATCH".into());
            }
            for key in [
                "productType",
                "displayName",
                "requirements",
                "requirementsEnforced",
                "locale",
            ] {
                fields.push(field(key.into(), &body[key], "NOT_RETURNED"));
            }
            for key in ["version", "latest", "releaseCandidate"] {
                fields.push(field(
                    format!("productTypeVersion.{key}"),
                    &body["productTypeVersion"][key],
                    "NOT_RETURNED",
                ));
            }
            let groups: serde_json::Map<String, Value> = body["propertyGroups"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.clone(), v["propertyNames"].clone()))
                .collect();
            fields.push(field(
                "propertyGroups".into(),
                &json!(groups),
                "NOT_RETURNED",
            ));
            fields.push(field(
                "schema.checksum".into(),
                &body["schema"]["checksum"],
                "NOT_RETURNED",
            ));
            fields.push(field(
                "metaSchema.checksum".into(),
                &body["metaSchema"]["checksum"],
                "NOT_RETURNED",
            ));
            // Schema links are short-lived presigned URLs; they are neither fetched nor retained.
            fields.push(json!({"field":"schema.link.resource","value":null,"evidence_state":"WITHHELD","unknown_reason":"PRESIGNED_LINK_NOT_RETAINED_OR_FETCHED"}));
        }
        _ => return Err("UNSUPPORTED_OFFICIAL_OPERATION".into()),
    }
    Ok(
        json!({"fields":fields,"ignored_other_marketplace_entries":ignored,"sales":null,"demand":null}),
    )
}

/// Attach per-field provenance. Applied to every normalized field before evidence is emitted.
pub fn stamp_fields(normalized: &mut Value, provenance: &Value) {
    if let Some(fields) = normalized["fields"].as_array_mut() {
        for f in fields {
            if let (Some(object), Some(p)) = (f.as_object_mut(), provenance.as_object()) {
                for (k, v) in p {
                    object.insert(k.clone(), v.clone());
                }
            }
        }
    }
}
/// Rewrite observation mode on already stamped fields (fixture capture promoted to LIVE receipt).
pub fn set_field_mode(normalized: &mut Value, mode: &str) {
    if let Some(fields) = normalized["fields"].as_array_mut() {
        for f in fields {
            f["observation_mode"] = json!(mode);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ecdev_core::provider::Provider;
    pub(crate) fn cases() -> Value {
        serde_json::from_str(include_str!(
            "../tests/fixtures/official-read-responses.json"
        ))
        .unwrap()
    }
    pub(crate) fn read_request(market: &str, query: Value) -> AcquireRequest {
        AcquireRequest {
            run_id: "read-fixture-run".into(),
            capability: READ_CAPABILITY.into(),
            market: market.into(),
            query,
        }
    }
    fn fixture_request(case: &Value) -> AcquireRequest {
        let mut query = case["query"].clone();
        query["fixture_responses"] =
            json!({case["operation"].as_str().unwrap(): case["response"].clone()});
        read_request(case["market"].as_str().unwrap(), query)
    }
    pub(crate) fn field<'a>(record: &'a Value, name: &str) -> &'a Value {
        record["normalized"]["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["field"] == name)
            .unwrap_or_else(|| panic!("missing field {name}"))
    }
    #[test]
    fn operation_model_hashes_match_locked_census() {
        let census = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../research/commerce/donors/census/amzn--selling-partner-api-models/files.jsonl"
        ))
        .unwrap();
        let hashes: Vec<Value> = census
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let locked = |path: &str| {
            hashes
                .iter()
                .find(|h| h["path"] == path)
                .map(|h| h["sha256"].as_str().unwrap().to_string())
        };
        for spec in OPERATIONS.iter() {
            assert_eq!(
                locked(spec.model_path).as_deref(),
                Some(spec.model_sha256),
                "{}",
                spec.key
            );
        }
        for (path, sha) in RESTRICTED_MODEL_SOURCES {
            assert_eq!(locked(path).as_deref(), Some(sha));
        }
        let fixtures = cases();
        assert_eq!(fixtures["commit"], MODEL_COMMIT);
        for case in fixtures["cases"].as_array().unwrap() {
            let spec = spec(case["operation"].as_str().unwrap()).unwrap();
            assert_eq!(case["source_path"], spec.model_path);
            assert_eq!(case["source_sha256"], spec.model_sha256);
            assert_eq!(case["provenance"], spec.fixture_provenance);
        }
    }
    #[test]
    fn locked_model_read_examples_normalize_with_field_provenance() {
        let fixtures = cases();
        for case in fixtures["cases"].as_array().unwrap() {
            let result = crate::Amazon.acquire(&fixture_request(case)).unwrap();
            let op = case["operation"].as_str().unwrap();
            assert_eq!(
                result.result["status"], "COMPLETE_FIXTURE_WITH_UNKNOWNS",
                "{op}: {}",
                result.result["records"][0]["reason"]
            );
            assert_eq!(result.result["source_layer"], "OFFICIAL_SP_API");
            assert_eq!(result.provider_cost["request_count"], 0);
            assert_eq!(result.result["new_live_acquisition"], false);
            let raw = case["response"]["raw_body"].as_str().unwrap();
            let hash = format!("{:x}", Sha256::digest(raw.as_bytes()));
            let observation = &result.observations[0];
            observation.validate().unwrap();
            assert_eq!(observation.raw_hash, hash);
            assert_eq!(observation.mode, ObservationMode::Fixture);
            assert_eq!(observation.provider, "amazon-sp-api");
            let record = &result.result["records"][0];
            assert_eq!(record["provenance"]["source_layer"], "OFFICIAL_SP_API");
            assert_eq!(record["provenance"]["evidence_state"], "OBSERVED");
            let fields = record["normalized"]["fields"].as_array().unwrap();
            assert!(!fields.is_empty());
            for f in fields {
                assert_eq!(f["provider"], "amazon-sp-api");
                assert_eq!(f["source_layer"], "OFFICIAL_SP_API");
                assert_eq!(f["raw_capture_sha256"], hash.as_str());
                assert_eq!(f["market"], "AMAZON_US");
                assert_eq!(f["marketplace_id"], "ATVPDKIKX0DER");
                assert_eq!(f["observation_mode"], "FIXTURE");
                assert!(f["observed_at"].as_str().is_some_and(|s| !s.is_empty()));
                assert!(f.get("receipt").is_some());
                assert!(matches!(
                    f["evidence_state"].as_str(),
                    Some("OBSERVED" | "UNKNOWN" | "WITHHELD")
                ));
                assert!(f["value"].is_null() || f["evidence_state"] == "OBSERVED");
            }
            let text = result.result.to_string();
            assert!(!text.contains("PUBLIC_AMAZON") && !text.contains("\"KEEPA\""));
            match op {
                "listings_item" => {
                    assert_eq!(field(record, "summary.asin")["value"], "B071VG5N9D");
                    let offers = &field(record, "offers")["value"];
                    assert_eq!(offers.as_array().unwrap().len(), 1);
                    assert_eq!(offers[0]["price"]["amount"], "100.00");
                    assert_eq!(offers[0]["price"]["currencyCode"], "USD");
                    assert_eq!(field(record, "summary.fnSku")["evidence_state"], "UNKNOWN");
                    assert_eq!(record["normalized"]["ignored_other_marketplace_entries"], 2);
                    assert!(record["normalized"]["sales"].is_null());
                }
                "inventory_summaries" => {
                    assert_eq!(
                        field(record, "inventorySummaries[SYNTH-SKU-1].totalQuantity")["value"],
                        7
                    );
                    assert_eq!(
                        field(record, "inventorySummaries[SYNTH-SKU-2].asin")["evidence_state"],
                        "UNKNOWN"
                    );
                    assert_eq!(
                        record["normalized"]["pagination"],
                        "MORE_PAGES_NOT_FOLLOWED"
                    );
                }
                "marketplace_participations" => {
                    assert_eq!(
                        field(record, "requested_marketplace.isParticipating")["value"],
                        true
                    );
                }
                "catalog_search" => {
                    // One requested code is listed by the returned item; the other by none.
                    assert_eq!(field(record, "numberOfResults")["value"], 1);
                    assert_eq!(
                        field(record, "items[B07N4M94X4].requested_identifiers_listed")["value"],
                        json!(["0887276302195"])
                    );
                    assert_eq!(
                        field(record, "identifiers_without_listed_item")["value"],
                        json!(["4901234567894"])
                    );
                    assert_eq!(field(record, "items[B07N4M94X4].brand")["value"], "SAMSUNG");
                    assert_eq!(
                        field(record, "pagination")["value"],
                        "MORE_PAGES_NOT_FOLLOWED"
                    );
                }
                "product_type_search" => {
                    assert_eq!(
                        field(record, "productTypes[LUGGAGE].displayName")["value"],
                        "Luggage"
                    );
                }
                "product_type_definition" => {
                    assert_eq!(
                        field(record, "schema.link.resource")["evidence_state"],
                        "WITHHELD"
                    );
                    assert!(
                        !record["normalized"]
                            .to_string()
                            .contains("https://schema-url")
                    );
                    assert_eq!(field(record, "productTypeVersion.latest")["value"], true);
                }
                other => panic!("unexpected {other}"),
            }
        }
    }
    #[test]
    fn read_builders_encode_paths_and_refuse_restricted_writes_and_unknowns() {
        let jan = crate::plan(&read_request(
            "AMAZON_JP",
            json!({"operation":"CATALOG_SEARCH_BY_IDENTIFIER","identifiers_type":"JAN","identifiers":["4901234567894"]}),
        ))
        .unwrap();
        let op = &jan["operations"][0];
        assert_eq!(op["operation_id"], "searchCatalogItems");
        assert_eq!(
            (
                op["query"]["identifiersType"].clone(),
                op["query"]["identifiers"].clone()
            ),
            (json!("JAN"), json!("4901234567894"))
        );
        assert_eq!(op["query"]["marketplaceIds"], "A1VC38T7YXB528");
        for bad in [
            json!({"operation":"CATALOG_SEARCH_BY_IDENTIFIER","identifiers_type":"JAN","identifiers":["4901234567890"]}),
            json!({"operation":"CATALOG_SEARCH_BY_IDENTIFIER","identifiers_type":"SKU","identifiers":["X"]}),
            json!({"operation":"CATALOG_SEARCH_BY_IDENTIFIER","identifiers_type":"ASIN","identifiers":["b07n4m94x4"]}),
            json!({"operation":"CATALOG_SEARCH_BY_IDENTIFIER","identifiers_type":"JAN","identifiers":[]}),
            json!({"operation":"CATALOG_SEARCH_BY_IDENTIFIER","identifiers_type":"JAN","identifiers":["4901234567894"],"keywords":["x"]}),
            json!({"operation":"CATALOG_SEARCH_BY_IDENTIFIER","identifiers_type":"JAN","identifiers":["4901234567894"],"page_size":21}),
        ] {
            assert!(
                crate::plan(&read_request("AMAZON_JP", bad.clone())).is_err(),
                "{bad}"
            );
        }
        let r = read_request(
            "AMAZON_US",
            json!({"operation":"LISTINGS_ITEM","seller_id":"A1EXAMPLESELLER","sku":"A/B C%"}),
        );
        let plan = crate::plan(&r).unwrap();
        let op = &plan["operations"][0];
        assert_eq!(
            op["requested_url"],
            "https://sellingpartnerapi-na.amazon.com/listings/2021-08-01/items/A1EXAMPLESELLER/A%2FB%20C%25"
        );
        assert_eq!(op["query"]["marketplaceIds"], "ATVPDKIKX0DER");
        assert_eq!(
            op["query"]["includedData"],
            "summaries,offers,fulfillmentAvailability,issues"
        );
        assert_eq!(op["method"], "GET");
        assert_eq!(op["operation_id"], "getListingsItem");
        assert_eq!(op["rdt"], "NOT_USED_NON_PII_OPERATION");
        let jp = crate::plan(&read_request(
            "AMAZON_JP",
            json!({"operation":"INVENTORY_SUMMARIES","seller_skus":["S1","S2"],"details":true}),
        ))
        .unwrap();
        assert_eq!(
            jp["operations"][0]["requested_url"],
            "https://sellingpartnerapi-fe.amazon.com/fba/inventory/v1/summaries"
        );
        assert_eq!(
            jp["operations"][0]["query"],
            json!({"details":"true","granularityType":"Marketplace","granularityId":"A1VC38T7YXB528","marketplaceIds":"A1VC38T7YXB528","sellerSkus":"S1,S2"})
        );
        let definition = crate::plan(&read_request(
            "AMAZON_US",
            json!({"operation":"PRODUCT_TYPE_DEFINITION","product_type":"LUGGAGE"}),
        ))
        .unwrap();
        assert_eq!(
            definition["operations"][0]["query"],
            json!({"marketplaceIds":"ATVPDKIKX0DER","requirements":"LISTING","requirementsEnforced":"ENFORCED","locale":"DEFAULT"})
        );
        for (query, reason) in [
            (json!({"operation":"ORDERS"}), "RESTRICTED_DOMAIN_DISABLED"),
            (
                json!({"operation":"ORDER_BUYER_INFO"}),
                "RESTRICTED_DOMAIN_DISABLED",
            ),
            (
                json!({"operation":"RESTRICTED_DATA_TOKEN"}),
                "RESTRICTED_DOMAIN_DISABLED",
            ),
            (
                json!({"operation":"PUT_LISTINGS_ITEM"}),
                "OFFICIAL_WRITES_DISABLED",
            ),
            (
                json!({"operation":"DELETE_LISTINGS_ITEM"}),
                "OFFICIAL_WRITES_DISABLED",
            ),
            (
                json!({"operation":"GET_EVERYTHING"}),
                "UNSUPPORTED_OFFICIAL_READ_OPERATION",
            ),
            (
                json!({"operation":"MARKETPLACE_PARTICIPATIONS","sku":"x"}),
                "UNKNOWN_OFFICIAL_READ_PARAMETER",
            ),
            (
                json!({"operation":"PRODUCT_TYPE_SEARCH","keywords":["a"],"item_name":"b"}),
                "KEYWORDS_AND_ITEM_NAME_EXCLUSIVE",
            ),
            (
                json!({"operation":"LISTINGS_ITEM","seller_id":"A1","sku":"x","included_data":["procurement"]}),
                "INVALID_LISTINGS_INCLUDED_DATA",
            ),
            (
                json!({"operation":"LISTINGS_ITEM","seller_id":"A-1","sku":"x"}),
                "INVALID_SELLER_ID",
            ),
            (
                json!({"operation":"LISTINGS_ITEM","seller_id":"A1","sku":"bad\nsku"}),
                "INVALID_SKU",
            ),
            (
                json!({"operation":"INVENTORY_SUMMARIES","seller_skus":["a,b"]}),
                "INVALID_OFFICIAL_READ_LIST_ITEM",
            ),
            (
                json!({"operation":"PRODUCT_TYPE_DEFINITION","product_type":"luggage"}),
                "INVALID_PRODUCT_TYPE",
            ),
            (
                json!({"operation":"PRODUCT_TYPE_DEFINITION","product_type":"LUGGAGE","requirements":"ALL"}),
                "INVALID_PRODUCT_TYPE_REQUIREMENTS",
            ),
        ] {
            assert_eq!(
                crate::plan(&read_request("AMAZON_US", query.clone()))
                    .err()
                    .as_deref(),
                Some(reason),
                "{query}"
            );
        }
        let skus: Vec<String> = (0..51).map(|i| format!("S{i}")).collect();
        assert!(
            crate::plan(&read_request(
                "AMAZON_US",
                json!({"operation":"INVENTORY_SUMMARIES","seller_skus":skus})
            ))
            .is_err()
        );
        assert_eq!(
            crate::plan(&read_request(
                "AMAZON_EU",
                json!({"operation":"MARKETPLACE_PARTICIPATIONS"})
            ))
            .err()
            .as_deref(),
            Some("UNSUPPORTED_OFFICIAL_MARKET")
        );
        let mut wrong = read_request("AMAZON_US", json!({"operation":"ORDERS"}));
        wrong.capability = "orders.read.official".into();
        assert_eq!(
            crate::plan(&wrong).err().as_deref(),
            Some("UNSUPPORTED_OFFICIAL_CAPABILITY")
        );
    }
    #[test]
    fn read_normalizers_reject_cross_market_and_correlation_mismatch() {
        let fixtures = cases();
        let by_op = |op: &str| {
            fixtures["cases"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["operation"] == op)
                .unwrap()
                .clone()
        };
        let reason = |case: &Value| {
            crate::Amazon
                .acquire(&fixture_request(case))
                .unwrap()
                .result["records"][0]["reason"]
                .clone()
        };
        let mut listing = by_op("listings_item");
        listing["query"]["sku"] = json!("OTHER-SKU");
        assert_eq!(reason(&listing), "LISTINGS_SKU_MISMATCH");
        let mut inventory = by_op("inventory_summaries");
        inventory["market"] = json!("AMAZON_JP");
        assert_eq!(
            reason(&inventory),
            "INVENTORY_GRANULARITY_OR_MARKET_MISMATCH"
        );
        let mut inventory = by_op("inventory_summaries");
        inventory["query"]["seller_skus"] = json!(["SYNTH-SKU-1"]);
        assert_eq!(reason(&inventory), "INVENTORY_SKU_OUTSIDE_REQUEST");
        let mut definition = by_op("product_type_definition");
        definition["query"]["requirements"] = json!("LISTING_OFFER_ONLY");
        assert_eq!(
            reason(&definition),
            "PRODUCT_TYPE_DEFINITION_CORRELATION_MISMATCH"
        );
        let mut sellers = by_op("marketplace_participations");
        sellers["market"] = json!("AMAZON_JP");
        let result = crate::Amazon.acquire(&fixture_request(&sellers)).unwrap();
        let record = &result.result["records"][0];
        assert_eq!(
            field(record, "requested_marketplace.isParticipating")["evidence_state"],
            "UNKNOWN"
        );
        let mut errors = by_op("product_type_search");
        errors["response"]["raw_body"] = json!("{\"errors\":[{\"code\":\"Unauthorized\"}]}");
        assert_eq!(reason(&errors), "OFFICIAL_RESPONSE_ERRORS");
        let mut misplaced = fixture_request(&by_op("product_type_search"));
        misplaced.query["fixture_responses"]["catalog"] = json!({"status":200,"raw_body":"{}"});
        assert_eq!(
            crate::Amazon.acquire(&misplaced).err().unwrap().reason,
            "INVALID_FIXTURE_OPERATION"
        );
        let mut throttled = by_op("listings_item");
        throttled["response"]["status"] = json!(429);
        let result = crate::Amazon.acquire(&fixture_request(&throttled)).unwrap();
        assert_eq!(result.result["records"][0]["reason"], "OFFICIAL_HTTP_429");
        assert_eq!(
            result.result["records"][0]["provenance"]["evidence_state"],
            "UNKNOWN"
        );
        assert!(result.result["records"][0]["normalized"].is_null());
    }
}
