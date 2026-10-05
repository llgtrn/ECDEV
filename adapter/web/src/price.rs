//! Exact formatted numeric amounts. Separator guessing is a disclosed heuristic.
//! Reviewed against locked price-parser; BSD notice retained with oracle fixtures.
use serde_json::{Value, json};
use std::sync::OnceLock;

fn lexicon() -> &'static Value {
    static DATA: OnceLock<Value> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(include_str!("../data/price-currency.json"))
            .expect("checked licensed currency data")
    })
}
fn digit(c: char) -> Option<u32> {
    if c.is_ascii() {
        return c.to_digit(10);
    }
    let code = c as u32;
    lexicon()["decimal_digit_starts"]
        .as_array()?
        .iter()
        .find_map(|start| {
            let start = start.as_u64()? as u32;
            (code >= start && code < start + 10).then(|| code - start)
        })
}
fn whitespace(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}
fn word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Currency tokens retain source spelling. They do not identify an ISO currency.
pub fn currency_symbol(price: Option<&str>, hint: Option<&str>) -> Option<String> {
    if [price, hint].into_iter().flatten().any(|s| s.len() > 4096) {
        return None;
    }
    fn search(text: &str, key: &str, dollar: bool) -> Option<String> {
        lexicon()[key]
            .as_array()?
            .iter()
            .enumerate()
            .flat_map(|(order, token)| {
                let token = token.as_str().unwrap();
                text.match_indices(token)
                    .filter(move |(at, _)| {
                        !dollar
                            || ((!text[..*at].chars().next_back().is_some_and(word))
                                && !text[*at + token.len()..]
                                    .chars()
                                    .next()
                                    .is_some_and(|c| word(c) && digit(c).is_none()))
                    })
                    .map(move |(at, _)| (at, order, token))
            })
            .min_by_key(|(at, order, _)| (*at, *order))
            .map(|(_, _, token)| token.trim_matches(whitespace).to_owned())
    }
    for text in [price, hint]
        .into_iter()
        .flatten()
        .filter(|s| s.contains('$'))
    {
        if let Some(token) = search(text, "dollar_codes", true) {
            return Some(token);
        }
    }
    for key in ["safe", "unsafe"] {
        for text in [price, hint].into_iter().flatten() {
            if let Some(token) = search(text, key, false) {
                return Some(token);
            }
        }
    }
    None
}

/// First price-like segment, with the donor's disclosed free/euro/percent heuristics.
pub fn price_text(input: &str) -> Option<String> {
    if input.len() > 4096 {
        return None;
    }
    let mut text = String::new();
    let mut in_space = false;
    for c in input.chars() {
        if whitespace(c) {
            if !in_space {
                text.push(' ');
            }
            in_space = true;
        } else {
            text.push(c);
            in_space = false;
        }
    }
    let chars: Vec<char> = text.chars().collect();
    let number_char = |c: char| digit(c).is_some() || matches!(c, ' ' | '.' | ',' | '\'');
    if chars.iter().filter(|c| **c == '€').count() == 1 {
        let euro = chars.iter().position(|c| *c == '€').unwrap();
        let mut after = euro + 1;
        while chars.get(after) == Some(&' ') {
            after += 1;
        }
        let digits_start = after;
        while chars.get(after).is_some_and(|c| digit(*c).is_some()) {
            after += 1;
        }
        let digits = after - digits_start;
        if digits > 0 && (digits_start == euro + 1 || digits == 2) {
            for start in 0..euro {
                let prefix = &chars[start..euro];
                if prefix.iter().copied().all(number_char)
                    && prefix
                        .iter()
                        .rev()
                        .find(|c| **c != ' ')
                        .is_some_and(|c| digit(*c).is_some())
                {
                    let end = (after + usize::from(after < chars.len())).min(chars.len());
                    return Some(chars[start..end].iter().filter(|c| **c != ' ').collect());
                }
            }
        }
    }
    for start in 0..chars.len() {
        let first_digit = start + usize::from(chars[start] == '.');
        if chars.get(first_digit).is_none_or(|c| digit(*c).is_none()) {
            continue;
        }
        let mut end = first_digit + 1;
        while chars.get(end).is_some_and(|c| number_char(*c)) {
            end += 1;
        }
        for stop in (first_digit + 1..=end).rev() {
            if chars
                .get(stop)
                .is_none_or(|c| *c != '%' && digit(*c).is_none())
            {
                let segment: String = chars[start..stop].iter().collect();
                let segment = segment.trim_end_matches([',', '.']).replace('\'', "");
                let segment = if segment.matches('.').count() == 1 {
                    segment.as_str()
                } else {
                    segment.trim_start_matches([',', '.'])
                };
                return Some(segment.trim_matches(whitespace).to_owned());
            }
        }
    }
    text.to_lowercase().contains("free").then(|| "0".into())
}

