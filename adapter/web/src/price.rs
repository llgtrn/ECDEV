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
    use std::cmp::Ordering;
    let code = c as u64;
    lexicon()["word_ranges"]
        .as_array()
        .expect("checked Unicode word data")
        .binary_search_by(|range| {
            if code < range[0].as_u64().unwrap() {
                Ordering::Greater
            } else if code > range[1].as_u64().unwrap() {
                Ordering::Less
            } else {
                Ordering::Equal
            }
        })
        .is_ok()
}

fn literal_search<'a>(
    text: &str,
    symbols: impl Iterator<Item = &'a str>,
    accept: impl Fn(usize, &str) -> bool,
) -> Option<(usize, &'a str)> {
    let mut best: Option<(usize, usize, &'a str)> = None;
    for (order, token) in symbols.enumerate() {
        for (at, _) in text.match_indices(token) {
            if accept(at, token)
                && best.is_none_or(|(pos, priority, _)| (at, order) < (pos, priority))
            {
                best = Some((at, order, token));
            }
        }
    }
    best.map(|(at, _, token)| (at, token))
}

/// Literal union search: leftmost position, then declaration order. Offsets are UTF-8 bytes.
/// An empty union is the empty pattern, matching at the beginning like the donor helper.
pub fn literal_alternative<'a>(text: &str, symbols: &[&'a str]) -> Option<(usize, &'a str)> {
    if symbols.is_empty() {
        return Some((0, ""));
    }
    literal_search(text, symbols.iter().copied(), |_, _| true)
}

