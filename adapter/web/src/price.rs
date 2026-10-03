//! Exact formatted numeric amounts. Separator guessing is a disclosed heuristic.
//! Reviewed against locked price-parser; BSD notice retained with oracle fixtures.
use serde_json::{Value, json};

pub fn decimal_separator(input: &str) -> Option<char> {
    let (index, separator) = input
        .char_indices()
        .rev()
        .find(|(_, c)| matches!(c, '.' | ',' | '€'))?;
    let tail = &input[index + separator.len_utf8()..];
    let count = tail.len();
    (tail.bytes().all(|b| b.is_ascii_digit()) && (count == 1 || count == 2 || count >= 4))
        .then_some(separator)
}

/// Decimal strings are canonicalized without binary floating point or a fixed precision.
/// Output growth is bounded; nonfinite values and non-ASCII numeric alphabets are unavailable.
pub fn parse_number(input: &str, explicit: Option<char>) -> Option<String> {
    if input.len() > 4096 {
        return None;
    }
    let input = input.trim().replace(' ', "");
    if input.is_empty() {
        return None;
    }
    let separator = explicit.or_else(|| decimal_separator(&input));
    let text = match separator {
        None => input.replace(['.', ','], ""),
        Some('.') => input.replace(',', ""),
        Some(',') => input.replace('.', "").replace(',', "."),
        Some('€') => input.replace(['.', ','], "").replace('€', "."),
        _ => return None,
    }
    .replace('_', "");
    let (negative, text) = if let Some(rest) = text.strip_prefix('-') {
        (true, rest)
    } else {
        (false, text.strip_prefix('+').unwrap_or(&text))
    };
    let parts: Vec<_> = text.split(['e', 'E']).collect();
    if parts.len() > 2 {
        return None;
    }
    let exponent: i32 = if parts.len() == 2 {
        parts[1].parse().ok()?
    } else {
        0
    };
    if exponent.unsigned_abs() > 4096 {
        return None;
    }
    let mantissa = parts[0];
    let pieces: Vec<_> = mantissa.split('.').collect();
    if pieces.len() > 2 {
        return None;
    }
    let whole = pieces[0];
    let fraction = pieces.get(1).copied().unwrap_or("");
    if whole.is_empty() && fraction.is_empty() {
        return None;
    }
    if !whole
        .bytes()
        .chain(fraction.bytes())
        .all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let digits = format!("{whole}{fraction}");
    let point = i32::try_from(whole.len()).ok()?.checked_add(exponent)?;
    let output = if point <= 0 {
        format!("0.{}{digits}", "0".repeat((-point) as usize))
    } else if point as usize >= digits.len() {
        format!("{digits}{}", "0".repeat(point as usize - digits.len()))
    } else {
        format!(
            "{}.{}",
            &digits[..point as usize],
            &digits[point as usize..]
        )
    };
    if output.len() > 8192 {
        return None;
    }
    let (whole, fraction) = output.split_once('.').unwrap_or((&output, ""));
    let whole = whole.trim_start_matches('0');
    let whole = if whole.is_empty() { "0" } else { whole };
    let fraction = fraction.trim_end_matches('0');
    let amount = if fraction.is_empty() {
        whole.to_string()
    } else {
        format!("{whole}.{fraction}")
    };
    Some(if negative && amount != "0" {
        format!("-{amount}")
    } else {
        amount
    })
}

pub fn formatted_money(input: &str, currency: &str) -> Value {
    let amount = parse_number(input, None);
    let minor = amount
        .as_ref()
        .and_then(|s| crate::money(&json!(s), currency));
    json!({"amount_decimal":amount,"minor":minor,"currency":currency,"raw_text":input,
        "decimal_separator":decimal_separator(&input.trim().replace(' ', "")),
        "interpretation":"NUMERIC_FORMAT_HEURISTIC_WITH_EXPLICIT_CURRENCY_NO_SYMBOL_INFERENCE",
        "conversion":"Exact supported currency minor units; unsupported precision and overflow unavailable"})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn locked_price_parser_numeric_oracle() {
        let oracle: Value =
            serde_json::from_str(include_str!("../tests/fixtures/price-number.json")).unwrap();
        assert_eq!(
            oracle["commit_sha"],
            "64e213a46a40473ba4f8aa3b249917fdc64d8a16"
        );
        assert_eq!(oracle["cases"].as_array().unwrap().len(), 1235);
        for case in oracle["cases"].as_array().unwrap() {
            let separator = case["decimal_separator"]
                .as_str()
                .and_then(|s| s.chars().next());
            assert_eq!(
                json!(parse_number(case["input"].as_str().unwrap(), separator)),
                case["expected"],
                "{}",
                case["name"]
            );
        }
    }
    #[test]
    fn precision_and_ambiguous_currency_do_not_invent_money() {
        assert_eq!(formatted_money("2,980", "JPY")["minor"], 2980);
        assert_eq!(formatted_money("12,99", "EUR")["minor"], 1299);
        assert!(formatted_money("2.98", "JPY")["minor"].is_null());
        assert!(formatted_money("12.99", "$")["minor"].is_null());
        assert!(formatted_money("-12.99", "USD")["minor"].is_null());
        assert!(formatted_money("999999999999999999999", "JPY")["minor"].is_null());
        assert!(parse_number("1e999999999", None).is_none());
    }
}