pub fn parse_price(
    input: Option<&str>,
    hint: Option<&str>,
    separator: Option<char>,
    group: Option<&str>,
) -> Value {
    if [input, hint].into_iter().flatten().any(|s| s.len() > 4096)
        || group.is_some_and(|s| s.len() > 64)
    {
        return json!({"amount":null,"currency":null,"amount_text":null,"amount_float":null});
    }
    let currency = currency_symbol(input, hint);
    let text = input.map(|s| {
        if let Some(group) = group {
            s.replace(group, "")
        } else {
            s.to_owned()
        }
    });
    let amount_text = text.as_deref().and_then(price_text);
    let amount = amount_text
        .as_deref()
        .and_then(|s| parse_number(s, separator));
    let amount_float = amount
        .as_deref()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|f| f.is_finite());
    json!({"amount":amount,"currency":currency,"amount_text":amount_text,"amount_float":amount_float})
}

pub fn decimal_separator(input: &str) -> Option<char> {
    // Python's donor regex `$` accepts exactly one final LF, including after digits.
    let input = input.strip_suffix('\n').unwrap_or(input);
    let (index, separator) = input
        .char_indices()
        .rev()
        .find(|(_, c)| matches!(c, '.' | ',' | '€'))?;
    let tail = &input[index + separator.len_utf8()..];
    let count = tail.chars().count();
    (tail.chars().all(|c| digit(c).is_some()) && (count == 1 || count == 2 || count >= 4))
        .then_some(separator)
}

/// Decimal strings are canonicalized without binary floating point or a fixed precision.
/// Output growth is bounded; nonfinite values and non-ASCII numeric alphabets are unavailable.
pub fn parse_number(input: &str, explicit: Option<char>) -> Option<String> {
    if input.len() > 4096 {
        return None;
    }
    let input: String = input
        .trim_matches(whitespace)
        .replace(' ', "")
        .chars()
        .map(|c| digit(c).and_then(|n| char::from_digit(n, 10)).unwrap_or(c))
        .collect();
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
    fn locked_price_decimal_helper_oracle() {
        let oracle: Value =
            serde_json::from_str(include_str!("../tests/fixtures/price-decimal-helper.json"))
                .unwrap();
        assert_eq!(
            oracle["commit_sha"],
            "64e213a46a40473ba4f8aa3b249917fdc64d8a16"
        );
        let alphabets = oracle["decimal_alphabets"].as_u64().unwrap() as usize;
        assert_eq!(
            oracle["cases"].as_array().unwrap().len(),
            alphabets * 3 * 4 * 5 + 14
        );
        assert!(alphabets >= 60);
        for case in oracle["cases"].as_array().unwrap() {
            assert_eq!(
                json!(decimal_separator(case["input"].as_str().unwrap())),
                case["expected"],
                "{}",
                case["input"]
            );
        }
    }
    #[test]
    fn locked_price_public_api_oracle() {
        let oracle: Value =
            serde_json::from_str(include_str!("../tests/fixtures/price-contract.json")).unwrap();
        assert_eq!(oracle["cases"].as_array().unwrap().len(), 2937);
        let mut mismatches = vec![];
        for case in oracle["cases"].as_array().unwrap() {
            let actual = parse_price(
                case["input"].as_str(),
                case["currency_hint"].as_str(),
                case["decimal_separator"]
                    .as_str()
                    .and_then(|s| s.chars().next()),
                case["digit_group_separator"].as_str(),
            );
            if actual != case["expected"] {
                mismatches.push(json!({"case":case,"actual":actual}));
            }
        }
        assert!(
            mismatches.is_empty(),
            "{} mismatches; first cases: {}",
            mismatches.len(),
            json!(mismatches.into_iter().take(10).collect::<Vec<_>>())
        );
    }
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
        assert_eq!(parse_number("１２３.４５", None), Some("123.45".into()));
        assert_eq!(
            parse_price(Some("$12.99"), Some("USD"), None, None)["currency"],
            "$"
        );
        assert!(parse_price(Some(&"1".repeat(4097)), None, None, None)["amount"].is_null());
        assert_eq!(
            parse_price(Some("50%"), None, None, None)["amount"],
            Value::Null
        );
    }
}