/// Currency tokens retain source spelling. They do not identify an ISO currency.
pub fn currency_symbol(price: Option<&str>, hint: Option<&str>) -> Option<String> {
    if [price, hint].into_iter().flatten().any(|s| s.len() > 4096) {
        return None;
    }
    fn search(text: &str, key: &str, dollar: bool) -> Option<String> {
        literal_search(
            text,
            lexicon()[key].as_array()?.iter().filter_map(Value::as_str),
            |at, token| {
                !dollar
                    || (!text[..at].chars().next_back().is_some_and(word)
                        && !text[at + token.len()..]
                            .chars()
                            .next()
                            .is_some_and(|c| word(c) && digit(c).is_none()))
            },
        )
        .map(|(_, token)| token.to_owned())
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PriceError {
    InvalidOperation,
    AssertionError,
    ValueError,
    ResourceLimit,
}
impl PriceError {
    pub fn name(self) -> &'static str {
        match self {
            Self::InvalidOperation => "InvalidOperation",
            Self::AssertionError => "AssertionError",
            Self::ValueError => "ValueError",
            Self::ResourceLimit => "ResourceLimit",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Special {
    Infinity,
    QuietNan,
    SignalNan,
}
/// Exact decimal atom. Storage is bounded without expanding a large exponent.
#[derive(Clone, Debug)]
pub struct DecimalAmount {
    negative: bool,
    coefficient: String,
    exponent: i64,
    special: Option<Special>,
}
impl DecimalAmount {
    pub fn parse(input: &str) -> Result<Self, PriceError> {
        if input.len() > 4096 {
            return Err(PriceError::ResourceLimit);
        }
        let text: String = input
            .trim_matches(whitespace)
            .chars()
            .filter(|c| *c != '_')
            .map(|c| digit(c).and_then(|n| char::from_digit(n, 10)).unwrap_or(c))
            .collect();
        let (negative, text) = if let Some(s) = text.strip_prefix('-') {
            (true, s)
        } else {
            (false, text.strip_prefix('+').unwrap_or(&text))
        };
        let lower = text.to_ascii_lowercase();
        if matches!(lower.as_str(), "inf" | "infinity") {
            return Ok(Self {
                negative,
                coefficient: String::new(),
                exponent: 0,
                special: Some(Special::Infinity),
            });
        }
        for (prefix, special) in [("snan", Special::SignalNan), ("nan", Special::QuietNan)] {
            if let Some(payload) = lower.strip_prefix(prefix) {
                if !payload.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(PriceError::InvalidOperation);
                }
                return Ok(Self {
                    negative,
                    coefficient: payload.trim_start_matches('0').to_owned(),
                    exponent: 0,
                    special: Some(special),
                });
            }
        }
        let mut parts = text.split(['e', 'E']);
        let mantissa = parts.next().unwrap();
        let exponent = match parts.next() {
            Some(s) => s.parse::<i64>().map_err(|_| PriceError::InvalidOperation)?,
            None => 0,
        };
        if parts.next().is_some() {
            return Err(PriceError::InvalidOperation);
        }
        let mut pieces = mantissa.split('.');
        let whole = pieces.next().unwrap();
        let fraction = pieces.next().unwrap_or("");
        if pieces.next().is_some()
            || (whole.is_empty() && fraction.is_empty())
            || !whole
                .bytes()
                .chain(fraction.bytes())
                .all(|b| b.is_ascii_digit())
        {
            return Err(PriceError::InvalidOperation);
        }
        let exponent = exponent
            .checked_sub(fraction.len() as i64)
            .ok_or(PriceError::ResourceLimit)?;
        let coefficient = format!("{whole}{fraction}");
        let coefficient = coefficient.trim_start_matches('0');
        Ok(Self {
            negative,
            coefficient: if coefficient.is_empty() {
                "0".into()
            } else {
                coefficient.into()
            },
            exponent,
            special: None,
        })
    }
    pub fn text(&self) -> String {
        let sign = if self.negative { "-" } else { "" };
        if let Some(special) = self.special {
            let name = match special {
                Special::Infinity => "Infinity",
                Special::QuietNan => "NaN",
                Special::SignalNan => "sNaN",
            };
            return format!("{sign}{name}{}", self.coefficient);
        }
        let adjusted = i128::from(self.exponent) + self.coefficient.len() as i128 - 1;
        if self.exponent > 0 || adjusted < -6 {
            let rest = &self.coefficient[1..];
            let mantissa = if rest.is_empty() {
                self.coefficient.clone()
            } else {
                format!("{}.{}", &self.coefficient[..1], rest)
            };
            return format!("{sign}{mantissa}E{adjusted:+}");
        }
        let point = self.coefficient.len() as i64 + self.exponent;
        let value = if point <= 0 {
            format!("0.{}{}", "0".repeat((-point) as usize), self.coefficient)
        } else if point as usize >= self.coefficient.len() {
            self.coefficient.clone()
        } else {
            format!(
                "{}.{}",
                &self.coefficient[..point as usize],
                &self.coefficient[point as usize..]
            )
        };
        format!("{sign}{value}")
    }
    fn float(&self) -> Result<f64, PriceError> {
        match self.special {
            Some(Special::SignalNan) => Err(PriceError::ValueError),
            Some(Special::QuietNan) => Ok(f64::NAN),
            Some(Special::Infinity) => Ok(if self.negative {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            }),
            None => self.text().parse().map_err(|_| PriceError::ValueError),
        }
    }
    fn equals(&self, other: &Self) -> Result<bool, PriceError> {
        if [self.special, other.special].contains(&Some(Special::SignalNan)) {
            return Err(PriceError::InvalidOperation);
        }
        if [self.special, other.special].contains(&Some(Special::QuietNan)) {
            return Ok(false);
        }
        if self.special.is_some() || other.special.is_some() {
            return Ok(self.special == other.special && self.negative == other.negative);
        }
        if self.coefficient == "0" && other.coefficient == "0" {
            return Ok(true);
        }
        let a = self.coefficient.trim_end_matches('0');
        let b = other.coefficient.trim_end_matches('0');
        Ok(self.negative == other.negative
            && a == b
            && i128::from(self.exponent) + (self.coefficient.len() - a.len()) as i128
                == i128::from(other.exponent) + (other.coefficient.len() - b.len()) as i128)
    }
}
fn string_repr(value: Option<&str>) -> String {
    let Some(value) = value else {
        return "None".into();
    };
    let quote = if value.contains('\'') && !value.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = quote.to_string();
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c => {
                let code = c as u64;
                let printable = lexicon()["printable_ranges"]
                    .as_array()
                    .expect("checked printability data")
                    .binary_search_by(|range| {
                        if code < range[0].as_u64().unwrap() {
                            std::cmp::Ordering::Greater
                        } else if code > range[1].as_u64().unwrap() {
                            std::cmp::Ordering::Less
                        } else {
                            std::cmp::Ordering::Equal
                        }
                    })
                    .is_ok();
                if printable {
                    out.push(c);
                } else if code <= 255 {
                    out.push_str(&format!("\\x{code:02x}"));
                } else if code <= 65535 {
                    out.push_str(&format!("\\u{code:04x}"));
                } else {
                    out.push_str(&format!("\\U{code:08x}"));
                }
            }
        }
    }
    out.push(quote);
    out
}
/// Native typed price object. Source hints stay derived; this does not create observed money.
#[derive(Clone, Debug)]
pub struct Price {
    pub amount: Option<DecimalAmount>,
    pub currency: Option<String>,
    pub amount_text: Option<String>,
}
impl Price {
    pub fn new(
        amount: Option<&str>,
        currency: Option<&str>,
        amount_text: Option<&str>,
    ) -> Result<Self, PriceError> {
        if [currency, amount_text]
            .into_iter()
            .flatten()
            .any(|s| s.len() > 4096)
        {
            return Err(PriceError::ResourceLimit);
        }
        Ok(Self {
            amount: amount.map(DecimalAmount::parse).transpose()?,
            currency: currency.map(str::to_owned),
            amount_text: amount_text.map(str::to_owned),
        })
    }
    pub fn amount_float(&self) -> Result<Option<f64>, PriceError> {
        self.amount.as_ref().map(DecimalAmount::float).transpose()
    }
    pub fn equals(&self, other: &Self) -> Result<bool, PriceError> {
        let equal = match (&self.amount, &other.amount) {
            (Some(a), Some(b)) => a.equals(b)?,
            (None, None) => true,
            _ => false,
        };
        Ok(equal && self.currency == other.currency && self.amount_text == other.amount_text)
    }
    pub fn repr(&self) -> String {
        let amount = self
            .amount
            .as_ref()
            .map(|a| format!("Decimal({})", string_repr(Some(&a.text()))))
            .unwrap_or_else(|| "None".into());
        format!(
            "Price(amount={amount}, currency={})",
            string_repr(self.currency.as_deref())
        )
    }
    pub fn fromstring(
        input: Option<&str>,
        hint: Option<&str>,
        separator: Option<char>,
        group: Option<&str>,
    ) -> Self {
        let explicit = separator.map(|c| c.to_string());
        Self::try_fromstring(input, hint, explicit.as_deref(), group).unwrap_or(Self {
            amount: None,
            currency: None,
            amount_text: None,
        })
    }
    /// Typed source API preserves invalid-selector errors. The legacy projection
    /// wrapper retains unknown output when its input is outside this boundary.
    pub fn try_fromstring(
        input: Option<&str>,
        hint: Option<&str>,
        separator: Option<&str>,
        group: Option<&str>,
    ) -> Result<Self, PriceError> {
        if [input, hint].into_iter().flatten().any(|s| s.len() > 4096)
            || group.is_some_and(|s| s.len() > 64)
        {
            return Err(PriceError::ResourceLimit);
        }
        let currency =
            currency_symbol(input, hint).map(|token| token.trim_matches(whitespace).to_owned());
        let text = input.map(|s| {
            group
                .map(|g| s.replace(g, ""))
                .unwrap_or_else(|| s.to_owned())
        });
        let amount_text = text.as_deref().and_then(price_text);
        let amount = parse_decimal_number(amount_text.as_deref(), separator)?;
        Ok(Self {
            amount,
            currency,
            amount_text,
        })
    }
    pub fn projection(&self) -> Value {
        let amount = self
            .amount
            .as_ref()
            .filter(|a| a.special.is_none())
            .and_then(|a| parse_number(&a.text(), Some('.')));
        let amount_float = amount
            .as_ref()
            .and_then(|_| self.amount_float().ok().flatten().filter(|f| f.is_finite()));
        json!({"amount":amount,"currency":self.currency,"amount_text":self.amount_text,"amount_float":amount_float})
    }
}

pub fn parse_price(
    input: Option<&str>,
    hint: Option<&str>,
    separator: Option<char>,
    group: Option<&str>,
) -> Value {
    Price::fromstring(input, hint, separator, group).projection()
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
fn number_text(input: &str, explicit: Option<char>) -> Option<String> {
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
    Some(text)
}
/// Direct typed numeric helper, with exact atoms and explicit selector/resource errors.
/// Invalid numeric strings are unavailable; nonfinite atoms remain classifiable.
pub fn parse_decimal_number(
    input: Option<&str>,
    explicit: Option<&str>,
) -> Result<Option<DecimalAmount>, PriceError> {
    let Some(input) = input.filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if input.len() > 4096 {
        return Err(PriceError::ResourceLimit);
    }
    let separator = match explicit {
        None | Some("") => None,
        Some(".") => Some('.'),
        Some(",") => Some(','),
        Some("€") => Some('€'),
        _ => return Err(PriceError::AssertionError),
    };
    let Some(text) = number_text(input, separator) else {
        return Ok(None);
    };
    match DecimalAmount::parse(&text) {
        Ok(amount) => Ok(Some(amount)),
        Err(PriceError::InvalidOperation) => Ok(None),
        Err(error) => Err(error),
    }
}
pub fn parse_number(input: &str, explicit: Option<char>) -> Option<String> {
    let text = number_text(input, explicit)?;
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
    fn locked_price_typed_scalar_oracle() {
        let oracle: Value =
            serde_json::from_str(include_str!("../tests/fixtures/price-scalar-contract.json"))
                .unwrap();
        assert_eq!(
            oracle["commit_sha"],
            "64e213a46a40473ba4f8aa3b249917fdc64d8a16"
        );
        assert_eq!(oracle["cases"].as_array().unwrap().len(), 1134);
        for case in oracle["cases"].as_array().unwrap() {
            match parse_decimal_number(case["input"].as_str(), case["decimal_separator"].as_str()) {
                Ok(amount) => {
                    assert!(case["error"].is_null(), "{}", case);
                    assert_eq!(
                        json!(amount.as_ref().map(DecimalAmount::text)),
                        case["expected"],
                        "{}",
                        case
                    );
                }
                Err(error) => assert_eq!(json!(error.name()), case["error"], "{}", case),
            }
        }
    }
    #[test]
    fn typed_price_selector_errors_and_resource_unknowns_are_explicit() {
        assert!(matches!(
            Price::try_fromstring(Some("$12"), None, Some("xx"), None),
            Err(PriceError::AssertionError)
        ));
        assert!(
            Price::try_fromstring(Some("foo"), None, Some("xx"), None)
                .unwrap()
                .amount
                .is_none()
        );
        assert!(parse_decimal_number(None, Some("xx")).unwrap().is_none());
        assert!(matches!(
            parse_decimal_number(Some(" \t"), Some("xx")),
            Err(PriceError::AssertionError)
        ));
        assert!(matches!(
            parse_decimal_number(Some(&"1".repeat(4097)), None),
            Err(PriceError::ResourceLimit)
        ));
        for raw in ["NaN", "Infinity", "sNaN", "1E-99999", "1E+99999"] {
            let price = Price {
                amount: parse_decimal_number(Some(raw), None).unwrap(),
                currency: Some("USD".into()),
                amount_text: Some(raw.into()),
            };
            assert!(price.projection()["amount"].is_null());
            assert!(price.projection()["amount_float"].is_null());
        }
    }
    fn object_from_input(input: &Value) -> Result<Price, PriceError> {
        Price::new(
            input["amount_decimal_input"].as_str(),
            input["currency"].as_str(),
            input["amount_text"].as_str(),
        )
    }
    fn classified_float(value: Result<Option<f64>, PriceError>) -> Value {
        match value {
            Err(error) => json!({"classification":"ERROR","error":error.name()}),
            Ok(None) => json!({"classification":"NULL","value":null}),
            Ok(Some(value)) => {
                json!({"classification":if value.is_nan(){"NAN"}else if value==f64::NEG_INFINITY{"NEGATIVE_INFINITY"}else if value.is_infinite(){"POSITIVE_INFINITY"}else{"FINITE"},"value":if value.is_finite(){json!(value)}else{Value::Null}})
            }
        }
    }
    #[test]
    fn locked_price_object_contract_oracle() {
        let oracle: Value =
            serde_json::from_str(include_str!("../tests/fixtures/price-object-contract.json"))
                .unwrap();
        assert_eq!(
            oracle["commit_sha"],
            "64e213a46a40473ba4f8aa3b249917fdc64d8a16"
        );
        assert_eq!(oracle["construction"].as_array().unwrap().len(), 1083);
        assert_eq!(oracle["comparisons"].as_array().unwrap().len(), 36);
        for case in oracle["construction"].as_array().unwrap() {
            match object_from_input(&case["input"]) {
                Err(error) => assert_eq!(json!(error.name()), case["error"], "{}", case),
                Ok(price) => {
                    assert!(case["error"].is_null(), "{}", case);
                    let expected = &case["expected"];
                    assert_eq!(
                        json!(price.amount.as_ref().map(DecimalAmount::text)),
                        expected["amount_decimal_text"],
                        "{}",
                        case
                    );
                    assert_eq!(json!(price.currency), expected["currency"], "{}", case);
                    assert_eq!(
                        json!(price.amount_text),
                        expected["amount_text"],
                        "{}",
                        case
                    );
                    assert_eq!(json!(price.repr()), expected["repr"], "{}", case);
                    let floating = classified_float(price.amount_float());
                    assert_eq!(
                        floating["classification"], expected["amount_float"]["classification"],
                        "{}",
                        case
                    );
                    if floating["classification"] == "FINITE" {
                        // Compare the typed float exactly, including signed zero. Python and
                        // serde spell the same JSON number as e-07 and e-7 respectively.
                        assert_eq!(
                            floating["value"].as_f64().unwrap().to_bits(),
                            expected["amount_float"]["value"]
                                .as_f64()
                                .unwrap()
                                .to_bits(),
                            "{}",
                            case
                        );
                    } else {
                        assert_eq!(floating, expected["amount_float"], "{}", case);
                    }
                }
            }
        }
        for case in oracle["comparisons"].as_array().unwrap() {
            let a = object_from_input(&case["left"]).unwrap();
            let b = object_from_input(&case["right"]).unwrap();
            match a.equals(&b) {
                Ok(equal) => {
                    assert!(case["error"].is_null(), "{}", case);
                    assert_eq!(json!(equal), case["expected"], "{}", case);
                }
                Err(error) => assert_eq!(json!(error.name()), case["error"], "{}", case),
            }
        }
    }
    #[test]
    fn native_price_object_projection_preserves_nonfinite_and_resource_unknowns() {
        for raw in [
            "NaN",
            "Infinity",
            "-Infinity",
            "sNaN",
            "1E+99999",
            "1E-99999",
        ] {
            let price = Price::new(Some(raw), Some("USD"), Some("original")).unwrap();
            let projection = price.projection();
            assert!(projection["amount"].is_null() && projection["amount_float"].is_null());
            assert_eq!(projection["currency"], "USD");
            assert_eq!(projection["amount_text"], "original");
        }
        assert!(matches!(
            Price::new(Some(&"1".repeat(4097)), None, None),
            Err(PriceError::ResourceLimit)
        ));
        assert!(Price::fromstring(Some(&"1".repeat(4097)), Some("USD"), None, None).projection()["amount"].is_null());
        let a = Price::new(Some("12.00"), Some("USD"), Some("raw")).unwrap();
        let b = Price::new(Some("12"), Some("USD"), Some("different")).unwrap();
        assert!(!a.equals(&b).unwrap());
        assert_eq!(a.repr(), "Price(amount=Decimal('12.00'), currency='USD')");
        assert_eq!(
            Price::fromstring(Some("$12.00"), None, None, None).repr(),
            "Price(amount=Decimal('12.00'), currency='$')"
        );
    }
    #[test]
    fn locked_price_source_helpers_oracle() {
        let oracle: Value =
            serde_json::from_str(include_str!("../tests/fixtures/price-source-helpers.json"))
                .unwrap();
        assert_eq!(
            oracle["commit_sha"],
            "64e213a46a40473ba4f8aa3b249917fdc64d8a16"
        );
        assert!(oracle["cases"].as_array().unwrap().len() > 1000);
        for case in oracle["cases"].as_array().unwrap() {
            let actual = match case["operation"].as_str().unwrap() {
                "currency" => currency_symbol(case["input"].as_str(), case["hint"].as_str()),
                "text" => price_text(case["input"].as_str().unwrap()),
                _ => panic!("unknown oracle operation"),
            };
            assert_eq!(json!(actual), case["expected"], "{}", case);
        }
    }
    #[test]
    fn locked_price_literal_helper_oracle() {
        let oracle: Value =
            serde_json::from_str(include_str!("../tests/fixtures/price-literal-helper.json"))
                .unwrap();
        assert_eq!(
            oracle["commit_sha"],
            "64e213a46a40473ba4f8aa3b249917fdc64d8a16"
        );
        assert_eq!(oracle["cases"].as_array().unwrap().len(), 1576);
        for case in oracle["cases"].as_array().unwrap() {
            let symbols: Vec<_> = case["symbols"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect();
            let actual = literal_alternative(case["input"].as_str().unwrap(), &symbols).map(
                |(at, token)| json!({"start_byte":at,"end_byte":at+token.len(),"matched":token}),
            );
            assert_eq!(json!(actual), case["expected"], "{}", case);
        }
    }
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
