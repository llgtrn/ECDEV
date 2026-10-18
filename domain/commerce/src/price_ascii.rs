//! Original bounded ASCII interpretation manufactured for the receiving price-text contract.
//! It preserves exact decimal text separately from its approximate floating image.
//! This is not the existing ECDEV heuristic parser, ISO currency resolution or observed money.

/// The four nullable derived fields: exact amount, currency token, original numeric text, float.
pub type PriceFields = (Option<String>, Option<String>, Option<String>, Option<f64>);

/// Interpret only the explicitly supported numeric subdomain, preserving refusal precedence.
pub fn parse(
    input: Option<&str>,
    currency_hint: Option<&str>,
    decimal_separator: Option<&str>,
    digit_group_separator: Option<&str>,
) -> Result<PriceFields, u8> {
    if input.is_some_and(|value| value.len() > 4096) {
        return Err(1);
    }
    if currency_hint.is_some_and(|value| value.len() > 4096) {
        return Err(2);
    }
    if digit_group_separator.is_some_and(|value| value.len() > 64) {
        return Err(3);
    }
    if currency_hint.is_some_and(|value| value.len() > 64) {
        return Err(4);
    }
    let currency = match currency_hint {
        None | Some("") => None,
        Some(value @ ("$" | "USD" | "EUR")) => Some(value.to_owned()),
        _ => return Err(5),
    };
    let selected = match decimal_separator {
        None | Some("") => None,
        Some(".") => Some(b'.'),
        Some(",") => Some(b','),
        _ => return Err(6),
    };
    if !matches!(digit_group_separator, None | Some("")) {
        return Err(7);
    }
    let text = input.unwrap_or_default().trim_matches(' ');
    if text.is_empty() {
        return Ok((None, currency, None, None));
    }
    let bytes = text.as_bytes();
    if !bytes[0].is_ascii_digit() || !bytes[bytes.len() - 1].is_ascii_digit() {
        return Err(8);
    }
    let mut separator = None;
    let mut digits = 0;
    let mut fraction_digits = 0;
    for (index, &byte) in bytes.iter().enumerate() {
        match byte {
            b'0'..=b'9' => {
                digits += 1;
                if digits > 18 {
                    return Err(9);
                }
                if separator.is_some() {
                    fraction_digits += 1;
                }
            }
            b'.' | b',' if separator.is_none() => separator = Some((index, byte)),
            _ => return Err(8),
        }
    }
    if fraction_digits > 6 {
        return Err(10);
    }
    if selected.is_some_and(|expected| separator.is_some_and(|(_, actual)| actual != expected)) {
        return Err(12);
    }
    if selected.is_none() && fraction_digits > 2 {
        return Err(11);
    }
    let (integer, fraction) = separator.map_or((text, ""), |(index, _)| {
        (&text[..index], &text[index + 1..])
    });
    let integer = integer.trim_start_matches('0');
    let mut exact = if integer.is_empty() { "0" } else { integer }.to_owned();
    let fraction = fraction.trim_end_matches('0');
    if !fraction.is_empty() {
        exact.push('.');
        exact.push_str(fraction);
    }
    let approximate = exact.parse::<f64>().map_err(|_| 13)?;
    if !approximate.is_finite() {
        return Err(13);
    }
    Ok((
        Some(exact),
        currency,
        Some(text.to_owned()),
        Some(approximate),
    ))
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn exact_text_survives_float_rounding_and_original_spelling_is_retained() {
        let value = parse(Some(" 00012,3400 "), Some("$"), Some(","), None).unwrap();
        assert_eq!(value.0.as_deref(), Some("12.34"));
        assert_eq!(value.1.as_deref(), Some("$"));
        assert_eq!(value.2.as_deref(), Some("00012,3400"));
        assert_eq!(value.3, Some(12.34));
        let large = parse(Some("9007199254741017"), None, None, None).unwrap();
        assert_eq!(large.0.as_deref(), Some("9007199254741017"));
        assert_eq!(large.3, Some("9007199254741017".parse::<f64>().unwrap()));
        assert_eq!(
            parse(Some("000.00"), None, None, None)
                .unwrap()
                .0
                .as_deref(),
            Some("0")
        );
    }

    #[test]
    fn metadata_checks_apply_even_without_numeric_input() {
        assert_eq!(parse(None, Some("JPY"), None, None), Err(5));
        assert_eq!(parse(None, None, Some(":"), None), Err(6));
        assert_eq!(parse(None, None, None, Some(",")), Err(7));
        assert_eq!(
            parse(None, Some("USD"), None, None),
            Ok((None, Some("USD".into()), None, None))
        );
        assert_eq!(
            parse(Some("  "), None, None, None),
            Ok((None, None, None, None))
        );
    }

    #[test]
    fn ambiguity_and_compound_refusals_have_defined_precedence() {
        assert_eq!(parse(Some("1.234"), None, None, None), Err(11));
        assert_eq!(parse(Some("1.2345678"), None, Some(","), None), Err(10));
        assert_eq!(parse(Some("1.234"), None, Some(","), None), Err(12));
        assert_eq!(
            parse(Some("1234567890123456789x1"), None, None, None),
            Err(9)
        );
        assert_eq!(
            parse(Some("1234567890123456789x"), None, None, None),
            Err(8)
        );
        assert_eq!(parse(Some("1.2,3"), None, None, None), Err(8));
        assert_eq!(parse(Some("1\t"), None, None, None), Err(8));
    }
}
