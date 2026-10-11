//! JSON numbers that keep their text, so no digit of a number read from a document is lost:
//! a 25-digit identifier, `1.10` and `1E5` serialise as they were written.

use std::fmt;

#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Number {
    text: String,
}

impl Number {
    /// Wraps text that is already a valid JSON number (the parser checks it).
    pub(crate) fn from_valid_text(text: &str) -> Number {
        Number {
            text: text.to_string(),
        }
    }

    pub fn from_f64(value: f64) -> Option<Number> {
        value.is_finite().then(|| Number {
            text: format_f64(value),
        })
    }

    /// The number exactly as written.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    pub fn as_i64(&self) -> Option<i64> {
        self.text.parse().ok()
    }

    pub fn as_u64(&self) -> Option<u64> {
        self.text.parse().ok()
    }

    pub fn as_i128(&self) -> Option<i128> {
        self.text.parse().ok()
    }

    pub fn as_u128(&self) -> Option<u128> {
        self.text.parse().ok()
    }

    pub fn as_f64(&self) -> Option<f64> {
        self.text.parse::<f64>().ok().filter(|f| f.is_finite())
    }

    pub fn is_i64(&self) -> bool {
        self.as_i64().is_some()
    }

    pub fn is_u64(&self) -> bool {
        self.as_u64().is_some()
    }

    /// True for a number written with a fraction or an exponent that fits a finite `f64`.
    pub fn is_f64(&self) -> bool {
        self.text.contains(['.', 'e', 'E']) && self.as_f64().is_some()
    }
}

impl fmt::Display for Number {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl fmt::Debug for Number {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Number({})", self.text)
    }
}

macro_rules! from_integer {
    ($($t:ty),*) => {$(
        impl From<$t> for Number {
            fn from(n: $t) -> Number {
                Number { text: n.to_string() }
            }
        }
    )*};
}
from_integer!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize
);

/// Shortest round-trip decimal text of a finite `f64`: fixed notation for decimal exponents from -5
/// to 15 (always with a fraction, `3.0`), scientific otherwise with a signed exponent (`1e+16`,
/// `1.5e-7`).
pub(crate) fn format_f64(value: f64) -> String {
    debug_assert!(value.is_finite());
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0.0".into()
        } else {
            "0.0".into()
        };
    }
    // `{:e}` yields the shortest round-trip digits as `d.ddde<exp>`.
    let sci = format!("{:e}", value.abs());
    let (mantissa, exp) = sci
        .split_once('e')
        .expect("LowerExp always has an exponent");
    let exp: isize = exp.parse().expect("exponent is an integer");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let (digits, exp) = break_tie_to_even(value.abs(), digits, exp);
    let length = digits.len() as isize;
    let k = exp - (length - 1);
    let kk = length + k;
    let mut out = String::new();
    if value < 0.0 {
        out.push('-');
    }
    if 0 <= k && kk <= 16 {
        out.push_str(&digits);
        out.extend(std::iter::repeat_n('0', k as usize));
        out.push_str(".0");
    } else if 0 < kk && kk <= 16 {
        out.push_str(&digits[..kk as usize]);
        out.push('.');
        out.push_str(&digits[kk as usize..]);
    } else if -5 < kk && kk <= 0 {
        out.push_str("0.");
        out.extend(std::iter::repeat_n('0', (-kk) as usize));
        out.push_str(&digits);
    } else if length == 1 {
        out.push_str(&digits);
        out.push('e');
        out.push_str(&signed(kk - 1));
    } else {
        out.push_str(&digits[..1]);
        out.push('.');
        out.push_str(&digits[1..]);
        out.push('e');
        out.push_str(&signed(kk - 1));
    }
    out
}

/// The standard library rounds a tie between two equally short candidates up; the layout the
/// document model has always had picks the even last digit. A tie needs the exact value to have
/// one digit more than the shortest text and that digit to be a 5, so the exact value is short
/// enough to compute in 128 bits.
fn break_tie_to_even(value: f64, digits: String, exp: isize) -> (String, isize) {
    let bits = value.to_bits();
    let biased = ((bits >> 52) & 0x7ff) as i32;
    let fraction = bits & ((1u64 << 52) - 1);
    let (mantissa, exponent) = if biased == 0 {
        (fraction, -1074)
    } else {
        (fraction | (1 << 52), biased - 1075)
    };
    if exponent >= 0 || -exponent > 32 {
        return (digits, exp);
    }
    // value = mantissa * 5^k / 10^k with k = -exponent.
    let exact = u128::from(mantissa) * 5u128.pow((-exponent) as u32);
    let text = exact.to_string();
    let trimmed = text.trim_end_matches('0');
    let len = digits.len();
    if trimmed.len() != len + 1 || !trimmed.ends_with('5') {
        return (digits, exp);
    }
    // Decimal exponent of the exact value's leading digit.
    let lead_exp = text.len() as isize - 1 - (-exponent) as isize;
    if lead_exp != exp {
        return (digits, exp);
    }
    let floor: u128 = trimmed[..len].parse().expect("digits");
    let pick = if floor.is_multiple_of(2) {
        floor
    } else {
        floor + 1
    };
    let candidate = pick.to_string();
    if candidate.len() != len {
        return (digits, exp);
    }
    let reads_back = format!("{}.{}e{exp}", &candidate[..1], &candidate[1..])
        .parse::<f64>()
        .ok()
        == Some(value);
    if reads_back {
        (candidate, exp)
    } else {
        (digits, exp)
    }
}

fn signed(exponent: isize) -> String {
    if exponent < 0 {
        exponent.to_string()
    } else {
        format!("+{exponent}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats_print_as_the_shortest_round_trip_text() {
        for (value, text) in [
            (0.0, "0.0"),
            (-0.0, "-0.0"),
            (1.0, "1.0"),
            (3.0, "3.0"),
            (0.1, "0.1"),
            (-1.5, "-1.5"),
            (123456.789, "123456.789"),
            (1e15, "1000000000000000.0"),
            (1e16, "1e+16"),
            (1.5e16, "1.5e+16"),
            (123456789012345680.0, "1.2345678901234568e+17"),
            (0.0001, "0.0001"),
            (0.00001, "0.00001"),
            (0.000001, "1e-6"),
            (0.000123, "0.000123"),
            (1.234e-7, "1.234e-7"),
            (f64::MAX, "1.7976931348623157e+308"),
            (f64::MIN_POSITIVE, "2.2250738585072014e-308"),
            (5e-324, "5e-324"),
        ] {
            assert_eq!(format_f64(value), text, "{value:e}");
            assert_eq!(text.parse::<f64>().unwrap(), value);
        }
    }

    #[test]
    fn numbers_keep_their_text_and_convert_exactly() {
        let n = Number::from_valid_text("12345678901234567890123");
        assert_eq!(n.as_str(), "12345678901234567890123");
        assert_eq!(
            (n.as_u64(), n.as_i64(), n.as_u128()),
            (None, None, Some(12345678901234567890123))
        );
        assert!(!n.is_f64());
        let n = Number::from_valid_text("1.10");
        assert_eq!(
            (n.as_f64(), n.is_f64(), n.as_u64()),
            (Some(1.1), true, None)
        );
        assert_eq!(n.to_string(), "1.10");
        assert_eq!(Number::from_valid_text("1e999").as_f64(), None);
        assert_eq!(Number::from(-7i32).to_string(), "-7");
        assert_eq!(Number::from_f64(f64::NAN), None);
        assert_eq!(Number::from_f64(2.5).unwrap().as_str(), "2.5");
    }
}
