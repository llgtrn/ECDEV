//! Receiver-owned discriminating expectations for the finite source contracts.
//! A standalone driver provides `crate::exact` and `crate::ascii` from exact delivered files.
//! The receiver driver adapts its actual public kernels to those same function signatures.
//! These expectations are not manufactured from the candidate implementation.
use crate::{ascii, exact};

type Fields = (Option<String>, Option<String>, Option<String>, Option<f64>);
fn text(
    input: Option<&str>,
    hint: Option<&str>,
    decimal: Option<&str>,
    group: Option<&str>,
) -> Result<Fields, u8> {
    ascii::parse(input, hint, decimal, group)
}

#[test]
fn exact_receiver_literals_and_locale_interpretation() {
    for (input, currency, locale, minor) in [
        ("  6,371.25  ", "USD", "en-US", 637125),
        ("6.371,25", "EUR", "de-DE", 637125),
        ("6.371", "VND", "vi-VN", 6371),
        ("1.025", "KWD", "en-US", 1025),
        ("1.025", "KWD", "de-DE", 1025000),
        ("09,087.04", "USD", "en-US", 908704),
        ("000001.5", "USD", "en-US", 150),
        ("90071992547410.19", "USD", "en-US", 9007199254741019),
    ] {
        assert_eq!(
            exact::parse(input.as_bytes(), currency, locale),
            Ok(minor),
            "{input}"
        );
    }
    for currency in ["USD", "EUR", "JPY", "KWD", "VND"] {
        let scale = match currency {
            "JPY" | "VND" => 1,
            "KWD" => 1000,
            _ => 100,
        };
        for locale in ["en-US", "de-DE", "vi-VN"] {
            assert_eq!(exact::parse(b"23", currency, locale), Ok(23 * scale));
        }
    }
}

#[test]
fn exact_refusal_order_is_part_of_contract() {
    assert_eq!(exact::parse(&vec![255; 4097], "", ""), Err(6));
    assert_eq!(exact::parse(&[255], "", ""), Err(1));
    assert_eq!(exact::parse(&[255], "usd", ""), Err(2));
    assert_eq!(exact::parse(&[255], "USD", ""), Err(3));
    assert_eq!(exact::parse(&[255], "USD", "EN-us"), Err(4));
    assert_eq!(exact::parse(&[255], "USD", "en-US"), Err(7));
    assert_eq!(exact::parse(b"  ", "USD", "en-US"), Err(5));
    assert_eq!(exact::parse(b"1,22.000", "USD", "en-US"), Err(9));
    assert_eq!(
        exact::parse(b"99999999999999999999999999999.000", "USD", "en-US"),
        Err(10)
    );
    assert_eq!(exact::parse(b"1,22.0x", "USD", "en-US"), Err(8));
}

#[test]
fn exact_no_salvage_or_rounding_or_group_guessing() {
    for input in [
        "+1", "-1", "1e2", "$1", "1 USD", "1 2", "\t1", "1\n", "１", "\u{a0}1", ".1", "1.",
        "1.2.3", "1,2.3.4",
    ] {
        assert_eq!(
            exact::parse(input.as_bytes(), "USD", "en-US"),
            Err(8),
            "{input:?}"
        );
    }
    for input in ["1,23", "1234,567", ",123", "123,", "1,,234", "1,234,56"] {
        assert_eq!(
            exact::parse(input.as_bytes(), "USD", "en-US"),
            Err(9),
            "{input}"
        );
    }
    for (input, currency) in [("1.000", "USD"), ("1.0", "JPY"), ("1.0000", "KWD")] {
        assert_eq!(exact::parse(input.as_bytes(), currency, "en-US"), Err(10));
    }
    for currency in ["usd", " USD", "USD ", "$", "XXX"] {
        assert_eq!(exact::parse(b"1", currency, "en-US"), Err(2));
    }
    for locale in ["en-us", "en", " en-US", "de_DE"] {
        assert_eq!(exact::parse(b"1", "USD", locale), Err(4));
    }
}

#[test]
fn exact_boundaries_are_checked_after_scaling_without_leading_zero_overflow() {
    for (maximum, next, currency) in [
        ("9223372036854775807", "9223372036854775808", "JPY"),
        ("92233720368547758.07", "92233720368547758.08", "USD"),
        ("9223372036854775.807", "9223372036854775.808", "KWD"),
    ] {
        assert_eq!(
            exact::parse(maximum.as_bytes(), currency, "en-US"),
            Ok(i64::MAX)
        );
        assert_eq!(exact::parse(next.as_bytes(), currency, "en-US"), Err(11));
    }
    let mut long = vec![b'0'; 4096];
    long[4095] = b'7';
    assert_eq!(exact::parse(&long, "USD", "en-US"), Ok(700));
    assert_eq!(exact::parse(&vec![b'9'; 4096], "JPY", "en-US"), Err(11));
    assert_eq!(exact::parse(b"1", &"X".repeat(33), "en-US"), Err(2));
    assert_eq!(exact::parse(b"1", "USD", &"x".repeat(33)), Err(4));
}

