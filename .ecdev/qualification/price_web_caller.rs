//! Bounded offline acceptance of the public HTML extraction caller.
//! Frozen API qualification does not establish target execution or admission.
use ecdev_core::{json, json::Value, sha256::Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::Read,
    path::{Component, Path, PathBuf},
};

const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
const PINS: [(&str, &str); 4] = [
    (
        "adapter/web/tests/fixtures/price-contract.json",
        "3729e24b33e660f6155ccca4ef5e8b199de4b2cab1e7b517cf403f930157d3ce",
    ),
    (
        "research/commerce/governance/price-bounded-mapping-db92bf27-v1.json",
        "300591b5db1f9b156ecf5282525e5d056c79986140cffff20fcd784b2e825792",
    ),
    (
        "adapter/web/src/lib.rs",
        "34539b4835e6a66bd2d091de1f7c7f6f543e7200b8c5920660cfdb5562a9e944",
    ),
    (
        "adapter/web/src/price.rs",
        "5aac2cf7a43e3d2fad31e233d1abdbb97d21197cf245cbc1f49d0901fecd8ec3",
    ),
];
const NEW_WEB: [usize; 5] = [117, 212, 855, 1193, 1849];
const EXCLUDED: [usize; 7] = [409, 1185, 1186, 1187, 1188, 1467, 1473];
const SELECTORS: [&str; 4] = [
    "input",
    "currency_hint",
    "decimal_separator",
    "digit_group_separator",
];

#[derive(Default)]
struct Checks {
    count: usize,
}
impl Checks {
    fn require(&mut self, condition: bool, context: impl AsRef<str>) -> Result<(), String> {
        if !condition {
            return Err(context.as_ref().to_owned());
        }
        self.count += 1;
        Ok(())
    }
}

