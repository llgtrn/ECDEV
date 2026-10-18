//! Strict, source-manufactured numeric interpretation for the receiving price contract.
//! Original implementation for owner-authorized component delivery; no donor parser/data.
//! The result is derived nonnegative minor units, not an observed price or financial effect.

/// Interpret the complete bounded ASCII payload under explicit currency and locale.
/// Error codes belong to the identified receiving contract, not a new error vocabulary.
pub fn parse(input: &[u8], currency: &str, locale: &str) -> Result<i64, u8> {
    if input.len() > 4096 {
        return Err(6);
    }
    let scale = match currency {
        "" => return Err(1),
        "USD" | "EUR" => 2,
        "JPY" | "VND" => 0,
        "KWD" => 3,
        _ => return Err(2),
    };
    let (decimal, group) = match locale {
        "" => return Err(3),
        "en-US" => (b'.', b','),
        "de-DE" | "vi-VN" => (b',', b'.'),
        _ => return Err(4),
    };
    std::str::from_utf8(input).map_err(|_| 7)?;
    let start = input.iter().position(|&b| b != b' ').unwrap_or(input.len());
    let end = input
        .iter()
        .rposition(|&b| b != b' ')
        .map_or(start, |i| i + 1);
    let number = &input[start..end];
    if number.is_empty() {
        return Err(5);
    }
    if number
        .iter()
        .any(|b| !b.is_ascii_digit() && *b != decimal && *b != group)
    {
        return Err(8);
    }
    let mut halves = number.split(|&b| b == decimal);
    let whole = halves.next().unwrap();
    let fraction = halves.next();
    if halves.next().is_some() || whole.is_empty() || fraction.is_some_and(|s| s.is_empty()) {
        return Err(8);
    }
    if fraction.is_some_and(|s| !s.iter().all(u8::is_ascii_digit)) {
        return Err(8);
    }
    if whole.contains(&group) {
        for (index, part) in whole.split(|&b| b == group).enumerate() {
            if (index == 0 && !(1..=3).contains(&part.len())) || (index != 0 && part.len() != 3) {
                return Err(9);
            }
        }
    }
    let fraction = fraction.unwrap_or_default();
    if fraction.len() > scale {
        return Err(10);
    }
    let mut units = 0_i64;
    for &digit in whole.iter().chain(fraction).filter(|&&b| b != group) {
        units = units
            .checked_mul(10)
            .and_then(|v| v.checked_add(i64::from(digit - b'0')))
            .ok_or(11)?;
    }
    for _ in fraction.len()..scale {
        units = units.checked_mul(10).ok_or(11)?;
    }
    Ok(units)
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn decimal_places_are_exact_and_never_rounded() {
        assert_eq!(parse(b" 7,654.32 ", "USD", "en-US"), Ok(765432));
        assert_eq!(parse(b"7.654,32", "EUR", "de-DE"), Ok(765432));
        assert_eq!(parse(b"1.2", "KWD", "en-US"), Ok(1200));
        assert_eq!(parse(b"1.000", "USD", "en-US"), Err(10));
        assert_eq!(parse(b"1.0", "JPY", "en-US"), Err(10));
    }

    #[test]
    fn limits_and_metadata_precede_payload_interpretation() {
        assert_eq!(parse(&vec![0xff; 4097], "", ""), Err(6));
        assert_eq!(parse(&[0xff], "", ""), Err(1));
        assert_eq!(parse(&[0xff], "USD", ""), Err(3));
        assert_eq!(parse(&[0xff], "USD", "en-US"), Err(7));
        assert_eq!(parse(b"  ", "USD", "en-US"), Err(5));
    }

    #[test]
    fn minor_unit_limit_and_group_structure_are_checked() {
        assert_eq!(parse(b"92233720368547758.07", "USD", "en-US"), Ok(i64::MAX));
        assert_eq!(parse(b"92233720368547758.08", "USD", "en-US"), Err(11));
        assert_eq!(
            parse(b"00000000000000000000000001", "USD", "en-US"),
            Ok(100)
        );
        for input in [b"1,23".as_slice(), b",123", b"1234,567", b"1,,234"] {
            assert_eq!(parse(input, "USD", "en-US"), Err(9));
        }
        for input in [b"+1".as_slice(), b"1\t", b"1 000", b".1", b"1.", b"1.2.3"] {
            assert_eq!(parse(input, "USD", "en-US"), Err(8));
        }
    }
}