#[test]
fn exact_independent_integer_formatting_witness() {
    let mut seed = 0x5cb9_4a63_2d17_c0e1_u64;
    for scale in [0_u32, 2, 3] {
        let currency = match scale {
            0 => "JPY",
            2 => "USD",
            _ => "KWD",
        };
        let factor = 10_i64.pow(scale);
        for _ in 0..256 {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let minor = (seed & i64::MAX as u64) as i64;
            let number = if scale == 0 {
                (minor / factor).to_string()
            } else {
                format!(
                    "{}.{:0width$}",
                    minor / factor,
                    minor % factor,
                    width = scale as usize
                )
            };
            assert_eq!(
                exact::parse(number.as_bytes(), currency, "en-US"),
                Ok(minor)
            );
            assert_eq!(
                exact::parse(number.replace('.', ",").as_bytes(), currency, "de-DE"),
                Ok(minor)
            );
        }
    }
}

#[test]
fn ascii_nullable_fields_and_presence_do_not_infer_currency() {
    for input in [None, Some(""), Some("  ")] {
        for (hint, expected) in [
            (None, None),
            (Some(""), None),
            (Some("$"), Some("$")),
            (Some("USD"), Some("USD")),
            (Some("EUR"), Some("EUR")),
        ] {
            for decimal in [None, Some(""), Some("."), Some(",")] {
                assert_eq!(
                    text(input, hint, decimal, None),
                    Ok((None, expected.map(str::to_owned), None, None))
                );
            }
        }
    }
    assert_eq!(text(None, Some("usd"), None, None), Err(5));
    assert_eq!(text(None, None, Some("auto"), None), Err(6));
    assert_eq!(text(None, None, None, Some(",")), Err(7));
}

#[test]
fn ascii_exact_field_literals_keep_original_spelling_and_rounded_float_separate() {
    for (input, selector, amount, spelling, float) in [
        (
            "  000873,1250  ",
            Some(","),
            "873.125",
            "000873,1250",
            873.125,
        ),
        ("0062.50", None, "62.5", "0062.50", 62.5),
        ("000.000000", Some("."), "0", "000.000000", 0.0),
        ("0,015625", Some(","), "0.015625", "0,015625", 0.015625),
        (
            "9007199254741017",
            None,
            "9007199254741017",
            "9007199254741017",
            9007199254741016.0,
        ),
    ] {
        assert_eq!(
            text(Some(input), Some("EUR"), selector, Some("")),
            Ok((
                Some(amount.to_owned()),
                Some("EUR".to_owned()),
                Some(spelling.to_owned()),
                Some(float)
            ))
        );
    }
}

#[test]
fn ascii_refusal_precedence_and_distinct_resource_limits() {
    assert_eq!(
        text(Some(&"x".repeat(4097)), Some(&"x".repeat(4097)), None, None),
        Err(1)
    );
    assert_eq!(
        text(None, Some(&"x".repeat(4097)), None, Some(&"x".repeat(65))),
        Err(2)
    );
    assert_eq!(
        text(None, Some("bad"), Some("bad"), Some(&"x".repeat(65))),
        Err(3)
    );
    assert_eq!(text(None, Some(&"x".repeat(65)), Some("bad"), None), Err(4));
    assert_eq!(text(None, Some("bad"), Some("bad"), Some("bad")), Err(5));
    assert_eq!(text(None, None, Some("bad"), Some("bad")), Err(6));
    assert_eq!(text(Some("1e2"), None, None, Some("bad")), Err(7));
    assert_eq!(text(Some("1.1234567"), None, Some(","), None), Err(10));
    assert_eq!(text(Some("1.123"), None, Some(","), None), Err(12));
    assert_eq!(text(Some("1.123"), None, None, None), Err(11));
    assert_eq!(
        text(Some("1234567890123456789x1"), None, None, None),
        Err(9)
    );
    assert_eq!(text(Some("1234567890123456789x"), None, None, None), Err(8));
}

#[test]
fn ascii_unsupported_domain_is_not_an_all_null_result() {
    for input in [
        "-1", "+1", "1e2", "$1", "1 USD", "1,234.5", "1.234,5", "1..2", "1,,2", "1 2", "\t1",
        "1\n", "１", "\u{a0}1", ".1", "1.",
    ] {
        assert_eq!(text(Some(input), None, None, None), Err(8), "{input:?}");
    }
    for input in ["1.125", "1,125"] {
        assert_eq!(text(Some(input), None, None, None), Err(11));
    }
    assert_eq!(
        text(Some("1"), None, Some(","), None),
        Ok((Some("1".into()), None, Some("1".into()), Some(1.0)))
    );
    assert!(text(Some(&"9".repeat(18)), None, None, None).is_ok());
    assert_eq!(text(Some(&"9".repeat(19)), None, None, None), Err(9));
}