// Reject symlinks in every component; bounded reads also inspect the opened file.
fn regular_read(path: &Path) -> Result<Vec<u8>, String> {
    let mut prefix = PathBuf::new();
    for component in path.components() {
        if matches!(component, Component::ParentDir) {
            return Err("parent path component denied".into());
        }
        prefix.push(component);
        let meta =
            fs::symlink_metadata(&prefix).map_err(|e| format!("{}: {e}", prefix.display()))?;
        if meta.file_type().is_symlink() {
            return Err(format!("symlink denied: {}", prefix.display()));
        }
    }
    let before = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !before.is_file() || before.len() > MAX_FILE_BYTES {
        return Err(format!("bounded regular file required: {}", path.display()));
    }
    let file = File::open(path).map_err(|e| e.to_string())?;
    let opened = file.metadata().map_err(|e| e.to_string())?;
    if !opened.is_file() || opened.len() > MAX_FILE_BYTES || opened.len() != before.len() {
        return Err("opened file changed type or size".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if (before.dev(), before.ino()) != (opened.dev(), opened.ino()) {
            return Err("opened file identity changed".into());
        }
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 != opened.len() || bytes.len() as u64 > MAX_FILE_BYTES {
        return Err("file length changed or exceeds bound".into());
    }
    let after = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !after.is_file() || after.len() != opened.len() {
        return Err("file changed during read".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if (opened.dev(), opened.ino()) != (after.dev(), after.ino()) {
            return Err("file identity changed during read".into());
        }
    }
    Ok(bytes)
}
fn frozen_read(root: &Path, relative: &str, expected: &str) -> Result<Vec<u8>, String> {
    let bytes = regular_read(&root.join(relative))?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    if digest != expected {
        return Err(format!(
            "frozen input digest mismatch: {relative}: {digest}"
        ));
    }
    Ok(bytes)
}
fn present(value: &Value, field: &str) -> bool {
    value.as_object().is_some_and(|o| o.contains_key(field))
}
fn request(case: &Value) -> Value {
    json!({"input":case["input"],"currency_hint":case["currency_hint"],"decimal_separator":case["decimal_separator"],"digit_group_separator":case["digit_group_separator"]})
}
fn request_presence(case: &Value) -> Value {
    json!({"input":!case["input"].is_null(),"currency_hint":!case["currency_hint"].is_null(),"decimal_separator":!case["decimal_separator"].is_null(),"digit_group_separator":!case["digit_group_separator"].is_null()})
}
fn web_shape(case: &Value) -> bool {
    case["input"].is_string()
        && case["decimal_separator"].is_null()
        && case["digit_group_separator"].is_null()
}
fn float_bits(value: &Value) -> Result<Option<String>, String> {
    if value.is_null() {
        return Ok(None);
    }
    let number = value
        .as_f64()
        .ok_or("amount_float must be finite number or null")?;
    if !number.is_finite() {
        return Err("nonfinite amount_float".into());
    }
    Ok(Some(format!("{:016x}", number.to_bits())))
}
fn product_from_offer(offer: Value) -> Value {
    json!({"@context":"https://schema.org","@type":"Product","name":"Bounded offline acceptance","offers":offer})
}
fn extract_product(node: &Value, checks: &mut Checks) -> Result<Value, String> {
    // JSON escaping prevents an embedded closing-script sequence changing HTML structure.
    // Decoding the JSON restores the exact original string and its UTF-8 bytes.
    let encoded = node.to_string().replace('<', "\\u003c");
    let html = format!("<script type=\"application/ld+json\">{encoded}</script>");
    let extracted = ecdev_web::extract(&html, "https://offline.example/product")?;
    checks.require(
        extracted["products"]
            .as_array()
            .is_some_and(|a| a.len() == 1),
        "expected one extracted JSON-LD product",
    )?;
    let product = extracted["products"][0].clone();
    checks.require(product["raw_json_ld"] == *node, "raw JSON-LD changed")?;
    checks.require(
        product["observed_offers"]
            .as_array()
            .is_some_and(|a| a.len() == 1),
        "expected one extracted offer",
    )?;
    let source_offer = &node["offers"];
    let offer = &product["observed_offers"][0];
    checks.require(
        offer["raw_price"] == source_offer["price"],
        "raw price changed",
    )?;
    checks.require(
        offer["currency"] == source_offer["priceCurrency"],
        "raw currency hint changed",
    )?;
    if source_offer["price"].is_string() {
        checks.require(
            offer["price_text_interpretation"]["used_for_observed_price"] == false,
            "derived interpretation became observed money authority",
        )?;
        checks.require(
            offer["price_text_interpretation"]["currency_token_is_iso_identity"] == false,
            "currency token became ISO identity",
        )?;
    } else {
        checks.require(
            offer["price_text_interpretation"].is_null(),
            "nonstring price reached interpretation branch",
        )?;
    }
    Ok(product)
}
fn observed(product: &Value) -> Value {
    json!({"product_price_minor":product["price_minor"],"product_price_status":product["price_status"],"offer_price_minor":product["observed_offers"][0]["price_minor"]})
}
fn unknown_money(product: &Value, checks: &mut Checks) -> Result<(), String> {
    checks.require(
        product["price_minor"].is_null(),
        "unexpected observed product money",
    )?;
    checks.require(
        product["price_status"] == "UNKNOWN",
        "expected UNKNOWN observed product status",
    )?;
    checks.require(
        product["observed_offers"][0]["price_minor"].is_null(),
        "unexpected observed offer money",
    )
}
fn negative_report(name: &str, node: &Value, product: &Value) -> Value {
    let offer = &product["observed_offers"][0];
    json!({"name":name,"status":"PASS","source_json_ld":node,"raw_json_ld":product["raw_json_ld"],"raw_price":offer["raw_price"],"raw_price_json":offer["raw_price"].to_string(),"price_field_present":present(&node["offers"],"price"),"currency_hint":{"present":present(&node["offers"],"priceCurrency"),"value":node["offers"]["priceCurrency"]},"source_observed":observed(product),"interpretation":offer["price_text_interpretation"],"target_execution_performed":false,"target_acceptance_established":false})
}
fn negatives(checks: &mut Checks) -> Result<Vec<Value>, String> {
    let mut reports = Vec::new();
    for (name, price) in [
        ("null_price", Value::Null),
        ("boolean_price", json!(true)),
        ("array_price", json!(["12.99"])),
        ("object_price", json!({"value":"12.99"})),
    ] {
        let node = product_from_offer(json!({"price":price,"priceCurrency":"USD"}));
        let product = extract_product(&node, checks)?;
        unknown_money(&product, checks)?;
        reports.push(negative_report(name, &node, &product));
    }
    let node = product_from_offer(json!({"priceCurrency":"USD"}));
    let product = extract_product(&node, checks)?;
    unknown_money(&product, checks)?;
    checks.require(
        !present(&product["raw_json_ld"]["offers"], "price"),
        "missing price field was inserted",
    )?;
    reports.push(negative_report("missing_price", &node, &product));
    // Exact numeric lexemes are source observations, never string interpretations.
    for (literal, currency, money) in [
        ("2980.0000000000001", "JPY", None),
        ("0.29000000000000001", "USD", None),
        ("9007199254740993.0", "JPY", Some(9_007_199_254_740_993_i64)),
        ("2.98e+3", "JPY", Some(2980_i64)),
        ("2.9e-1", "USD", Some(29_i64)),
    ] {
        let offer: Value = json::from_str(&format!(
            "{{\"price\":{literal},\"priceCurrency\":\"{currency}\"}}"
        ))
        .map_err(|e| e.to_string())?;
        let node = product_from_offer(offer);
        let product = extract_product(&node, checks)?;
        let raw = &product["observed_offers"][0]["raw_price"];
        checks.require(
            raw.as_number()
                .is_some_and(|number| number.as_str() == literal),
            format!("numeric source lexeme changed: {literal}"),
        )?;
        checks.require(
            product["price_minor"] == json!(money),
            "numeric observed money changed",
        )?;
        checks.require(
            product["observed_offers"][0]["price_minor"] == json!(money),
            "numeric observed offer money changed",
        )?;
        reports.push(negative_report(
            &format!("numeric_lexeme_{literal}"),
            &node,
            &product,
        ));
    }
    for (raw, hint, amount) in [
        ("$12.99", Some("USD"), "12.99"),
        ("free shipping", None, "0"),
    ] {
        let mut offer = json!({"price":raw});
        if let Some(hint) = hint {
            offer["priceCurrency"] = json!(hint);
        }
        let node = product_from_offer(offer);
        let product = extract_product(&node, checks)?;
        unknown_money(&product, checks)?;
        checks.require(
            product["observed_offers"][0]["price_text_interpretation"]["result"]["amount"]
                == amount,
            "formatted derived amount changed",
        )?;
        reports.push(negative_report(raw, &node, &product));
    }
    let mut hints = Vec::new();
    for (name, hint) in [
        ("hint_absent", None),
        ("hint_null", Some(Value::Null)),
        ("hint_nonstring", Some(json!(42))),
        ("hint_empty_string", Some(json!(""))),
        ("hint_currency_string", Some(json!("EUR"))),
    ] {
        let mut offer = json!({"price":"12.99"});
        if let Some(hint) = hint {
            offer["priceCurrency"] = hint;
        }
        let node = product_from_offer(offer);
        let product = extract_product(&node, checks)?;
        let raw_offer = &product["raw_json_ld"]["offers"];
        let option_present = raw_offer["priceCurrency"].as_str().is_some();
        checks.require(
            option_present == matches!(name, "hint_empty_string" | "hint_currency_string"),
            "hint option presence changed",
        )?;
        if name == "hint_currency_string" {
            checks.require(
                product["observed_offers"][0]["price_text_interpretation"]["result"]["currency"]
                    == "EUR",
                "string hint not forwarded",
            )?;
        } else {
            unknown_money(&product, checks)?;
        }
        hints.push((name, product.clone()));
        let mut report = negative_report(name, &node, &product);
        report["caller_hint_option"] =
            json!({"present":option_present,"value":raw_offer["priceCurrency"].as_str()});
        reports.push(report);
    }
    checks.require(
        hints[0].1["raw_json_ld"] != hints[3].1["raw_json_ld"],
        "None hint collapsed into Some empty",
    )?;
    let node = product_from_offer(json!({"price":"","priceCurrency":""}));
    let product = extract_product(&node, checks)?;
    unknown_money(&product, checks)?;
    checks.require(
        product["observed_offers"][0]["price_text_interpretation"]["state"] == "DERIVED",
        "Some empty input skipped branch",
    )?;
    checks.require(
        present(&product["raw_json_ld"]["offers"], "price"),
        "Some empty input collapsed into missing",
    )?;
    reports.push(negative_report(
        "empty_string_price_and_hint",
        &node,
        &product,
    ));
    let raw = "€12.99 </script><script>inert & 雪</script>";
    let hint = "\"<&雪>";
    let node = product_from_offer(json!({"price":raw,"priceCurrency":hint}));
    let product = extract_product(&node, checks)?;
    unknown_money(&product, checks)?;
    checks.require(
        product["observed_offers"][0]["raw_price"]
            .as_str()
            .map(str::as_bytes)
            == Some(raw.as_bytes()),
        "Unicode/HTML source bytes changed",
    )?;
    reports.push(negative_report(
        "unicode_and_embedded_closing_script",
        &node,
        &product,
    ));
    let node = product_from_offer(json!({"price":"$".repeat(4097),"priceCurrency":"USD"}));
    let product = extract_product(&node, checks)?;
    unknown_money(&product, checks)?;
    let interpretation = &product["observed_offers"][0]["price_text_interpretation"];
    checks.require(
        interpretation["state"] == "UNKNOWN",
        "over-bound price not UNKNOWN",
    )?;
    checks.require(
        interpretation["result"]
            == json!({"amount":null,"currency":null,"amount_text":null,"amount_float":null}),
        "over-bound price not null projection",
    )?;
    reports.push(negative_report("price_over_4096_bytes", &node, &product));
    Ok(reports)
}
fn run() -> Result<Value, String> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 2 {
        return Err("usage: price-web-caller SOURCE_ROOT".into());
    }
    let root = Path::new(&args[1]);
    if !root.is_absolute() {
        return Err("SOURCE_ROOT must be absolute".into());
    }
    let mut frozen = Vec::new();
    for (path, digest) in PINS {
        frozen.push(frozen_read(root, path, digest)?);
    }
    let corpus: Value = json::from_slice(&frozen[0]).map_err(|e| e.to_string())?;
    let sidecar: Value = json::from_slice(&frozen[1]).map_err(|e| e.to_string())?;
    let cases = corpus["cases"].as_array().ok_or("corpus cases required")?;
    let mut checks = Checks::default();
    checks.require(cases.len() == 2937, "corpus count changed")?;
    let indices: Vec<usize> = sidecar["coverage"]["qualified_case_indices"]
        .as_array()
        .ok_or("qualified indices required")?
        .iter()
        .map(|v| {
            v.as_u64()
                .and_then(|v| usize::try_from(v).ok())
                .ok_or("invalid qualified index".to_string())
        })
        .collect::<Result<_, _>>()?;
    let set: BTreeSet<usize> = indices.iter().copied().collect();
    checks.require(
        indices.len() == 106 && set.len() == 106,
        "API qualification count or uniqueness changed",
    )?;
    checks.require(
        sidecar["coverage"]["supported"] == 106
            && sidecar["coverage"]["unmapped"] == 2831
            && sidecar["coverage"]["total"] == 2937,
        "full API totals changed",
    )?;
    let web_count = cases.iter().filter(|case| web_shape(case)).count();
    checks.require(
        web_count == 2710 && cases.len() - web_count == 227,
        "web shape partition changed",
    )?;
    let mut results = Vec::new();
    let mut excluded = Vec::new();
    let mut excluded_indices = Vec::new();
    let mut new_indices = Vec::new();
    for index in indices {
        let case = cases.get(index).ok_or("qualified index outside corpus")?;
        for selector in SELECTORS {
            checks.require(
                present(case, selector) && (case[selector].is_string() || case[selector].is_null()),
                format!("invalid corpus selector {index}/{selector}"),
            )?;
        }
        if !web_shape(case) {
            let reason = if case["input"].is_null() {
                "INPUT_NONE_STRING_PRICE_REQUIRED"
            } else {
                "EXPLICIT_SELECTORS_UNAVAILABLE_TO_WEB_CALLER"
            };
            excluded.push(json!({"index":index,"name":case["name"],"source_request":request(case),"source_request_presence":request_presence(case),"reason":reason,"actual_source_caller_invoked":false,"target_execution_performed":false}));
            excluded_indices.push(index);
            continue;
        }
        checks.require(
            case["error"].is_null(),
            format!("qualified case has error {index}"),
        )?;
        let raw = case["input"].as_str().ok_or("string input required")?;
        let hint = case["currency_hint"].as_str();
        let mut offer = json!({"price":raw});
        if let Some(hint) = hint {
            offer["priceCurrency"] = json!(hint);
        }
        let node = product_from_offer(offer);
        let product =
            extract_product(&node, &mut checks).map_err(|e| format!("case {index}: {e}"))?;
        checks.require(
            present(&product["raw_json_ld"]["offers"], "priceCurrency") == hint.is_some(),
            format!("hint presence changed {index}"),
        )?;
        let observed_offer = &product["observed_offers"][0];
        let interpretation = &observed_offer["price_text_interpretation"];
        checks.require(
            interpretation["state"] == "DERIVED",
            format!("qualified case interpretation not DERIVED {index}"),
        )?;
        let derived = &interpretation["result"];
        checks.require(
            derived.as_object().is_some_and(|o| o.len() == 4),
            format!("derived projection shape changed {index}"),
        )?;
        for field in ["amount", "currency", "amount_text"] {
            checks.require(
                derived[field] == case["expected"][field],
                format!("original corpus mismatch {index}/{field}"),
            )?;
        }
        let bits = float_bits(&derived["amount_float"])?;
        checks.require(
            bits == float_bits(&case["expected"]["amount_float"])?,
            format!("float bits mismatch {index}"),
        )?;
        if NEW_WEB.contains(&index) {
            new_indices.push(index);
        }
        results.push(json!({"index":index,"name":case["name"],"qualification_cohort":if NEW_WEB.contains(&index){"NEW_PENDING"}else{"PRIOR_QUALIFIED"},"source_request":request(case),"source_request_presence":request_presence(case),"raw_json_ld":product["raw_json_ld"],"raw_price":observed_offer["raw_price"],"currency_hint":{"present":hint.is_some(),"value":hint},"source_observed":observed(&product),"interpretation":{"state":interpretation["state"],"result":derived,"amount_float_bits":bits,"used_for_observed_price":false,"currency_token_is_iso_identity":false},"hypothetical_request":{"input_utf8":raw.as_bytes(),"currency_hint":{"present":hint.is_some(),"value":hint},"decimal_separator":null,"digit_group_separator":null},"actual_source_caller_invoked":true,"target_execution_performed":false,"target_acceptance_established":false}));
    }
    checks.require(
        excluded_indices == EXCLUDED,
        "seven qualified API-only exclusions changed",
    )?;
    checks.require(
        new_indices == NEW_WEB && results.len() == 99,
        "99-case web-qualified cohort changed",
    )?;
    let unmapped_web = cases
        .iter()
        .enumerate()
        .filter(|(i, case)| web_shape(case) && !set.contains(i))
        .count();
    checks.require(
        unmapped_web == 2611,
        "web-shaped API-unmapped count changed",
    )?;
    let negative_cases = negatives(&mut checks)?;
    let mut frozen_inputs = Vec::new();
    for ((path, digest), before) in PINS.into_iter().zip(frozen) {
        let after = frozen_read(root, path, digest)?;
        checks.require(
            after == before,
            format!("frozen input changed after execution: {path}"),
        )?;
        frozen_inputs.push(json!({"path":path,"sha256":digest,"size_bytes":after.len(),"verified_before":true,"verified_after":true}));
    }
    Ok(
        json!({"format":"ECDEV_PRICE_WEB_CALLER_V1","status":"PASS","entry":"ecdev_web::extract -> products -> price::parse_price","execution_mode":"OFFLINE_SYNTHETIC_JSON_LD_ACTUAL_SOURCE_CALLER","frozen_inputs":frozen_inputs,"counts":{"corpus_total":2937,"api_qualified":106,"api_unmapped":2831,"web_shape":web_count,"non_web_shape":cases.len()-web_count,"web_qualified":results.len(),"web_unmapped":unmapped_web,"qualified_web_excluded":excluded.len(),"web_old_qualified":94,"web_new_qualified":new_indices.len(),"negative_cases":negative_cases.len()},"checks":{"status":"PASS","assertion_count":checks.count,"original_corpus_four_fields":true,"nullable_finite_float_bits_exact":true,"raw_json_ld_and_price_preserved":true,"hint_option_presence_preserved":true,"derived_not_observed_money_authority":true,"currency_token_not_iso_identity":true,"full_api_totals_preserved":true,"qualified_web_cohort_exact":true,"bounded_regular_frozen_inputs_before_after":true},"cases":results,"excluded_cases":excluded,"negative_cases":negative_cases,"target_execution_performed":false,"target_acceptance_established":false,"authority_granted":false,"limitations":["Synthetic offline HTML exercises the actual source caller; no remote capture or target runtime is executed.","API qualification and Web caller reachability are separate predicates.","Hypothetical payloads preserve decoded input bytes and hint Option presence; they grant no rights or admission."]}),
    )
}
fn main() {
    match run() {
        Ok(report) => println!("{report}"),
        Err(error) => {
            eprintln!("price-web-caller: {error}");
            std::process::exit(1);
        }
    }
}
