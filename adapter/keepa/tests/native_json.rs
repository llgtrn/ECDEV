use ecdev_core::json;

#[test]
fn native_macro_preserves_exact_money_and_unknowns() {
    // This minor-unit amount is the first integer binary64 cannot represent exactly.
    const AMOUNT_MINOR: u64 = 9_007_199_254_740_993;
    let amount_evaluations = std::cell::Cell::new(0);
    let key_evaluations = std::cell::Cell::new(0);
    let amount = || {
        amount_evaluations.set(amount_evaluations.get() + 1);
        AMOUNT_MINOR
    };
    let source_key = || {
        key_evaluations.set(key_evaluations.get() + 1);
        "source"
    };
    let source = "USD 日本😀 \"quoted\"\\path\n\t";
    let value = json!({
        "amount_minor": amount(),
        "currency": "USD",
        "cost_minor": None::<u64>,
        "effect_observed": None::<bool>,
        "explicit_null": null,
        "known_zero_cost_minor": Some(0u64),
        "known_no_effect": Some(false),
        (source_key()): source,
    });
    assert_eq!(amount_evaluations.get(), 1);
    assert_eq!(key_evaluations.get(), 1);

    let bytes = ecdev_core::json::to_vec(&value).unwrap();
    assert_eq!(
        std::str::from_utf8(&bytes).unwrap(),
        concat!(
            r#"{"amount_minor":9007199254740993,"cost_minor":null,"currency":"USD","#,
            r#""effect_observed":null,"explicit_null":null,"known_no_effect":false,"#,
            r#""known_zero_cost_minor":0,"source":"USD 日本😀 \"quoted\"\\path\n\t"}"#,
        )
    );
    let decoded: ecdev_core::json::Value = ecdev_core::json::from_slice(&bytes).unwrap();
    assert_eq!(decoded, value);
    assert_eq!(decoded["amount_minor"].as_u64(), Some(AMOUNT_MINOR));
    assert_eq!(decoded["source"].as_str(), Some(source));
    for field in ["cost_minor", "effect_observed", "explicit_null"] {
        assert_eq!(decoded.get(field), Some(&ecdev_core::json::Value::Null));
    }
    assert_eq!(decoded.get("absent"), None);
    assert_eq!(decoded["known_zero_cost_minor"].as_u64(), Some(0));
    assert_eq!(decoded["known_no_effect"].as_bool(), Some(false));
    assert_eq!(amount_evaluations.get(), 1);
    assert_eq!(key_evaluations.get(), 1);
}
